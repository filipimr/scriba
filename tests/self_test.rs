//! Self-test completo com o modelo base e o VAD de `models/`
//! (`cargo xtask model --only base,silero`).

use scriba::asr::{BASE_FILE, Source, VAD_FILE};

mod common;

#[test]
fn self_test_transcreve_e_codifica() {
    let report = scriba::self_test::run_default().unwrap();
    println!("{report}");
}

#[test]
fn self_test_falha_com_audio_sem_fala() {
    let model = Source::File(common::models_dir().join(BASE_FILE));
    let vad = Source::File(common::models_dir().join(VAD_FILE));
    let silence = std::fs::read(common::fixture("silencio.wav")).unwrap();
    let Err(err) = scriba::self_test::run(model, vad, &silence) else {
        panic!("silêncio passou no self-test");
    };
    assert!(err.to_string().contains("palavras-chave"), "{err}");
}
