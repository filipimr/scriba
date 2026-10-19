//! `media::probe` e `media::extract_audio` com as fixtures reais e o FFmpeg estático.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use scriba::media::{self, Error, SAMPLE_RATE};

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
