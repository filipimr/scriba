//! Integração da fila: store e worker reais, com pipelines de teste que travam, falham ou
//! entram em pânico.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use scriba::jobs::{
    Ctx, Failure, Job, JobId, Jobs, Media, Options, Outputs, PANIC_MESSAGE, Pipeline, RemoveError,
    State,
};

/// Comporta-se pelo nome do arquivo: `panico` entra em pânico, `espera` roda até ser
/// cancelado; o resto termina na hora. Conta quantos jobs começou.
#[derive(Default)]
struct TestPipeline {
    runs: AtomicUsize,
}

impl Pipeline for TestPipeline {
    fn run(&self, job: &Job, ctx: &Ctx) -> Result<Outputs, Failure> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if job.name.contains("panico") {
            panic!("pânico de teste");
        }
        while job.name.contains("espera") {
            ctx.check()?;
            thread::sleep(Duration::from_millis(5));
        }
        Ok(vec![])
    }
}

fn start(dir: &Path) -> (Jobs, scriba::jobs::Worker, Arc<TestPipeline>) {
    let pipeline = Arc::new(TestPipeline::default());
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
        options: Options::new(true, false).unwrap(),
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
    assert_eq!(next.state, State::Done { outputs: vec![] });
    assert!(
        !job.input.exists() && !next.input.exists(),
        "uploads apagados"
    );
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
    assert_eq!(jobs.remove(id), Err(RemoveError::Active));

    jobs.cancel(id);
    wait_for(&jobs, id, "fim", State::is_terminal);
    assert_eq!(jobs.remove(id), Ok(()));
    assert!(jobs.get(id).is_none() && jobs.list().is_empty());
    assert_eq!(jobs.remove(id), Err(RemoveError::NotFound));
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

    let jobs = Jobs::without_worker(uploads.clone()).unwrap();
    assert!(!uploads.join("sobra.mp4").exists());
    let a = submit(&jobs, "a.wav");
    let b = submit(&jobs, "b.wav");
    let ids: Vec<JobId> = jobs.list().iter().map(|j| j.id).collect();
    assert_eq!(ids, [b, a]);
    assert_eq!(jobs.get(a).unwrap().state, State::Queued, "sem worker");
}
