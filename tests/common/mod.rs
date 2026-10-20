//! O que os testes de transcrição compartilham: fixtures, modelos e o texto conhecido.

// Cada binário de teste usa uma parte.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use scriba::asr::{self, Model, Models, Source};

pub fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

pub fn fixture(name: &str) -> PathBuf {
    root().join("tests/fixtures").join(name)
}

pub fn models_dir() -> PathBuf {
    root().join("models")
}

/// Só o base na CPU e o VAD, como no CI (`cargo xtask model --only base,silero`), mesmo
/// que `models/` tenha outros.
pub fn base_models() -> Models {
    let dir = models_dir();
    Models {
        large: None,
        cpu: Some(Model {
            name: "base",
            source: Source::File(dir.join(asr::BASE_FILE)),
        }),
        vad: Some(Source::File(dir.join(asr::VAD_FILE))),
    }
}

/// O áudio de uma fixture em 16 kHz mono.
pub fn audio(name: &str) -> Vec<f32> {
    scriba::media::extract_audio(&fixture(name), |_| {}, &AtomicBool::new(false)).unwrap()
}

/// O texto lido no `fala.wav` (o mesmo de `xtask/src/fixtures.rs`).
pub const FALA: &str = "Bom dia. Esta é uma gravação de teste do Scriba. \
    A reunião sobre o orçamento começa amanhã às nove horas.";

/// As duas frases do `pausas.wav`, com 3 s de silêncio entre elas.
pub const PAUSAS: [&str; 2] = [
    "Primeira parte da reunião, sobre o orçamento.",
    "Segunda parte, depois da pausa, sobre o cronograma.",
];

const KEYWORDS: [&str; 6] = [
    "bom dia",
    "scriba",
    "reunião",
    "orçamento",
    "amanhã",
    "nove horas",
];

/// Quantas palavras-chave do `fala.wav` aparecem no texto (sem caixa).
pub fn keyword_hits(text: &str) -> usize {
    let text = text.to_lowercase();
    KEYWORDS.iter().filter(|k| text.contains(*k)).count()
}
