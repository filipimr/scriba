//! Modelos Whisper e VAD embutidos no executável via `include_bytes!`.
//!
//! Isolado num crate próprio para que o `include_bytes!` de centenas de MB só seja
//! recompilado quando os modelos mudam.

/// large-v3-turbo q5_0 (~0,6 GB), para as GPUs.
#[cfg(feature = "embedded-model")]
pub const WHISPER: Option<&[u8]> = Some(include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../models/ggml-large-v3-turbo-q5_0.bin"
)));
#[cfg(not(feature = "embedded-model"))]
pub const WHISPER: Option<&[u8]> = None;

/// O modelo da CPU, `small` q5_1 (~180 MB): transcreve sem GPU e no `scriba --self-test`.
#[cfg(feature = "embedded-cpu-model")]
pub const CPU_WHISPER: Option<&[u8]> = Some(include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../models/ggml-small-q5_1.bin"
)));
#[cfg(not(feature = "embedded-cpu-model"))]
pub const CPU_WHISPER: Option<&[u8]> = None;

/// VAD Silero (~0,9 MB): vai junto com qualquer modelo embutido.
#[cfg(feature = "embedded-cpu-model")]
pub const VAD: Option<&[u8]> = Some(include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../models/ggml-silero-v6.2.0.bin"
)));
#[cfg(not(feature = "embedded-cpu-model"))]
pub const VAD: Option<&[u8]> = None;

/// Amostra de fala para o `scriba --self-test` da release.
#[cfg(feature = "embedded-cpu-model")]
pub const TEST_SAMPLE: Option<&[u8]> = Some(include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/fala.wav"
)));
#[cfg(not(feature = "embedded-cpu-model"))]
pub const TEST_SAMPLE: Option<&[u8]> = None;
