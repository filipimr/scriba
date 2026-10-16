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

#[cfg(feature = "embedded-model")]
pub const VAD: Option<&[u8]> = Some(include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../models/ggml-silero-v6.2.0.bin"
)));
#[cfg(not(feature = "embedded-model"))]
pub const VAD: Option<&[u8]> = None;
