//! Harvester core: pure state machine and view-model helpers.
mod archive_display;
pub mod blacklist;
mod briefing;

mod cache_utils;
mod effect;
#[doc(hidden)]
pub mod fixture_support;
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
mod triage;
mod triage_cache;
mod ui_intent;
mod update;
mod view_model;
pub mod working_corpus;

pub use briefing::{
    ArticleSummaryResult, BriefingArticle, BriefingArticleId, BriefingPhase, BriefingSession,
    LoadedArticle, TriageSelectionPolicy,
};

pub use cache_utils::model_ids_compatible;
pub use effect::{Effect, PersistenceSnapshot, StopPolicy};
pub use harvester_engine::llm::SummaryEntities;
pub use import_session::{ImportPhase, ImportSessionState};
pub use llm_quota_view::{
    build_llm_quota_view, LlmQuotaLimits, LlmQuotaSeverity, LlmQuotaState, LlmQuotaUsage,
    LlmQuotaView,
};
pub use msg::{LlmResultKind, Msg};
pub use poll_stats_fmt::format_poll_stats;
pub use pre_triage_filter::{
    ArticleFilterEntry, ArticleFilterKey, AutoVerdict, FilterReason, ManualDecision,
    PreTriagePhase, PreTriagePolicy, PreTriageSession,
};
pub use run_progress::{
    ActivityEntry, ActivityOutcome, PipelineActivity, PipelineRunPhase, PipelineStage,
    RunCompletionNotice, RunProgress, RunProgressView, RunState, StageProgress, StageRecord,
    StageStatus, ACTIVITY_FEED_CAPACITY,
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
    evaluate_reprocess_notice, normalize_url_for_dedupe, AiAvailability, AiUnavailableReason,
    AppState, ArchiveTokenEstimates, BatchObservation, BatchStatus, CompletedJobSnapshot, JobId,
    JobOrigin, JobResultKind, LinkSnapshotRecord, LlmRequestState, LlmResultIndex,
    PreTriageActionability, SessionState, Stage, UnfinishedStageVerdict, UnfinishedStageVerdicts,
    UnfinishedWork, UnfinishedWorkClass, UnfinishedWorkSummary,
    DEFAULT_REPROCESS_NOTICE_ARTICLE_THRESHOLD, DEFAULT_REPROCESS_NOTICE_QUOTA_PERCENT,
    MAX_EXTRACTED_LINKS,
};
pub use ui_intent::{HostAction, IntentContext, IntentEffect, UiIntent};
// ImportPhase is re-exported from import_session above; BatchObservation uses it.
pub use summary_cache::{
    context_hash, SummaryCache, SummaryCacheEntry, SummaryCacheKey, SummaryCacheKeyError,
};
pub use triage::{
    ArticleTriageResult, ArticleTriageState, TriageArticle, TriageArticleId, TriagePhase,
    TriageSession,
};
pub use triage_cache::{TriageCache, TriageCacheEntry, TriageCacheKey, TriageCacheKeyError};
pub use update::update;
pub use view_model::{
    AppViewModel, ArchivePartialCoverageView, DesktopJobListView, JobFilterStatus, JobListRowView,
    JobRowView, LinkRowView, LlmModelUsageView, ReprocessNoticeView, RightPaneView, ScoreBand,
    SelectedJobView, SelectedJobVisibility, SignalCandidateOutcome, SignalCandidateRow,
    SignalCandidateRowState, StopFinishButtonState, TriageAnnotationView, DEFAULT_WINDOW_HEIGHT,
    DEFAULT_WINDOW_WIDTH, DESKTOP_JOB_LIST_MAX_ROWS, DESKTOP_JOB_LIST_RECENT_WINDOW_HOURS,
    TOKEN_LIMIT,
};
pub use working_corpus::{CurrentWorkingCorpus, CurrentWorkingCorpusSource};

mod pipeline_waves;
pub use pipeline_waves::{PipelineRunScope, PipelineWave, PipelineWaves};

pub mod result_store;
pub use result_store::{ResultStore, SavedResult};

pub use tabs::JobListMode;

pub use preview::format_summary_for_preview;
