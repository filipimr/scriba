//! `media::probe`, `media::extract_audio`, `media::extract_audio_into`, `media::encode_audio`
//! e `media::encode_mix` com as fixtures reais e o FFmpeg estático.
//!
//! `guardar_audio_longo` (local) codifica o áudio longo de `SCRIBA_LONG_AUDIO`, mede tempo e
//! tamanho e deixa o `.m4a` em `%TEMP%\scriba-longo`:
//! `cargo test --release --test media -- --ignored --nocapture`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use ffmpeg_next as ffmpeg;
use scriba::media::{self, AUDIO_BIT_RATE, Error, SAMPLE_RATE};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn extract(path: &Path) -> Result<Vec<f32>, Error> {
    media::extract_audio(path, |_| {}, &AtomicBool::new(false))
}

fn secs(samples: &[f32]) -> f64 {
    samples.len() as f64 / f64::from(SAMPLE_RATE)
}

const FORMATS: [&str; 8] = [
    "fala.mp4",
    "fala.mp3",
    "fala.m4a",
    "fala.ogg",
    "fala.mkv",
    "fala.mov",
    "fala-48k-estereo.m4a",
    "fala.wav",
];

#[test]
fn todos_os_formatos_dao_a_duracao_do_wav() {
    let reference = secs(&extract(&fixture("fala.wav")).unwrap());
    assert!(reference > 10.0, "fala.wav com {reference} s");
    for name in FORMATS {
        let audio = extract(&fixture(name)).unwrap();
        let got = secs(&audio);
        let probe = media::probe(&fixture(name)).unwrap();
        let probed = probe.duration_ms as f64 / 1_000.0;
        println!("{name}: extraído {got:.3} s, probe {probed:.3} s (wav {reference:.3} s)");
        // Codecs com perda acrescentam alguns ms de atraso e enchimento.
        assert!((got - reference).abs() <= 0.15, "{name}: {got} s");
        // O vídeo do `fala.mp4` vai até 13,7 s (o `-shortest` antigo do xtask passava do
        // áudio), e o probe dá a duração do arquivo inteiro.
        let expected = if name == "fala.mp4" { 13.7 } else { reference };
        assert!(
            (probed - expected).abs() <= 0.15,
            "{name}: probe {probed} s"
        );
        assert!(audio.iter().all(|s| s.is_finite() && s.abs() <= 1.5));
        assert!(probe.has_audio, "{name}");
    }
}

#[test]
fn probe_le_video_e_taxa_de_bits() {
    let video = media::probe(&fixture("fala.mp4")).unwrap();
    assert!(video.has_video && video.has_audio);
    assert_eq!(video.resolution, Some((320, 240)));
    assert!(video.bit_rate.is_some_and(|b| b > 10_000), "{video:?}");

    let audio = media::probe(&fixture("fala.mp3")).unwrap();
    assert!(!audio.has_video && audio.has_audio);
    assert_eq!(audio.resolution, None);

    let mute = media::probe(&fixture("video-sem-audio.mp4")).unwrap();
    assert!(mute.has_video && !mute.has_audio);
}

#[test]
fn video_sem_audio_da_erro_amigavel() {
    let err = extract(&fixture("video-sem-audio.mp4")).unwrap_err();
    assert_eq!(err, Error::NoAudio);
    assert_eq!(
        err.to_string(),
        "Este arquivo não tem áudio. Escolha um vídeo com som."
    );
}

#[test]
fn bytes_que_nao_sao_midia_dao_erro_amigavel() {
    let tmp = tempfile::tempdir().unwrap();
    for (name, bytes) in [
        (
            "planilha.mp4",
            b"isto definitivamente nao e um video\n".repeat(200),
        ),
        ("vazio.mp3", Vec::new()),
        (
            "binario.wav",
            (0..4096u32).map(|i| (i * 7 % 251) as u8).collect(),
        ),
    ] {
        let path = tmp.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        let err = extract(&path).unwrap_err();
        assert!(
            matches!(err, Error::NotMedia | Error::Unreadable),
            "{name}: {err:?}"
        );
        assert!(media::probe(&path).is_err(), "{name}");
    }
    let err = media::probe(&tmp.path().join("nao-existe.mp4")).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Não reconheci este arquivo como vídeo ou áudio."
    );
}

#[test]
fn mp4_truncado_da_erro_amigavel_sem_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let bytes = std::fs::read(fixture("fala.mp4")).unwrap();
    for fraction in [0.1, 0.5, 0.9] {
        let path = tmp.path().join(format!("truncado-{fraction}.mp4"));
        std::fs::write(&path, &bytes[..(bytes.len() as f64 * fraction) as usize]).unwrap();
        match extract(&path) {
            Err(err) => {
                assert!(
                    matches!(err, Error::NotMedia | Error::Unreadable),
                    "{fraction}: {err:?}"
                );
                assert!(!err.to_string().is_empty());
            }
            // O FFmpeg pode aproveitar o que veio antes do corte.
            Ok(audio) => assert!(!audio.is_empty()),
        }
    }
}

#[test]
fn wav_truncado_aproveita_o_inicio() {
    let tmp = tempfile::tempdir().unwrap();
    let bytes = std::fs::read(fixture("fala.wav")).unwrap();
    let path = tmp.path().join("metade.wav");
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    let got = secs(&extract(&path).unwrap());
    assert!((4.0..7.0).contains(&got), "{got} s");
}

#[test]
fn progresso_cresce_e_chega_a_100() {
    for name in ["fala.mp4", "fala.wav", "fala-48k-estereo.m4a"] {
        let mut seen = Vec::new();
        media::extract_audio(&fixture(name), |p| seen.push(p), &AtomicBool::new(false)).unwrap();
        assert_eq!(seen.first(), Some(&0), "{name}: {seen:?}");
        assert_eq!(seen.last(), Some(&100), "{name}: {seen:?}");
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "{name}: {seen:?}");
        assert!(seen.len() > 5, "{name}: {seen:?}");
    }
}

#[test]
fn cancelar_interrompe() {
    let cancel = AtomicBool::new(false);
    let mut last = 0;
    let err = media::extract_audio(
        &fixture("fala.mp4"),
        |p| {
            last = p;
            if p >= 30 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    )
    .unwrap_err();
    assert_eq!(err, Error::Canceled);
    assert!(last < 100, "{last}");

    let canceled = AtomicBool::new(true);
    assert_eq!(
        media::extract_audio(&fixture("fala.wav"), |_| {}, &canceled).unwrap_err(),
        Error::Canceled
    );
}

#[test]
fn erros_viram_falha_do_job() {
    use scriba::jobs::Failure;
    assert_eq!(Failure::from(Error::Canceled), Failure::Canceled);
    assert_eq!(
        Failure::from(Error::Unreadable),
        Failure::Error("Não consegui ler o áudio deste arquivo. Ele pode estar corrompido.".into())
    );
}

#[test]
fn em_pedacos_entrega_o_mesmo_audio() {
    for name in ["fala.mp4", "fala-48k-estereo.m4a", "fala.wav"] {
        let whole = extract(&fixture(name)).unwrap();
        let mut chunks = 0;
        let mut joined = Vec::new();
        let total = media::extract_audio_into(
            &fixture(name),
            |samples| {
                chunks += 1;
                joined.extend_from_slice(samples);
                Ok(())
            },
            |_| {},
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(joined, whole, "{name}");
        assert_eq!(total, whole.len() as u64, "{name}");
        assert!(chunks > 10, "{name}: {chunks} pedaços");
    }
}

#[test]
fn disco_cheio_da_erro_amigavel() {
    let err = media::extract_audio_into(
        &fixture("fala.wav"),
        |_| Err(std::io::Error::other("disco cheio")),
        |_| {},
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(err, Error::Disk);
    assert_eq!(
        scriba::jobs::Failure::from(err),
        scriba::jobs::Failure::Error(
            "Não consegui gravar o áudio temporário. Confira o espaço livre em disco e tente \
             de novo."
                .into()
        )
    );
}

/// O que um `.m4a` gravado tem por dentro.
#[derive(Debug)]
struct M4a {
    aac: bool,
    channels: u16,
    rate: u32,
    /// Do contêiner.
    secs: f64,
    /// A marca do `ftyp`: `M4A ` é a que o celular reconhece como áudio.
    brand: [u8; 4],
}

fn inspect(path: &Path) -> M4a {
    // O probe inicializa o FFmpeg.
    let probe = media::probe(path).unwrap();
    assert!(probe.has_audio && !probe.has_video, "{probe:?}");
    let ictx = ffmpeg::format::input(path).unwrap();
    assert_eq!(ictx.streams().count(), 1, "só a faixa de áudio");
    let stream = ictx.streams().best(ffmpeg::media::Type::Audio).unwrap();
    let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .unwrap()
        .decoder()
        .audio()
        .unwrap();
    let bytes = std::fs::read(path).unwrap();
    M4a {
        aac: decoder.id() == ffmpeg::codec::Id::AAC,
        channels: decoder.channels(),
        rate: decoder.rate(),
        secs: probe.duration_ms as f64 / 1_000.0,
        brand: bytes[8..12].try_into().unwrap(),
    }
}

fn encode(input: &Path, output: &Path) -> Result<(), Error> {
    media::encode_audio(input, output, |_| {}, &AtomicBool::new(false))
}

#[test]
fn todos_os_formatos_viram_aac_mono_com_a_duracao_do_original() {
    let tmp = tempfile::tempdir().unwrap();
    for name in FORMATS {
        let original = secs(&extract(&fixture(name)).unwrap());
        // O `.parte` do app: sem extensão de mídia, o muxer vem pelo nome.
        let out = tmp.path().join(format!("{name}.m4a.parte"));
        encode(&fixture(name), &out).unwrap();
        let m4a = inspect(&out);
        let decoded = secs(&extract(&out).unwrap());
        let size = std::fs::metadata(&out).unwrap().len();
        println!(
            "{name}: original {original:.3} s → m4a {:.3} s (decodificado {decoded:.3} s), {} Hz, {size} bytes",
            m4a.secs, m4a.rate
        );
        assert!(m4a.aac && m4a.channels == 1, "{name}: {m4a:?}");
        assert_eq!(&m4a.brand, b"M4A ", "{name}");
        // O `fala-48k-estereo.m4a` cai para o teto; os outros já vêm a 16 ou 22,05 kHz.
        let rate = ffmpeg::format::input(&fixture(name))
            .unwrap()
            .streams()
            .best(ffmpeg::media::Type::Audio)
            .map(|s| {
                ffmpeg::codec::context::Context::from_parameters(s.parameters())
                    .unwrap()
                    .decoder()
                    .audio()
                    .unwrap()
                    .rate()
            })
            .unwrap();
        assert_eq!(m4a.rate, media::audio_rate(rate), "{name}");
        // A duração é a do áudio do original, não a do contêiner (o vídeo do `fala.mp4`
        // passa do áudio).
        assert!((m4a.secs - original).abs() <= 0.1, "{name}: {m4a:?}");
        assert!((decoded - original).abs() <= 0.1, "{name}: {decoded} s");
        // 64 kbps, com folga para o cabeçalho e para o encoder abaixo da meta.
        let max = (AUDIO_BIT_RATE as f64 / 8.0 * original * 1.2) as u64 + 4_096;
        assert!(size <= max, "{name}: {size} bytes");
    }
}

#[test]
fn guardar_o_audio_tem_progresso_e_o_silencio_sai_valido() {
    let tmp = tempfile::tempdir().unwrap();
    let mut seen = Vec::new();
    let out = tmp.path().join("fala.m4a");
    media::encode_audio(
        &fixture("fala.mp4"),
        &out,
        |p| seen.push(p),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(seen.first(), Some(&0), "{seen:?}");
    assert_eq!(seen.last(), Some(&100), "{seen:?}");
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");

    let out = tmp.path().join("silencio.m4a");
    encode(&fixture("silencio.wav"), &out).unwrap();
    let m4a = inspect(&out);
    let original = secs(&extract(&fixture("silencio.wav")).unwrap());
    assert!((m4a.secs - original).abs() <= 0.1, "{m4a:?}");
}

#[test]
fn cancelar_a_gravacao_nao_deixa_arquivo() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("cancelado.m4a");
    let cancel = AtomicBool::new(false);
    let err = media::encode_audio(
        &fixture("fala.wav"),
        &out,
        |p| {
            if p >= 30 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    )
    .unwrap_err();
    assert_eq!(err, Error::Canceled);
    assert!(!out.exists());
}

#[test]
fn guardar_sem_audio_ou_em_pasta_invalida_da_erro_amigavel() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("mudo.m4a");
    assert_eq!(
        encode(&fixture("video-sem-audio.mp4"), &out).unwrap_err(),
        Error::NoAudio
    );
    assert!(!out.exists());

    let err = encode(&fixture("fala.wav"), &tmp.path().join("nao/existe.m4a")).unwrap_err();
    assert_eq!(err, Error::Encode);
    assert_eq!(
        scriba::jobs::Failure::from(err),
        scriba::jobs::Failure::Error(
            "Não consegui guardar o áudio. Confira o espaço livre e se a pasta permite gravação, e tente de novo."
                .into()
        )
    );
}

fn rms(samples: &[f32]) -> f64 {
    (samples
        .iter()
        .map(|&s| f64::from(s) * f64::from(s))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt()
}

fn mix(inputs: &[&Path], output: &Path) -> Result<(), Error> {
    media::encode_mix(inputs, output, |_| {}, &AtomicBool::new(false))
}

/// Uma trilha de gravação curta: `secs` de um tom, a 32 kHz mono 16-bit.
fn short_track(path: &Path, secs: f64) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 32_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..(secs * 32_000.0) as u32 {
        let t = f64::from(i) / 32_000.0;
        w.write_sample(((t * 440.0 * std::f64::consts::TAU).sin() * 8_000.0) as i16)
            .unwrap();
    }
    w.finalize().unwrap();
}

/// "Guardar áudio" de uma gravação: as duas trilhas viram um `.m4a` só, com a duração da
/// mais longa e as duas vozes.
#[test]
fn gravacao_vira_um_m4a_com_as_duas_trilhas() {
    let tmp = tempfile::tempdir().unwrap();
    let (voce, outros) = (fixture("voce.wav"), fixture("outros.wav"));
    let out = tmp.path().join("Gravação.m4a.parte");
    let mut seen = Vec::new();
    media::encode_mix(
        &[&voce, &outros],
        &out,
        |p| seen.push(p),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(seen.first(), Some(&0), "{seen:?}");
    assert_eq!(seen.last(), Some(&100), "{seen:?}");
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");

    let m4a = inspect(&out);
    let original = secs(&extract(&voce).unwrap());
    println!("mistura: {m4a:?}, trilha {original:.3} s");
    assert!(
        m4a.aac && m4a.channels == 1 && m4a.rate == 32_000,
        "{m4a:?}"
    );
    assert_eq!(&m4a.brand, b"M4A ");
    assert!((m4a.secs - original).abs() <= 0.1, "{m4a:?}");
    // As falas não se sobrepõem: a mistura tem a energia das duas.
    let (mixed, alone) = (extract(&out).unwrap(), extract(&voce).unwrap());
    assert!(
        rms(&mixed) > rms(&alone) * 1.2,
        "{} × {}",
        rms(&mixed),
        rms(&alone)
    );

    // Trilhas de tamanhos diferentes: a mais curta vira silêncio no fim.
    let short = tmp.path().join("curta.wav");
    short_track(&short, 2.0);
    let out = tmp.path().join("diferentes.m4a");
    mix(&[&short, &outros], &out).unwrap();
    assert!((inspect(&out).secs - original).abs() <= 0.1);
    let out = tmp.path().join("so-curta.m4a");
    mix(&[&short], &out).unwrap();
    assert!((inspect(&out).secs - 2.0).abs() <= 0.1);
}

#[test]
fn cancelar_a_mistura_nao_deixa_arquivo() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("cancelado.m4a");
    let cancel = AtomicBool::new(false);
    let err = media::encode_mix(
        &[&fixture("voce.wav"), &fixture("outros.wav")],
        &out,
        |p| {
            if p >= 30 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    )
    .unwrap_err();
    assert_eq!(err, Error::Canceled);
    assert!(!out.exists());
}

#[test]
fn mistura_fora_do_formato_da_erro_amigavel() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("ruim.m4a");
    // O `fala.wav` é de 16 kHz: não é uma trilha da gravação junto da outra.
    for inputs in [
        vec![fixture("voce.wav"), fixture("fala.wav")],
        vec![fixture("fala.mp3")],
        vec![tmp.path().join("nao-existe.wav")],
    ] {
        let inputs: Vec<&Path> = inputs.iter().map(PathBuf::as_path).collect();
        assert_eq!(
            mix(&inputs, &out).unwrap_err(),
            Error::Unreadable,
            "{inputs:?}"
        );
        assert!(!out.exists());
    }
    assert_eq!(mix(&[], &out).unwrap_err(), Error::NotMedia);
    let err = mix(&[&fixture("voce.wav")], &tmp.path().join("nao/existe.m4a")).unwrap_err();
    assert_eq!(err, Error::Encode);
}

#[test]
#[ignore = "audio longo"]
fn guardar_audio_longo() {
    let path = PathBuf::from(
        std::env::var("SCRIBA_LONG_AUDIO").expect("defina SCRIBA_LONG_AUDIO com um áudio longo"),
    );
    let dir = std::env::temp_dir().join("scriba-longo");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join(format!(
        "{}.m4a",
        path.file_name().unwrap().to_string_lossy()
    ));
    let t0 = Instant::now();
    encode(&path, &out).unwrap();
    let took = t0.elapsed().as_secs_f64();
    let m4a = inspect(&out);
    let size = std::fs::metadata(&out).unwrap().len();
    let original = media::probe(&path).unwrap().duration_ms as f64 / 1_000.0;
    println!(
        "{}: {original:.1} s de mídia → {} em {took:.1} s ({:.0}× tempo real), {:.1} MB ({:.1} MB/h), {} Hz, {:.3} s",
        path.display(),
        out.display(),
        original / took,
        size as f64 / 1e6,
        size as f64 / 1e6 / (m4a.secs / 3_600.0),
        m4a.rate,
        m4a.secs,
    );
    assert!(m4a.aac && m4a.channels == 1, "{m4a:?}");
}
