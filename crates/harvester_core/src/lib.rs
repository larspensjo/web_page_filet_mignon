//! Harvester core: pure state machine and view-model helpers.
mod archive_display;
mod batch;
pub mod blacklist;
mod briefing;
pub mod briefing_snapshot;
mod cache_utils;
mod effect;
pub mod entity_index;
pub mod import_session;
mod llm_quota_view;
mod msg;
mod poll_stats_fmt;
mod pre_triage_coordinator;
mod pre_triage_filter;
mod preview;
mod run_progress;
pub mod signal_candidate;
pub mod signal_candidate_cache;
mod source_state;
mod state;
mod summary_cache;
mod tabs;
pub mod trends;
mod triage;
mod triage_cache;
mod ui_intent;
mod update;
mod url_age;
mod view_model;
pub mod working_corpus;

pub use batch::{CollectedEntry, CollectedOutcome, FrozenBatchKey, StageKind};
pub use briefing::{
    format_previous_briefings_block, ArticleSummaryResult, BriefingArticle, BriefingArticleId,
    BriefingHistoryEntry, BriefingHistoryStory, BriefingItem, BriefingPhase, BriefingResult,
    BriefingSession, BriefingStoryResult, LoadedArticle, TriageSelectionPolicy,
};
pub use briefing_snapshot::{
    build_briefing_snapshot, BriefingSnapshot, SnapshotArticle, BRIEFING_SNAPSHOT_BUDGET_BYTES,
};
pub use cache_utils::model_ids_compatible;
pub use effect::{Effect, PersistenceSnapshot, StopPolicy};
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
pub use run_progress::{
    ActivityEntry, ActivityOutcome, PipelineActivity, PipelineRunPhase, PipelineStage,
    RunCompletionNotice, RunProgress, RunProgressView, StageProgress, StageRecord, StageStatus,
    ACTIVITY_FEED_CAPACITY,
};
pub use signal_candidate::{
    compute_dialog_default, ArchiveFinalSelection, ArchiveSelectionSource, OverrideKey,
    ScoredCandidate, SelectionPolicy, SignalCandidateArchiveSelection,
    SignalCandidateDialogDefault, SignalCandidateSelection, SignalCandidateSession,
    SignalCandidateState,
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
    PreTriageActionability, ProviderAlert, SessionState, Stage, MAX_EXTRACTED_LINKS,
};
pub use ui_intent::{HostAction, IntentContext, IntentEffect, UiIntent};
// ImportPhase is re-exported from import_session above; BatchObservation uses it.
pub use summary_cache::{
    context_hash, SummaryCache, SummaryCacheEntry, SummaryCacheKey, SummaryCacheKeyError,
};
pub use tabs::{JobListMode, TrendCategory, WorkspaceView};
pub use trends::{
    choose_display_label, compute_trends, normalize_entity_key, CategoryTrend, EntityLine,
    EntityTrendData, IsoWeek,
};
pub use triage::{
    ArticleTriageResult, ArticleTriageState, TriageArticle, TriageArticleId, TriagePhase,
    TriageSession,
};
pub use triage_cache::{TriageCache, TriageCacheEntry, TriageCacheKey, TriageCacheKeyError};
pub use update::update;
pub use view_model::{
    AppViewModel, ArchivePartialCoverageView, CategoryTrendView, DesktopJobListView,
    EntityLineView, IndirectLinkPhase, IndirectLinkSummary, InlineWarningView, JobFilterStatus,
    JobListRowView, JobRowView, LeftPaneView, LinkRowView, LlmModelUsageView, PreviewContextView,
    PreviewHeaderView, RightPaneView, ScoreBand, SelectedJobView, SelectedJobVisibility,
    SignalCandidateOutcome, SignalCandidatePreviewView, SignalCandidateRow,
    SignalCandidateRowState, StopFinishButtonState, TrendsTabView, TriageAnnotationView,
    DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH, DESKTOP_JOB_LIST_MAX_ROWS,
    DESKTOP_JOB_LIST_RECENT_WINDOW_HOURS, TOKEN_LIMIT,
};
pub use working_corpus::{CurrentWorkingCorpus, CurrentWorkingCorpusSource};
