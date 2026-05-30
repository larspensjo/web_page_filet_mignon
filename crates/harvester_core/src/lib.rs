//! Harvester core: pure state machine and view-model helpers.
mod briefing;
mod cache_utils;
mod context_draft;
mod effect;
pub mod entity_index;
pub mod import_session;
mod llm_quota_view;
mod msg;
mod poll_stats_fmt;
mod pre_triage_coordinator;
mod pre_triage_filter;
mod preview;
mod prompt_lab;
pub mod signal_candidate;
pub mod signal_candidate_cache;
mod source_state;
mod state;
mod summary_cache;
mod tabs;
pub mod trends;
mod triage;
mod triage_cache;
mod ui_geometry;
mod update;
mod url_age;
mod view_model;
pub mod working_corpus;

pub use briefing::{
    format_previous_briefings_block, ArticleSummaryResult, BriefingArticle, BriefingArticleId,
    BriefingHistoryEntry, BriefingHistoryStory, BriefingPhase, BriefingResult, BriefingSession,
    BriefingStoryResult, CorpusFingerprint, LoadedArticle, TriageSelectionPolicy,
};
pub use cache_utils::model_ids_compatible;
pub use context_draft::{parse_draft_text, serialize_pairs, ContextValidationError};
pub use effect::{Effect, StopPolicy};
pub use entity_index::{EntityIndex, EntityIndexEntry};
pub use harvester_engine::llm::SummaryEntities;
pub use import_session::{ImportPhase, ImportSessionState};
pub use llm_quota_view::{
    build_llm_quota_view, build_poll_quota_warning, LlmQuotaLimits, LlmQuotaSeverity,
    LlmQuotaState, LlmQuotaUsage, LlmQuotaView, PollQuotaWarning,
};
pub use msg::{LlmResultKind, Msg};
pub use poll_stats_fmt::format_poll_stats;
pub use pre_triage_filter::{
    ArticleFilterEntry, ArticleFilterKey, AutoVerdict, FilterReason, ManualDecision,
    PreTriagePhase, PreTriagePolicy, PreTriageSession,
};
pub use preview::PreviewContentKind;
pub use prompt_lab::{
    ModelCatalogSource, PromptLabInputSource, PromptLabRunId, PromptLabRunRecord,
    PromptLabRunStatus, PromptLabStage, PromptLabTemplateSnapshot,
};
pub use signal_candidate::{
    compute_dialog_default, OverrideKey, ScoredCandidate, SelectionPolicy,
    SignalCandidateArchiveSelection, SignalCandidateDialogDefault, SignalCandidateSelection,
    SignalCandidateSession, SignalCandidateState,
};
pub use signal_candidate_cache::{
    SignalCandidateCache, SignalCandidateCacheEntry, SignalCandidateCacheKey,
    SignalCandidateCacheKeyError, SignalCandidateInputBundle,
};
pub use source_state::{SourceInstanceState, SourcePollStat, SourceStateIndex};
pub use state::{
    normalize_url_for_dedupe, AiAvailability, AiUnavailableReason, AppState, ArchiveTokenEstimates,
    BatchNextAction, BatchObservation, BatchStatus, CompletedJobSnapshot, JobId, JobOrigin,
    JobResultKind, LinkDownloadState, LinkSnapshotRecord, LlmRequestState, LlmResultIndex,
    PreTriageActionability, SessionState, Stage,
};
// ImportPhase is re-exported from import_session above; BatchObservation uses it.
pub use summary_cache::{
    context_hash, SummaryCache, SummaryCacheEntry, SummaryCacheKey, SummaryCacheKeyError,
};
pub use tabs::{AppTab, JobListScope, LeftTab, TrendCategory};
pub use trends::{
    choose_display_label, compute_trends, normalize_entity_key, CategoryTrend, EntityLine,
    EntityTrendData, IsoWeek,
};
pub use triage::{
    ArticleTriageResult, ArticleTriageState, TriageArticle, TriageArticleId, TriagePhase,
    TriageSession,
};
pub use triage_cache::{TriageCache, TriageCacheEntry, TriageCacheKey, TriageCacheKeyError};
pub use ui_geometry::calc_left_width;
pub use update::update;
pub use view_model::{
    AppViewModel, CategoryTrendView, EntityLineView, IndirectLinkPhase, IndirectLinkSummary,
    InlineWarningView, JobFilterStatus, JobRowView, LayoutViewModel, LeftPaneHeaderView,
    LeftPaneView, LinkRowView, LlmModelUsageView, OperationProgress, PreviewContextView,
    PreviewHeaderView, PromptLabCompareBatchView, PromptLabCompareCandidateView,
    PromptLabComparePolicyView, PromptLabCompareRowView, PromptLabRunSummaryView, PromptLabView,
    RightPaneView, ScoreBand, SignalCandidateOutcome, SignalCandidatePreviewView,
    SignalCandidateRow, SignalCandidateRowState, StopFinishButtonState, TrendsTabView,
    TriageAnnotationView, DEFAULT_JOBS_PANEL_WIDTH, DEFAULT_LEFT_PANEL_WIDTH, DEFAULT_WINDOW_WIDTH,
    INPUT_PANEL_FIXED_WIDTH, MIN_JOBS_PANEL_WIDTH, TOKEN_LIMIT,
};
pub use working_corpus::{CurrentWorkingCorpus, CurrentWorkingCorpusSource};
