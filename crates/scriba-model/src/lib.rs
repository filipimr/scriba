//! Modelo Whisper e VAD embutidos no executável via `include_bytes!`.
//!
//! Isolado num crate próprio para que o `include_bytes!` de ~0,6 GB só seja
//! recompilado quando os modelos mudam.

#[cfg(feature = "embedded-model")]
pub const WHISPER: Option<&[u8]> = Some(include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../models/ggml-large-v3-turbo-q5_0.bin"
)));
#[cfg(not(feature = "embedded-model"))]
pub const WHISPER: Option<&[u8]> = None;

/// VAD Silero (~0,9 MB): vai junto com qualquer modelo embutido.
#[cfg(any(feature = "embedded-model", feature = "embedded-test-model"))]
pub const VAD: Option<&[u8]> = Some(include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../models/ggml-silero-v6.2.0.bin"
)));
#[cfg(not(any(feature = "embedded-model", feature = "embedded-test-model")))]
pub const VAD: Option<&[u8]> = None;

/// Modelo `base` q5_1 (~60 MB) e amostra de fala para o `scriba --self-test` da
/// release, que roda em CPU no runner sem GPU (com o VAD acima).
#[cfg(feature = "embedded-test-model")]
pub const TEST_WHISPER: Option<&[u8]> = Some(include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../models/ggml-base-q5_1.bin"
)));
#[cfg(not(feature = "embedded-test-model"))]
pub const TEST_WHISPER: Option<&[u8]> = None;

#[cfg(feature = "embedded-test-model")]
pub const TEST_SAMPLE: Option<&[u8]> = Some(include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/fala.wav"
)));
#[cfg(not(feature = "embedded-test-model"))]
pub const TEST_SAMPLE: Option<&[u8]> = None;
