use crate::briefing::BriefingSession;
use crate::pre_triage_filter::{PreTriagePhase, PreTriageSession};
use crate::source_state::SourceStateIndex;
use crate::summary_cache::SummaryCache;
use crate::tabs::JobListMode;
use crate::triage::{ArticleTriageResult, TriagePhase, TriageSession};
use crate::triage_cache::TriageCache;
use crate::view_model::LastPasteStats;
use crate::Effect;
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::LinkKind;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

mod ai_availability;
mod archive_meter;
mod startup_readiness;
pub use startup_readiness::{
    InitialArticleWindowOutcome, StartupInputOutcome, StartupReadiness, StartupReadinessStatus,
};
mod batch;
mod briefing_access;

mod cache_state;
mod ingest;
mod job_access;
mod job_state;
mod link_helpers;
mod llm;
pub(crate) use llm::ModelDispatchHalt;
mod pre_triage_access;
mod prompt;
mod provider_alert;
mod run_progress;
pub(crate) use run_progress::PollPipelineJobSnapshot;
mod saved_results;
mod signal_candidate_access;
mod source_poll;
mod ui_state;
mod unfinished_work;
mod view_builder;

#[cfg(test)]
mod tests;

use cache_state::{
    MetadataLoadState, SummaryCacheMetadataSnapshot, SummaryCacheMetrics,
    TriageCacheMetadataSnapshot, TriageCacheRunMetrics,
};
use job_state::JobState;
use link_helpers::{build_link_rows, map_job_filter_status, normalize_extracted_link};
use ui_state::{MetricsState, UiState};

#[cfg(test)]
use crate::view_model::JobFilterStatus;

pub use unfinished_work::{
    evaluate_reprocess_notice, UnfinishedStageVerdict, UnfinishedStageVerdicts, UnfinishedWork,
    UnfinishedWorkClass, UnfinishedWorkSummary, DEFAULT_REPROCESS_NOTICE_ARTICLE_THRESHOLD,
    DEFAULT_REPROCESS_NOTICE_QUOTA_PERCENT,
};

pub type JobId = u64;

/// Maximum extracted links retained for one job.
pub const MAX_EXTRACTED_LINKS: usize = 5_000;
const CHECKPOINT_SAVING_STATUS_MESSAGE: &str = "Checkpoint saving...";
pub(crate) const EXPORT_UNAVAILABLE_STATUS_MESSAGE: &str =
    "Export is unavailable while a run is in progress";

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingBriefingCheckpointSave {
    save_id: u64,
    previous_since_utc: Option<chrono::DateTime<chrono::Utc>>,
    pending_since_utc: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PreTriageLoadContext {
    reason: crate::pre_triage_coordinator::PreTriageRefreshReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PollPipelineProgressState {
    source_scan_done: bool,
    job_ids: BTreeSet<JobId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiAvailability {
    Available,
    Unavailable { reason: AiUnavailableReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiUnavailableReason {
    ResultStoreUnavailable,
    MissingApiKey,
    NoTriageModel,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingBriefingCheckpointSaveSnapshot {
    pub save_id: u64,
    pub previous_since_utc: Option<chrono::DateTime<chrono::Utc>>,
    pub pending_since_utc: Option<chrono::DateTime<chrono::Utc>>,
}

pub(crate) enum TriageCacheLookupResult<'a> {
    Hit {
        result: &'a ArticleTriageResult,
        stored_model_id: &'a str,
    },
    Miss,
    KeyUnavailable,
}

/// Canonical representation of a link extracted from a completed job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRecord {
    pub index: u32,
    pub url: String,
    pub anchor_text: Option<String>,
    pub kind: LinkKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JobOrigin {
    #[default]
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkSnapshotRecord {
    pub url: String,
    pub downloaded_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedJobSnapshot {
    pub url: String,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub links: Vec<LinkSnapshotRecord>,
    pub fetched_utc: Option<String>,
}

/// Completed-job persistence projection, deliberately without a link collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlimJobRecord {
    pub url: String,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub fetched_utc: Option<String>,
}

/// Snapshot of batch processing state for headless runners.
/// Provides observable metrics without UI dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchObservation {
    /// True if work is actively being polled/processed.
    pub poll_in_progress: bool,
    /// Current session state.
    pub session_state: SessionState,
    /// Total number of jobs.
    pub jobs_total: usize,
    /// Number of jobs in Done stage.
    pub jobs_done: usize,
    /// Number of jobs that failed.
    pub jobs_failed: usize,
    /// Number of jobs with in-flight LLM requests.
    pub jobs_in_flight: usize,
    /// Pre-triage phase status.
    pub pre_triage_phase: PreTriagePhase,
    /// Total articles loaded into pre-triage.
    pub pre_triage_total: usize,
    /// Articles currently included by pre-triage decisions.
    pub pre_triage_included: usize,
    /// Articles still requiring manual review.
    pub pre_triage_review: usize,
    /// Articles filtered out by pre-triage.
    pub pre_triage_filtered: usize,
    /// Triage phase status.
    pub triage_phase: TriagePhase,
    /// Total articles loaded for triage.
    pub triage_total: usize,
    /// Articles awaiting triage.
    pub triage_pending: usize,
    /// Articles currently being triaged.
    pub triage_in_flight: usize,
    /// Articles with triage complete.
    pub triage_completed: usize,
    /// Articles that failed triage.
    pub triage_failed: usize,
    /// Total articles in summary preparation session.
    pub summary_total: usize,
    /// Articles awaiting summary generation.
    pub summary_pending: usize,
    /// Articles currently being summarized.
    pub summary_in_flight: usize,
    /// Articles with summary complete.
    pub summary_completed: usize,
    /// Articles that failed summary generation.
    pub summary_failed: usize,
    /// Total signal-candidate URLs in the current observation epoch.
    pub signal_total: usize,
    /// Signal-candidate URLs awaiting or actively undergoing scoring.
    pub signal_pending_or_in_flight: usize,
    /// Signal-candidate URLs with completed scoring.
    pub signal_completed: usize,
    /// Signal-candidate URLs with failed scoring.
    pub signal_failed: usize,
    /// Triage cache hits during the latest triage cache run.
    pub triage_cache_hits: usize,
    /// Triage cache misses during the latest triage cache run.
    pub triage_cache_misses: usize,
    /// Triage cache key-unavailable count during the latest triage cache run.
    pub triage_cache_key_unavailable: usize,
    /// Summary cache hits during the latest summary cache run.
    pub summary_cache_hits: usize,
    /// Summary cache misses during the latest summary cache run.
    pub summary_cache_misses: usize,
    /// Summary cache key-unavailable count during the latest summary cache run.
    pub summary_cache_key_unavailable: usize,
    /// Phase of the current import session.
    pub import_phase: crate::import_session::ImportPhase,
    /// Count of successfully persisted imports in the current session.
    pub imports_completed: usize,
    /// Count of per-file import failures in the current session.
    pub imports_failed: usize,
    /// True while an import request is in flight.
    pub import_in_flight: bool,
    /// Per-source poll statistics for the most recent poll cycle.
    pub source_poll_stats: Vec<crate::SourcePollStat>,
}

/// Token cost estimates for the two archive modes, computed at dialog-open time.
///
/// **Limitation:** `full_tokens` is summed from `AppState::jobs`. Articles whose
/// `JobState` has been pruned, or imported articles without a job, contribute 0.
/// The dialog may therefore show a smaller archive size than the file produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ArchiveTokenEstimates {
    /// Sum of article token counts from job state in full-article mode.
    pub full_tokens: u64,
    /// Estimated tokens in summary mode: cached summary output tokens, falling
    /// back to full article tokens when no summary is cached.
    pub summary_tokens: u64,
    /// Number of requested articles with a cached summary.
    pub summary_coverage: usize,
}

/// Whether pre-triage currently exposes an actionable corpus for triage startup.
///
/// This is intentionally separate from [`crate::PreTriagePhase`], which remains
/// a display/workflow phase. For example, `Reviewing` can still be actionable
/// because unresolved review rows are tentatively included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreTriageActionability {
    Loading,
    Ready,
    ReadyWithPendingReview,
    Unavailable,
}

/// Headless batch run status derived from reducer-owned state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchStatus {
    Running,
    Settled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub(crate) startup_inputs: StartupReadiness,
    session: SessionState,
    jobs: BTreeMap<JobId, JobState>,
    archive_article_tokens: batch::ArchiveArticleTokenLookup,
    pub(crate) saved_articles_ready: bool,
    pub(crate) restored_checkpoint_ready: bool,
    pub(crate) pending_selected_article_url: Option<String>,
    saved_articles: BTreeMap<String, harvester_engine::WindowArticle>,
    saved_newest_summaries: HashMap<String, crate::SummaryCacheKey>,
    saved_urls_by_hash: HashMap<String, Vec<String>>,
    saved_urls_by_signal_key: HashMap<crate::SignalCandidateCacheKey, Vec<String>>,
    saved_results: BTreeMap<String, saved_results::SavedArticleResults>,
    saved_scope_clock: Option<chrono::DateTime<chrono::Utc>>,
    saved_results_global_revision: u64,
    metrics: MetricsState,
    ui: UiState,
    seen_urls: HashSet<String>,
    last_paste_stats: Option<LastPasteStats>,
    dirty: bool,
    next_job_id: JobId,
    next_llm_request_id: u64,
    archive_request_id: u64,
    next_briefing_checkpoint_save_id: u64,
    pinned_archive_corpus: Option<crate::working_corpus::CurrentWorkingCorpus>,
    pinned_signal_candidate_selection:
        Option<crate::signal_candidate::SignalCandidateArchiveSelection>,
    llm_requests: LlmResultIndex,
    briefing: BriefingSession,
    briefing_since_utc: Option<chrono::DateTime<chrono::Utc>>,
    pending_briefing_checkpoint_save: Option<PendingBriefingCheckpointSave>,
    briefing_checkpoint_status_message: Option<String>,
    triage: TriageSession,
    pre_triage: PreTriageSession,
    pre_triage_load_context: Option<PreTriageLoadContext>,
    source_states: SourceStateIndex,
    prompt_contexts: HashMap<PromptId, Vec<(String, String)>>,
    prompt_contexts_ready: bool,
    prompt_contexts_load_failed: bool,
    prompt_template_files_loaded: bool,
    active_prompt_versions: HashMap<PromptId, PromptVersion>,
    effective_models: HashMap<PromptId, String>,
    ai_availability: AiAvailability,
    result_store_failure: Option<String>,
    consecutive_rate_limit_failures: u32,
    summary_cache: SummaryCache,
    signal_candidate: crate::signal_candidate::SignalCandidateSession,
    signal_exclusions: crate::signal_candidate::SignalExclusions,
    signal_candidate_cache: crate::signal_candidate_cache::SignalCandidateCache,
    signal_candidate_inputs:
        HashMap<String, crate::update::signal_candidate::SignalCandidateInputSnapshot>,
    signal_candidate_threshold: u8,
    briefing_metadata_state: MetadataLoadState,
    summary_cache_metadata_snapshot: Option<SummaryCacheMetadataSnapshot>,
    summary_cache_metrics: SummaryCacheMetrics,
    summary_cache_warmup_logged: bool,
    triage_cache: TriageCache,
    triage_metadata_state: MetadataLoadState,
    triage_cache_metadata_snapshot: Option<TriageCacheMetadataSnapshot>,
    triage_cache_run_metrics: TriageCacheRunMetrics,
    triage_cache_run_start_logged: bool,
    llm_max_in_flight: usize,
    model_dispatch_halt_reason: Option<llm::ModelDispatchHalt>,
    /// Session-scoped per-model token usage. Only CacheStatus::Miss runs are counted.
    llm_usage_by_model: BTreeMap<String, (u64, u64)>,
    /// Authoritative session-scoped quota usage and configured limits.
    llm_quota: crate::LlmQuotaState,
    job_list_mode: JobListMode,
    /// Host-observed time used by deterministic view projection. Hosts must
    /// reduce a `Msg::Tick` before constructing the first view.
    last_observed_utc: Option<chrono::DateTime<chrono::Utc>>,
    run_progress: Option<crate::RunProgress>,
    next_run_id: u64,
    pipeline_run_phase: crate::PipelineRunPhase,
    pub(crate) pipeline_admission: Option<crate::pipeline_waves::PipelineAdmission>,
    pub(crate) pipeline_waves: crate::PipelineWaves,
    run_completion_notice: Option<crate::RunCompletionNotice>,
    /// Test-only bypass counter for injecting pre-triage request IDs without driving
    /// the coordinator (used by `start_triage_for_test` and related helpers).
    /// In production, request IDs are allocated exclusively by the coordinator.
    #[cfg(test)]
    next_triage_request_id: u64,
    /// The request ID of the currently in-flight pre-triage load, if any.
    /// Kept in sync with the coordinator's in-flight ID.
    triage_in_flight_request_id: Option<u64>,
    /// Reducer-owned tracker for article jobs emitted by the current source poll.
    poll_pipeline: Option<PollPipelineProgressState>,
    /// Logical tick counter driven by `Msg::Tick`; used by the pre-triage refresh coordinator.
    tick: u64,
    /// Configuration and preparation pending for a processing start.
    pub(crate) processing_start: Option<crate::update::processing::PendingStart>,
    pub(crate) processing_budget: Option<usize>,
    /// Reducer-owned coordinator for batching pre-triage refresh demand.
    pub(crate) pre_triage_coordinator: crate::pre_triage_coordinator::PreTriageRefreshCoordinator,
    /// True when app/batch loop should dispatch one `Msg::EvaluatePreTriageRefresh`.
    pre_triage_refresh_eval_pending: bool,
    /// Coalesced cause bit for pending pre-triage refresh evaluation.
    pre_triage_refresh_eval_job_done: bool,
    /// Reducer-owned state for the imported-corpus workflow.
    pub(crate) import_session: crate::import_session::ImportSessionState,
    pub(crate) blacklist: crate::blacklist::BlacklistState,
    pending_intake: Vec<String>,
    pub(crate) fetch_time_recovery_done: bool,
    runtime_state_notice: Option<String>,
    unfinished_work: UnfinishedWork,
    unfinished_classes: HashMap<(String, String), UnfinishedWorkClass>,
    unfinished_inputs_revision: u64,
    unfinished_global_revision: u64,
    pub(crate) pending_results: Vec<crate::SavedResult>,
}

pub struct IngestResult {
    pub effects: Vec<Effect>,
    pub enqueued: usize,
    pub skipped: usize,
    pub enqueued_job_ids: Vec<JobId>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: SessionState::Idle,
            jobs: BTreeMap::new(),
            archive_article_tokens: batch::ArchiveArticleTokenLookup::default(),
            saved_articles_ready: false,
            restored_checkpoint_ready: false,
            pending_selected_article_url: None,
            saved_articles: BTreeMap::new(),
            saved_newest_summaries: HashMap::new(),
            saved_urls_by_hash: HashMap::new(),
            saved_urls_by_signal_key: HashMap::new(),
            saved_results: BTreeMap::new(),
            saved_scope_clock: None,
            saved_results_global_revision: 0,
            metrics: MetricsState::default(),
            ui: UiState::default(),
            seen_urls: HashSet::new(),
            last_paste_stats: None,
            dirty: false,
            next_job_id: 1,
            next_llm_request_id: 1,
            archive_request_id: 0,
            next_briefing_checkpoint_save_id: 1,
            pinned_archive_corpus: None,
            pinned_signal_candidate_selection: None,
            llm_requests: LlmResultIndex::new(),
            briefing: BriefingSession::default(),
            briefing_since_utc: None,
            pending_briefing_checkpoint_save: None,
            briefing_checkpoint_status_message: None,
            triage: TriageSession::default(),
            pre_triage: PreTriageSession::default(),
            pre_triage_load_context: None,
            source_states: SourceStateIndex::default(),
            prompt_contexts: HashMap::new(),
            prompt_contexts_ready: false,
            prompt_contexts_load_failed: false,
            prompt_template_files_loaded: false,
            active_prompt_versions: HashMap::new(),
            effective_models: HashMap::new(),
            ai_availability: AiAvailability::Available,
            result_store_failure: None,
            consecutive_rate_limit_failures: 0,
            summary_cache: SummaryCache::new(),
            signal_candidate: crate::signal_candidate::SignalCandidateSession::default(),
            signal_exclusions: crate::signal_candidate::SignalExclusions::default(),
            startup_inputs: StartupReadiness::default(),
            signal_candidate_cache: crate::signal_candidate_cache::SignalCandidateCache::default(),
            signal_candidate_inputs: HashMap::new(),
            signal_candidate_threshold: crate::signal_candidate::DEFAULT_SELECTION_THRESHOLD,
            briefing_metadata_state: MetadataLoadState::Idle,
            summary_cache_metadata_snapshot: None,
            summary_cache_metrics: SummaryCacheMetrics::default(),
            summary_cache_warmup_logged: false,
            triage_cache: TriageCache::new(),
            triage_metadata_state: MetadataLoadState::Idle,
            triage_cache_metadata_snapshot: None,
            triage_cache_run_metrics: TriageCacheRunMetrics::default(),
            triage_cache_run_start_logged: false,
            llm_max_in_flight: 1,
            model_dispatch_halt_reason: None,
            llm_usage_by_model: BTreeMap::new(),
            llm_quota: crate::LlmQuotaState::default(),
            job_list_mode: JobListMode::default(),
            last_observed_utc: None,
            run_progress: None,
            next_run_id: 1,
            pipeline_run_phase: crate::PipelineRunPhase::Idle,
            pipeline_admission: None,
            pipeline_waves: Default::default(),
            run_completion_notice: None,
            #[cfg(test)]
            next_triage_request_id: 1,
            triage_in_flight_request_id: None,
            poll_pipeline: None,
            tick: 0,
            processing_start: None,
            processing_budget: None,
            pre_triage_coordinator: crate::pre_triage_coordinator::PreTriageRefreshCoordinator::new(
            ),
            pre_triage_refresh_eval_pending: false,
            pre_triage_refresh_eval_job_done: false,
            import_session: crate::import_session::ImportSessionState::default(),
            blacklist: crate::blacklist::BlacklistState::default(),
            pending_intake: Vec::new(),
            fetch_time_recovery_done: false,
            runtime_state_notice: None,
            unfinished_work: UnfinishedWork::Unknown,
            unfinished_classes: HashMap::new(),
            unfinished_inputs_revision: 0,
            unfinished_global_revision: 0,
            pending_results: Vec::new(),
        }
    }
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn blacklist(&self) -> &crate::blacklist::BlacklistState {
        &self.blacklist
    }

    pub fn set_blacklist(&mut self, blacklist: crate::blacklist::BlacklistState) {
        self.blacklist = blacklist;
    }
}

/// Normalize URL for deduplication: trim whitespace, lowercase, strip trailing `/`.
///
/// Re-exported from `harvester_engine` so both layers share the same canonical implementation.
pub use harvester_engine::normalize_url_for_dedupe;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SessionState {
    #[default]
    Idle,
    Running,
    /// Intake closed: ignore new URL ingestion while draining in-flight work.
    /// Do not auto-resume from this state unless a feature flag explicitly allows it.
    Finishing,
    Finished,
}

pub type LlmResultIndex = BTreeMap<u64, LlmRequestState>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmRequestState {
    Pending {
        prompt_id: PromptId,
    },
    Completed {
        output_json: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Stage {
    #[default]
    Queued,
    Downloading,
    Sanitizing,
    Converting,
    Tokenizing,
    Writing,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobResultKind {
    Success,
    Failed { reason: String },
}
