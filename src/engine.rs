//! O pipeline do app: `media` extrai o áudio, `asr` transcreve e `transcript` grava a pasta
//! da saída com o `.txt`, o `.md` e o `job.json`. Falha e cancelamento não gravam nada.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::asr::Asr;
use crate::jobs::{Ctx, Failure, Job, Media, Outputs, Pipeline, Probe, Stage};
use crate::media::{self, SAMPLE_RATE};
use crate::transcript::{self, Header, Manifest, MediaKind, Segment, Timings, Transcript};

pub const SAVE_ERROR: &str =
    "Não consegui salvar a transcrição. Confira o espaço livre em disco e tente de novo.";

pub struct Engine {
    output_dir: PathBuf,
    asr: Asr,
}

impl Engine {
    /// Detecta as GPUs agora (síncrono): o rodapé e a estimativa já sabem o backend.
    pub fn new(output_dir: PathBuf) -> Self {
        Self::with_asr(output_dir, Asr::detect())
    }

    pub fn with_asr(output_dir: PathBuf, asr: Asr) -> Self {
        Self { output_dir, asr }
    }
}

impl Pipeline for Engine {
    fn probe(&self, _name: &str, input: &Path) -> Result<Probe, Failure> {
        let probe = media::probe(input)?;
        if !probe.has_audio {
            return Err(media::Error::NoAudio.into());
        }
        Ok(Probe {
            media: if probe.has_video {
                Media::Video
            } else {
                Media::Audio
            },
            duration_ms: probe.duration_ms,
            estimate: self.asr.estimate(probe.duration_ms),
        })
    }

    fn backend(&self) -> Option<String> {
        self.asr.backend_label()
    }

    fn run(&self, job: &Job, ctx: &Ctx) -> Result<Outputs, Failure> {
        let t0 = Instant::now();
        let audio = media::extract_audio(
            &job.input,
            |pct| ctx.progress(Stage::ExtractingAudio, pct),
            ctx.cancel_flag(),
        )?;
        let extract_ms = elapsed_ms(t0);
        let audio_ms = audio.len() as u64 * 1_000 / u64::from(SAMPLE_RATE);

        let t1 = Instant::now();
        let transcription = self.asr.transcribe(
            &audio,
            |pct| ctx.progress(Stage::Transcribing, pct),
            ctx.cancel_flag(),
        )?;
        drop(audio);
        let transcribe_ms = elapsed_ms(t1);
        ctx.check()?;

        let timings = Timings {
            extract_ms,
            transcribe_ms,
            total_ms: elapsed_ms(t0),
        };
        write_outputs(
            &self.output_dir,
            job,
            // O contêiner pode não informar a duração; o áudio extraído sempre tem.
            if job.duration_ms > 0 {
                job.duration_ms
            } else {
                audio_ms
            },
            &transcription.segments,
            &transcription.plan.backend.to_string(),
            transcription.model,
            timings,
        )
    }
}

fn elapsed_ms(since: Instant) -> u64 {
    since.elapsed().as_millis() as u64
}

/// Grava a pasta do job em `output_dir`: o cabeçalho usa o nome original e a hora do envio.
/// Um erro de disco vira a mensagem amigável; o detalhe vai para o stderr.
pub fn write_outputs(
    output_dir: &Path,
    job: &Job,
    duration_ms: u64,
    segments: &[Segment],
    backend: &str,
    model: &str,
    timings: Timings,
) -> Result<Outputs, Failure> {
    let header = Header {
        file: job.name.clone(),
        date: job.created,
        duration_ms,
    };
    let manifest = Manifest {
        id: job.id.to_string(),
        name: job.name.clone(),
        media: match job.media {
            Media::Video => MediaKind::Video,
            Media::Audio => MediaKind::Audio,
        },
        size_bytes: job.size,
        duration_ms,
        created: job.created.rfc3339(),
        backend: backend.into(),
        model: model.into(),
        timings,
        ..Manifest::default()
    };
    let written = transcript::write(output_dir, &Transcript::new(header, segments), &manifest)
        .map_err(|e| {
            eprintln!(
                "não consegui gravar a saída em {}: {e}",
                output_dir.display()
            );
            Failure::Error(SAVE_ERROR.into())
        })?;
    Ok(Outputs {
        dir: written.dir,
        txt: written.txt,
        md: written.md,
    })
}
