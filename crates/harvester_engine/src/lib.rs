//! Harvester engine: IO pipeline and effect execution.
pub mod content_prep;
mod convert;
mod decode;
mod engine;
mod export;
mod extract;
mod fetch;
mod filename;
mod frontmatter;
mod links;
pub mod llm;
mod path_policy;
mod persist;
mod preview;
mod quota;
mod text_safety;
mod token;
mod types;
mod url_policy;

pub use convert::{Converter, Html2MdConverter};
pub use decode::{decode_html, DecodeError, DecodedHtml};
pub use engine::{EngineConfig, EngineHandle};
pub use export::{build_concatenated_export, ExportError, ExportOptions, ExportSummary};
pub use extract::{ExtractedContent, Extractor, ReadabilityLikeExtractor};
pub use fetch::{FetchSettings, Fetcher, ProgressSink, ReqwestFetcher};
pub use filename::deterministic_filename;
pub use frontmatter::{
    build_markdown_document, parse_frontmatter, unescape_yaml_value, FrontmatterFields,
};
pub use links::{ConversionOutput, ExtractedLink, LinkExtractingConverter, LinkKind};
pub use path_policy::is_confined_to;
pub use persist::{ensure_output_dir, AtomicFileWriter, PersistError};
pub use quota::{QuotaTracker, SessionQuotas};
pub use text_safety::truncate_to_char_boundary;
pub use token::{TokenCounter, WhitespaceTokenCounter};
pub use types::{
    EngineEvent, FailureKind, FetchError, FetchMetadata, FetchOutput, JobId, JobOutcome,
    JobProgress, Stage,
};
pub use url_policy::{UrlPolicy, UrlPolicyViolation};

pub use content_prep::{
    compute_prompt_overhead, derive_clean_text, truncate_to_budget, BoilerplatePolicy,
    BoilerplateResult, CleanText, CleanTextReport, ContentBudget, ContentPrepConfig,
    NormalizationPolicy, PreparedCollection, PreparedInput, TruncationBoundary,
    NONCE_OVERHEAD_BYTES, TRUNCATION_MARKER,
};
