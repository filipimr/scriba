//! Self-test completo com o modelo da CPU de `models/` (o `small` ou, como no CI, o `base`) e
//! o VAD (`cargo xtask model --only base,silero`).

use scriba::asr::{BASE_FILE, Model, Source, VAD_FILE};

mod common;

#[test]
fn self_test_transcreve_e_codifica() {
    let report = scriba::self_test::run_default().unwrap();
    println!("{report}");
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
