//! `scriba --self-test`: prova, no exe de release, que o modelo e o VAD carregam da memória,
//! que o pipeline do app (`media` + `asr`) transcreve pt-BR em CPU e que o FFmpeg estático
//! codifica H.264.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, bail, ensure};
use ffmpeg::format::Pixel;
use ffmpeg::util::frame;
use ffmpeg::{Dictionary, Rational, codec, encoder};
use ffmpeg_next as ffmpeg;

use crate::asr::{self, Asr, Model, Models, Plan, Source};
use crate::media;

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

/// Variável com o tempo máximo da transcrição, em segundos. Só o `release.yml` a define:
/// um whisper.cpp compilado sem otimização leva dezenas de segundos e falha aqui.
pub const MAX_SECS_VAR: &str = "SCRIBA_SELF_TEST_MAX_SECS";

pub struct Report {
    pub text: String,
    pub transcribe: Duration,
    pub encode: Duration,
    /// Recursos de CPU com que o whisper.cpp foi compilado.
    pub system: &'static str,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "transcrição {:.1?}, x264 {:.1?} → {}\nwhisper.cpp: {}",
            self.transcribe,
            self.encode,
            self.text.trim(),
            self.system.trim()
        )
    }
}

/// Usa o modelo, o VAD e a amostra embutidos; sem a feature `embedded-test-model`, lê de
/// `models/` e `tests/fixtures/` do repositório. Falha se a transcrição passar do limite
/// de [`MAX_SECS_VAR`], quando definido.
pub fn run_default() -> anyhow::Result<Report> {
    let report = match (
        scriba_model::TEST_WHISPER,
        scriba_model::VAD,
        scriba_model::TEST_SAMPLE,
    ) {
        (Some(model), Some(vad), Some(sample)) => {
            run(Source::Embedded(model), Source::Embedded(vad), sample)?
        }
        _ => {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"));
            let file = |name: &str| {
                let path = root.join("models").join(name);
                ensure!(
                    path.is_file(),
                    "{name} não embutido: rode `cargo xtask model --only base,silero`"
                );
                Ok(Source::File(path))
            };
            let sample = std::fs::read(root.join("tests/fixtures/fala.wav"))?;
            run(file(asr::BASE_FILE)?, file(asr::VAD_FILE)?, &sample)?
        }
    };
    check_limit(
        report.transcribe,
        std::env::var(MAX_SECS_VAR).ok().as_deref(),
    )?;
    Ok(report)
}

fn check_limit(transcribe: Duration, max_secs: Option<&str>) -> anyhow::Result<()> {
    let Some(max) = max_secs else {
        return Ok(());
    };
    let max: f64 = max
        .parse()
        .with_context(|| format!("{MAX_SECS_VAR} inválido: {max:?}"))?;
    ensure!(
        transcribe.as_secs_f64() <= max,
        "a transcrição levou {:.1} s, acima do limite de {max} s ({MAX_SECS_VAR}): \
         o whisper.cpp pode ter sido compilado sem otimização",
        transcribe.as_secs_f64()
    );
    Ok(())
}

/// Transcreve `sample` (qualquer formato que o FFmpeg leia) com o modelo base na CPU e
/// codifica um clipe x264.
pub fn run(model: Source, vad: Source, sample: &[u8]) -> anyhow::Result<Report> {
    let input = temp_path("scriba-self-test.wav");
    std::fs::write(&input, sample)?;
    let never = AtomicBool::new(false);
    let audio = media::extract_audio(&input, |_| {}, &never);
    let _ = std::fs::remove_file(&input);
    let audio = audio?;

    let models = Models {
        large: None,
        cpu: Some(Model {
            name: "base",
            source: model,
        }),
        vad: Some(vad),
    };
    let t0 = Instant::now();
    let transcription = Asr::new(vec![Plan::cpu()], models).transcribe(&audio, |_| {}, &never)?;
    let transcribe = t0.elapsed();
    let text = transcription
        .segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
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
        system: whisper_rs::print_system_info(),
    })
}

fn keyword_hits(text: &str) -> usize {
    let t = text.to_lowercase();
    KEYWORDS.iter().filter(|k| t.contains(*k)).count()
}

/// Único por chamada: dois self-tests no mesmo processo (os testes) não dividem o arquivo.
fn temp_path(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{}-{n}-{name}", std::process::id()))
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
        assert_eq!(keyword_hits("nada aqui"), 0);
    }

    #[test]
    fn amostra_que_nao_e_audio_falha() {
        let Err(err) = run(
            Source::Embedded(b""),
            Source::Embedded(b""),
            b"isto nao e audio",
        ) else {
            panic!("bytes que não são áudio passaram");
        };
        assert!(err.to_string().contains("Não reconheci"), "{err}");
    }

    #[test]
    fn report_mostra_texto_tempos_e_recursos_da_cpu() {
        let r = Report {
            text: " bom dia ".into(),
            transcribe: Duration::from_millis(1500),
            encode: Duration::from_millis(200),
            system: "CPU : AVX2 = 1 | ",
        };
        let s = r.to_string();
        assert!(s.contains("bom dia") && s.contains("x264"), "{s}");
        assert!(s.ends_with("whisper.cpp: CPU : AVX2 = 1 |"), "{s}");
    }

    #[test]
    fn limite_de_tempo_so_quando_definido() {
        let t = Duration::from_secs_f64(12.5);
        assert!(check_limit(t, None).is_ok());
        assert!(check_limit(t, Some("20")).is_ok());
        let err = check_limit(t, Some("10")).unwrap_err().to_string();
        assert!(
            err.contains("12.5 s") && err.contains("sem otimização"),
            "{err}"
        );
        assert!(check_limit(t, Some("vinte")).is_err());
    }
}
