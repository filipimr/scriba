//! Ciclo de vida do app: instância única, servidor local, bandeja e navegador.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::Context;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IDOK, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_OKCANCEL, MB_SETFOREGROUND,
    MB_TOPMOST, MESSAGEBOX_RESULT, MESSAGEBOX_STYLE, MessageBoxW,
};

use crate::capture::{AudioSource, CpalSource, Limits, Recorder, Status};
use crate::cleanup;
use crate::clock::SystemClock;
use crate::config::{Config, Paths};
use crate::dialog::{SaveDialog, SystemDialog};
use crate::engine::Engine;
use crate::instance::{self, Claim, Instance};
use crate::jobs::history;
use crate::jobs::{Jobs, Pipeline};
use crate::logs;
use crate::opener::{Opener, SystemOpener};
use crate::shutdown::{Reason, Shutdown};
use crate::tray::{self, Desk, Pending, Tray};
use crate::web::{self, AppState, Desktop, SHUTDOWN_GRACE};

/// Quanto a segunda execução espera a primeira responder (ela pode estar abrindo agora).
const SECOND_RUN_WAIT: Duration = Duration::from_secs(10);

pub struct Options {
    pub port: u16,
    pub open_browser: bool,
    /// O ícone na bandeja (`--no-tray` o tira, para o E2E; a janela oculta continua).
    pub tray: bool,
    pub paths: Paths,
    /// O pipeline falso no lugar do real (E2E).
    #[cfg(feature = "test-support")]
    pub fake_pipeline: bool,
    /// O áudio falso (`ScriptedSource`) no lugar do microfone e do som do PC (E2E).
    #[cfg(feature = "test-support")]
    pub fake_audio: bool,
}

/// Sobe o servidor e bloqueia até o pedido de encerramento (Ctrl+C, "Sair" na bandeja, fim
/// da sessão do Windows ou `POST /api/app/sair`). Se outra execução já serve a mesma pasta
/// de dados, só abre o navegador nela. Na instância que serve, o stdout e o stderr vão
/// também para o `scriba.log`; um erro é registrado lá (e, sem ninguém vendo o stderr,
/// mostrado numa janela) antes de voltar.
pub fn run(opts: Options) -> ExitCode {
    let visible = logs::stderr_visible();
    let _lock = match instance::claim(&opts.paths.data_dir) {
        Ok(Claim::Mine(lock)) => Some(lock),
        Ok(Claim::Taken) => return second_run(&opts, visible),
        Err(e) => {
            eprintln!("não consegui criar o mutex da instância única: {e}");
            None
        }
    };
    let logs_dir = opts.paths.logs_dir();
    let logs = logs::init(&logs_dir)
        .inspect_err(|e| eprintln!("não consegui abrir o log em {}: {e}", logs_dir.display()))
        .ok();
    let mut ending = None;
    let result = serve(opts, &mut ending);
    if let Err(e) = &result {
        eprintln!("erro: {e:#}");
    }
    drop(logs);
    // Só agora o `WM_ENDSESSION` volta e o Windows pode encerrar o processo.
    if let Some(Ending { shutdown, tray }) = ending {
        shutdown.finish();
        if let Some(tray) = tray {
            tray.stop();
        }
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if !visible {
                fatal_dialog(&format!(
                    "Não consegui abrir o Scriba: {e:#}\n\nDetalhes em {}.",
                    logs_dir.display()
                ));
            }
            ExitCode::FAILURE
        }
    }
}

/// Outra execução tem o mutex desta pasta de dados: espera ela responder e abre o navegador
/// nela.
fn second_run(opts: &Options, visible: bool) -> ExitCode {
    match instance::wait_alive(&opts.paths.instance_file(), SECOND_RUN_WAIT) {
        Some(existing) => {
            announce(
                "Scriba já está aberto em",
                &existing.url(),
                opts.open_browser,
            );
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("{NOT_RESPONDING}");
            if !visible {
                message_box(NOT_RESPONDING, MB_OK | MB_ICONERROR);
            }
            ExitCode::FAILURE
        }
    }
}

const NOT_RESPONDING: &str = "O Scriba já está aberto, mas não respondeu.";

/// O que precisa fechar depois dos logs: o fim do `WM_ENDSESSION` e a bandeja.
struct Ending {
    shutdown: Shutdown,
    tray: Option<Tray>,
}

fn serve(opts: Options, ending: &mut Option<Ending>) -> anyhow::Result<()> {
    let file = opts.paths.instance_file();
    std::fs::create_dir_all(&opts.paths.data_dir)
        .with_context(|| format!("criando {}", opts.paths.data_dir.display()))?;

    let listener = web::bind(opts.port).context("abrindo a porta local")?;
    // Só depois de confirmar que esta é a única instância: a fila esvazia `uploads/`.
    let (jobs, worker) = Jobs::start(opts.paths.uploads_dir(), pipeline(&opts))
        .context("preparando a pasta de uploads")?;
    eprintln!(
        "Scriba {} · {} · dados em {} · saídas em {} · {}",
        env!("CARGO_PKG_VERSION"),
        logs::windows_version(),
        opts.paths.data_dir.display(),
        opts.paths.output_dir.display(),
        jobs.backend()
            .unwrap_or_else(|| "sem modelo de transcrição".into())
    );
    restore_history(&jobs, &opts.paths);
    resume_recordings(&jobs, &opts.paths.recordings_dir());
    let recorder = Recorder::new(
        audio_source(&opts),
        opts.paths.recordings_dir(),
        Limits::default(),
    );
    {
        // Cada gravação terminada (menos a do Ctrl+C, que volta na próxima abertura) vai
        // para a fila.
        let jobs = jobs.clone();
        recorder.on_finish(move |rec| {
            jobs.submit_recording(rec);
        });
    }
    let instance = Instance::new(listener.local_addr()?.port());
    instance.write(&file)?;
    let opener = opener(&opts);
    let dialog = dialog(&opts);
    let tray_enabled = opts.tray;
    let state = AppState::new(
        instance.token.clone(),
        instance.id.clone(),
        instance.port,
        opts.paths,
        jobs,
        Desktop {
            opener,
            dialog,
            recorder: recorder.clone(),
        },
    );
    let shutdown = state.shutdown.clone();
    *ending = Some(Ending {
        shutdown: shutdown.clone(),
        tray: start_tray(tray_enabled, &state, &instance.url()),
    });
    announce("Scriba em", &instance.url(), opts.open_browser);

    let runtime = tokio::runtime::Runtime::new()?;
    let result = runtime.block_on(async {
        {
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                // Sem console (a release aberta pelo Explorer), nunca dispara.
                if tokio::signal::ctrl_c().await.is_ok() {
                    shutdown.request(Reason::CtrlC);
                }
            });
        }
        {
            // O zelador: o áudio sem resposta há 24 h é descartado.
            let jobs = state.jobs.clone();
            tokio::spawn(async move {
                let mut every = tokio::time::interval(JANITOR_EVERY);
                loop {
                    every.tick().await;
                    jobs.expire_audio();
                }
            });
        }
        let stop = {
            let (state, recorder) = (state.clone(), recorder.clone());
            async move {
                let reason = shutdown.requested().await;
                eprintln!("encerrando: {}", reason.label());
                // O SSE dos medidores termina já, sem esperar o prazo do encerramento.
                state.close();
                if reason != Reason::EndSession {
                    return SHUTDOWN_GRACE;
                }
                // No logoff, o Windows pode encerrar o processo a qualquer momento: os WAVs
                // fecham antes de tudo, e o servidor não espera as requisições.
                let _ = tokio::task::spawn_blocking(move || keep(recorder.finish_on_exit())).await;
                Duration::ZERO
            }
        };
        web::serve_until(listener, state, stop).await
    });
    drop(runtime);
    // Fecha os WAVs antes do worker: a gravação volta na fila na próxima abertura.
    keep(recorder.finish_on_exit());
    // Também descarta os envios que esperavam a resposta a "Guardar o áudio?".
    if !worker.shutdown(SHUTDOWN_GRACE) {
        eprintln!("o job em andamento não parou a tempo; encerrando mesmo assim");
    }
    instance.remove(&file);
    eprintln!("Scriba encerrado");
    result.context("servidor")
}

/// A gravação que o encerramento fechou volta na fila na próxima abertura.
fn keep(rec: Option<crate::capture::Recording>) {
    if let Some(rec) = rec {
        eprintln!(
            "gravação guardada para a próxima abertura: {}",
            rec.dir.display()
        );
    }
}

/// A janela oculta e o ícone na bandeja, e a thread que trata os cliques. Sem bandeja (sem
/// Explorer), o app segue: o erro só vai para o log.
fn start_tray(icon: bool, state: &AppState, url: &str) -> Option<Tray> {
    let (events, received) = mpsc::channel();
    let recording = {
        let recorder = state.recorder.clone();
        Arc::new(move || matches!(recorder.status(), Status::Recording { .. }))
    };
    let tray = Tray::start(tray::Options {
        icon,
        events,
        recording,
        shutdown: state.shutdown.clone(),
    })
    .inspect_err(|e| eprintln!("bandeja indisponível: {e}"))
    .ok()?;
    let state = state.clone();
    let desk = SystemDesk { url: url.into() };
    // A confirmação do "Sair" bloqueia esta thread, não a da janela. Ela termina quando a
    // janela fecha (o canal fecha); se a confirmação estiver aberta, o fim do processo a leva.
    let spawned = std::thread::Builder::new()
        .name("scriba-bandeja-acoes".into())
        .spawn(move || {
            let pending = || Pending::of(&state.recorder.status(), &state.jobs.list());
            tray::run(received, &desk, &pending, &state.shutdown);
        });
    if let Err(e) = spawned {
        eprintln!("bandeja indisponível: {e}");
    }
    Some(tray)
}

/// O navegador e a `MessageBox` de verdade.
struct SystemDesk {
    url: String,
}

impl Desk for SystemDesk {
    fn open(&self) {
        if let Err(e) = open::that_detached(&self.url) {
            eprintln!("não consegui abrir o navegador: {e}");
        }
    }

    fn confirm(&self, title: &str, text: &str) -> bool {
        let text = format!("{text}\n\n{}", tray::QUIT_QUESTION);
        message_box_titled(
            title,
            &text,
            MB_OKCANCEL | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST,
        ) == IDOK
    }
}

/// A janela do erro que impediu o app de abrir, quando ninguém vê o stderr (o exe aberto
/// pelo Explorer).
fn fatal_dialog(text: &str) {
    message_box(text, MB_OK | MB_ICONERROR);
}

fn message_box(text: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    message_box_titled("Scriba", text, style)
}

fn message_box_titled(title: &str, text: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide(title));
    // SAFETY: as duas strings terminam em zero e vivem durante a chamada.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), style) }
}

/// De quanto em quanto tempo o zelador procura o áudio sem resposta há 24 h.
const JANITOR_EVERY: Duration = Duration::from_secs(10 * 60);

/// As transcrições de `output_dir` voltam à lista (as 50 mais recentes), e os temporários
/// antigos são apagados: os do `--self-test` e os `.m4a.parte` nas pastas do histórico e na
/// última pasta do "Guardar áudio".
pub fn restore_history(jobs: &Jobs, paths: &Paths) {
    let history = history::load(&paths.output_dir, history::LIMIT);
    eprintln!(
        "histórico: {} transcrição(ões) na lista{}",
        history.jobs.len(),
        if history.older {
            ", e mais antigas na pasta"
        } else {
            ""
        }
    );
    let mut audio_dirs = history.dirs;
    audio_dirs.extend(Config::load(&paths.config_file()).audio_dir);
    jobs.restore(history.jobs, history.older);
    cleanup::run(&SystemClock, &std::env::temp_dir(), &audio_dirs);
}

/// As gravações que a execução anterior não transcreveu (Ctrl+C, crash) voltam à fila, da
/// mais antiga para a mais nova; as já transcritas, vazias ou ilegíveis são apagadas.
/// Devolve quantas voltaram.
pub fn resume_recordings(jobs: &Jobs, root: &Path) -> usize {
    let found = crate::capture::recover(root);
    let count = found.len();
    for rec in found {
        eprintln!("gravação retomada: {}", rec.dir.display());
        jobs.submit_recording(rec);
    }
    count
}

/// O `Engine` detecta as GPUs agora, antes de o servidor responder.
fn pipeline(opts: &Options) -> Arc<dyn Pipeline> {
    let output_dir = opts.paths.output_dir.clone();
    #[cfg(feature = "test-support")]
    if opts.fake_pipeline {
        return Arc::new(crate::jobs::FakePipeline::new(output_dir));
    }
    Arc::new(Engine::new(output_dir))
}

/// O microfone e o som do PC pelo cpal; com `--fake-audio`, os tons do `ScriptedSource`.
fn audio_source(opts: &Options) -> Arc<dyn AudioSource> {
    #[cfg(feature = "test-support")]
    if opts.fake_audio {
        return Arc::new(crate::capture::ScriptedSource::new(&opts.paths.data_dir));
    }
    let _ = opts;
    Arc::new(CpalSource::new())
}

/// Com o pipeline falso, "Abrir pasta" só imprime a pasta (o E2E lê o stdout).
fn opener(opts: &Options) -> Arc<dyn Opener> {
    #[cfg(feature = "test-support")]
    if opts.fake_pipeline {
        return Arc::new(crate::opener::PrintOpener);
    }
    let _ = opts;
    Arc::new(SystemOpener)
}

/// Com o pipeline falso, o "Salvar como" responde o que o E2E escreveu em
/// `<data-dir>/salvar-como.txt`.
fn dialog(opts: &Options) -> Arc<dyn SaveDialog> {
    #[cfg(feature = "test-support")]
    if opts.fake_pipeline {
        return Arc::new(crate::dialog::ScriptedDialog::new(&opts.paths.data_dir));
    }
    let _ = opts;
    Arc::new(SystemDialog)
}

fn announce(prefix: &str, url: &str, open_browser: bool) {
    println!("{prefix} {url}");
    let _ = std::io::stdout().flush();
    if open_browser && let Err(e) = open::that_detached(url) {
        eprintln!("não consegui abrir o navegador: {e}");
    }
}
