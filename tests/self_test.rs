//! Self-test completo com o modelo base de `models/` (`cargo xtask model --only base`).

use std::path::Path;

#[test]
fn self_test_transcreve_e_codifica() {
    let report = scriba::self_test::run_default().unwrap();
    println!("{report}");
}

#[test]
fn self_test_falha_com_audio_sem_fala() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let model = std::fs::read(root.join("models/ggml-base-q5_1.bin")).unwrap();
    let silence = std::fs::read(root.join("tests/fixtures/silencio.wav")).unwrap();
    assert!(scriba::self_test::run(&model, &silence).is_err());
}
