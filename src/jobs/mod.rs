//! Fila de jobs: cada arquivo enviado vira um job, guardado em memória e processado por um
//! worker numa thread dedicada (um job por vez), fora do runtime Tokio.

mod fake;
mod id;
mod model;
mod pipeline;
mod store;
mod worker;

pub use fake::FakePipeline;
pub use id::{InvalidId, JobId};
pub use model::{
    Event, FORMATS, Failure, Invalid, Job, Media, Options, Outputs, PANIC_MESSAGE, Stage, State,
};
pub use pipeline::{Ctx, Pipeline};
pub use store::{Jobs, RemoveError};
pub use worker::Worker;
