//! Spikes de risco da Fase 0. Cada spike é um teste em `tests/`.
//!
//! Testes marcados com `#[ignore = "hardware"]` precisam de GPU, placa de som ou
//! do modelo grande, e rodam só localmente: `cargo test -p spikes -- --ignored`.

use std::path::{Path, PathBuf};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext};

/// Raiz do workspace.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn fixture(name: &str) -> PathBuf {
    root().join("tests/fixtures").join(name)
}

pub fn model(name: &str) -> PathBuf {
    root().join("models").join(name)
}

/// Lê um WAV mono 16-bit como f32 em [-1, 1].
pub fn read_wav_f32(path: &Path) -> anyhow::Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path)?;
    let samples = r
        .samples::<i16>()
        .map(|s| s.map(|v| f32::from(v) / 32768.0))
        .collect::<Result<_, _>>()?;
    Ok(samples)
}

/// Transcreve áudio 16 kHz mono em pt-BR e devolve o texto concatenado.
pub fn transcribe(ctx: &WhisperContext, audio: &[f32]) -> anyhow::Result<String> {
    let mut state = ctx.create_state()?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("pt"));
    params.set_no_context(true);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    state.full(params, audio)?;
    let mut text = String::new();
    for seg in state.as_iter() {
        text.push_str(&seg.to_string());
    }
    Ok(text)
}

/// Quantas palavras-chave da fixture `fala` aparecem no texto (sem caixa).
pub fn keyword_hits(text: &str) -> usize {
    let t = text.to_lowercase();
    [
        "bom dia",
        "scriba",
        "reunião",
        "orçamento",
        "amanhã",
        "nove horas",
    ]
    .iter()
    .filter(|k| t.contains(*k))
    .count()
}
