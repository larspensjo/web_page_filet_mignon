//! Harvester engine: IO pipeline and effect execution.
mod engine;
mod fetch;
mod types;

pub use engine::EngineHandle;
pub use fetch::{FetchSettings, Fetcher, ProgressSink, ReqwestFetcher};
pub use types::{
    EngineEvent, FetchError, FetchMetadata, FetchOutput, FailureKind, JobId, JobProgress, Stage,
};
