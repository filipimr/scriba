//! Pipeline falso da Fase 3: percorre as etapas do job com progresso simulado, para validar
//! a fila e a UI antes dos motores reais. Ganchos pelo nome do arquivo, para testes e
//! demonstração: `lento` (10× mais devagar), `falha` (erro na metade da primeira etapa) e
//! `panico` (panic no mesmo ponto).

use std::thread;
use std::time::Duration;

use super::model::{Failure, Job, Outputs};
use super::pipeline::{Ctx, Pipeline};

/// Passos de 5% por etapa.
const STEPS: u8 = 20;

pub struct FakePipeline {
    tick: Duration,
}

impl FakePipeline {
    pub const FAILURE: &'static str = "Não foi possível processar este arquivo (falha simulada).";

    /// `tick` é a pausa entre os passos: cada etapa leva `20 × tick`.
    pub fn new(tick: Duration) -> Self {
        Self { tick }
    }
}

impl Default for FakePipeline {
    /// ~2 s por etapa.
    fn default() -> Self {
        Self::new(Duration::from_millis(100))
    }
}

impl Pipeline for FakePipeline {
    fn run(&self, job: &Job, ctx: &Ctx) -> Result<Outputs, Failure> {
        let name = job.name.to_lowercase();
        let tick = if name.contains("lento") {
            self.tick * 10
        } else {
            self.tick
        };
        for (i, stage) in job.options.stages().into_iter().enumerate() {
            for step in 0..=STEPS {
                ctx.check()?;
                let pct = step * (100 / STEPS);
                if i == 0 && pct == 50 {
                    if name.contains("panico") {
                        panic!("pânico simulado pelo pipeline falso: {}", job.name);
                    }
                    if name.contains("falha") {
                        return Err(Failure::Error(Self::FAILURE.into()));
                    }
                }
                ctx.progress(stage, pct);
                if step < STEPS {
                    thread::sleep(tick);
                }
            }
        }
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::jobs::{JobId, Media, Options, Stage, State};

    fn job(name: &str, transcribe: bool, compress: bool) -> Job {
        Job {
            id: JobId::generate(),
            name: name.into(),
            media: Media::Video,
            size: 1,
            options: Options::new(transcribe, compress).unwrap(),
            input: "nada".into(),
            state: State::Queued,
        }
    }

    /// Roda sem pausas e devolve o resultado e o progresso relatado.
    fn run(job: &Job, canceled: bool) -> (Result<Outputs, Failure>, Vec<(Stage, u8)>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let ctx = Ctx::new(Arc::new(AtomicBool::new(canceled)), move |s, p| {
            sink.lock().unwrap().push((s, p));
        });
        let result = FakePipeline::new(Duration::ZERO).run(job, &ctx);
        let seen = seen.lock().unwrap().clone();
        (result, seen)
    }

    #[test]
    fn percorre_as_etapas_das_opcoes() {
        let (result, seen) = run(&job("reuniao.mp4", true, true), false);
        assert_eq!(result, Ok(vec![]));
        let mut stages: Vec<Stage> = seen.iter().map(|(s, _)| *s).collect();
        stages.dedup();
        assert_eq!(
            stages,
            [
                Stage::ExtractingAudio,
                Stage::Transcribing,
                Stage::Compressing
            ]
        );
        assert_eq!(seen.first(), Some(&(Stage::ExtractingAudio, 0)));
        assert_eq!(seen.last(), Some(&(Stage::Compressing, 100)));
        assert!(seen.windows(2).all(|w| w[0].0 != w[1].0 || w[0].1 < w[1].1));
    }

    #[test]
    fn falha_na_metade_da_primeira_etapa() {
        let (result, seen) = run(&job("Falha.wav", false, true), false);
        assert_eq!(result, Err(Failure::Error(FakePipeline::FAILURE.into())));
        assert_eq!(seen.last(), Some(&(Stage::Compressing, 45)));
    }

    #[test]
    fn panico_na_metade_da_primeira_etapa() {
        let j = job("panico.mp3", true, false);
        assert!(catch_unwind(AssertUnwindSafe(|| run(&j, false))).is_err());
    }

    #[test]
    fn cancelado_para_antes_do_proximo_passo() {
        let (result, seen) = run(&job("reuniao.mp4", true, false), true);
        assert_eq!(result, Err(Failure::Canceled));
        assert!(seen.is_empty());
    }
}
