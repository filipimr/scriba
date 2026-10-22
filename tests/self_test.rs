//! Self-test com o modelo da CPU de `models/` (o `small` ou, como no CI, o `base`) e o VAD
//! (`cargo xtask model --only base,silero`); na suíte de hardware, como a release: o backend
//! que o app escolheria, com o large.

use scriba::asr::{BASE_FILE, Model, Models, Source, VAD_FILE};
use scriba::self_test::Large;

mod common;

/// Os modelos de `models/` sem o large: os candidatos são só a CPU, como no runner.
#[test]
fn self_test_transcreve_e_codifica() {
    let models = Models {
        large: None,
        ..Models::from_dir(&common::models_dir())
    };
    let sample = std::fs::read(common::fixture("fala.wav")).unwrap();
    let report = scriba::self_test::run_with(models, &sample).unwrap();
    println!("{report}");
    assert!(!report.gpu);
    assert_eq!(report.large, Large::Absent);
}

#[test]
fn self_test_falha_com_audio_sem_fala() {
    let model = Model {
        name: "base",
        source: Source::File(common::models_dir().join(BASE_FILE)),
    };
    let vad = Source::File(common::models_dir().join(VAD_FILE));
    let silence = std::fs::read(common::fixture("silencio.wav")).unwrap();
    let Err(err) = scriba::self_test::run(model, vad, &silence) else {
        panic!("silêncio passou no self-test");
    };
    assert!(err.to_string().contains("palavras-chave"), "{err}");
}

/// Como a release na RTX: o large de `models/` transcreve na GPU (`--release --features
/// vulkan`, com `cargo xtask model --only large,small,silero`).
#[test]
#[ignore = "hardware"]
fn self_test_na_gpu_com_o_large() {
    let report = scriba::self_test::run_default().unwrap();
    println!("{report}");
    assert!(report.gpu, "{report}");
    assert_eq!(report.large, Large::Used);
}
