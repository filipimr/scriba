//! Integração da fila: store e worker reais, com pipelines de teste que travam, falham ou
//! entram em pânico.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use scriba::clock::LocalTime;
use scriba::jobs::{
    Ctx, Failure, Job, JobId, Jobs, Media, Outputs, PANIC_MESSAGE, Pipeline, Probe, RemoveError,
    RetryError, State,
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
    assert!(!next.input.exists(), "o concluído apaga o envio");
    assert!(worker.shutdown(Duration::from_secs(5)));
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
    assert!(!job.input.exists(), "concluído apaga o envio");
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
    let State::Done { outputs } = job.state else {
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
    let State::Done { outputs } = job.state else {
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
