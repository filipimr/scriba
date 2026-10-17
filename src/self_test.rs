//! `scriba --self-test`: prova, no exe de release, que o modelo carrega da memória,
//! que o whisper transcreve pt-BR em CPU e que o FFmpeg estático codifica H.264.

use std::fmt;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, bail, ensure};
use ffmpeg::format::Pixel;
use ffmpeg::util::frame;
use ffmpeg::{Dictionary, Rational, codec, encoder};
use ffmpeg_next as ffmpeg;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Palavras ditas na amostra `tests/fixtures/fala.wav`.
const KEYWORDS: &[&str] = &[
    "bom dia",
    "scriba",
    "reunião",
    "orçamento",
    "amanhã",
    "nove horas",
];
const MIN_HITS: usize = 3;

pub struct Report {
    pub text: String,
    pub transcribe: Duration,
    pub encode: Duration,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "transcrição {:.1?}, x264 {:.1?} → {}",
            self.transcribe,
            self.encode,
            self.text.trim()
        )
    }
}

/// Usa o modelo e a amostra embutidos; sem a feature `embedded-test-model`, lê de
/// `models/` e `tests/fixtures/` do repositório.
pub fn run_default() -> anyhow::Result<Report> {
    match (scriba_model::TEST_WHISPER, scriba_model::TEST_SAMPLE) {
        (Some(model), Some(sample)) => run(model, sample),
        _ => {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"));
            let model = std::fs::read(root.join("models/ggml-base-q5_1.bin"))
                .context("modelo não embutido: rode `cargo xtask model --only base`")?;
            let sample = std::fs::read(root.join("tests/fixtures/fala.wav"))?;
            run(&model, &sample)
        }
    }
}

pub fn run(model: &[u8], wav: &[u8]) -> anyhow::Result<Report> {
    let t0 = Instant::now();
    let text = transcribe_cpu(model, &read_wav_f32(wav)?)?;
    let transcribe = t0.elapsed();
    let hits = keyword_hits(&text);
    if hits < MIN_HITS {
        bail!("só {hits} de {MIN_HITS} palavras-chave na transcrição: {text:?}");
    }

    let t1 = Instant::now();
    let out = temp_path("scriba-self-test.mp4");
    let secs = encode_gray_clip(&out);
    let _ = std::fs::remove_file(&out);
    let secs = secs?;
    ensure!(
        (secs - 1.0).abs() <= 0.15,
        "clipe x264 com {secs:.2} s, esperado 1 s"
    );
    Ok(Report {
        text,
        transcribe,
        encode: t1.elapsed(),
    })
}

/// Lê um WAV 16 kHz mono 16-bit como f32 em [-1, 1].
fn read_wav_f32(wav: &[u8]) -> anyhow::Result<Vec<f32>> {
    let mut r = hound::WavReader::new(Cursor::new(wav))?;
    let spec = r.spec();
    ensure!(
        spec.channels == 1 && spec.sample_rate == 16_000 && spec.bits_per_sample == 16,
        "amostra precisa ser WAV 16 kHz mono 16-bit: {spec:?}"
    );
    Ok(r.samples::<i16>()
        .map(|s| s.map(|v| f32::from(v) / 32768.0))
        .collect::<Result<_, _>>()?)
}

fn transcribe_cpu(model: &[u8], audio: &[f32]) -> anyhow::Result<String> {
    let mut cp = WhisperContextParameters::default();
    cp.use_gpu(false);
    let ctx = WhisperContext::new_from_buffer_with_params(model, cp)?;
    let mut state = ctx.create_state()?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("pt"));
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    params.set_n_threads(cores.saturating_sub(2).max(1) as i32);
    params.set_no_context(true);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    state.full(params, audio)?;
    Ok(state.as_iter().map(|seg| seg.to_string()).collect())
}

fn keyword_hits(text: &str) -> usize {
    let t = text.to_lowercase();
    KEYWORDS.iter().filter(|k| t.contains(*k)).count()
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{}-{name}", std::process::id()))
}

/// Codifica 1 s de vídeo cinza 320x240 a 10 fps com libx264 em MP4 e devolve a duração lida.
fn encode_gray_clip(out: &Path) -> anyhow::Result<f64> {
    ffmpeg::init()?;
    let codec = encoder::find_by_name("libx264").context("libx264 ausente no FFmpeg")?;
    let mut octx = ffmpeg::format::output(out)?;
    let global_header = octx
        .format()
        .flags()
        .contains(ffmpeg::format::Flags::GLOBAL_HEADER);
    let mut ost = octx.add_stream(codec)?;
    let mut enc = codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()?;
    let tb = Rational::new(1, 10);
    enc.set_width(320);
    enc.set_height(240);
    enc.set_format(Pixel::YUV420P);
    enc.set_time_base(tb);
    enc.set_frame_rate(Some(Rational::new(10, 1)));
    if global_header {
        enc.set_flags(codec::Flags::GLOBAL_HEADER);
    }
    let mut enc = enc.open_as_with(codec, Dictionary::new())?;
    ost.set_parameters(&enc);
    ost.set_time_base(tb);
    octx.write_header()?;
    let ost_tb = octx.stream(0).context("stream de saída")?.time_base();

    let drain = |enc: &mut encoder::Video,
                 octx: &mut ffmpeg::format::context::Output|
     -> anyhow::Result<()> {
        let mut pkt = ffmpeg::Packet::empty();
        while enc.receive_packet(&mut pkt).is_ok() {
            pkt.set_stream(0);
            pkt.rescale_ts(tb, ost_tb);
            pkt.write_interleaved(octx)?;
        }
        Ok(())
    };
    let mut f = frame::Video::new(Pixel::YUV420P, 320, 240);
    for p in 0..3 {
        f.data_mut(p).fill(128);
    }
    for i in 0..10 {
        f.set_pts(Some(i));
        enc.send_frame(&f)?;
        drain(&mut enc, &mut octx)?;
    }
    enc.send_eof()?;
    drain(&mut enc, &mut octx)?;
    octx.write_trailer()?;
    drop(octx);

    let ictx = ffmpeg::format::input(out)?;
    Ok(ictx.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conta_palavras_chave_sem_caixa() {
        assert_eq!(keyword_hits("Bom dia, SCRIBA! A reunião é amanhã."), 4);
        assert_eq!(keyword_hits("nada aqui"), 1); // quebrado de propósito
    }

    #[test]
    fn recusa_wav_fora_do_formato() {
        let mut buf = Cursor::new(Vec::new());
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        hound::WavWriter::new(&mut buf, spec)
            .unwrap()
            .finalize()
            .unwrap();
        assert!(read_wav_f32(buf.get_ref()).is_err());
    }

    #[test]
    fn report_mostra_texto_e_tempos() {
        let r = Report {
            text: " bom dia ".into(),
            transcribe: Duration::from_millis(1500),
            encode: Duration::from_millis(200),
        };
        let s = r.to_string();
        assert!(s.contains("bom dia") && s.contains("x264"), "{s}");
    }
}
