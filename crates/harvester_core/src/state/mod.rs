use crate::briefing::BriefingSession;
use crate::pre_triage_filter::{
    ArticleFilterKey, ManualDecision, PreTriagePhase, PreTriageSession,
};
#[cfg(test)]
use crate::preview::{self, PreviewContentKind};
use crate::prompt_lab::{
    PromptLabRunId, PromptLabRunOverrides, PromptLabStage, PromptLabState,
    PromptLabTemplateSnapshot,
};
use crate::source_state::SourceStateIndex;
use crate::summary_cache::SummaryCache;
use crate::tabs::{AppTab, JobListScope, LeftTab, TrendCategory};
use crate::triage::{ArticleTriageResult, TriagePhase, TriageSession};
use crate::triage_cache::TriageCache;
use crate::url_age::AgeEstimate;
#[cfg(test)]
use crate::view_model::JobFilterStatus;
use crate::view_model::LastPasteStats;
#[cfg(test)]
use crate::view_model::OperationProgress;
use crate::Effect;
use harvester_engine::llm::prompt::{PromptId, PromptRegistry, PromptVersion};
use harvester_engine::LinkKind;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

mod ai_availability;
mod batch;
mod briefing_orchestration;
mod briefing_snapshot_access;
mod cache_state;
mod indirect_links;
mod ingest;
mod job_access;
mod job_state;
mod link_helpers;
mod llm;
mod pre_triage_access;
mod prompt;
mod provider_alert;
mod signal_candidate_access;
mod source_poll;
mod ui_state;
mod view_builder;

use briefing_orchestration::BriefingOrchestration;
use cache_state::{
    MetadataLoadState, SummaryCacheMetadataSnapshot, SummaryCacheMetrics,
    TriageCacheMetadataSnapshot, TriageCacheRunMetrics,
};
#[cfg(test)]
use indirect_links::IndirectLink;
use indirect_links::IndirectLinkPool;
use job_state::JobState;
#[cfg(test)]
use job_state::PreviewQuality;
use link_helpers::{
    build_link_rows, domain_from_url, format_lab_briefing_markdown, format_lab_summary_markdown,
    format_lab_triage_markdown, map_job_filter_status, normalize_extracted_link,
};
use ui_state::{MetricsState, PreviewMode, PreviewState, UiState};

pub use provider_alert::ProviderAlert;
pub(crate) use signal_candidate_access::BriefingGenerateReadiness;

pub type JobId = u64;

const MAX_EXTRACTED_LINKS: usize = 5_000;
const CHECKPOINT_SAVING_STATUS_MESSAGE: &str = "Checkpoint saving...";

fn default_prompt_template_snapshots() -> HashMap<PromptId, PromptLabTemplateSnapshot> {
    let registry = PromptRegistry::with_defaults();
    let prompt_ids = [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
        PromptId::AggregateBriefing,
    ];
    prompt_ids
        .into_iter()
        .filter_map(|prompt_id| {
            registry.active_effective(prompt_id).map(|template| {
                (
                    prompt_id,
                    PromptLabTemplateSnapshot {
                        template: template.to_owned(),
                        source: template.source(),
                    },
                )
            })
        })
        .collect()
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PreTriageLoadProgress {
    request_id: u64,
    files_scanned: usize,
    files_total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PollPipelineProgressState {
    source_scan_done: bool,
    job_ids: BTreeSet<JobId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiAvailability {
    Available,
    Unavailable { reason: AiUnavailableReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiUnavailableReason {
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
    Hit(&'a ArticleTriageResult),
    Miss,
    KeyUnavailable,
}

/// Represents the download status for a specific link.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkDownloadState {
    NotDownloaded,
    Downloading,
    Downloaded { path: PathBuf },
    Failed { error: String },
}

/// Canonical representation of a link extracted from a completed job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRecord {
    pub index: u32,
    pub url: String,
    pub anchor_text: Option<String>,
    pub kind: LinkKind,
    pub download_state: LinkDownloadState,
    pub age_estimate: Option<AgeEstimate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum JobOrigin {
    #[default]
    Direct,
    Indirect {
        source_job_id: JobId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSnapshotRecord {
    pub url: String,
    pub downloaded_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedJobSnapshot {
    pub url: String,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub links: Vec<LinkSnapshotRecord>,
    pub fetched_utc: Option<String>,
}

/// Maximum allowed value for any per-flow in-flight limit.
pub const MAX_IN_FLIGHT_LIMIT: usize = 10;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

/// The next automatic action that batch orchestration may dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchNextAction {
    None,
    DispatchTriage,
    DispatchSummaries,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    session: SessionState,
    jobs: BTreeMap<JobId, JobState>,
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
    briefing_history: Vec<crate::briefing::BriefingHistoryEntry>,
    briefing_since_utc: Option<chrono::DateTime<chrono::Utc>>,
    pending_briefing_checkpoint_save: Option<PendingBriefingCheckpointSave>,
    briefing_checkpoint_status_message: Option<String>,
    triage: TriageSession,
    pre_triage: PreTriageSession,
    pre_triage_load_context: Option<PreTriageLoadContext>,
    pre_triage_load_progress: Option<PreTriageLoadProgress>,
    pre_triage_manual_overrides: HashMap<ArticleFilterKey, ManualDecision>,
    indirect_link_pool: IndirectLinkPool,
    indirect_poll_in_progress: bool,
    source_states: SourceStateIndex,
    prompt_contexts: HashMap<PromptId, Vec<(String, String)>>,
    prompt_contexts_load_failed: bool,
    prompt_template_files_loaded: bool,
    active_prompt_versions: HashMap<PromptId, PromptVersion>,
    effective_models: HashMap<PromptId, String>,
    ai_availability: AiAvailability,
    provider_alert: Option<provider_alert::ProviderAlert>,
    consecutive_rate_limit_failures: u32,
    prompt_lab_templates: HashMap<PromptId, PromptLabTemplateSnapshot>,
    summary_cache: SummaryCache,
    signal_candidate: crate::signal_candidate::SignalCandidateSession,
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
    briefing_orchestration: BriefingOrchestration,
    /// Maximum number of concurrent triage LLM requests (default: 1, max: MAX_IN_FLIGHT_LIMIT).
    triage_max_in_flight: usize,
    /// Maximum number of concurrent summary LLM requests (default: 1, max: MAX_IN_FLIGHT_LIMIT).
    summary_max_in_flight: usize,
    prompt_lab: PromptLabState,
    next_prompt_lab_run_id: u64,
    prompt_lab_next_resolve_id: u64,
    /// Session-scoped per-model token usage. Only CacheStatus::Miss runs are counted.
    llm_usage_by_model: BTreeMap<String, (u64, u64)>,
    /// Authoritative session-scoped quota usage and configured limits.
    llm_quota: crate::LlmQuotaState,
    /// Currently active right-pane tab.
    active_tab: AppTab,
    /// Currently active left-pane tab.
    left_tab: LeftTab,
    /// Scope filter for job-oriented tabs (All vs SinceCheckpoint).
    job_list_scope: JobListScope,
    /// Currently active trend category in the Trends tab.
    active_trend_category: TrendCategory,
    /// Persisted entity index loaded from disk (or rebuilt from caches).
    entity_index: Option<crate::entity_index::EntityIndex>,
    /// Pre-computed trend data derived from `entity_index`.
    entity_trend_data: Option<crate::trends::EntityTrendData>,
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
    /// Reducer-owned coordinator for batching pre-triage refresh demand.
    pub(crate) pre_triage_coordinator: crate::pre_triage_coordinator::PreTriageRefreshCoordinator,
    /// True when app/batch loop should dispatch one `Msg::EvaluatePreTriageRefresh`.
    pre_triage_refresh_eval_pending: bool,
    /// Coalesced cause bit for pending pre-triage refresh evaluation.
    pre_triage_refresh_eval_job_done: bool,
    /// Reducer-owned state for the imported-corpus workflow.
    pub(crate) import_session: crate::import_session::ImportSessionState,
    pub(crate) blacklist: crate::blacklist::BlacklistState,
}

pub(crate) struct PromptLabPendingRunRegistration {
    pub run_id: PromptLabRunId,
    pub stage: PromptLabStage,
    pub prompt_id: PromptId,
    pub input_snapshot: String,
    pub request_id: u64,
    pub overrides: PromptLabRunOverrides,
    pub compare_batch_id: Option<crate::prompt_lab::PromptLabCompareBatchId>,
    pub compare_candidate_id: Option<u64>,
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
            briefing_history: vec![],
            briefing_since_utc: None,
            pending_briefing_checkpoint_save: None,
            briefing_checkpoint_status_message: None,
            triage: TriageSession::default(),
            pre_triage: PreTriageSession::default(),
            pre_triage_load_context: None,
            pre_triage_load_progress: None,
            pre_triage_manual_overrides: HashMap::new(),
            indirect_link_pool: IndirectLinkPool::new(),
            indirect_poll_in_progress: false,
            source_states: SourceStateIndex::default(),
            prompt_contexts: HashMap::new(),
            prompt_contexts_load_failed: false,
            prompt_template_files_loaded: false,
            active_prompt_versions: HashMap::new(),
            effective_models: HashMap::new(),
            ai_availability: AiAvailability::Available,
            provider_alert: None,
            consecutive_rate_limit_failures: 0,
            summary_cache: SummaryCache::new(),
            signal_candidate: crate::signal_candidate::SignalCandidateSession::default(),
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
            briefing_orchestration: BriefingOrchestration::default(),
            triage_max_in_flight: 1,
            summary_max_in_flight: 1,
            prompt_lab: PromptLabState::default(),
            next_prompt_lab_run_id: 1,
            prompt_lab_next_resolve_id: 1,
            prompt_lab_templates: default_prompt_template_snapshots(),
            llm_usage_by_model: BTreeMap::new(),
            llm_quota: crate::LlmQuotaState::default(),
            active_tab: AppTab::default(),
            left_tab: LeftTab::default(),
            job_list_scope: JobListScope::default(),
            active_trend_category: TrendCategory::default(),
            entity_index: None,
            entity_trend_data: None,
            #[cfg(test)]
            next_triage_request_id: 1,
            triage_in_flight_request_id: None,
            poll_pipeline: None,
            tick: 0,
            pre_triage_coordinator: crate::pre_triage_coordinator::PreTriageRefreshCoordinator::new(
            ),
            pre_triage_refresh_eval_pending: false,
            pre_triage_refresh_eval_job_done: false,
            import_session: crate::import_session::ImportSessionState::default(),
            blacklist: crate::blacklist::BlacklistState::default(),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobResultKind {
    Success,
    Failed { reason: String },
}

#[cfg(test)]
mod tests;
