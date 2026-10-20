//! O pipeline do app: `media` extrai o áudio para um arquivo temporário ao lado do envio,
//! `asr` transcreve lendo um bloco por vez e `transcript` grava a pasta da saída com o
//! `.txt`, o `.md` e o `job.json`. Falha e cancelamento não gravam nada, e o temporário é
//! apagado em qualquer desfecho. "Guardar áudio" recodifica o envio num `.m4a`
//! (`media::encode_audio`).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::asr::{Asr, Samples};
use crate::jobs::{Ctx, Estimate, Failure, Job, Media, Outputs, Pipeline, Probe, Stage};
use crate::media::{self, SAMPLE_RATE, Spool};
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
            estimate: self
                .asr
                .estimate(probe.duration_ms)
                .map(|e| with_extraction(e, probe.duration_ms)),
        })
    }

    fn backend(&self) -> Option<String> {
        self.asr.backend_label()
    }

    fn run(&self, job: &Job, ctx: &Ctx) -> Result<Outputs, Failure> {
        let t0 = Instant::now();
        let mut audio = Spool::create(&spool_path(&job.input)).map_err(|e| {
            eprintln!("não consegui criar o áudio temporário: {e}");
            media::Error::Disk
        })?;
        media::extract_audio_into(
            &job.input,
            |samples| audio.push(samples),
            |pct| ctx.progress(Stage::ExtractingAudio, pct),
            ctx.cancel_flag(),
        )?;
        let extract_ms = elapsed_ms(t0);
        let audio_ms = audio.len() as u64 * 1_000 / u64::from(SAMPLE_RATE);

        let t1 = Instant::now();
        let transcription = self.asr.transcribe_samples(
            &mut audio,
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

    fn save_audio(
        &self,
        job: &Job,
        output: &Path,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<(), Failure> {
        Ok(media::encode_audio(&job.input, output, progress, cancel)?)
    }
}

/// Extração do áudio, em segundos por segundo de mídia. Na etapa 4.6, 1 h de vídeo (H.264
/// 720p + AAC) saiu em 1,3 s e 3 h de MP3 em 6 s (≤ 0,0005); o dobro cobre discos mais lentos.
pub const EXTRACT_RTF: f64 = 0.001;

/// A estimativa da transcrição mais a extração do áudio de `duration_ms` de mídia.
pub fn with_extraction(estimate: Estimate, duration_ms: u64) -> Estimate {
    let extract = (duration_ms as f64 / 1_000.0 * EXTRACT_RTF).ceil() as u64;
    Estimate {
        secs: estimate.secs + extract,
        ..estimate
    }
}

/// O áudio extraído fica ao lado do envio (`uploads/<id>.f32`): a pasta é esvaziada ao
/// iniciar, o que leva as sobras de um crash.
pub fn spool_path(input: &Path) -> PathBuf {
    input.with_extension("f32")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimativa_soma_a_extracao() {
        let cpu = Estimate {
            secs: 488,
            gpu: false,
        };
        // 1 h × 0,001 = 3,6 s, arredondado para cima.
        assert_eq!(
            with_extraction(cpu, 3_600_000),
            Estimate {
                secs: 492,
                gpu: false
            }
        );
        assert_eq!(with_extraction(cpu, 0).secs, 488);
        assert_eq!(with_extraction(cpu, 1).secs, 489);
    }
}
