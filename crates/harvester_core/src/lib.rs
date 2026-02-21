//! Harvester core: pure state machine and view-model helpers.
mod briefing;
mod cache_utils;
mod context_draft;
mod effect;
mod msg;
mod pre_triage_filter;
mod preview;
mod prompt_lab;
mod source_state;
mod state;
mod summary_cache;
mod triage;
mod triage_cache;
mod ui_geometry;
mod update;
mod url_age;
mod view_model;

pub use briefing::{
    format_previous_briefings_block, ArticleSummaryResult, BriefingArticle, BriefingArticleId,
    BriefingHistoryEntry, BriefingHistoryTheme, BriefingPhase, BriefingResult, BriefingSession,
    BriefingThemeResult, CorpusFingerprint, LoadedArticle, TriageSelectionPolicy,
};
pub use cache_utils::model_ids_compatible;
pub use context_draft::{parse_draft_text, serialize_pairs, ContextValidationError};
pub use effect::{Effect, StopPolicy};
pub use msg::{LlmResultKind, Msg};
pub use pre_triage_filter::{
    ArticleFilterEntry, ArticleFilterKey, AutoVerdict, FilterReason, ManualDecision,
    PreTriagePhase, PreTriagePolicy, PreTriageSession,
};
pub use preview::PreviewContentKind;
pub use prompt_lab::{
    ModelCatalogSource, PromptLabInputSource, PromptLabRunId, PromptLabRunRecord,
    PromptLabRunStatus, PromptLabStage, PromptLabTemplateSnapshot,
};
pub use source_state::{SourceInstanceState, SourceStateIndex};
pub use state::{
    normalize_url_for_dedupe, AppState, BatchObservation, CompletedJobSnapshot, JobId,
    JobResultKind, LinkDownloadState, LinkSnapshotRecord, LlmRequestState, LlmResultIndex,
    SessionState, Stage,
};
pub use summary_cache::{
    context_hash, SummaryCache, SummaryCacheEntry, SummaryCacheKey, SummaryCacheKeyError,
};
pub use triage::{
    ArticleTriageResult, ArticleTriageState, TriageArticle, TriageArticleId, TriagePhase,
    TriageSession,
};
pub use triage_cache::{TriageCache, TriageCacheEntry, TriageCacheKey, TriageCacheKeyError};
pub use ui_geometry::calc_left_width;
pub use update::update;
pub use view_model::{
    AppViewModel, JobFilterStatus, JobRowView, LinkRowView, LlmModelUsageView, PreviewHeaderView,
    PromptLabCompareBatchView, PromptLabCompareCandidateView, PromptLabComparePolicyView,
    PromptLabCompareRowView, PromptLabRunSummaryView, PromptLabView, TriageAnnotationView,
    DEFAULT_JOBS_PANEL_WIDTH, DEFAULT_LEFT_PANEL_WIDTH, DEFAULT_WINDOW_WIDTH,
    INPUT_PANEL_FIXED_WIDTH, MIN_JOBS_PANEL_WIDTH, TOKEN_LIMIT,
};
