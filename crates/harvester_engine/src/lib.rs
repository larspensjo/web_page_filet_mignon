//! Harvester engine: IO pipeline and effect execution.
mod engine;
mod convert;
mod extract;
mod decode;
mod fetch;
mod types;

pub use engine::EngineHandle;
pub use convert::{Converter, Html2MdConverter};
pub use decode::{decode_html, DecodeError, DecodedHtml};
pub use extract::{ExtractedContent, Extractor, ReadabilityLikeExtractor};
pub use fetch::{FetchSettings, Fetcher, ProgressSink, ReqwestFetcher};
pub use types::{
    EngineEvent, FetchError, FetchMetadata, FetchOutput, FailureKind, JobId, JobProgress, Stage,
};
