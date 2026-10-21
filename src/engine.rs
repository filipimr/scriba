//! O pipeline do app: `media` extrai o áudio para um arquivo temporário ao lado do envio,
//! `asr` transcreve lendo um bloco por vez e `transcript` grava a pasta da saída com o
//! `.txt`, o `.md` e o `job.json`. Falha e cancelamento não gravam nada, e o temporário é
//! apagado em qualquer desfecho. "Guardar áudio" recodifica o envio num `.m4a`
//! (`media::encode_audio`).
//!
//! Uma gravação passa as duas trilhas pelo mesmo caminho, cada uma com o seu temporário
//! dentro da pasta dela: "Outros" (`sistema.wav`) e "Você" (`mic.wav`). As transcrições são
//! mescladas por tempo e o eco sai (`transcript::merge`); o "Guardar áudio" mistura as duas
//! trilhas (`media::encode_mix`).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::asr::{Asr, Samples};
use crate::capture::{Track, storage};
use crate::jobs::{Ctx, Estimate, Failure, Job, Media, Outputs, Pipeline, Probe, Stage};
use crate::media::{self, SAMPLE_RATE, Spool};
use crate::transcript::{
    self, Header, Manifest, MediaKind, Segment, Speaker, Timings, Transcript, merge,
};

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
            estimate: self.estimate(probe.duration_ms),
        })
    }

    fn backend(&self) -> Option<String> {
        self.asr.backend_label()
    }

    fn estimate(&self, duration_ms: u64) -> Option<Estimate> {
        self.asr
            .estimate(duration_ms)
            .map(|e| with_extraction(e, duration_ms))
    }

    fn run(&self, job: &Job, ctx: &Ctx) -> Result<Outputs, Failure> {
        if job.media == Media::Recording {
            return self.run_recording(job, ctx);
        }
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
        if job.media == Media::Recording {
            let tracks: Vec<PathBuf> = Track::ALL
                .iter()
                .map(|t| job.input.join(t.file()))
                .filter(|p| p.is_file())
                .collect();
            let tracks: Vec<&Path> = tracks.iter().map(PathBuf::as_path).collect();
            return Ok(media::encode_mix(&tracks, output, progress, cancel)?);
        }
        Ok(media::encode_audio(&job.input, output, progress, cancel)?)
    }
}

/// As trilhas de uma gravação, na ordem em que são transcritas.
const RECORDING_TRACKS: [(Track, Speaker); 2] = [
    (Track::System, Speaker::Outros),
    (Track::Mic, Speaker::Voce),
];

impl Engine {
    /// Extrai as duas trilhas (cada uma com metade do progresso da etapa) e depois as
    /// transcreve do mesmo jeito, para o card nunca voltar de etapa. Uma trilha sem nenhuma
    /// amostra (o app caiu antes do 1º `flush`) fica de fora.
    fn run_recording(&self, job: &Job, ctx: &Ctx) -> Result<Outputs, Failure> {
        let t0 = Instant::now();
        let tracks: Vec<(Track, Speaker)> = RECORDING_TRACKS
            .into_iter()
            .filter(|&(track, _)| storage::wav_frames(&job.input, track).is_some_and(|f| f > 0))
            .collect();
        let n = tracks.len();
        let mut spools = Vec::with_capacity(n);
        for (i, &(track, speaker)) in tracks.iter().enumerate() {
            let wav = job.input.join(track.file());
            let mut audio = Spool::create(&spool_path(&wav)).map_err(|e| {
                eprintln!("não consegui criar o áudio temporário: {e}");
                media::Error::Disk
            })?;
            media::extract_audio_into(
                &wav,
                |samples| audio.push(samples),
                |pct| ctx.progress(Stage::ExtractingAudio, share(i, n, pct)),
                ctx.cancel_flag(),
            )?;
            spools.push((speaker, audio));
        }
        let extract_ms = elapsed_ms(t0);
        let audio_ms = spools
            .iter()
            .map(|(_, a)| a.len() as u64 * 1_000 / u64::from(SAMPLE_RATE))
            .max()
            .unwrap_or(0);

        let t1 = Instant::now();
        let (mut outros, mut voce) = (Vec::new(), Vec::new());
        let mut used = None;
        for (i, (speaker, mut audio)) in spools.into_iter().enumerate() {
            let transcription = self.asr.transcribe_samples(
                &mut audio,
                |pct| ctx.progress(Stage::Transcribing, share(i, n, pct)),
                ctx.cancel_flag(),
            )?;
            drop(audio);
            ctx.check()?;
            match speaker {
                Speaker::Outros => outros = transcription.segments,
                Speaker::Voce => voce = transcription.segments,
            }
            used = Some((transcription.plan.backend.to_string(), transcription.model));
        }
        let transcribe_ms = elapsed_ms(t1);
        let (backend, model) = used.unwrap_or_else(|| (String::new(), ""));
        let timings = Timings {
            extract_ms,
            transcribe_ms,
            total_ms: elapsed_ms(t0),
        };
        write_outputs(
            &self.output_dir,
            job,
            if job.duration_ms > 0 {
                job.duration_ms
            } else {
                audio_ms
            },
            &merge::merge(&outros, &voce),
            &backend,
            model,
            timings,
        )
    }
}

/// O progresso da parte `i` de `n` dentro da etapa: cada parte fica com a sua fatia.
fn share(i: usize, n: usize, pct: u8) -> u8 {
    ((i * 100 + usize::from(pct.min(100))) / n.max(1)) as u8
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
/// iniciar, o que leva as sobras de um crash. Numa gravação, ao lado de cada trilha
/// (`gravacoes/<id>/mic.f32`), e o `capture::recover` apaga as sobras.
pub fn spool_path(input: &Path) -> PathBuf {
    input.with_extension("f32")
}

fn elapsed_ms(since: Instant) -> u64 {
    since.elapsed().as_millis() as u64
}

/// Grava a pasta do job em `output_dir`: o cabeçalho usa o nome original e a hora do envio
/// (numa gravação, os falantes e a hora do Gravar). Um erro de disco vira a mensagem
/// amigável; o detalhe vai para o stderr.
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
        recording: job.media == Media::Recording,
    };
    let manifest = Manifest {
        id: job.id.to_string(),
        name: job.name.clone(),
        media: match job.media {
            Media::Video => MediaKind::Video,
            Media::Audio => MediaKind::Audio,
            Media::Recording => MediaKind::Recording,
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

    #[test]
    fn progresso_em_partes() {
        assert_eq!([0, 50, 100].map(|p| share(0, 2, p)), [0, 25, 50]);
        assert_eq!([0, 50, 100].map(|p| share(1, 2, p)), [50, 75, 100]);
        assert_eq!(share(0, 1, 37), 37);
        assert_eq!(share(0, 0, 37), 37);
        assert_eq!(share(1, 2, 250), 100);
    }
}
