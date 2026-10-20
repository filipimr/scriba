//! Mídia pelo FFmpeg estático: `probe` lê a duração, a taxa de bits e a resolução do
//! original, e `extract_audio` entrega a melhor faixa de áudio em 16 kHz mono f32, sem
//! decodificar o vídeo. `extract_audio_into` entrega o áudio aos poucos (para o [`Spool`],
//! em disco), sem segurar a duração inteira na memória. `encode_audio` guarda o áudio do
//! original num `.m4a` AAC mono.

mod spool;

use std::fmt;
use std::io;
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

pub use spool::Spool;

/// Taxa de amostragem que o whisper espera.
pub const SAMPLE_RATE: u32 = 16_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NotMedia,
    NoAudio,
    Unreadable,
    /// O áudio extraído não coube no disco.
    Disk,
    /// O `.m4a` não foi gravado (pasta sem permissão, disco cheio).
    Encode,
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
            Self::Disk => {
                "Não consegui gravar o áudio temporário. Confira o espaço livre em disco e tente de novo."
            }
            Self::Encode => {
                "Não consegui guardar o áudio. Confira o espaço livre e se a pasta permite gravação, e tente de novo."
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
    progress: impl FnMut(u8),
    cancel: &AtomicBool,
) -> Result<Vec<f32>, Error> {
    let mut samples = Vec::new();
    extract_audio_into(
        path,
        |chunk| {
            samples.extend_from_slice(chunk);
            Ok(())
        },
        progress,
        cancel,
    )?;
    Ok(samples)
}

/// Como [`extract_audio`], mas entrega as amostras a `sink` aos poucos, na ordem, e devolve
/// quantas foram. Um erro do `sink` (disco cheio) vira [`Error::Disk`].
pub fn extract_audio_into(
    path: &Path,
    mut sink: impl FnMut(&[f32]) -> io::Result<()>,
    progress: impl FnMut(u8),
    cancel: &AtomicBool,
) -> Result<u64, Error> {
    Source::open(path)?.decode(
        SAMPLE_RATE,
        |samples| {
            sink(samples).map_err(|e| {
                eprintln!("media: não consegui guardar o áudio extraído: {e}");
                Error::Disk
            })
        },
        progress,
        cancel,
    )
}

/// Taxa de bits do `.m4a` guardado: AAC-LC mono a 64 kbps é transparente para fala com o
/// encoder nativo do FFmpeg, ~29 MB por hora.
pub const AUDIO_BIT_RATE: usize = 64_000;

/// Teto da taxa de amostragem do `.m4a`: 32 kHz dá ~15 kHz de banda, que cobre a fala e
/// ainda deixa a música de fundo aceitável. Acima disso, os bits iriam para o que não se
/// ouve numa reunião.
pub const AUDIO_MAX_RATE: u32 = 32_000;

/// Taxas que o AAC aceita, até o teto.
const AAC_RATES: [u32; 7] = [8_000, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000];

/// A taxa do `.m4a` para um original em `input` Hz: a do original, se o AAC a aceita e ela
/// está abaixo do teto; senão a menor taxa AAC acima dela, nunca acima de
/// [`AUDIO_MAX_RATE`]. Desconhecida (0) vira o teto.
pub fn audio_rate(input: u32) -> u32 {
    if input == 0 {
        return AUDIO_MAX_RATE;
    }
    AAC_RATES
        .into_iter()
        .find(|&r| r >= input)
        .unwrap_or(AUDIO_MAX_RATE)
}

/// Recodifica o áudio de `input` (a melhor faixa; um vídeo vira só o som) num `.m4a` AAC-LC
/// mono em `output`, a [`AUDIO_BIT_RATE`] e na taxa de [`audio_rate`]. `progress` e
/// `cancel` funcionam como na [`extract_audio`]. Em qualquer erro, inclusive o
/// cancelamento, o arquivo parcial é apagado.
pub fn encode_audio(
    input: &Path,
    output: &Path,
    progress: impl FnMut(u8),
    cancel: &AtomicBool,
) -> Result<(), Error> {
    let result = encode(input, output, progress, cancel);
    // O muxer já foi fechado: o Windows deixa apagar.
    if result.is_err()
        && let Err(e) = std::fs::remove_file(output)
        && e.kind() != io::ErrorKind::NotFound
    {
        eprintln!("media: não apaguei {}: {e}", output.display());
    }
    result
}

fn encode(
    input: &Path,
    output: &Path,
    progress: impl FnMut(u8),
    cancel: &AtomicBool,
) -> Result<(), Error> {
    let mut source = Source::open(input)?;
    let rate = audio_rate(source.decoder.rate());
    let mut writer = AacWriter::create(output, rate)?;
    source.decode(rate, |samples| writer.push(samples), progress, cancel)?;
    writer.finish()
}

/// A melhor faixa de áudio de um arquivo, aberta e pronta para decodificar.
struct Source<'a> {
    path: &'a Path,
    ictx: ffmpeg::format::context::Input,
    index: usize,
    decoder: ffmpeg::decoder::Audio,
    /// Do contêiner, para o progresso; 0 quando ele não informa.
    duration_ms: u64,
    time_base: f64,
    start: i64,
}

impl<'a> Source<'a> {
    fn open(path: &'a Path) -> Result<Self, Error> {
        let ictx = open(path)?;
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
        let decoder = codec::context::Context::from_parameters(
            ictx.stream(index).expect("faixa escolhida").parameters(),
        )
        .and_then(|c| c.decoder().audio())
        .map_err(|e| unreadable(path, "decoder", e))?;
        Ok(Self {
            path,
            ictx,
            index,
            decoder,
            duration_ms,
            time_base,
            start,
        })
    }

    /// Decodifica tudo e entrega ao `sink`, na ordem, as amostras reamostradas para `rate`
    /// Hz mono f32; devolve quantas foram. `progress` recebe de 0 a 100 (pelo pts sobre a
    /// duração), só quando o valor muda; `cancel` é checado a cada pacote.
    fn decode(
        &mut self,
        rate: u32,
        mut sink: impl FnMut(&[f32]) -> Result<(), Error>,
        mut progress: impl FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<u64, Error> {
        let path = self.path;
        let mut out = Decoded::new(rate);
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
            match packet.read(&mut self.ictx) {
                Ok(()) => {}
                Err(ffmpeg::Error::Eof) => break,
                Err(e) => return Err(unreadable(path, "leitura", e)),
            }
            if packet.stream() != self.index {
                continue;
            }
            if self.duration_ms > 0
                && let Some(pts) = packet.pts()
            {
                let ms = (pts - self.start) as f64 * self.time_base * 1_000.0;
                report((ms / self.duration_ms as f64 * 100.0).clamp(0.0, 99.0) as u8);
            }
            // Um pacote ruim no meio não derruba o arquivo inteiro.
            if self.decoder.send_packet(&packet).is_err() {
                bad_packets += 1;
                continue;
            }
            out.receive(&mut self.decoder)
                .map_err(|e| unreadable(path, "reamostragem", e))?;
            out.drain(&mut sink)?;
        }
        if self.decoder.send_eof().is_ok() {
            out.receive(&mut self.decoder)
                .map_err(|e| unreadable(path, "reamostragem", e))?;
        }
        out.flush()
            .map_err(|e| unreadable(path, "reamostragem", e))?;
        out.drain(&mut sink)?;
        if out.total == 0 {
            eprintln!(
                "media: {} sem amostras ({bad_packets} pacotes ruins)",
                path.display()
            );
            return Err(Error::Unreadable);
        }
        report(100);
        Ok(out.total)
    }
}

fn unreadable(path: &Path, step: &str, e: ffmpeg::Error) -> Error {
    eprintln!("media: {} falhou em {step}: {e}", path.display());
    Error::Unreadable
}

/// O `.m4a` em gravação: corta as amostras em frames do tamanho que o encoder AAC pede e
/// grava os pacotes no MP4 (muxer `ipod`, o do `.m4a`).
struct AacWriter<'a> {
    path: &'a Path,
    octx: ffmpeg::format::context::Output,
    encoder: ffmpeg::encoder::Audio,
    rate: u32,
    frame_size: usize,
    /// Amostras que ainda não completam um frame.
    pending: Vec<f32>,
    /// Em amostras: a base de tempo do encoder é 1/rate.
    pts: i64,
    stream_time_base: ffmpeg::Rational,
}

impl<'a> AacWriter<'a> {
    fn create(path: &'a Path, rate: u32) -> Result<Self, Error> {
        let fail = |step: &str, e: ffmpeg::Error| encode_error(path, step, e);
        let codec = ffmpeg::encoder::find(codec::Id::AAC)
            .ok_or_else(|| fail("encoder", ffmpeg::Error::EncoderNotFound))?;
        // Pelo nome do muxer: o arquivo temporário do chamador não tem extensão de mídia.
        let mut octx = ffmpeg::format::output_as(path, "ipod").map_err(|e| fail("arquivo", e))?;
        let global_header = octx
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER);
        let time_base = ffmpeg::Rational::new(1, rate as i32);
        let mut encoder = codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .map_err(|e| fail("encoder", e))?;
        encoder.set_rate(rate as i32);
        encoder.set_channel_layout(ChannelLayout::MONO);
        encoder.set_format(Sample::F32(sample::Type::Planar));
        encoder.set_bit_rate(AUDIO_BIT_RATE);
        encoder.set_time_base(time_base);
        if global_header {
            encoder.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let encoder = encoder.open_as(codec).map_err(|e| fail("encoder", e))?;
        let mut stream = octx.add_stream(codec).map_err(|e| fail("faixa", e))?;
        stream.set_parameters(&encoder);
        stream.set_time_base(time_base);
        // O índice no começo do arquivo: o celular e os apps de mensagem tocam sem
        // precisar baixar tudo.
        let mut options = ffmpeg::Dictionary::new();
        options.set("movflags", "+faststart");
        octx.write_header_with(options)
            .map_err(|e| fail("cabeçalho", e))?;
        let stream_time_base = octx.stream(0).expect("faixa criada").time_base();
        let frame_size = (encoder.frame_size() as usize).max(1);
        Ok(Self {
            path,
            octx,
            encoder,
            rate,
            frame_size,
            pending: Vec::with_capacity(frame_size * 2),
            pts: 0,
            stream_time_base,
        })
    }

    /// Acrescenta amostras e codifica cada frame completo.
    fn push(&mut self, samples: &[f32]) -> Result<(), Error> {
        self.pending.extend_from_slice(samples);
        let full = self.pending.len() / self.frame_size * self.frame_size;
        if full == 0 {
            return Ok(());
        }
        let pending = std::mem::take(&mut self.pending);
        for frame in pending[..full].chunks(self.frame_size) {
            self.send(frame)?;
        }
        self.pending = pending;
        self.pending.drain(..full);
        Ok(())
    }

    /// Codifica o resto (o AAC aceita o último frame menor), esvazia o encoder e fecha o
    /// arquivo.
    fn finish(mut self) -> Result<(), Error> {
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            self.send(&rest)?;
        }
        self.encoder
            .send_eof()
            .map_err(|e| encode_error(self.path, "fim", e))?;
        self.write_packets()?;
        self.octx
            .write_trailer()
            .map_err(|e| encode_error(self.path, "índice", e))
    }

    fn send(&mut self, samples: &[f32]) -> Result<(), Error> {
        let mut frame = frame::Audio::new(
            Sample::F32(sample::Type::Planar),
            samples.len(),
            ChannelLayout::MONO,
        );
        frame.set_rate(self.rate);
        frame.plane_mut::<f32>(0)[..samples.len()].copy_from_slice(samples);
        frame.set_pts(Some(self.pts));
        self.pts += samples.len() as i64;
        self.encoder
            .send_frame(&frame)
            .map_err(|e| encode_error(self.path, "codificação", e))?;
        self.write_packets()
    }

    fn write_packets(&mut self) -> Result<(), Error> {
        let time_base = ffmpeg::Rational::new(1, self.rate as i32);
        let mut packet = Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(0);
            packet.rescale_ts(time_base, self.stream_time_base);
            packet
                .write_interleaved(&mut self.octx)
                .map_err(|e| encode_error(self.path, "gravação", e))?;
        }
        Ok(())
    }
}

fn encode_error(path: &Path, step: &str, e: ffmpeg::Error) -> Error {
    eprintln!("media: {} falhou em {step}: {e}", path.display());
    Error::Encode
}

/// Recebe os frames do decoder e reamostra para `rate` Hz mono f32. As amostras ficam em
/// `samples` só até o próximo [`Decoded::drain`].
struct Decoded {
    rate: u32,
    // O resampler nasce do primeiro frame: o layout informado pelo decoder antes de
    // decodificar pode vir vazio (WAV mono) e o swr recusa com "Input changed". Se o
    // formato muda no meio do arquivo, ele é recriado.
    resampler: Option<resampling::Context>,
    samples: Vec<f32>,
    /// Amostras já entregues.
    total: u64,
}

impl Decoded {
    fn new(rate: u32) -> Self {
        Self {
            rate,
            resampler: None,
            samples: Vec::new(),
            total: 0,
        }
    }

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
                    self.rate,
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

    /// Entrega ao `sink` o que já foi reamostrado.
    fn drain(&mut self, sink: &mut impl FnMut(&[f32]) -> Result<(), Error>) -> Result<(), Error> {
        if self.samples.is_empty() {
            return Ok(());
        }
        sink(&self.samples)?;
        self.total += self.samples.len() as u64;
        self.samples.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taxa_do_m4a_nunca_sobe_e_tem_teto() {
        for (input, expected) in [
            (0, 32_000),
            (7_000, 8_000),
            (16_000, 16_000),
            (20_000, 22_050),
            (22_050, 22_050),
            (24_000, 24_000),
            (32_000, 32_000),
            (44_100, 32_000),
            (48_000, 32_000),
            (96_000, 32_000),
        ] {
            assert_eq!(audio_rate(input), expected, "{input} Hz");
        }
    }
}
