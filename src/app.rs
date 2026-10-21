//! Ciclo de vida do app: instância única, servidor local e navegador.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;

use crate::capture::{AudioSource, CpalSource, Limits, Recorder};
use crate::config::Paths;
use crate::dialog::{SaveDialog, SystemDialog};
use crate::engine::Engine;
use crate::instance::Instance;
use crate::jobs::{Jobs, Pipeline};
use crate::logs;
use crate::opener::{Opener, SystemOpener};
use crate::web::{self, AppState, Desktop, SHUTDOWN_GRACE};

pub struct Options {
    pub port: u16,
    pub open_browser: bool,
    pub paths: Paths,
    /// O pipeline falso no lugar do real (E2E).
    #[cfg(feature = "test-support")]
    pub fake_pipeline: bool,
    /// O áudio falso (`ScriptedSource`) no lugar do microfone e do som do PC (E2E).
    #[cfg(feature = "test-support")]
    pub fake_audio: bool,
}

/// Sobe o servidor e bloqueia até Ctrl+C. Se outra instância já responde, só reabre o
/// navegador nela. Na instância que serve, o stdout e o stderr vão também para o
/// `scriba.log`; um erro é registrado lá (e, sem ninguém vendo o stderr, mostrado numa
/// janela) antes de voltar.
pub fn run(opts: Options) -> ExitCode {
    if let Some(existing) = Instance::read(&opts.paths.instance_file()).filter(Instance::is_alive) {
        announce(
            "Scriba já está aberto em",
            &existing.url(),
            opts.open_browser,
        );
        return ExitCode::SUCCESS;
    }
    let visible = logs::stderr_visible();
    let logs_dir = opts.paths.logs_dir();
    let logs = logs::init(&logs_dir)
        .inspect_err(|e| eprintln!("não consegui abrir o log em {}: {e}", logs_dir.display()))
        .ok();
    let result = serve(opts);
    if let Err(e) = &result {
        eprintln!("erro: {e:#}");
    }
    drop(logs);
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

fn serve(opts: Options) -> anyhow::Result<()> {
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
    announce("Scriba em", &instance.url(), opts.open_browser);

    let ctrl_c = {
        let state = state.clone();
        async move {
            let _ = tokio::signal::ctrl_c().await;
            // O SSE dos medidores termina já, sem esperar o prazo do encerramento.
            state.close();
        }
    };
    let result = tokio::runtime::Runtime::new()?.block_on(web::serve(
        listener,
        state,
        ctrl_c,
        SHUTDOWN_GRACE,
    ));
    // Fecha os WAVs antes do worker: a gravação volta na fila na próxima abertura.
    if let Some(rec) = recorder.finish_on_exit() {
        eprintln!(
            "gravação guardada para a próxima abertura: {}",
            rec.dir.display()
        );
    }
    // Também descarta os envios que esperavam a resposta a "Guardar o áudio?".
    if !worker.shutdown(SHUTDOWN_GRACE) {
        eprintln!("o job em andamento não parou a tempo; encerrando mesmo assim");
    }
    instance.remove(&file);
    eprintln!("Scriba encerrado");
    result.context("servidor")
}

/// A janela do erro que impediu o app de abrir, quando ninguém vê o stderr (o exe aberto
/// pelo Explorer).
fn fatal_dialog(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("Scriba"));
    // SAFETY: as duas strings terminam em zero e vivem durante a chamada.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
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
