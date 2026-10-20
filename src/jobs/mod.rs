//! Fila de jobs: cada arquivo enviado vira um job, guardado em memória e processado por um
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
    Estimate, Event, FORMATS, Failure, Invalid, Job, Media, Outputs, PANIC_MESSAGE, Probe, Stage,
    State,
};
pub use pipeline::{Ctx, Pipeline};
pub use store::{Jobs, RemoveError, RetryError};
pub use worker::Worker;
