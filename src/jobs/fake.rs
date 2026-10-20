//! Pipeline falso, para os testes e o E2E (`--fake-pipeline`, feature `test-support`):
//! percorre as etapas com progresso simulado, sem ler o arquivo, e grava uma transcrição fixa
//! com o módulo `transcript` real. Ganchos pelo nome do arquivo:
//!
//! - `lento`: 10× mais devagar, também ao guardar o áudio;
//! - `falha`: erro na metade da primeira etapa;
//! - `falha-uma-vez`: o mesmo erro, só na primeira tentativa do job;
//! - `panico`: panic no mesmo ponto;
//! - `sem-audio`: o probe recusa o envio;
//! - `audio-ruim`: "Guardar áudio" falha na metade;
//! - `audio-demorado`: só o "Guardar áudio" 10× mais devagar.
//!
//! "Guardar áudio" também não lê o envio: grava um texto fixo no lugar do `.m4a`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use super::id::JobId;
use super::model::{Estimate, Failure, Job, Media, Outputs, Probe, Stage};
use super::pipeline::{Ctx, Pipeline};
use crate::transcript::{Segment, Timings};

/// Passos de 5% por etapa.
const STEPS: u8 = 20;

pub struct FakePipeline {
    tick: Duration,
    output_dir: PathBuf,
    /// Jobs `falha-uma-vez` que já falharam.
    failed_once: Mutex<HashSet<JobId>>,
}

impl FakePipeline {
    pub const FAILURE: &'static str = "Não foi possível processar este arquivo (falha simulada).";
    pub const AUDIO_FAILURE: &'static str = "Não consegui guardar o áudio (falha simulada).";
    pub const BACKEND: &'static str = "Transcrição no processador";
    /// Duração que o probe falso informa: 2 min.
    pub const DURATION_MS: u64 = 120_000;
    /// "Cerca de 4 min no processador".
    pub const ESTIMATE: Estimate = Estimate {
        secs: 240,
        gpu: false,
    };
    /// O texto da transcrição gravada.
    pub const SEGMENTS: [(u64, u64, &'static str); 2] = [
        (0, 4_000, "Bom dia, esta é uma transcrição simulada."),
        (8_000, 12_000, "O Scriba não ouviu este arquivo."),
    ];

    /// Grava as saídas em `output_dir`, com ~2 s por etapa.
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            tick: Duration::from_millis(100),
            output_dir,
            failed_once: Mutex::default(),
        }
    }

    /// `tick` é a pausa entre os passos: cada etapa leva `20 × tick`.
    pub fn tick(mut self, tick: Duration) -> Self {
        self.tick = tick;
        self
    }

    /// A pausa para o arquivo `name` (em minúsculas).
    fn tick_for(&self, name: &str) -> Duration {
        if name.contains("lento") {
            self.tick * 10
        } else {
            self.tick
        }
    }
}

impl Pipeline for FakePipeline {
    fn probe(&self, name: &str, _input: &Path) -> Result<Probe, Failure> {
        if name.to_lowercase().contains("sem-audio") {
            return Err(crate::media::Error::NoAudio.into());
        }
        Ok(Probe {
            media: Media::detect(name).map_or(Media::Audio, |(media, _)| media),
            duration_ms: Self::DURATION_MS,
            estimate: Some(Self::ESTIMATE),
        })
    }

    fn backend(&self) -> Option<String> {
        Some(Self::BACKEND.into())
    }

    fn save_audio(
        &self,
        job: &Job,
        output: &Path,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<(), Failure> {
        let name = job.name.to_lowercase();
        let tick = if name.contains("audio-demorado") {
            self.tick * 10
        } else {
            self.tick_for(&name)
        };
        for step in 0..=STEPS {
            if cancel.load(Ordering::Relaxed) {
                return Err(Failure::Canceled);
            }
            let pct = step * (100 / STEPS);
            if pct == 50 && name.contains("audio-ruim") {
                return Err(Failure::Error(Self::AUDIO_FAILURE.into()));
            }
            progress(pct);
            if step < STEPS {
                thread::sleep(tick);
            }
        }
        std::fs::write(output, format!("áudio simulado de {}", job.name)).map_err(|e| {
            eprintln!("não consegui gravar {}: {e}", output.display());
            crate::media::Error::Encode.into()
        })
    }

    fn run(&self, job: &Job, ctx: &Ctx) -> Result<Outputs, Failure> {
        let name = job.name.to_lowercase();
        let tick = self.tick_for(&name);
        for (i, stage) in [Stage::ExtractingAudio, Stage::Transcribing]
            .into_iter()
            .enumerate()
        {
            for step in 0..=STEPS {
                ctx.check()?;
                let pct = step * (100 / STEPS);
                if i == 0 && pct == 50 {
                    if name.contains("panico") {
                        panic!("pânico simulado pelo pipeline falso: {}", job.name);
                    }
                    let first_try = || {
                        self.failed_once
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .insert(job.id)
                    };
                    if name.contains("falha-uma-vez") {
                        if first_try() {
                            return Err(Failure::Error(Self::FAILURE.into()));
                        }
                    } else if name.contains("falha") {
                        return Err(Failure::Error(Self::FAILURE.into()));
                    }
                }
                ctx.progress(stage, pct);
                if step < STEPS {
                    thread::sleep(tick);
                }
            }
        }
        let segments: Vec<Segment> = Self::SEGMENTS
            .iter()
            .map(|&(start_ms, end_ms, text)| Segment {
                start_ms,
                end_ms,
                text: text.into(),
            })
            .collect();
        crate::engine::write_outputs(
            &self.output_dir,
            job,
            job.duration_ms,
            &segments,
            "simulado",
            "nenhum",
            Timings::default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::clock::LocalTime;
    use crate::jobs::{JobId, Media, State};

    fn job(name: &str) -> Job {
        Job {
            id: JobId::generate(),
            name: name.into(),
            media: Media::Video,
            size: 1,
            duration_ms: FakePipeline::DURATION_MS,
            created: LocalTime::now(),
            estimate: None,
            input: "nada".into(),
            state: State::Queued,
        }
    }

    /// Roda sem pausas e devolve o resultado e o progresso relatado.
    fn run(
        pipeline: &FakePipeline,
        job: &Job,
        canceled: bool,
    ) -> (Result<Outputs, Failure>, Vec<(Stage, u8)>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let ctx = Ctx::new(Arc::new(AtomicBool::new(canceled)), move |s, p| {
            sink.lock().unwrap().push((s, p));
        });
        let result = pipeline.run(job, &ctx);
        let seen = seen.lock().unwrap().clone();
        (result, seen)
    }

    fn fake(dir: &tempfile::TempDir) -> FakePipeline {
        FakePipeline::new(dir.path().into()).tick(Duration::ZERO)
    }

    #[test]
    fn percorre_as_etapas_e_grava_a_transcricao() {
        let dir = tempfile::tempdir().unwrap();
        let (result, seen) = run(&fake(&dir), &job("reuniao.mp4"), false);
        let outputs = result.unwrap();
        let mut stages: Vec<Stage> = seen.iter().map(|(s, _)| *s).collect();
        stages.dedup();
        assert_eq!(stages, [Stage::ExtractingAudio, Stage::Transcribing]);
        assert_eq!(seen.first(), Some(&(Stage::ExtractingAudio, 0)));
        assert_eq!(seen.last(), Some(&(Stage::Transcribing, 100)));
        assert!(seen.windows(2).all(|w| w[0].0 != w[1].0 || w[0].1 < w[1].1));

        assert!(outputs.dir.starts_with(dir.path()));
        let txt = std::fs::read_to_string(&outputs.txt).unwrap();
        assert!(txt.contains("Arquivo: reuniao.mp4"), "{txt}");
        assert!(txt.contains("[00:00:00] Bom dia"), "{txt}");
        assert!(txt.contains("[00:00:08] O Scriba"), "{txt}");
        assert!(outputs.md.is_file());
    }

    #[test]
    fn falha_na_metade_da_primeira_etapa() {
        let dir = tempfile::tempdir().unwrap();
        let (result, seen) = run(&fake(&dir), &job("Falha.wav"), false);
        assert_eq!(result, Err(Failure::Error(FakePipeline::FAILURE.into())));
        assert_eq!(seen.last(), Some(&(Stage::ExtractingAudio, 45)));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn falha_uma_vez_passa_na_segunda_tentativa() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = fake(&dir);
        let j = job("falha-uma-vez.mp3");
        assert!(run(&pipeline, &j, false).0.is_err());
        assert!(run(&pipeline, &j, false).0.is_ok());
        // Outro job com o mesmo nome também falha uma vez.
        assert!(run(&pipeline, &job("falha-uma-vez.mp3"), false).0.is_err());
    }

    #[test]
    fn panico_na_metade_da_primeira_etapa() {
        let dir = tempfile::tempdir().unwrap();
        let (pipeline, j) = (fake(&dir), job("panico.mp3"));
        assert!(catch_unwind(AssertUnwindSafe(|| run(&pipeline, &j, false))).is_err());
    }

    #[test]
    fn cancelado_para_antes_do_proximo_passo() {
        let dir = tempfile::tempdir().unwrap();
        let (result, seen) = run(&fake(&dir), &job("reuniao.mp4"), true);
        assert_eq!(result, Err(Failure::Canceled));
        assert!(seen.is_empty());
    }

    #[test]
    fn guardar_audio_simulado() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = fake(&dir);
        let out = dir.path().join("a.m4a");
        let mut seen = Vec::new();
        pipeline
            .save_audio(
                &job("reuniao.mp4"),
                &out,
                &mut |p| seen.push(p),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "áudio simulado de reuniao.mp4"
        );
        assert_eq!((seen.first(), seen.last()), (Some(&0), Some(&100)));

        let out = dir.path().join("b.m4a");
        let save = |name: &str, canceled: bool| {
            pipeline.save_audio(&job(name), &out, &mut |_| {}, &AtomicBool::new(canceled))
        };
        assert_eq!(
            save("Audio-Ruim.mp4", false),
            Err(Failure::Error(FakePipeline::AUDIO_FAILURE.into()))
        );
        assert_eq!(save("reuniao.mp4", true), Err(Failure::Canceled));
        assert!(!out.exists());
    }

    #[test]
    fn probe_pelo_nome() {
        let dir = tempfile::tempdir().unwrap();
        let pipeline = fake(&dir);
        let probe = pipeline.probe("aula.MKV", Path::new("nada")).unwrap();
        assert_eq!(probe.media, Media::Video);
        assert_eq!(probe.duration_ms, FakePipeline::DURATION_MS);
        assert_eq!(probe.estimate, Some(FakePipeline::ESTIMATE));
        assert_eq!(
            pipeline.probe("Sem-Audio.mp4", Path::new("nada")),
            Err(Failure::Error(
                "Este arquivo não tem áudio. Escolha um vídeo com som.".into()
            ))
        );
        assert_eq!(pipeline.backend().as_deref(), Some(FakePipeline::BACKEND));
    }
}
