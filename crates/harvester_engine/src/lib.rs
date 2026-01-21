//! Harvester engine: IO pipeline and effect execution.
mod convert;
mod decode;
mod engine;
mod export;
mod extract;
mod fetch;
mod filename;
mod frontmatter;
mod persist;
mod token;
mod types;

pub use convert::{Converter, Html2MdConverter};
pub use decode::{decode_html, DecodeError, DecodedHtml};
pub use engine::EngineHandle;
pub use export::{build_concatenated_export, ExportError, ExportOptions, ExportSummary};
pub use extract::{ExtractedContent, Extractor, ReadabilityLikeExtractor};
pub use fetch::{FetchSettings, Fetcher, ProgressSink, ReqwestFetcher};
pub use filename::deterministic_filename;
pub use frontmatter::build_markdown_document;
pub use persist::{ensure_output_dir, AtomicFileWriter, PersistError};
pub use token::{TokenCounter, WhitespaceTokenCounter};
pub use types::{
    EngineEvent, FailureKind, FetchError, FetchMetadata, FetchOutput, JobId, JobOutcome,
    JobProgress, Stage,
};
