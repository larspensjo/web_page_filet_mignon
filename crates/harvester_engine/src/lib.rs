//! Harvester engine: IO pipeline and effect execution.
mod engine;
mod convert;
mod extract;
mod decode;
mod token;
mod frontmatter;
mod filename;
mod persist;
mod fetch;
mod types;

pub use engine::EngineHandle;
pub use convert::{Converter, Html2MdConverter};
pub use decode::{decode_html, DecodeError, DecodedHtml};
pub use extract::{ExtractedContent, Extractor, ReadabilityLikeExtractor};
pub use frontmatter::build_markdown_document;
pub use fetch::{FetchSettings, Fetcher, ProgressSink, ReqwestFetcher};
pub use filename::deterministic_filename;
pub use persist::{ensure_output_dir, AtomicFileWriter, PersistError};
pub use token::{TokenCounter, WhitespaceTokenCounter};
pub use types::{
    EngineEvent, FetchError, FetchMetadata, FetchOutput, FailureKind, JobId, JobOutcome,
    JobProgress, Stage,
};
