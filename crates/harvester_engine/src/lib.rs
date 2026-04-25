//! Harvester engine: IO pipeline and effect execution.
mod archive_url;
mod blocker_page;
mod brave_poll;
mod brave_seen_set;
pub mod briefing;
pub mod content_extraction;
pub mod content_prep;
mod convert;
mod decode;
mod engine;
mod export;
mod extract;
mod fetch;
mod filename;
mod frontmatter;
pub mod import;
mod links;
pub mod llm;
mod path_policy;
mod persist;
mod preview;
mod quota;
mod rss_parse;
mod rss_seen_set;
pub mod since_filter;
mod source_config;
mod source_poll;
mod text_safety;
mod token;
mod types;
mod url_policy;

pub use archive_url::archive_url_key;
pub use brave_poll::{parse_brave_news_response, BraveNewsItem, BravePollError};
pub use brave_seen_set::{normalize_url_for_dedupe, BraveSeenSet};
pub use briefing::{
    load_and_prepare_articles, load_and_prepare_articles_by_path,
    load_and_prepare_articles_filtered, load_and_prepare_articles_filtered_with_progress,
    load_and_prepare_articles_for_triage, scan_archive_article_metadata, ArchiveArticleMeta,
    ArticleScanProgress, LoadedArticle,
};
pub use convert::{Converter, Html2MdConverter};
pub use decode::{decode_html, DecodeError, DecodedHtml};
pub use engine::{EngineConfig, EngineHandle};
pub use export::{
    build_concatenated_export, build_triage_archive, ExportError, ExportOptions, ExportSummary,
};
pub use extract::{ExtractedContent, Extractor, ReadabilityLikeExtractor};
pub use fetch::{FetchSettings, Fetcher, ProgressSink, ReqwestFetcher, RetrySettings};
pub use filename::{
    deterministic_filename, import_filename_base, resolve_non_overwriting_filename,
};
pub use frontmatter::{
    build_imported_markdown_document, build_markdown_document, parse_frontmatter,
    unescape_yaml_value, FrontmatterFields, ImportedFrontmatterFields,
};
pub use import::{
    import_saved_webpages, import_single_saved_webpage, scan_saved_webpage_dir, ImportFailure,
    ImportFailureStage, ImportOptions, ImportReport, ImportedArchiveRef, ImportedDocument,
    SavedWebpageFile, SavedWebpageScanResult,
};
pub use links::{ConversionOutput, ExtractedLink, LinkExtractingConverter, LinkKind};
pub use path_policy::is_confined_to;
pub use persist::{ensure_output_dir, AtomicFileWriter, PersistError};
pub use quota::{QuotaTracker, SessionQuotas};
pub use source_config::{
    BraveNewsSourceConfig, SourceConfig, SourceId, SourceIdError, SourceKind, SourceRegistry,
    SourceRegistryValidationError, SourceType,
};
pub use source_poll::{
    poll_curated_source, poll_file_source, poll_rss_source, validate_source_file_path,
    SourcePollError, SourcePollResult,
};
pub use text_safety::truncate_to_char_boundary;
pub use token::{TokenCounter, WhitespaceTokenCounter};
pub use types::{
    EngineEvent, FailureKind, FetchError, FetchMetadata, FetchOutput, JobId, JobOutcome,
    JobProgress, Stage,
};
pub use url_policy::{UrlPolicy, UrlPolicyViolation};

pub use rss_parse::{parse_feed_content, FeedEntry, FeedParseError, RssPollItem};
pub use rss_seen_set::RssSeenSet;

pub use content_extraction::{
    BlockDropCounts, CandidateKind, ExtractedArticle, ExtractionDiagnostics, ExtractionOutcome,
    ExtractionPipeline, ExtractionPolicy,
};

pub use content_prep::{
    compute_prompt_overhead, derive_clean_text, truncate_to_budget, BoilerplatePolicy,
    BoilerplateResult, CleanText, CleanTextReport, ContentBudget, ContentPrepConfig,
    NormalizationPolicy, PreparedCollection, PreparedInput, TruncationBoundary,
    NONCE_OVERHEAD_BYTES, TRUNCATION_MARKER,
};
