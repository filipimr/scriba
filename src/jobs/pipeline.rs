//! O pipeline que processa um job (falso nesta fase; FFmpeg e Whisper na Fase 4) e o
//! contexto que ele recebe para relatar o progresso e saber do cancelamento.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::model::{Failure, Job, Outputs, Stage};

pub trait Pipeline: Send + Sync {
    /// Roda na thread do worker. Deve chamar `ctx.check()?` com frequência para atender ao
    /// cancelamento.
    fn run(&self, job: &Job, ctx: &Ctx) -> Result<Outputs, Failure>;
}

pub struct Ctx {
    cancel: Arc<AtomicBool>,
    report: Box<dyn Fn(Stage, u8) + Send + Sync>,
}

impl Ctx {
    pub fn new(
        cancel: Arc<AtomicBool>,
        report: impl Fn(Stage, u8) + Send + Sync + 'static,
    ) -> Self {
        Self {
            cancel,
            report: Box::new(report),
        }
    }

    pub fn progress(&self, stage: Stage, pct: u8) {
        (self.report)(stage, pct);
    }

    pub fn canceled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// `Err(Failure::Canceled)` se o cancelamento foi pedido; feito para usar com `?`.
    pub fn check(&self) -> Result<(), Failure> {
        if self.canceled() {
            Err(Failure::Canceled)
        } else {
            Ok(())
        }
    }
}
