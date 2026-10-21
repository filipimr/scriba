//! Integração da fila: store e worker reais, com pipelines de teste que travam, falham ou
//! entram em pânico, e as gravações (a pasta das trilhas sai junto com o job).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use scriba::capture::{End, Recording, Track, storage};
use scriba::clock::LocalTime;
use scriba::jobs::{
    Audio, AudioError, Ctx, DIALOG_ERROR, Estimate, Failure, Job, JobId, Jobs, Media, Outputs,
    PANIC_MESSAGE, Pipeline, Probe, RECORDING_PREFIX, RemoveError, RetryError, State, part_path,
};

/// Comporta-se pelo nome do arquivo: `panico` entra em pânico, `espera` roda até ser
/// cancelado, `falha-uma-vez` falha na primeira tentativa; o resto termina na hora e cria a
/// pasta da saída em `out`. Conta quantos jobs começou.
struct TestPipeline {
    out: PathBuf,
    runs: AtomicUsize,
    failed: Mutex<HashSet<JobId>>,
}

impl Pipeline for TestPipeline {
    fn probe(&self, _: &str, _: &Path) -> Result<Probe, Failure> {
        unreachable!("a fila não faz o probe")
    }

    fn backend(&self) -> Option<String> {
        None
    }

    /// 1 s por segundo de áudio.
    fn estimate(&self, duration_ms: u64) -> Option<Estimate> {
        Some(Estimate {
            secs: duration_ms / 1_000,
            gpu: false,
        })
    }

    fn run(&self, job: &Job, ctx: &Ctx) -> Result<Outputs, Failure> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if job.name.contains("panico") {
            panic!("pânico de teste");
        }
        while job.name.contains("espera") {
            ctx.check()?;
            thread::sleep(Duration::from_millis(5));
        }
        if job.name.contains("falha-uma-vez") && self.failed.lock().unwrap().insert(job.id) {
            return Err(Failure::Error("Falhou desta vez.".into()));
        }
        let dir = self.out.join(job.id.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        Ok(Outputs {
            dir,
            ..Outputs::default()
        })
    }

    /// Grava "áudio de <nome>" em `output` aos poucos (10 passos de 5 ms). `audio-sem-fim`
    /// grava até ser cancelado; `audio-ruim` falha; `audio-quebra` entra em pânico.
    fn save_audio(
        &self,
        job: &Job,
        output: &Path,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<(), Failure> {
        std::fs::write(output, b"parcial").unwrap();
        if job.name.contains("audio-quebra") {
            panic!("pânico de teste no áudio");
        }
        let mut step = 0;
        while step <= 10 || job.name.contains("audio-sem-fim") {
            if cancel.load(Ordering::SeqCst) {
                std::fs::remove_file(output).unwrap();
                return Err(Failure::Canceled);
            }
            if step == 5 && job.name.contains("audio-ruim") {
                std::fs::remove_file(output).unwrap();
                return Err(Failure::Error("Disco cheio.".into()));
            }
            progress((step * 10).min(100) as u8);
            step += 1;
            thread::sleep(Duration::from_millis(5));
        }
        std::fs::write(output, format!("áudio de {}", job.name)).unwrap();
        Ok(())
    }
}

fn start(dir: &Path) -> (Jobs, scriba::jobs::Worker, Arc<TestPipeline>) {
    let pipeline = Arc::new(TestPipeline {
        out: dir.join("saidas"),
        runs: AtomicUsize::new(0),
        failed: Mutex::default(),
    });
    let (jobs, worker) = Jobs::start(dir.join("uploads"), pipeline.clone()).unwrap();
    (jobs, worker, pipeline)
}

/// Grava um arquivo em `uploads` e põe o job na fila.
fn submit(jobs: &Jobs, name: &str) -> JobId {
    let id = JobId::generate();
    let input = jobs.upload_path(id, "wav");
    std::fs::write(&input, b"audio").unwrap();
    jobs.submit(Job {
        id,
        name: name.into(),
        media: Media::Audio,
        size: 5,
        duration_ms: 1_000,
        created: LocalTime::now(),
        estimate: None,
        input,
        state: State::Queued,
    });
    id
}

fn wait_for(jobs: &Jobs, id: JobId, what: &str, done: impl Fn(&State) -> bool) -> Job {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let job = jobs.get(id).expect("job existe");
        if done(&job.state) {
            return job;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: ficou em {:?}",
            job.state
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn running(s: &State) -> bool {
    matches!(s, State::Running { .. })
}

fn done(s: &State) -> bool {
    matches!(s, State::Done { .. })
}

#[test]
fn panic_vira_falhou_e_a_fila_segue() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    let bad = submit(&jobs, "panico.wav");
    let good = submit(&jobs, "reuniao.wav");

    let job = wait_for(&jobs, bad, "pânico", State::is_terminal);
    assert_eq!(
        job.state,
        State::Failed {
            message: PANIC_MESSAGE.into()
        }
    );
    let next = wait_for(&jobs, good, "job seguinte", State::is_terminal);
    assert!(done(&next.state), "{:?}", next.state);
    assert!(job.input.exists(), "o que falhou guarda o envio");
    assert!(
        next.input.exists(),
        "o concluído guarda o envio até a resposta a \"Guardar o áudio?\""
    );
    assert!(worker.shutdown(Duration::from_secs(5)));
    assert!(!job.input.exists() && !next.input.exists(), "fechar apaga");
}

#[test]
fn tentar_de_novo_reaproveita_o_envio() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, pipeline) = start(dir.path());
    let id = submit(&jobs, "falha-uma-vez.wav");
    let failed = wait_for(&jobs, id, "falha", State::is_terminal);
    assert_eq!(
        failed.state,
        State::Failed {
            message: "Falhou desta vez.".into()
        }
    );
    assert!(failed.input.exists());

    let job = jobs.retry(id).unwrap();
    assert!(
        matches!(job.state, State::Queued | State::Running { .. }),
        "{:?}",
        job.state
    );
    let job = wait_for(&jobs, id, "nova tentativa", State::is_terminal);
    assert!(done(&job.state), "{:?}", job.state);
    assert!(
        job.input.exists(),
        "concluído guarda o envio até a resposta"
    );
    assert_eq!(pipeline.runs.load(Ordering::SeqCst), 2);

    assert_eq!(jobs.retry(id).unwrap_err(), RetryError::NotFailed);
    assert_eq!(
        jobs.retry(JobId::generate()).unwrap_err(),
        RetryError::NotFound
    );
    assert!(worker.shutdown(Duration::from_secs(5)));
}

#[test]
fn cancelamento_pedido_antes_nao_vale_para_a_nova_tentativa() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    let id = submit(&jobs, "falha-uma-vez.wav");
    wait_for(&jobs, id, "falha", State::is_terminal);
    // Cancelar o que já falhou não muda o estado nem apaga o envio.
    let job = jobs.cancel(id).unwrap();
    assert!(matches!(job.state, State::Failed { .. }));
    assert!(job.input.exists());

    jobs.retry(id).unwrap();
    let job = wait_for(&jobs, id, "nova tentativa", State::is_terminal);
    assert!(done(&job.state), "{:?}", job.state);
    assert!(worker.shutdown(Duration::from_secs(5)));
}

#[test]
fn cancelar_job_rodando_para_o_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    let id = submit(&jobs, "espera.wav");
    wait_for(&jobs, id, "início", running);

    let job = jobs.cancel(id).unwrap();
    assert!(
        matches!(
            job.state,
            State::Running {
                canceling: true,
                ..
            }
        ),
        "{:?}",
        job.state
    );
    let job = wait_for(&jobs, id, "cancelamento", State::is_terminal);
    assert_eq!(job.state, State::Canceled);
    assert!(!job.input.exists());
    assert!(worker.shutdown(Duration::from_secs(5)));
}

#[test]
fn cancelado_na_fila_nunca_roda() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, pipeline) = start(dir.path());
    let first = submit(&jobs, "espera.wav");
    let second = submit(&jobs, "reuniao.wav");
    wait_for(&jobs, first, "início", running);

    let job = jobs.cancel(second).unwrap();
    assert_eq!(job.state, State::Canceled, "na fila, cancela na hora");
    assert!(!job.input.exists());
    jobs.cancel(first);
    wait_for(&jobs, first, "fim", State::is_terminal);
    assert!(worker.shutdown(Duration::from_secs(5)));
    assert_eq!(pipeline.runs.load(Ordering::SeqCst), 1);
}

#[test]
fn remover_so_depois_de_terminar() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    let id = submit(&jobs, "espera.wav");
    wait_for(&jobs, id, "início", running);
    assert_eq!(jobs.remove(id, true), Err(RemoveError::Active));

    jobs.cancel(id);
    wait_for(&jobs, id, "fim", State::is_terminal);
    assert_eq!(
        jobs.remove(id, false),
        Ok(()),
        "cancelado não pede confirmação"
    );
    assert!(jobs.get(id).is_none() && jobs.list().is_empty());
    assert_eq!(jobs.remove(id, false), Err(RemoveError::NotFound));
    assert!(worker.shutdown(Duration::from_secs(5)));
}

#[test]
fn remover_o_que_falhou_apaga_o_envio() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    let id = submit(&jobs, "falha-uma-vez.wav");
    let job = wait_for(&jobs, id, "falha", State::is_terminal);
    assert!(job.input.exists());
    assert_eq!(jobs.remove(id, false), Ok(()));
    assert!(!job.input.exists());
    assert!(worker.shutdown(Duration::from_secs(5)));
}

#[test]
fn remover_concluido_pede_confirmacao_e_manda_a_pasta_para_a_lixeira() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    let id = submit(&jobs, "reuniao.wav");
    let job = wait_for(&jobs, id, "fim", State::is_terminal);
    let State::Done { outputs, .. } = job.state else {
        panic!("{:?}", job.state);
    };
    assert!(outputs.dir.is_dir());

    assert_eq!(jobs.remove(id, false), Err(RemoveError::NeedsConfirm));
    assert!(outputs.dir.is_dir() && jobs.get(id).is_some());
    assert_eq!(jobs.remove(id, true), Ok(()));
    assert!(!outputs.dir.exists());
    assert!(jobs.get(id).is_none());

    // Pasta apagada fora do app: o job sai mesmo assim.
    let id = submit(&jobs, "outra.wav");
    let job = wait_for(&jobs, id, "fim", State::is_terminal);
    let State::Done { outputs, .. } = job.state else {
        panic!("{:?}", job.state);
    };
    std::fs::remove_dir(&outputs.dir).unwrap();
    assert_eq!(jobs.remove(id, true), Ok(()));
    assert!(worker.shutdown(Duration::from_secs(5)));
}

#[test]
fn encerrar_interrompe_o_job_ativo() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, pipeline) = start(dir.path());
    let id = submit(&jobs, "espera.wav");
    let queued = submit(&jobs, "reuniao.wav");
    wait_for(&jobs, id, "início", running);

    assert!(worker.shutdown(Duration::from_secs(5)));
    assert_eq!(jobs.get(id).unwrap().state, State::Canceled);
    assert_eq!(jobs.get(queued).unwrap().state, State::Queued, "não começa");
    assert_eq!(pipeline.runs.load(Ordering::SeqCst), 1);
}

#[test]
fn lista_do_mais_recente_e_limpa_uploads_antigos() {
    let dir = tempfile::tempdir().unwrap();
    let uploads = dir.path().join("uploads");
    std::fs::create_dir_all(&uploads).unwrap();
    std::fs::write(uploads.join("sobra.mp4"), b"x").unwrap();

    let pipeline = Arc::new(TestPipeline {
        out: dir.path().join("saidas"),
        runs: AtomicUsize::new(0),
        failed: Mutex::default(),
    });
    let jobs = Jobs::without_worker(uploads.clone(), pipeline).unwrap();
    assert!(!uploads.join("sobra.mp4").exists());
    let a = submit(&jobs, "a.wav");
    let b = submit(&jobs, "b.wav");
    let ids: Vec<JobId> = jobs.list().iter().map(|j| j.id).collect();
    assert_eq!(ids, [b, a]);
    assert_eq!(jobs.get(a).unwrap().state, State::Queued, "sem worker");
}

/// A resposta a "Guardar o áudio?" do job concluído.
fn audio(job: &Job) -> Audio {
    match &job.state {
        State::Done { audio, .. } => audio.clone(),
        other => panic!("não concluiu: {other:?}"),
    }
}

fn wait_audio(jobs: &Jobs, id: JobId, what: &str, done: impl Fn(&Audio) -> bool) -> Job {
    wait_for(
        jobs,
        id,
        what,
        |s| matches!(s, State::Done { audio, .. } if done(audio)),
    )
}

/// Um job concluído, com o envio ainda em `uploads/`.
fn finished(jobs: &Jobs, name: &str) -> Job {
    let id = submit(jobs, name);
    let job = wait_for(jobs, id, "fim", State::is_terminal);
    assert_eq!(audio(&job), Audio::PENDING);
    assert!(job.input.exists());
    job
}

#[test]
fn descartar_apaga_o_envio_e_e_definitivo() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    let job = finished(&jobs, "reuniao.wav");
    let after = jobs.discard_audio(job.id).unwrap();
    assert_eq!(audio(&after), Audio::Discarded);
    assert!(!job.input.exists());
    assert_eq!(
        jobs.discard_audio(job.id).unwrap_err(),
        AudioError::Answered
    );
    assert_eq!(
        jobs.save_audio(job.id, |_, _| unreachable!()).unwrap_err(),
        AudioError::Answered
    );
    assert_eq!(
        jobs.discard_audio(JobId::generate()).unwrap_err(),
        AudioError::NotFound
    );
    // Remover depois de descartar: só a pasta da transcrição vai para a Lixeira.
    assert_eq!(jobs.remove(job.id, true), Ok(()));

    let running = submit(&jobs, "espera.wav");
    wait_for(&jobs, running, "início", self::running);
    assert_eq!(
        jobs.discard_audio(running).unwrap_err(),
        AudioError::NotDone
    );
    jobs.cancel(running);
    assert!(worker.shutdown(Duration::from_secs(5)));
}

#[test]
fn guardar_grava_no_arquivo_escolhido_e_apaga_o_envio() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    let job = finished(&jobs, "reuniao.wav");
    let dest = dir.path().join("Áudios").join("Reunião.m4a");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    // Um arquivo com o mesmo nome: a janela já perguntou se pode substituir.
    std::fs::write(&dest, b"antigo").unwrap();

    let seen = Arc::new(Mutex::new(None));
    let (chosen, record) = (dest.clone(), seen.clone());
    let saving = jobs
        .save_audio(job.id, move |job, outputs| {
            *record.lock().unwrap() = Some((job.name.clone(), outputs.dir.clone()));
            Ok(Some(chosen))
        })
        .unwrap();
    assert_eq!(audio(&saving), Audio::Choosing);
    let job = wait_audio(&jobs, job.id, "guardado", Audio::answered);
    assert_eq!(audio(&job), Audio::Saved { path: dest.clone() });
    assert_eq!(
        std::fs::read_to_string(&dest).unwrap(),
        "áudio de reuniao.wav"
    );
    assert!(!part_path(&dest).exists());
    assert!(!job.input.exists(), "guardado apaga o envio");
    let State::Done { outputs, .. } = &job.state else {
        unreachable!()
    };
    assert_eq!(
        seen.lock().unwrap().clone(),
        Some(("reuniao.wav".into(), outputs.dir.clone())),
        "a janela recebe o job e a pasta da transcrição"
    );
    assert_eq!(
        jobs.discard_audio(job.id).unwrap_err(),
        AudioError::Answered
    );
    assert!(worker.shutdown(Duration::from_secs(5)));
    assert!(dest.exists(), "fechar o app não apaga o que foi guardado");
}

#[test]
fn janela_cancelada_ou_que_nao_abre_mantem_a_pergunta() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    let job = finished(&jobs, "reuniao.wav");

    // Enquanto a janela está aberta, nada mais vale.
    let (open, close) = std::sync::mpsc::channel::<Option<PathBuf>>();
    jobs.save_audio(job.id, move |_, _| Ok(close.recv().unwrap()))
        .unwrap();
    assert_eq!(
        jobs.save_audio(job.id, |_, _| unreachable!()).unwrap_err(),
        AudioError::Busy
    );
    assert_eq!(jobs.discard_audio(job.id).unwrap_err(), AudioError::Busy);
    assert_eq!(jobs.remove(job.id, true), Err(RemoveError::AudioBusy));
    assert_eq!(audio(&jobs.cancel_audio(job.id).unwrap()), Audio::Choosing);
    open.send(None).unwrap();
    let job = wait_audio(&jobs, job.id, "janela cancelada", |a| !a.busy());
    assert_eq!(audio(&job), Audio::PENDING);
    assert!(job.input.exists());

    jobs.save_audio(job.id, |_, _| Err(std::io::Error::other("sem COM")))
        .unwrap();
    let job = wait_audio(&jobs, job.id, "janela que não abre", |a| !a.busy());
    assert_eq!(
        audio(&job),
        Audio::Pending {
            error: Some(DIALOG_ERROR.into())
        }
    );
    assert!(job.input.exists());
    assert!(worker.shutdown(Duration::from_secs(5)));
}

#[test]
fn cancelar_a_gravacao_volta_a_pergunta_sem_sobras() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    let job = finished(&jobs, "audio-sem-fim.wav");
    let dest = dir.path().join("cancelado.m4a");
    let chosen = dest.clone();
    jobs.save_audio(job.id, move |_, _| Ok(Some(chosen)))
        .unwrap();
    wait_audio(
        &jobs,
        job.id,
        "gravando",
        |a| matches!(a, Audio::Saving { pct, .. } if *pct > 0),
    );
    assert_eq!(jobs.remove(job.id, true), Err(RemoveError::AudioBusy));
    let canceling = jobs.cancel_audio(job.id).unwrap();
    assert!(
        matches!(
            audio(&canceling),
            Audio::Saving {
                canceling: true,
                ..
            }
        ),
        "{:?}",
        canceling.state
    );
    let job = wait_audio(&jobs, job.id, "cancelado", |a| !a.busy());
    assert_eq!(audio(&job), Audio::PENDING);
    assert!(!dest.exists() && !part_path(&dest).exists());
    assert!(job.input.exists());
    assert!(worker.shutdown(Duration::from_secs(5)));
}

#[test]
fn falha_ou_panico_ao_gravar_volta_com_o_erro() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    for (name, message) in [
        ("audio-ruim.wav", "Disco cheio."),
        ("audio-quebra.wav", PANIC_MESSAGE),
    ] {
        let job = finished(&jobs, name);
        let dest = dir.path().join(format!("{name}.m4a"));
        let chosen = dest.clone();
        jobs.save_audio(job.id, move |_, _| Ok(Some(chosen)))
            .unwrap();
        let job = wait_audio(&jobs, job.id, name, |a| !a.busy());
        assert_eq!(
            audio(&job),
            Audio::Pending {
                error: Some(message.into())
            },
            "{name}"
        );
        assert!(!dest.exists() && !part_path(&dest).exists(), "{name}");
        assert!(job.input.exists(), "{name}");
    }
    assert!(worker.shutdown(Duration::from_secs(5)));
}

#[test]
fn encerrar_cancela_a_gravacao_e_apaga_os_envios() {
    let dir = tempfile::tempdir().unwrap();
    let (jobs, worker, _) = start(dir.path());
    let waiting = finished(&jobs, "reuniao.wav");
    let saving = finished(&jobs, "audio-sem-fim.wav");
    let dest = dir.path().join("sem-fim.m4a");
    let chosen = dest.clone();
    jobs.save_audio(saving.id, move |_, _| Ok(Some(chosen)))
        .unwrap();
    wait_audio(&jobs, saving.id, "gravando", |a| {
        matches!(a, Audio::Saving { .. })
    });

    assert!(worker.shutdown(Duration::from_secs(5)));
    assert_eq!(audio(&jobs.get(saving.id).unwrap()), Audio::PENDING);
    assert!(!dest.exists() && !part_path(&dest).exists());
    assert!(!waiting.input.exists() && !saving.input.exists());
    assert_eq!(
        std::fs::read_dir(jobs.uploads_dir()).unwrap().count(),
        0,
        "uploads/ vazia"
    );
}

/// Uma gravação terminada em `root`, como o `Recorder` deixa: as duas trilhas com 1 s e o
/// `gravacao.json` parado.
fn recording(root: &Path) -> Recording {
    let id = JobId::generate();
    let dir = storage::create(root, id).unwrap();
    storage::write_manifest(
        &dir,
        &storage::Manifest {
            version: storage::MANIFEST_VERSION,
            id: id.to_string(),
            created: LocalTime::now(),
            mic: "Microfone".into(),
            output: "Alto-falantes".into(),
            state: storage::RecState::Parada,
        },
    )
    .unwrap();
    for track in Track::ALL {
        let mut w = hound::WavWriter::create(dir.join(track.file()), storage::wav_spec()).unwrap();
        for i in 0..32_000 {
            w.write_sample((i % 100) as i16).unwrap();
        }
        w.finalize().unwrap();
    }
    storage::load(&dir, End::User).unwrap()
}

/// Põe a gravação na fila com outro nome (os ganchos do `TestPipeline` vêm do nome).
fn submit_recording(jobs: &Jobs, rec: Recording, name: &str) -> Job {
    let job = Job {
        name: name.into(),
        ..Job::recording(rec, None)
    };
    jobs.submit(job.clone());
    job
}

fn transcribed(dir: &Path) -> bool {
    storage::read_manifest(dir).is_some_and(|m| m.state == storage::RecState::Transcrita)
}

/// Descartar, guardar e remover uma gravação apagam a pasta dela inteira; concluir a marca
/// como transcrita.
#[test]
fn respostas_da_gravacao_apagam_a_pasta() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("gravacoes");
    let (jobs, worker, _) = start(dir.path());

    let ids: Vec<JobId> = (0..3)
        .map(|_| jobs.submit_recording(recording(&root)))
        .collect();
    let finished: Vec<Job> = ids
        .iter()
        .map(|&id| wait_for(&jobs, id, "gravação concluída", done))
        .collect();
    for job in &finished {
        assert_eq!(job.media, Media::Recording);
        assert!(job.name.starts_with(RECORDING_PREFIX), "{}", job.name);
        assert!(job.input.is_dir() && transcribed(&job.input), "{job:?}");
    }

    jobs.discard_audio(finished[0].id).unwrap();
    assert!(!finished[0].input.exists(), "descartar apaga a pasta");

    let dest = dir.path().join("gravacao.m4a");
    let chosen = dest.clone();
    jobs.save_audio(finished[1].id, move |_, _| Ok(Some(chosen)))
        .unwrap();
    wait_audio(&jobs, finished[1].id, "guardado", Audio::answered);
    assert!(dest.exists());
    assert!(!finished[1].input.exists(), "guardar apaga a pasta");

    jobs.remove(finished[2].id, true).unwrap();
    assert!(!finished[2].input.exists(), "remover apaga a pasta");
    assert!(worker.shutdown(Duration::from_secs(5)));
}

/// Cancelar a transcrição de uma gravação descarta as trilhas, na fila ou rodando.
#[test]
fn cancelar_a_gravacao_apaga_as_trilhas() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("gravacoes");
    let (jobs, worker, _) = start(dir.path());
    let active = submit_recording(&jobs, recording(&root), "Gravação espera");
    wait_for(&jobs, active.id, "rodando", running);
    let queued = submit_recording(&jobs, recording(&root), "Gravação na fila");

    assert_eq!(jobs.cancel(queued.id).unwrap().state, State::Canceled);
    assert!(!queued.input.exists(), "na fila");
    jobs.cancel(active.id).unwrap();
    wait_for(&jobs, active.id, "cancelada", |s| *s == State::Canceled);
    assert!(!active.input.exists(), "rodando");
    assert!(worker.shutdown(Duration::from_secs(5)));
}

/// Fechar o app apaga só a gravação já transcrita que esperava a resposta; a que estava na
/// fila, rodando (cancelada pelo encerramento) ou que falhou volta na próxima abertura.
#[test]
fn encerrar_guarda_as_gravacoes_nao_transcritas() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("gravacoes");
    let (jobs, worker, _) = start(dir.path());
    let pending = jobs.submit_recording(recording(&root));
    let pending = wait_for(&jobs, pending, "concluída", done);
    let failed = submit_recording(&jobs, recording(&root), "Gravação panico");
    wait_for(&jobs, failed.id, "falhou", |s| {
        matches!(s, State::Failed { .. })
    });
    let active = submit_recording(&jobs, recording(&root), "Gravação espera");
    wait_for(&jobs, active.id, "rodando", running);
    let queued = submit_recording(&jobs, recording(&root), "Gravação na fila");

    assert!(worker.shutdown(Duration::from_secs(5)));
    assert_eq!(jobs.get(active.id).unwrap().state, State::Canceled);
    assert!(!pending.input.exists(), "transcrita e sem resposta");
    for job in [&failed, &active, &queued] {
        assert!(job.input.is_dir(), "{}", job.name);
    }
    let mut back: Vec<JobId> = scriba::capture::recover(&root)
        .into_iter()
        .map(|r| r.id)
        .collect();
    back.sort();
    let mut expected = vec![failed.id, active.id, queued.id];
    expected.sort();
    assert_eq!(back, expected);
}

/// Ao abrir o app, as gravações não transcritas voltam à fila e concluem; a já transcrita
/// (o app caiu antes da resposta) é apagada.
#[test]
fn gravacoes_retomadas_ao_abrir() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("gravacoes");
    let (old, new) = (recording(&root), recording(&root));
    let gone = recording(&root);
    storage::mark_transcribed(&gone.dir).unwrap();

    let (jobs, worker, _) = start(dir.path());
    assert_eq!(scriba::app::resume_recordings(&jobs, &root), 2);
    assert!(!gone.dir.exists());
    let order: Vec<JobId> = jobs.list().iter().rev().map(|j| j.id).collect();
    assert_eq!(order, [old.id, new.id], "da mais antiga para a mais nova");
    for rec in [&old, &new] {
        let job = wait_for(&jobs, rec.id, "retomada", done);
        assert_eq!(job.media, Media::Recording);
        assert_eq!((job.size, job.duration_ms), (rec.size, 1_000));
        // O `TestPipeline` estima 1 s por segundo; a gravação, o dobro.
        assert_eq!(
            job.estimate,
            Some(Estimate {
                secs: 2,
                gpu: false
            })
        );
        assert_eq!(job.input, rec.dir);
    }
    assert!(worker.shutdown(Duration::from_secs(5)));
    assert!(!old.dir.exists() && !new.dir.exists());
    assert_eq!(scriba::app::resume_recordings(&jobs, &root), 0);
}
