//! Mídia pelo FFmpeg estático: `probe` lê a duração, a taxa de bits e a resolução do
//! original, e `extract_audio` entrega a melhor faixa de áudio em 16 kHz mono f32, sem
//! decodificar o vídeo.

use std::fmt;
use std::path::Path;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use ffmpeg::format::stream::Disposition;
use ffmpeg::format::{Sample, sample};
use ffmpeg::software::resampling;
use ffmpeg::util::frame;
use ffmpeg::{ChannelLayout, Packet, codec, media};
use ffmpeg_next as ffmpeg;

use crate::jobs::Failure;

/// Taxa de amostragem que o whisper espera.
pub const SAMPLE_RATE: u32 = 16_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NotMedia,
    NoAudio,
    Unreadable,
    Canceled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotMedia => "Não reconheci este arquivo como vídeo ou áudio.",
            Self::NoAudio => "Este arquivo não tem áudio. Escolha um vídeo com som.",
            Self::Unreadable => {
                "Não consegui ler o áudio deste arquivo. Ele pode estar corrompido."
            }
            Self::Canceled => "Cancelado.",
        })
    }
}

impl std::error::Error for Error {}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        match e {
            Error::Canceled => Failure::Canceled,
            other => Failure::Error(other.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// 0 quando o contêiner não informa.
    pub duration_ms: u64,
    /// Taxa de bits total, em bits por segundo.
    pub bit_rate: Option<u64>,
    /// Largura e altura do vídeo.
    pub resolution: Option<(u32, u32)>,
    pub has_audio: bool,
    /// Capa de MP3 ou M4A não conta como vídeo.
    pub has_video: bool,
}

fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        ffmpeg::init().expect("FFmpeg estático");
        ffmpeg::util::log::set_level(ffmpeg::util::log::Level::Error);
    });
}

fn open(path: &Path) -> Result<ffmpeg::format::context::Input, Error> {
    init();
    ffmpeg::format::input(path).map_err(|e| {
        eprintln!("media: {} não abriu: {e}", path.display());
        Error::NotMedia
    })
}

pub fn probe(path: &Path) -> Result<Probe, Error> {
    let ictx = open(path)?;
    let audio = ictx.streams().best(media::Type::Audio);
    let video = ictx
        .streams()
        .filter(|s| s.parameters().medium() == media::Type::Video)
        .find(|s| !s.disposition().contains(Disposition::ATTACHED_PIC));
    if audio.is_none() && video.is_none() {
        return Err(Error::NotMedia);
    }
    let resolution = video.as_ref().and_then(|s| {
        let v = codec::context::Context::from_parameters(s.parameters())
            .ok()?
            .decoder()
            .video()
            .ok()?;
        (v.width() > 0 && v.height() > 0).then(|| (v.width(), v.height()))
    });
    let bit_rate = u64::try_from(ictx.bit_rate()).ok().filter(|&b| b > 0);
    Ok(Probe {
        duration_ms: duration_ms(&ictx),
        bit_rate,
        resolution,
        has_audio: audio.is_some(),
        has_video: video.is_some(),
    })
}

/// Duração do contêiner ou, se ele não informa, a maior das faixas.
fn duration_ms(ictx: &ffmpeg::format::context::Input) -> u64 {
    let container = ictx.duration();
    if container > 0 {
        return (container as f64 * 1_000.0 / f64::from(ffmpeg::ffi::AV_TIME_BASE)) as u64;
    }
    ictx.streams()
        .filter(|s| s.duration() > 0)
        .map(|s| (s.duration() as f64 * f64::from(s.time_base()) * 1_000.0) as u64)
        .max()
        .unwrap_or(0)
}

/// Decodifica a melhor faixa de áudio e reamostra para 16 kHz mono f32. `progress` recebe
/// de 0 a 100 (pelo pts sobre a duração), só quando o valor muda; `cancel` é checado a
/// cada pacote.
pub fn extract_audio(
    path: &Path,
    mut progress: impl FnMut(u8),
    cancel: &AtomicBool,
) -> Result<Vec<f32>, Error> {
    let mut ictx = open(path)?;
    let duration_ms = duration_ms(&ictx);
    let (index, time_base, start) = {
        let stream = ictx.streams().best(media::Type::Audio).ok_or_else(|| {
            let has_video = ictx
                .streams()
                .any(|s| s.parameters().medium() == media::Type::Video);
            if has_video {
                Error::NoAudio
            } else {
                Error::NotMedia
            }
        })?;
        let start = stream.start_time();
        let start = if start == ffmpeg::ffi::AV_NOPTS_VALUE {
            0
        } else {
            start
        };
        (stream.index(), f64::from(stream.time_base()), start)
    };
    let mut decoder = codec::context::Context::from_parameters(
        ictx.stream(index).expect("faixa escolhida").parameters(),
    )
    .and_then(|c| c.decoder().audio())
    .map_err(|e| unreadable(path, "decoder", e))?;

    let mut out = Decoded::default();
    let mut last_pct = None;
    let mut report = |pct: u8| {
        if last_pct.is_none_or(|last| pct > last) {
            last_pct = Some(pct);
            progress(pct);
        }
    };
    report(0);
    let mut bad_packets = 0u32;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Canceled);
        }
        let mut packet = Packet::empty();
        match packet.read(&mut ictx) {
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => break,
            Err(e) => return Err(unreadable(path, "leitura", e)),
        }
        if packet.stream() != index {
            continue;
        }
        if duration_ms > 0
            && let Some(pts) = packet.pts()
        {
            let ms = (pts - start) as f64 * time_base * 1_000.0;
            report((ms / duration_ms as f64 * 100.0).clamp(0.0, 99.0) as u8);
        }
        // Um pacote ruim no meio não derruba o arquivo inteiro.
        if decoder.send_packet(&packet).is_err() {
            bad_packets += 1;
            continue;
        }
        out.receive(&mut decoder)
            .map_err(|e| unreadable(path, "reamostragem", e))?;
    }
    if decoder.send_eof().is_ok() {
        out.receive(&mut decoder)
            .map_err(|e| unreadable(path, "reamostragem", e))?;
    }
    let samples = out
        .finish()
        .map_err(|e| unreadable(path, "reamostragem", e))?;
    if samples.is_empty() {
        eprintln!(
            "media: {} sem amostras ({bad_packets} pacotes ruins)",
            path.display()
        );
        return Err(Error::Unreadable);
    }
    report(100);
    Ok(samples)
}

fn unreadable(path: &Path, step: &str, e: ffmpeg::Error) -> Error {
    eprintln!("media: {} falhou em {step}: {e}", path.display());
    Error::Unreadable
}

/// Recebe os frames do decoder e reamostra para 16 kHz mono f32.
#[derive(Default)]
struct Decoded {
    // O resampler nasce do primeiro frame: o layout informado pelo decoder antes de
    // decodificar pode vir vazio (WAV mono) e o swr recusa com "Input changed". Se o
    // formato muda no meio do arquivo, ele é recriado.
    resampler: Option<resampling::Context>,
    samples: Vec<f32>,
}

impl Decoded {
    fn receive(&mut self, decoder: &mut ffmpeg::decoder::Audio) -> Result<(), ffmpeg::Error> {
        let mut decoded = frame::Audio::empty();
        while decoder.receive_frame(&mut decoded).is_ok() {
            if decoded.channel_layout().is_empty() {
                decoded.set_channel_layout(ChannelLayout::default(i32::from(decoded.channels())));
            }
            let changed = self.resampler.as_ref().is_none_or(|rs| {
                let input = rs.input();
                input.format != decoded.format()
                    || input.channel_layout != decoded.channel_layout()
                    || input.rate != decoded.rate()
            });
            if changed {
                self.flush()?;
                self.resampler = Some(resampling::Context::get(
                    decoded.format(),
                    decoded.channel_layout(),
                    decoded.rate(),
                    Sample::F32(sample::Type::Packed),
                    ChannelLayout::MONO,
                    SAMPLE_RATE,
                )?);
            }
            let rs = self.resampler.as_mut().expect("criado acima");
            let mut resampled = frame::Audio::empty();
            rs.run(&decoded, &mut resampled)?;
            self.push(&resampled);
        }
        Ok(())
    }

    /// Esvazia o que o resampler ainda guarda.
    fn flush(&mut self) -> Result<(), ffmpeg::Error> {
        let Some(rs) = self.resampler.as_mut() else {
            return Ok(());
        };
        // Sem entrada, o swr precisa do frame de saída já alocado no formato final.
        loop {
            let mut resampled = frame::Audio::new(
                Sample::F32(sample::Type::Packed),
                4_096,
                ChannelLayout::MONO,
            );
            rs.flush(&mut resampled)?;
            if resampled.samples() == 0 {
                return Ok(());
            }
            self.samples
                .extend_from_slice(&resampled.plane::<f32>(0)[..resampled.samples()]);
        }
    }

    fn push(&mut self, frame: &frame::Audio) {
        if frame.samples() > 0 {
            self.samples
                .extend_from_slice(&frame.plane::<f32>(0)[..frame.samples()]);
        }
    }

    fn finish(mut self) -> Result<Vec<f32>, ffmpeg::Error> {
        self.flush()?;
        Ok(self.samples)
    }
}
