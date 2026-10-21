//! Fila de jobs: cada arquivo enviado (ou gravação) vira um job, guardado em memória e processado por um
//! worker numa thread dedicada (um job por vez), fora do runtime Tokio.

#[cfg(any(test, feature = "test-support"))]
mod fake;
mod id;
mod model;
mod pipeline;
mod store;
mod worker;

#[cfg(any(test, feature = "test-support"))]
pub use fake::FakePipeline;
pub use id::{InvalidId, JobId};
pub use model::{
    Audio, AudioEvent, Estimate, Event, FORMATS, Failure, Invalid, Job, Media, Outputs,
    PANIC_MESSAGE, Probe, RECORDING_PREFIX, Stage, State,
};
pub use pipeline::{Ctx, Pipeline};
pub use store::{AudioError, DIALOG_ERROR, Jobs, RemoveError, RetryError, part_path};
pub use worker::Worker;
