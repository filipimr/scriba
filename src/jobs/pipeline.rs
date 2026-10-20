//! O pipeline que processa um job (o `Engine` no app; o `FakePipeline` nos testes) e o
//! contexto que ele recebe para relatar o progresso e saber do cancelamento.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::model::{Failure, Job, Outputs, Probe, Stage};

pub trait Pipeline: Send + Sync {
    /// Lê o arquivo recém-enviado: tipo, duração e estimativa. Um erro recusa o envio com a
    /// mensagem dele. Bloqueia: o servidor chama em `spawn_blocking`.
    fn probe(&self, name: &str, input: &Path) -> Result<Probe, Failure>;

    /// Onde a transcrição roda, para o rodapé ("Transcrição na GPU: …").
    fn backend(&self) -> Option<String>;

    /// Roda na thread do worker. Deve chamar `ctx.check()?` com frequência para atender ao
    /// cancelamento.
    fn run(&self, job: &Job, ctx: &Ctx) -> Result<Outputs, Failure>;

    /// "Guardar áudio": grava o áudio do envio do job concluído em `output` (um `.m4a`).
    /// Roda numa thread própria, fora do worker. `progress` vai de 0 a 100; com `cancel`,
    /// devolve `Failure::Canceled`. Em erro, não deixa `output` para trás.
    fn save_audio(
        &self,
        job: &Job,
        output: &Path,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<(), Failure>;
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

    /// A flag em si, para os motores que a consultam durante o trabalho.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
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
