//! Harvester core: pure state machine and view-model helpers.
mod briefing;
mod cache_utils;
mod effect;
mod msg;
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
    ArticleSummaryResult, BriefingArticle, BriefingArticleId, BriefingPhase, BriefingResult,
    BriefingSession, BriefingThemeResult, CorpusFingerprint, LoadedArticle, TriageSelectionPolicy,
};
pub use cache_utils::model_ids_compatible;
pub use effect::{Effect, StopPolicy};
pub use msg::{LlmResultKind, Msg};
pub use prompt_lab::{
    PromptLabInputSource, PromptLabRunId, PromptLabRunRecord, PromptLabRunStatus, PromptLabStage,
};
pub use source_state::{SourceInstanceState, SourceStateIndex};
pub use state::{
    normalize_url_for_dedupe, AppState, CompletedJobSnapshot, JobId, JobResultKind,
    LinkDownloadState, LinkSnapshotRecord, LlmRequestState, LlmResultIndex, SessionState, Stage,
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
    AppViewModel, JobRowView, LinkRowView, PreviewHeaderView, PromptLabRunSummaryView,
    PromptLabView, TriageAnnotationView, DEFAULT_JOBS_PANEL_WIDTH, DEFAULT_LEFT_PANEL_WIDTH,
    DEFAULT_WINDOW_WIDTH, INPUT_PANEL_FIXED_WIDTH, MIN_JOBS_PANEL_WIDTH, TOKEN_LIMIT,
};
