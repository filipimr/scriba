//! O worker: uma thread dedicada que roda um job por vez, fora do runtime Tokio. Um panic no
//! pipeline vira "Falhou" e não derruba a fila.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedReceiver;

use super::id::JobId;
use super::model::Event;
use super::pipeline::{Ctx, Pipeline};
use super::store::{Jobs, Msg, remove_quietly};

pub struct Worker {
    jobs: Jobs,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub(super) fn spawn(
        jobs: Jobs,
        mut rx: UnboundedReceiver<Msg>,
        pipeline: Arc<dyn Pipeline>,
    ) -> Self {
        let queue = jobs.clone();
        let thread = thread::Builder::new()
            .name("scriba-worker".into())
            .spawn(move || {
                while let Some(Msg::Run(id)) = rx.blocking_recv() {
                    if queue.stopping() {
                        break;
                    }
                    run_one(&queue, id, pipeline.as_ref());
                }
            })
            .expect("criando a thread do worker");
        Self {
            jobs,
            thread: Some(thread),
        }
    }

    /// Cancela o job em andamento e espera a thread terminar, por até `timeout`. Devolve
    /// `false` se o pipeline não atendeu ao cancelamento a tempo.
    pub fn shutdown(mut self, timeout: Duration) -> bool {
        self.jobs.stop();
        let Some(thread) = self.thread.take() else {
            return true;
        };
        let deadline = Instant::now() + timeout;
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
        thread.join().is_ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.jobs.stop();
        }
    }
}

fn run_one(jobs: &Jobs, id: JobId, pipeline: &dyn Pipeline) {
    let Some((job, cancel)) = jobs.begin(id) else {
        return;
    };
    let progress = jobs.clone();
    let ctx = Ctx::new(cancel, move |stage, pct| {
        progress.apply(id, Event::Progress(stage, pct));
    });
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| pipeline.run(&job, &ctx)));
    jobs.apply(
        id,
        match outcome {
            Ok(result) => Event::Finished(result),
            Err(_) => Event::Panicked,
        },
    );
    remove_quietly(&job.input);
}
