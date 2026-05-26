use crate::briefing::BriefingSession;
use crate::pre_triage_filter::{
    ArticleFilterKey, ManualDecision, PreTriagePhase, PreTriageSession,
};
use crate::preview::{self, PreviewContentKind};
use crate::prompt_lab::{
    PromptLabRunId, PromptLabRunOverrides, PromptLabStage, PromptLabState,
    PromptLabTemplateSnapshot,
};
use crate::source_state::{SourceInstanceState, SourceStateIndex};
use crate::summary_cache::SummaryCache;
use crate::tabs::{AppTab, JobListScope, LeftTab, TrendCategory};
use crate::triage::{ArticleTriageResult, TriagePhase, TriageSession};
use crate::triage_cache::TriageCache;
use crate::url_age::AgeEstimate;
#[cfg(test)]
use crate::view_model::OperationProgress;
use crate::view_model::{JobFilterStatus, LastPasteStats};
use crate::Effect;
use harvester_engine::llm::prompt::{PromptId, PromptRegistry, PromptVersion};
use harvester_engine::llm::run_metadata::LlmRunMetadata;
use harvester_engine::{ExtractedLink, ImportedArchiveRef, LinkKind, SourceId};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

mod briefing_orchestration;
mod cache_state;
mod indirect_links;
mod job_state;
mod link_helpers;
mod ui_state;
mod view_builder;

use briefing_orchestration::BriefingOrchestration;
use cache_state::{
    MetadataLoadState, SummaryCacheMetadataSnapshot, SummaryCacheMetrics,
    TriageCacheMetadataSnapshot, TriageCacheRunMetrics,
};
use indirect_links::{should_collect_indirect_link, IndirectLink, IndirectLinkPool};
use job_state::JobState;
#[cfg(test)]
use job_state::PreviewQuality;
use link_helpers::{
    build_link_rows, domain_from_url, format_lab_briefing_markdown, format_lab_summary_markdown,
    format_lab_triage_markdown, map_job_filter_status, normalize_extracted_link,
};
use ui_state::{MetricsState, PreviewMode, PreviewState, UiState};

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
    active_prompt_versions: HashMap<PromptId, PromptVersion>,
    effective_models: HashMap<PromptId, String>,
    ai_availability: AiAvailability,
    prompt_lab_templates: HashMap<PromptId, PromptLabTemplateSnapshot>,
    summary_cache: SummaryCache,
    signal_candidate: crate::signal_candidate::SignalCandidateSession,
    signal_candidate_cache: crate::signal_candidate_cache::SignalCandidateCache,
    signal_candidate_inputs:
        HashMap<String, crate::update::signal_candidate::SignalCandidateInputSnapshot>,
    signal_candidate_threshold: u8,
    signal_candidate_cap: usize,
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
            active_prompt_versions: HashMap::new(),
            effective_models: HashMap::new(),
            ai_availability: AiAvailability::Available,
            summary_cache: SummaryCache::new(),
            signal_candidate: crate::signal_candidate::SignalCandidateSession::default(),
            signal_candidate_cache: crate::signal_candidate_cache::SignalCandidateCache::default(),
            signal_candidate_inputs: HashMap::new(),
            signal_candidate_threshold: crate::signal_candidate::DEFAULT_SELECTION_THRESHOLD,
            signal_candidate_cap: crate::signal_candidate::DEFAULT_SELECTION_CAP,
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
        }
    }
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum concurrent triage LLM requests. Clamped to MAX_IN_FLIGHT_LIMIT.
    pub fn set_triage_max_in_flight(&mut self, limit: usize) {
        self.triage_max_in_flight = limit.clamp(1, MAX_IN_FLIGHT_LIMIT);
    }

    /// Set the maximum concurrent summary LLM requests. Clamped to MAX_IN_FLIGHT_LIMIT.
    pub fn set_summary_max_in_flight(&mut self, limit: usize) {
        self.summary_max_in_flight = limit.clamp(1, MAX_IN_FLIGHT_LIMIT);
    }

    pub fn triage_max_in_flight(&self) -> usize {
        self.triage_max_in_flight
    }

    pub fn summary_max_in_flight(&self) -> usize {
        self.summary_max_in_flight
    }

    /// Returns a snapshot of batch processing state for headless monitoring.
    /// Provides metrics without UI dependencies.
    pub fn batch_observation(&self) -> BatchObservation {
        // Count jobs by outcome visibility for batch settling:
        // - in-flight includes queued and actively processing jobs (no final outcome yet)
        // - done counts successful completions
        // - failed counts terminal failures
        let jobs_total = self.jobs.len();
        let mut jobs_done = 0;
        let mut jobs_failed = 0;
        let mut jobs_in_flight = 0;

        for job in self.jobs.values() {
            match job.outcome.as_ref() {
                Some(JobResultKind::Success) => jobs_done += 1,
                Some(JobResultKind::Failed { .. }) => jobs_failed += 1,
                None => jobs_in_flight += 1,
            }
        }

        // Triage metrics
        let (triage_total, triage_pending, triage_in_flight, triage_completed, triage_failed) =
            self.triage.observation_counts();
        let pre_triage_total = self.pre_triage.entries().len();
        let pre_triage_included = self.pre_triage.resolved_included_articles().len();
        let pre_triage_review = self
            .pre_triage
            .entries()
            .iter()
            .filter(|entry| {
                matches!(entry.auto_verdict, crate::AutoVerdict::Review)
                    && entry.manual_decision.is_none()
            })
            .count();
        let pre_triage_filtered =
            pre_triage_total.saturating_sub(pre_triage_included + pre_triage_review);
        let summary_total = self.briefing.articles().len();
        let summary_pending = self.briefing.pending_count();
        let summary_in_flight = self.briefing.in_progress_count();
        let summary_completed = self.briefing.completed_summary_count();
        let summary_failed = self.briefing.failed_summary_count();

        BatchObservation {
            poll_in_progress: self.source_states.is_poll_in_progress(),
            session_state: self.session,
            jobs_total,
            jobs_done,
            jobs_failed,
            jobs_in_flight,
            pre_triage_phase: self.pre_triage.phase().clone(),
            pre_triage_total,
            pre_triage_included,
            pre_triage_review,
            pre_triage_filtered,
            triage_phase: self.triage.phase().clone(),
            triage_total,
            triage_pending,
            triage_in_flight,
            triage_completed,
            triage_failed,
            summary_total,
            summary_pending,
            summary_in_flight,
            summary_completed,
            summary_failed,
            triage_cache_hits: self.triage_cache_run_metrics.hits() as usize,
            triage_cache_misses: self.triage_cache_run_metrics.misses() as usize,
            triage_cache_key_unavailable: self.triage_cache_run_metrics.key_unavailable() as usize,
            summary_cache_hits: self.summary_cache_metrics.hits(),
            summary_cache_misses: self.summary_cache_metrics.misses(),
            summary_cache_key_unavailable: self.summary_cache_metrics.key_unavailable(),
            import_phase: self.import_session.phase,
            imports_completed: self.import_session.imports_completed,
            imports_failed: self.import_session.imports_failed,
            import_in_flight: self.import_session.phase
                == crate::import_session::ImportPhase::Importing,
            source_poll_stats: self.source_states.last_completed_poll_stats().to_vec(),
        }
    }

    pub fn llm_request_state(&self, request_id: u64) -> Option<&LlmRequestState> {
        self.llm_requests.get(&request_id)
    }

    pub fn allocate_next_llm_request_id(&mut self) -> u64 {
        let id = self.next_llm_request_id;
        self.next_llm_request_id = self.next_llm_request_id.saturating_add(1);
        id
    }

    pub fn allocate_next_archive_request_id(&mut self) -> u64 {
        self.archive_request_id = self.archive_request_id.saturating_add(1);
        self.archive_request_id
    }

    pub fn archive_request_id(&self) -> u64 {
        self.archive_request_id
    }

    pub(crate) fn allocate_next_briefing_checkpoint_save_id(&mut self) -> u64 {
        let save_id = self.next_briefing_checkpoint_save_id;
        self.next_briefing_checkpoint_save_id =
            self.next_briefing_checkpoint_save_id.saturating_add(1);
        save_id
    }

    /// Pin a corpus snapshot for the current archive dialog session.
    ///
    /// Called when the archive dialog is opened so that both the open and submit
    /// handlers operate on the identical corpus the user saw when confirming.
    pub fn pin_archive_corpus(&mut self, corpus: crate::working_corpus::CurrentWorkingCorpus) {
        self.pinned_archive_corpus = Some(corpus);
    }

    /// Returns the corpus pinned at archive-open time, or `None` if no dialog is active.
    pub fn pinned_archive_corpus(&self) -> Option<&crate::working_corpus::CurrentWorkingCorpus> {
        self.pinned_archive_corpus.as_ref()
    }

    /// Clears the pinned corpus after the archive dialog session ends (submit, export
    /// completion, or export failure).
    ///
    /// Note: there is no `ArchiveCancelled` message dispatched when the user cancels the
    /// dialog (the UI returns early without dispatching), so we cannot clear on cancel.
    /// A subsequent `ArchiveClicked` will naturally overwrite the pin.
    pub fn clear_pinned_archive_corpus(&mut self) {
        self.pinned_archive_corpus = None;
    }

    /// Pin the signal-candidate archive selection snapshot for the current dialog session.
    pub fn pin_signal_candidate_selection(
        &mut self,
        selection: crate::signal_candidate::SignalCandidateArchiveSelection,
    ) {
        self.pinned_signal_candidate_selection = Some(selection);
    }

    /// Returns the signal-candidate snapshot pinned at archive-open time, if any.
    pub fn pinned_signal_candidate_selection(
        &self,
    ) -> Option<&crate::signal_candidate::SignalCandidateArchiveSelection> {
        self.pinned_signal_candidate_selection.as_ref()
    }

    /// Clears the pinned signal-candidate snapshot after the archive dialog session ends.
    pub fn clear_pinned_signal_candidate_selection(&mut self) {
        self.pinned_signal_candidate_selection = None;
    }

    /// Records LLM token usage from a completed run.
    /// Only CacheStatus::Miss runs are counted; empty or whitespace-only model names are ignored.
    pub fn record_llm_usage_from_metadata(&mut self, metadata: &LlmRunMetadata) {
        use harvester_engine::llm::run_metadata::CacheStatus;
        if metadata.cache_status != CacheStatus::Miss {
            return;
        }
        let model = metadata.resolved_model.trim();
        if model.is_empty() {
            return;
        }
        let entry = self
            .llm_usage_by_model
            .entry(model.to_string())
            .or_default();
        entry.0 = entry.0.saturating_add(u64::from(metadata.input_tokens));
        entry.1 = entry.1.saturating_add(u64::from(metadata.output_tokens));
    }

    /// Returns a sorted (alphabetical) snapshot of per-model token usage for rendering.
    pub fn llm_usage_rows(&self) -> Vec<crate::view_model::LlmModelUsageView> {
        self.llm_usage_by_model
            .iter()
            .map(
                |(model, &(input_tokens, output_tokens))| crate::view_model::LlmModelUsageView {
                    model: model.clone(),
                    input_tokens,
                    output_tokens,
                },
            )
            .collect()
    }

    pub fn llm_quota(&self) -> &crate::LlmQuotaState {
        &self.llm_quota
    }

    pub(crate) fn set_llm_quota_limits(&mut self, limits: crate::LlmQuotaLimits) {
        self.llm_quota.limits = Some(limits);
        self.llm_quota.ai_available = true;
    }

    pub(crate) fn set_llm_quota_usage(&mut self, usage: crate::LlmQuotaUsage) {
        self.llm_quota.usage = usage;
    }

    pub fn signal_candidate(&self) -> &crate::signal_candidate::SignalCandidateSession {
        &self.signal_candidate
    }

    pub fn signal_candidate_mut(&mut self) -> &mut crate::signal_candidate::SignalCandidateSession {
        &mut self.signal_candidate
    }

    pub fn signal_candidate_cache(&self) -> &crate::signal_candidate_cache::SignalCandidateCache {
        &self.signal_candidate_cache
    }

    pub(crate) fn set_signal_candidate_cache(
        &mut self,
        cache: crate::signal_candidate_cache::SignalCandidateCache,
    ) {
        self.signal_candidate_cache = cache;
    }

    pub fn try_reuse_signal_candidate(
        &self,
        key: &crate::signal_candidate_cache::SignalCandidateCacheKey,
    ) -> Option<harvester_engine::llm::dto::SignalCandidateResult> {
        self.signal_candidate_cache
            .get(key)
            .map(|entry| entry.result.clone())
    }

    pub fn store_signal_candidate_result(
        &mut self,
        key: crate::signal_candidate_cache::SignalCandidateCacheKey,
        result: harvester_engine::llm::dto::SignalCandidateResult,
        now_utc: String,
    ) {
        self.signal_candidate_cache.insert(
            key,
            crate::signal_candidate_cache::SignalCandidateCacheEntry {
                result,
                created_at_utc: now_utc,
            },
        );
    }

    pub(crate) fn signal_candidate_input_snapshot(
        &self,
        url: &str,
    ) -> Option<&crate::update::signal_candidate::SignalCandidateInputSnapshot> {
        self.signal_candidate_inputs.get(url)
    }

    pub(crate) fn set_signal_candidate_input_snapshot(
        &mut self,
        url: &str,
        snapshot: crate::update::signal_candidate::SignalCandidateInputSnapshot,
    ) {
        self.signal_candidate_inputs
            .insert(url.to_string(), snapshot);
    }

    pub(crate) fn clear_signal_candidate_input_snapshot(&mut self, url: &str) {
        self.signal_candidate_inputs.remove(url);
    }

    pub fn signal_candidate_threshold(&self) -> u8 {
        self.signal_candidate_threshold
    }

    pub fn signal_candidate_cap(&self) -> usize {
        self.signal_candidate_cap
    }

    pub fn set_signal_candidate_threshold(&mut self, threshold: u8) {
        self.signal_candidate_threshold = threshold.clamp(0, 100);
    }

    pub fn set_signal_candidate_cap(&mut self, cap: usize) {
        self.signal_candidate_cap = cap.max(1);
    }

    pub fn summary_cache_key_for_url(&self, url: &str) -> Option<crate::SummaryCacheKey> {
        let content_hash = self
            .briefing()
            .articles()
            .iter()
            .find(|article| article.url == url)
            .map(|article| article.content_hash.as_str())?;

        self.summary_cache()
            .iter()
            .filter(|(key, _)| {
                key.content_hash == content_hash && key.prompt_id == PromptId::ArticleSummary
            })
            .max_by(|(_, a), (_, b)| {
                let parsed_a = chrono::DateTime::parse_from_rfc3339(&a.created_at_utc)
                    .ok()
                    .map(|dt| dt.with_timezone(&chrono::Utc));
                let parsed_b = chrono::DateTime::parse_from_rfc3339(&b.created_at_utc)
                    .ok()
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                match (parsed_a, parsed_b) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    _ => a.created_at_utc.cmp(&b.created_at_utc),
                }
            })
            .map(|(key, _)| key.clone())
    }

    pub fn record_pending_llm_request(&mut self, request_id: u64, prompt_id: PromptId) {
        self.llm_requests
            .insert(request_id, LlmRequestState::Pending { prompt_id });
    }

    pub fn record_llm_result(&mut self, request_id: u64, state: LlmRequestState) {
        self.llm_requests.insert(request_id, state);
    }

    pub fn reset_llm_requests(&mut self) {
        self.llm_requests.clear();
        self.next_llm_request_id = 1;
    }

    pub(crate) fn briefing(&self) -> &BriefingSession {
        &self.briefing
    }

    pub(crate) fn briefing_mut(&mut self) -> &mut BriefingSession {
        &mut self.briefing
    }

    pub(crate) fn set_briefing(&mut self, briefing: BriefingSession) {
        self.briefing = briefing;
        self.dirty = true;
    }

    pub fn briefing_history(&self) -> &[crate::briefing::BriefingHistoryEntry] {
        &self.briefing_history
    }

    /// Prepends `entry` (newest first) and caps the list at 3 entries.
    pub fn push_briefing_history(&mut self, entry: crate::briefing::BriefingHistoryEntry) {
        self.briefing_history.insert(0, entry);
        self.briefing_history.truncate(3);
    }

    pub fn set_briefing_history(&mut self, entries: Vec<crate::briefing::BriefingHistoryEntry>) {
        self.briefing_history = entries;
    }

    pub fn briefing_since_utc(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.briefing_since_utc
    }

    pub(crate) fn set_briefing_since_utc(&mut self, v: Option<chrono::DateTime<chrono::Utc>>) {
        self.briefing_since_utc = v;
    }

    #[cfg(test)]
    pub(crate) fn pending_briefing_checkpoint_save(
        &self,
    ) -> Option<PendingBriefingCheckpointSaveSnapshot> {
        self.pending_briefing_checkpoint_save
            .as_ref()
            .map(|pending| PendingBriefingCheckpointSaveSnapshot {
                save_id: pending.save_id,
                previous_since_utc: pending.previous_since_utc,
                pending_since_utc: pending.pending_since_utc,
            })
    }

    pub(crate) fn begin_briefing_checkpoint_save(
        &mut self,
        pending_since_utc: Option<chrono::DateTime<chrono::Utc>>,
    ) -> u64 {
        let save_id = self.allocate_next_briefing_checkpoint_save_id();
        // A newer user-driven checkpoint change replaces any older pending save.
        // Matching is done by save_id, so late acks for older requests are dropped.
        self.pending_briefing_checkpoint_save = Some(PendingBriefingCheckpointSave {
            save_id,
            previous_since_utc: self.briefing_since_utc,
            pending_since_utc,
        });
        self.briefing_since_utc = pending_since_utc;
        self.briefing_checkpoint_status_message =
            Some(CHECKPOINT_SAVING_STATUS_MESSAGE.to_string());
        save_id
    }

    pub(crate) fn finish_briefing_checkpoint_save_success(&mut self, save_id: u64) -> bool {
        match self.pending_briefing_checkpoint_save.as_ref() {
            Some(pending) if pending.save_id == save_id => {
                self.pending_briefing_checkpoint_save = None;
                self.briefing_checkpoint_status_message = None;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn finish_briefing_checkpoint_save_failure(
        &mut self,
        save_id: u64,
        reason: &str,
    ) -> bool {
        match self.pending_briefing_checkpoint_save.as_ref() {
            Some(pending) if pending.save_id == save_id => {
                self.briefing_since_utc = pending.previous_since_utc;
                self.pending_briefing_checkpoint_save = None;
                self.briefing_checkpoint_status_message =
                    Some(format!("Checkpoint save failed: {reason}"));
                true
            }
            _ => false,
        }
    }

    pub(crate) fn clear_briefing_checkpoint_save_tracking(&mut self) {
        self.pending_briefing_checkpoint_save = None;
        self.briefing_checkpoint_status_message = None;
    }

    pub fn briefing_checkpoint_status_message(&self) -> Option<&str> {
        self.briefing_checkpoint_status_message.as_deref()
    }

    /// Backfills `fetched_utc` on jobs that have it as `None`, keyed by URL.
    /// Used to recover timestamps for jobs restored from pre-feature persisted state.
    pub(crate) fn backfill_jobs_fetched_utc(
        &mut self,
        url_to_fetched: &HashMap<String, chrono::DateTime<chrono::Utc>>,
    ) {
        for job in self.jobs.values_mut() {
            if job.fetched_utc.is_none() {
                if let Some(&dt) = url_to_fetched.get(&job.url) {
                    job.fetched_utc = Some(dt);
                }
            }
        }
    }

    pub(crate) fn triage(&self) -> &TriageSession {
        &self.triage
    }

    pub(crate) fn triage_mut(&mut self) -> &mut TriageSession {
        &mut self.triage
    }

    pub(crate) fn set_triage(&mut self, triage: TriageSession) {
        self.triage = triage;
        self.dirty = true;
    }

    pub(crate) fn pre_triage(&self) -> &PreTriageSession {
        &self.pre_triage
    }

    pub fn pre_triage_actionability(&self) -> crate::PreTriageActionability {
        match self.pre_triage.phase() {
            PreTriagePhase::LoadingArticles => crate::PreTriageActionability::Loading,
            PreTriagePhase::ReadyToTriage => {
                if self.pre_triage.resolved_included_articles().is_empty() {
                    crate::PreTriageActionability::Unavailable
                } else {
                    crate::PreTriageActionability::Ready
                }
            }
            PreTriagePhase::Reviewing => {
                if self.pre_triage.resolved_included_articles().is_empty() {
                    crate::PreTriageActionability::Unavailable
                } else {
                    crate::PreTriageActionability::ReadyWithPendingReview
                }
            }
            PreTriagePhase::Idle | PreTriagePhase::Failed { .. } => {
                crate::PreTriageActionability::Unavailable
            }
        }
    }

    pub fn can_start_triage_from_pre_triage(&self) -> bool {
        matches!(
            self.pre_triage_actionability(),
            crate::PreTriageActionability::Ready
                | crate::PreTriageActionability::ReadyWithPendingReview
        )
    }

    /// Consumes the pre-triage included articles for use in a triage session,
    /// resetting pre-triage to Idle. Returns `None` if pre-triage is not in an
    /// interactive phase or has no resolved articles. This is a one-way
    /// transition that ensures pre-triage cannot remain action-ready after its
    /// articles have been handed off.
    pub(crate) fn consume_interactive_pre_triage_articles_for_triage(
        &mut self,
    ) -> Option<Vec<crate::briefing::LoadedArticle>> {
        if !self.can_start_triage_from_pre_triage() {
            return None;
        }
        let articles = self.pre_triage.resolved_included_articles();
        if articles.is_empty() {
            return None;
        }
        self.pre_triage.reset();
        self.dirty = true;
        Some(articles)
    }

    /// Returns the current working corpus by applying source-precedence rules.
    ///
    /// See [`crate::working_corpus`] for the full precedence description.
    pub fn current_working_corpus(&self) -> crate::working_corpus::CurrentWorkingCorpus {
        crate::working_corpus::CurrentWorkingCorpus::select(
            self.pre_triage(),
            self.triage(),
            self.briefing_triage_policy(),
        )
    }

    pub fn batch_next_action(&self) -> crate::BatchNextAction {
        let pre_triage_included = self.pre_triage.resolved_included_articles().len();

        if self.can_start_triage_from_pre_triage()
            && !self.briefing_orchestration_requested()
            && self.triage.can_start()
            && self.triage.total() < pre_triage_included
        {
            return crate::BatchNextAction::DispatchTriage;
        }

        if matches!(self.triage.phase(), crate::TriagePhase::Complete)
            && self.triage.completed_count() > 0
            && self.briefing.can_start()
            && !self.triage.is_active()
            && self.briefing.articles().is_empty()
        {
            return crate::BatchNextAction::DispatchSummaries;
        }

        crate::BatchNextAction::None
    }

    pub fn batch_status(&self) -> crate::BatchStatus {
        let batch = self.batch_observation();
        let has_active_work = batch.poll_in_progress
            || matches!(
                batch.pre_triage_phase,
                crate::PreTriagePhase::LoadingArticles
            )
            || matches!(
                batch.triage_phase,
                crate::TriagePhase::LoadingArticles | crate::TriagePhase::Triaging
            )
            || batch.jobs_in_flight > 0
            || batch.triage_in_flight > 0
            || batch.summary_in_flight > 0
            || batch.summary_pending > 0
            || batch.import_in_flight;

        if has_active_work {
            crate::BatchStatus::Running
        } else {
            crate::BatchStatus::Settled
        }
    }

    /// Returns the corpus for archive export: triage-completed articles only.
    ///
    /// Pre-triage articles (even when ready) are excluded — they need triage first.
    pub(crate) fn archive_corpus(&self) -> crate::working_corpus::CurrentWorkingCorpus {
        crate::working_corpus::CurrentWorkingCorpus::select_for_archive(
            self.triage(),
            self.briefing_triage_policy(),
        )
    }

    /// Compute token estimates for the two archive modes for the given ordered URL list.
    ///
    /// **Limitation:** `full_tokens` aggregates `JobState::tokens`; articles whose job
    /// has been pruned, or imported articles without a job, contribute 0 and are likely
    /// underreported. Summary coverage uses the active triage session's URL to
    /// content-hash map.
    pub(crate) fn archive_token_estimates(&self, urls: &[String]) -> ArchiveTokenEstimates {
        use harvester_engine::archive_url_key;

        let url_tokens: HashMap<String, u64> = self
            .jobs
            .values()
            .filter_map(|job| {
                job.tokens
                    .map(|tokens| (archive_url_key(&job.url), tokens as u64))
            })
            .collect();

        let mut full_tokens = 0u64;
        let mut summary_tokens = 0u64;
        let mut summary_coverage = 0usize;

        for url in urls {
            let article_tokens = url_tokens.get(&archive_url_key(url)).copied().unwrap_or(0);
            full_tokens = full_tokens.saturating_add(article_tokens);

            let maybe_summary = self
                .triage()
                .article_content_hash(url)
                .and_then(|hash| self.summary_cache().lookup_any_by_content_hash(hash));

            if let Some(entry) = maybe_summary {
                summary_tokens = summary_tokens.saturating_add(entry.result.output_tokens as u64);
                summary_coverage += 1;
            } else {
                summary_tokens = summary_tokens.saturating_add(article_tokens);
            }
        }

        ArchiveTokenEstimates {
            full_tokens,
            summary_tokens,
            summary_coverage,
        }
    }

    pub(crate) fn summary_output_tokens_for_url(&self, url: &str) -> Option<u32> {
        let hash = self
            .triage()
            .article_content_hash(url)
            .or_else(|| self.pre_triage.article_content_hash(url))?;
        self.summary_cache()
            .lookup_any_by_content_hash(hash)
            .map(|entry| entry.result.output_tokens)
    }

    pub(crate) fn set_pre_triage(&mut self, pre_triage: PreTriageSession) {
        if !matches!(pre_triage.phase(), PreTriagePhase::LoadingArticles) {
            self.pre_triage_load_context = None;
            self.pre_triage_load_progress = None;
        }
        self.pre_triage = pre_triage;
        self.dirty = true;
    }

    pub(crate) fn set_pre_triage_load_context(
        &mut self,
        reason: crate::pre_triage_coordinator::PreTriageRefreshReason,
    ) {
        self.pre_triage_load_context = Some(PreTriageLoadContext { reason });
        self.dirty = true;
    }

    pub(crate) fn set_pre_triage_load_progress(
        &mut self,
        request_id: u64,
        files_scanned: usize,
        files_total: usize,
    ) {
        let progress = PreTriageLoadProgress {
            request_id,
            files_scanned,
            files_total,
        };
        if self.pre_triage_load_progress != Some(progress) {
            self.pre_triage_load_progress = Some(progress);
            self.dirty = true;
        }
    }

    pub(crate) fn clear_pre_triage_load_progress(&mut self) {
        if self.pre_triage_load_progress.take().is_some() {
            self.dirty = true;
        }
    }

    pub(crate) fn pre_triage_load_progress(&self) -> Option<(usize, usize, u64)> {
        self.pre_triage_load_progress.map(
            |PreTriageLoadProgress {
                 request_id,
                 files_scanned,
                 files_total,
             }| { (files_scanned, files_total, request_id) },
        )
    }

    pub fn is_pre_triage_reviewing(&self) -> bool {
        self.pre_triage.is_interactive()
    }

    pub fn pre_triage_key_for_job(&self, job_id: JobId) -> Option<crate::ArticleFilterKey> {
        self.pre_triage.key_for_job(job_id)
    }

    pub fn pre_triage_manual_overrides(&self) -> &HashMap<ArticleFilterKey, ManualDecision> {
        &self.pre_triage_manual_overrides
    }

    pub(crate) fn set_pre_triage_manual_overrides(
        &mut self,
        overrides: HashMap<ArticleFilterKey, ManualDecision>,
    ) {
        self.pre_triage_manual_overrides = overrides;
        self.pre_triage
            .apply_manual_overrides(&self.pre_triage_manual_overrides);
        self.dirty = true;
    }

    pub(crate) fn set_pre_triage_manual_decision(
        &mut self,
        key: ArticleFilterKey,
        decision: ManualDecision,
    ) -> bool {
        if self.pre_triage.set_manual_decision(&key, decision).is_err() {
            return false;
        }
        self.pre_triage_manual_overrides.insert(key, decision);
        self.dirty = true;
        true
    }

    pub(crate) fn clear_pre_triage_manual_overrides(&mut self) {
        self.pre_triage_manual_overrides.clear();
        self.pre_triage.clear_manual_decisions();
        self.dirty = true;
    }

    /// Allocate the next request ID for a pre-triage load.
    ///
    /// Only available in tests — used to inject a request ID into messages
    /// without driving the coordinator. In production, IDs are allocated
    /// exclusively by `PreTriageRefreshCoordinator`.
    #[cfg(test)]
    pub(crate) fn alloc_triage_request_id(&mut self) -> u64 {
        let id = self.next_triage_request_id;
        self.next_triage_request_id += 1;
        id
    }

    /// Record that a pre-triage load with the given request ID is in flight.
    pub(crate) fn set_triage_in_flight(&mut self, id: u64) {
        self.triage_in_flight_request_id = Some(id);
    }

    /// Clear the in-flight pre-triage load request (call on response or cancellation).
    pub(crate) fn clear_triage_in_flight(&mut self) {
        self.triage_in_flight_request_id = None;
    }

    /// Return the request ID of the currently in-flight pre-triage load, if any.
    pub fn triage_in_flight_request_id(&self) -> Option<u64> {
        self.triage_in_flight_request_id
    }

    /// Advance the logical tick counter by one. Called on every `Msg::Tick`.
    pub(crate) fn advance_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Return the current logical tick value.
    pub(crate) fn current_tick(&self) -> u64 {
        self.tick
    }

    #[allow(dead_code)]
    pub(crate) fn source_states(&self) -> &SourceStateIndex {
        &self.source_states
    }

    #[allow(dead_code)]
    pub(crate) fn source_state(&self, id: &SourceId) -> Option<&SourceInstanceState> {
        self.source_states.source_state(id)
    }

    #[allow(dead_code)]
    pub(crate) fn record_source_poll(&mut self, id: &SourceId, url_count: usize) {
        self.source_states.record_source_poll(id, url_count);
        self.dirty = true;
    }

    pub(crate) fn record_poll_stat(&mut self, stat: crate::SourcePollStat) {
        self.source_states.record_poll_stat(stat);
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub(crate) fn record_source_error(&mut self, id: &SourceId, error: String) {
        self.source_states.record_source_error(id, error);
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub(crate) fn start_poll(&mut self) -> bool {
        let started = self.source_states.start_poll();
        if started {
            self.poll_pipeline = Some(PollPipelineProgressState::default());
            self.dirty = true;
        }
        started
    }

    pub(crate) fn record_poll_pipeline_jobs(&mut self, job_ids: &[JobId]) {
        if job_ids.is_empty() {
            return;
        }
        if let Some(tracker) = &mut self.poll_pipeline {
            tracker.job_ids.extend(job_ids.iter().copied());
            self.dirty = true;
        }
    }

    pub(crate) fn set_poll_total(&mut self, total: usize) {
        self.source_states.set_poll_total(total);
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub(crate) fn end_poll(&mut self) {
        self.source_states.end_poll();
        if let Some(tracker) = &mut self.poll_pipeline {
            tracker.source_scan_done = true;
        }
        self.clear_settled_poll_pipeline_if_complete();
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub(crate) fn is_poll_in_progress(&self) -> bool {
        self.source_states.is_poll_in_progress()
    }

    pub fn ordered_completed_job_urls_snapshot(&self) -> Vec<String> {
        self.jobs
            .values()
            .filter_map(|job| {
                if job.stage == Stage::Done && job.outcome == Some(JobResultKind::Success) {
                    Some(job.url.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    pub(crate) fn request_pre_triage_refresh_evaluation(&mut self, triggered_by_job_done: bool) {
        self.pre_triage_refresh_eval_pending = true;
        if triggered_by_job_done {
            self.pre_triage_refresh_eval_job_done = true;
        }
    }

    pub fn take_pre_triage_refresh_evaluation_request(&mut self) -> Option<bool> {
        if !self.pre_triage_refresh_eval_pending {
            return None;
        }
        self.pre_triage_refresh_eval_pending = false;
        let triggered_by_job_done = self.pre_triage_refresh_eval_job_done;
        self.pre_triage_refresh_eval_job_done = false;
        Some(triggered_by_job_done)
    }

    /// Returns the current dirty flag and clears it in one step.
    pub fn consume_dirty(&mut self) -> bool {
        let was_dirty = self.dirty;
        self.dirty = false;
        was_dirty
    }

    /// Marks the state as dirty, signaling that a re-render is needed.
    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn completed_jobs_snapshot(&self) -> Vec<CompletedJobSnapshot> {
        self.jobs
            .values()
            .filter(|job| job.outcome == Some(JobResultKind::Success))
            .map(|job| CompletedJobSnapshot {
                url: job.url.clone(),
                tokens: job.tokens,
                bytes: job.bytes,
                links: job
                    .links
                    .iter()
                    .map(|link| LinkSnapshotRecord {
                        url: link.url.clone(),
                        downloaded_path: match &link.download_state {
                            LinkDownloadState::Downloaded { path } => {
                                Some(path.to_string_lossy().to_string())
                            }
                            _ => None,
                        },
                    })
                    .collect(),
                fetched_utc: job.fetched_utc.map(|dt| dt.to_rfc3339()),
            })
            .collect()
    }

    #[allow(dead_code)]
    pub fn job_links(&self, job_id: JobId) -> Option<&[LinkRecord]> {
        self.jobs.get(&job_id).map(|job| job.links())
    }

    /// Returns the completed triage result for a job, if that job has one.
    pub fn triage_result_for_job(&self, job_id: JobId) -> Option<&ArticleTriageResult> {
        self.jobs
            .get(&job_id)
            .and_then(|job| self.triage.result_for_url(&job.url))
    }

    /// Get the context variables for a specific prompt, if loaded.
    /// Returns an empty slice if no context has been loaded for this prompt.
    pub fn context_for(&self, prompt_id: PromptId) -> &[(String, String)] {
        self.prompt_contexts
            .get(&prompt_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub(crate) fn set_prompt_contexts(
        &mut self,
        contexts: HashMap<PromptId, Vec<(String, String)>>,
    ) {
        self.prompt_contexts = contexts;
        self.prompt_contexts_load_failed = false;
    }

    pub(crate) fn mark_prompt_contexts_load_failed(&mut self) {
        self.prompt_contexts_load_failed = true;
    }

    pub(crate) fn prompt_contexts_load_failed(&self) -> bool {
        self.prompt_contexts_load_failed
    }

    /// Get the active prompt version for a specific prompt.
    pub fn active_version_for(&self, prompt_id: PromptId) -> Option<PromptVersion> {
        self.active_prompt_versions.get(&prompt_id).copied()
    }

    /// Get the effective model for a specific prompt.
    pub fn effective_model_for(&self, prompt_id: PromptId) -> Option<&str> {
        self.effective_models.get(&prompt_id).map(|s| s.as_str())
    }

    pub fn ai_availability(&self) -> &AiAvailability {
        &self.ai_availability
    }

    pub fn triage_ai_available(&self) -> bool {
        matches!(self.ai_availability, AiAvailability::Available)
    }

    pub fn briefing_ai_available(&self) -> bool {
        matches!(self.ai_availability, AiAvailability::Available)
    }

    pub(crate) fn set_ai_availability(&mut self, availability: AiAvailability) {
        self.llm_quota.ai_available = matches!(availability, AiAvailability::Available);
        self.ai_availability = availability;
    }

    fn poll_pipeline_article_progress(&self) -> Option<(usize, usize)> {
        let tracker = self.poll_pipeline.as_ref()?;
        if !tracker.source_scan_done || tracker.job_ids.is_empty() {
            return None;
        }
        let total = tracker.job_ids.len();
        let settled = tracker
            .job_ids
            .iter()
            .filter(|job_id| {
                self.jobs
                    .get(job_id)
                    .and_then(|job| job.outcome.as_ref())
                    .is_some()
            })
            .count();
        (settled < total).then_some((settled, total))
    }

    fn clear_settled_poll_pipeline_if_complete(&mut self) {
        let Some(tracker) = self.poll_pipeline.as_ref() else {
            return;
        };
        if !tracker.source_scan_done {
            return;
        }
        let all_settled = tracker.job_ids.iter().all(|job_id| {
            self.jobs
                .get(job_id)
                .and_then(|job| job.outcome.as_ref())
                .is_some()
        });
        if all_settled {
            self.poll_pipeline = None;
            self.dirty = true;
        }
    }

    pub(crate) fn reconcile_ai_availability_from_metadata(&mut self) {
        let triage_model_available = self.effective_models.contains_key(&PromptId::ArticleTriage);
        match (&self.ai_availability, triage_model_available) {
            (
                AiAvailability::Unavailable {
                    reason: AiUnavailableReason::MissingApiKey,
                },
                _,
            ) => {}
            (_, true) => {
                self.ai_availability = AiAvailability::Available;
                self.llm_quota.ai_available = true;
            }
            (_, false) => {
                self.ai_availability = AiAvailability::Unavailable {
                    reason: AiUnavailableReason::NoTriageModel,
                };
                self.llm_quota.ai_available = false;
            }
        }
    }

    fn ai_unavailable_reason(&self) -> Option<AiUnavailableReason> {
        match self.ai_availability {
            AiAvailability::Available => None,
            AiAvailability::Unavailable { reason } => Some(reason),
        }
    }

    fn ai_unavailable_reason_text(&self) -> Option<&'static str> {
        match self.ai_unavailable_reason() {
            Some(AiUnavailableReason::MissingApiKey) => Some("OPENAI_API_KEY is not set"),
            Some(AiUnavailableReason::NoTriageModel) => Some("no triage model is available"),
            None => None,
        }
    }

    fn ai_unavailable_message(&self) -> Option<String> {
        self.ai_unavailable_reason_text()
            .map(|reason| format!("AI features unavailable: {reason}"))
    }

    fn ai_warning_banner(&self) -> Option<crate::InlineWarningView> {
        matches!(
            self.ai_unavailable_reason(),
            Some(AiUnavailableReason::MissingApiKey)
        )
        .then(|| crate::InlineWarningView {
            title: "AI features are disabled".to_string(),
            body: "Set OPENAI_API_KEY in the launch environment and restart to enable triage and briefing.".to_string(),
        })
    }

    fn triage_blocked_reason(&self) -> Option<String> {
        if let Some(reason) = self.ai_unavailable_reason() {
            return Some(match reason {
                AiUnavailableReason::MissingApiKey => {
                    "AI setup is incomplete because OPENAI_API_KEY is not set".to_string()
                }
                AiUnavailableReason::NoTriageModel => "no triage model is available".to_string(),
            });
        }

        if matches!(self.pre_triage.phase(), PreTriagePhase::LoadingArticles) {
            return Some(match self.pre_triage_load_context {
                Some(PreTriageLoadContext {
                    reason:
                        crate::pre_triage_coordinator::PreTriageRefreshReason::RestoreCompletedJobs,
                    ..
                }) => "Triage is unavailable while startup prepares the article set".to_string(),
                _ => "Triage is unavailable while the article set is being prepared".to_string(),
            });
        }

        None
    }

    fn briefing_blocked_reason(&self) -> Option<String> {
        self.ai_unavailable_reason().map(|reason| match reason {
            AiUnavailableReason::MissingApiKey => {
                "AI setup is incomplete because OPENAI_API_KEY is not set".to_string()
            }
            AiUnavailableReason::NoTriageModel => "no triage model is available".to_string(),
        })
    }

    fn pre_triage_loading_operation_label(&self) -> String {
        match self.pre_triage_load_context.map(|context| context.reason) {
            Some(crate::pre_triage_coordinator::PreTriageRefreshReason::RestoreCompletedJobs) => {
                "Preparing triage list".to_string()
            }
            Some(crate::pre_triage_coordinator::PreTriageRefreshReason::JobDone) => {
                "Updating triage candidates".to_string()
            }
            None => "Preparing triage list".to_string(),
        }
    }

    pub(crate) fn set_llm_metadata(
        &mut self,
        active_versions: HashMap<PromptId, PromptVersion>,
        effective_models: HashMap<PromptId, String>,
        templates: HashMap<PromptId, PromptLabTemplateSnapshot>,
    ) {
        self.active_prompt_versions = active_versions;
        self.effective_models = effective_models;
        self.prompt_lab_templates = templates;
    }

    pub fn prompt_lab_template_snapshot(
        &self,
        prompt_id: PromptId,
    ) -> Option<&PromptLabTemplateSnapshot> {
        self.prompt_lab_templates.get(&prompt_id)
    }

    pub(crate) fn restore_completed_jobs(&mut self, entries: Vec<CompletedJobSnapshot>) {
        if entries.is_empty() {
            return;
        }

        self.jobs.clear();
        self.seen_urls.clear();
        self.metrics = MetricsState::default();
        self.ui.urls.clear();
        self.ui.clear_preview();
        self.ui.clear_input_buffer();
        self.last_paste_stats = None;
        self.next_job_id = 1;
        self.reset_llm_requests();
        self.pre_triage = PreTriageSession::default();
        self.pre_triage_load_context = None;
        self.pre_triage_load_progress = None;
        self.pre_triage_manual_overrides.clear();

        for entry in entries {
            let CompletedJobSnapshot {
                url,
                tokens,
                bytes,
                links: link_snapshots,
                fetched_utc: snapshot_fetched_utc,
            } = entry;
            let restored_fetched_utc = snapshot_fetched_utc
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            self.jobs.insert(
                job_id,
                JobState {
                    url: url.clone(),
                    stage: Stage::Done,
                    outcome: Some(JobResultKind::Success),
                    tokens,
                    bytes,
                    content_preview: None,
                    preview_quality: None,
                    links: Vec::new(),
                    origin: JobOrigin::Direct,
                    fetched_utc: restored_fetched_utc,
                },
            );
            let extracted_links: Vec<ExtractedLink> = link_snapshots
                .iter()
                .map(|record| ExtractedLink {
                    url: record.url.clone(),
                    text: None,
                    kind: LinkKind::Hyperlink,
                })
                .collect();
            if let Some(job) = self.jobs.get_mut(&job_id) {
                job.attach_extracted_links(extracted_links);
                job.apply_link_snapshots(&link_snapshots);
            }
            let normalized = normalize_url_for_dedupe(&url);
            self.seen_urls.insert(normalized);
            if let Some(tokens) = tokens {
                self.metrics.total_tokens = self.metrics.total_tokens.saturating_add(tokens as u64);
            }
        }

        self.metrics.total_urls = self.jobs.len();
        self.session = SessionState::Idle;
        self.dirty = true;
        self.briefing = BriefingSession::default();
        self.triage = TriageSession::default();
        self.source_states = SourceStateIndex::default();
    }

    pub(crate) fn revert_preview_to_briefing(&mut self) {
        self.ui.set_preview_mode(PreviewMode::Briefing);
        self.dirty = true;
    }

    pub(crate) fn apply_imported_archive_entries(&mut self, entries: &[ImportedArchiveRef]) {
        if entries.is_empty() {
            return;
        }

        for entry in entries {
            let restored_fetched_utc = chrono::DateTime::parse_from_rfc3339(&entry.fetched_utc)
                .ok()
                .map(|dt| dt.with_timezone(&chrono::Utc));
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            self.jobs.insert(
                job_id,
                JobState {
                    url: entry.canonical_url.clone(),
                    stage: Stage::Done,
                    outcome: Some(JobResultKind::Success),
                    tokens: None,
                    bytes: None,
                    content_preview: None,
                    preview_quality: None,
                    links: Vec::new(),
                    origin: JobOrigin::Direct,
                    fetched_utc: restored_fetched_utc,
                },
            );
            self.seen_urls
                .insert(normalize_url_for_dedupe(&entry.canonical_url));
        }

        self.dirty = true;
    }

    /// Resolve the best available preview content for a given URL.
    ///
    /// Follows strict priority order:
    /// 1. Summary (if available)
    /// 2. Triage result (if summary missing but triage completed)
    /// 3. Exclusion reasons (if filtered/excluded in pre-triage)
    /// 4. Fallback message (if nothing else available)
    ///
    /// Returns (PreviewContentKind, formatted content).
    fn resolve_best_preview(&self, url: &str) -> (PreviewContentKind, String) {
        // Priority 1: Summary
        if let Some(summary) = self.briefing.summary_for_url(url) {
            return (
                PreviewContentKind::Summary,
                preview::format_summary_for_preview(summary),
            );
        }

        // Priority 2: Triage
        if let Some(triage_result) = self.triage.result_for_url(url) {
            let title =
                preview::best_effort_article_title(self.triage.source_title_for_url(url), url);
            return (
                PreviewContentKind::Triage,
                preview::format_triage_for_preview(title.as_deref(), triage_result),
            );
        }

        // Priority 3: Exclusion reasons
        if let Some(entry) = self.pre_triage.entry_for_url(url) {
            // Only show exclusion preview if article was excluded or needs review
            use crate::pre_triage_filter::{AutoVerdict, ManualDecision};
            let is_excluded = matches!(
                (entry.auto_verdict, entry.manual_decision),
                (AutoVerdict::HardExclude, None)
                    | (AutoVerdict::Review, None)
                    | (_, Some(ManualDecision::Exclude))
            );
            if is_excluded {
                return (
                    PreviewContentKind::Exclusion,
                    preview::format_exclusion_for_preview(entry),
                );
            }
        }

        // Priority 4: Fallback
        (
            PreviewContentKind::Fallback,
            preview::format_fallback_preview(),
        )
    }

    /// Refresh the preview for the currently selected job, if any.
    ///
    /// Re-runs resolve_best_preview and updates the UI state if the content changed.
    /// This is called after triage/summary completion to ensure the preview stays current.
    pub(crate) fn refresh_selected_preview(&mut self) {
        let Some(selected_job_id) = self.ui.selected_job_id() else {
            return;
        };
        let Some(job) = self.jobs.get(&selected_job_id) else {
            return;
        };

        let (kind, content) = self.resolve_best_preview(&job.url);
        let changed = self.ui.select_job(selected_job_id, Some((&content, kind)));
        if changed {
            engine_logging::engine_info!(
                "[preview] Preview upgraded for job {} (url={})",
                selected_job_id,
                job.url
            );
            self.dirty = true;
        }
    }

    pub(crate) fn select_job(&mut self, job_id: JobId) {
        let Some(job) = self.jobs.get(&job_id) else {
            return;
        };

        let (kind, content) = self.resolve_best_preview(&job.url);

        let changed = self.ui.select_job(job_id, Some((&content, kind)));
        if changed {
            self.ui.set_preview_mode(PreviewMode::SelectedJob);
            self.dirty = true;
        } else {
            // Even if preview content is same, ensure mode is set correctly
            self.ui.set_preview_mode(PreviewMode::SelectedJob);
        }
    }

    /// URL of the currently selected and summarized article.
    /// Returns None if no job is selected or if the selected job has no summary.
    pub fn selected_article_url(&self) -> Option<String> {
        let job_id = self.ui.selected_job_id()?;
        let job = self.jobs.get(&job_id)?;
        self.briefing.summary_for_url(&job.url)?;
        Some(job.url.clone())
    }

    pub fn selected_job_id(&self) -> Option<JobId> {
        self.ui.selected_job_id()
    }

    pub(crate) fn selected_job_has_summary(&self) -> bool {
        self.ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id))
            .and_then(|job| self.briefing.summary_for_url(&job.url))
            .is_some()
    }

    /// URL of the currently selected job, regardless of summarization state.
    pub(crate) fn selected_job_url(&self) -> Option<String> {
        let job_id = self.ui.selected_job_id()?;
        let job = self.jobs.get(&job_id)?;
        Some(job.url.clone())
    }

    pub(crate) fn link_metadata(
        &self,
        job_id: JobId,
        link_index: u32,
    ) -> Option<(String, Option<PathBuf>)> {
        self.jobs.get(&job_id).and_then(|job| {
            job.links
                .iter()
                .find(|record| record.index == link_index)
                .map(|record| {
                    (
                        record.url.clone(),
                        match &record.download_state {
                            LinkDownloadState::Downloaded { path } => Some(path.clone()),
                            _ => None,
                        },
                    )
                })
        })
    }

    pub fn link_state(&self, job_id: JobId, link_index: u32) -> Option<(LinkDownloadState, bool)> {
        self.jobs.get(&job_id).and_then(|job| {
            job.links
                .iter()
                .find(|record| record.index == link_index)
                .map(|record| (record.download_state.clone(), record.age_estimate.is_some()))
        })
    }

    pub fn job_filter_status(&self, job_id: JobId) -> Option<JobFilterStatus> {
        if !matches!(
            self.pre_triage.phase(),
            PreTriagePhase::Reviewing | PreTriagePhase::ReadyToTriage
        ) {
            return None;
        }

        let job = self.jobs.get(&job_id)?;
        self.pre_triage
            .entry_for_url(&job.url)
            .map(map_job_filter_status)
    }

    pub fn set_link_age_estimate(
        &mut self,
        job_id: JobId,
        link_index: u32,
        estimate: Option<AgeEstimate>,
    ) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            if let Some(record) = job
                .links
                .iter_mut()
                .find(|record| record.index == link_index)
            {
                record.age_estimate = estimate;
                self.dirty = true;
                return true;
            }
        }
        false
    }

    pub(crate) fn mark_link_download_requested(&mut self, job_id: JobId, link_index: u32) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_download_requested(link_index);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_link_download_completed(
        &mut self,
        job_id: JobId,
        link_index: u32,
        path: PathBuf,
    ) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_download_completed(link_index, path);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_link_download_failed(
        &mut self,
        job_id: JobId,
        link_index: u32,
        error: String,
    ) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_download_failed(link_index, error);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_link_deleted(&mut self, job_id: JobId, link_index: u32) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_deleted(link_index);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn session(&self) -> SessionState {
        self.session
    }

    pub(crate) fn stop_finish_button_state(&self) -> crate::StopFinishButtonState {
        let batch = self.batch_observation();
        let has_active_work = batch.jobs_in_flight > 0
            || batch.poll_in_progress
            || batch.import_in_flight
            || matches!(
                batch.triage_phase,
                crate::TriagePhase::LoadingArticles | crate::TriagePhase::Triaging
            )
            || matches!(
                self.briefing.phase(),
                crate::BriefingPhase::LoadingArticles
                    | crate::BriefingPhase::Summarizing
                    | crate::BriefingPhase::GeneratingBriefing
            );

        if matches!(self.session, SessionState::Running) && has_active_work {
            crate::StopFinishButtonState::Enabled {
                policy: crate::StopPolicy::Finish,
            }
        } else {
            crate::StopFinishButtonState::Disabled
        }
    }

    pub(crate) fn set_urls(&mut self, urls: Vec<String>) {
        self.ui.urls = urls;
        self.metrics.total_urls = self.ui.urls.len();
        self.dirty = true;
    }

    pub(crate) fn set_input_buffer(&mut self, text: String) {
        self.ui.set_input_buffer(text);
    }

    pub(crate) fn input_buffer(&self) -> &str {
        self.ui.input_buffer()
    }

    pub(crate) fn clear_input_buffer(&mut self) {
        self.ui.clear_input_buffer();
    }

    pub(crate) fn jobs_search_query(&self) -> &str {
        self.ui.jobs_search_query()
    }

    pub(crate) fn set_jobs_search_query(&mut self, text: String) {
        if self.ui.jobs_search_query() != text {
            self.ui.set_jobs_search_query(text);
            self.dirty = true;
        }
    }

    pub(crate) fn clear_jobs_search_query(&mut self) {
        if !self.ui.jobs_search_query().is_empty() {
            self.ui.clear_jobs_search_query();
            self.dirty = true;
        }
    }

    pub(crate) fn enqueue_jobs_from_ui(&mut self) -> Vec<(JobId, String)> {
        let mut enqueued = Vec::new();
        for url in self.ui.urls.iter() {
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            self.jobs.insert(
                job_id,
                Self::build_job_state(url.clone(), JobOrigin::Direct),
            );
            enqueued.push((job_id, url.clone()));
        }
        self.ui.urls.clear();
        self.dirty = true;
        enqueued
    }

    pub(crate) fn ingest_urls(&mut self, urls: Vec<String>) -> IngestResult {
        let mut unique = Vec::new();
        let mut skipped = 0;
        for url in urls {
            let normalized = normalize_url_for_dedupe(&url);
            if self.is_url_seen(&normalized) {
                skipped += 1;
            } else {
                unique.push(url);
            }
        }

        if unique.is_empty() {
            return IngestResult {
                effects: Vec::new(),
                enqueued: 0,
                skipped,
                enqueued_job_ids: Vec::new(),
            };
        }

        let should_start = self.session() == SessionState::Idle;
        if should_start {
            self.start_session();
        }

        self.set_urls(unique);
        let enqueued = self.enqueue_jobs_from_ui();
        let enqueued_count = enqueued.len();
        let mut effects = Vec::with_capacity(enqueued.len() + usize::from(should_start));
        if should_start {
            effects.push(Effect::StartSession);
        }
        let enqueued_job_ids = enqueued.iter().map(|(job_id, _)| *job_id).collect();
        for (job_id, url) in enqueued {
            effects.push(Effect::EnqueueUrl { job_id, url });
        }

        IngestResult {
            effects,
            enqueued: enqueued_count,
            skipped,
            enqueued_job_ids,
        }
    }

    fn build_job_state(url: String, origin: JobOrigin) -> JobState {
        JobState {
            url,
            stage: Stage::Queued,
            outcome: None,
            tokens: None,
            bytes: None,
            content_preview: None,
            preview_quality: None,
            links: Vec::new(),
            origin,
            fetched_utc: None,
        }
    }

    fn has_seen_url(&self, normalized_url: &str) -> bool {
        self.seen_urls.contains(normalized_url)
    }

    fn collect_indirect_links_from_job(&mut self, job_id: JobId) {
        let job = match self.jobs.get(&job_id) {
            Some(job) => job,
            None => return,
        };
        if job.origin != JobOrigin::Direct {
            return;
        }
        for link in &job.links {
            if link.kind != LinkKind::Hyperlink {
                continue;
            }
            if !should_collect_indirect_link(&job.url, &link.url) {
                continue;
            }
            let normalized = normalize_url_for_dedupe(&link.url);
            if normalized.is_empty() || self.has_seen_url(&normalized) {
                continue;
            }
            self.indirect_link_pool.add_link(IndirectLink {
                url: link.url.clone(),
                source_job_id: job_id,
            });
        }
    }

    pub(crate) fn ingest_indirect_links(&mut self, links: Vec<IndirectLink>) -> IngestResult {
        let mut unique = Vec::new();
        let mut skipped = 0;
        for link in links {
            let normalized = normalize_url_for_dedupe(&link.url);
            if normalized.is_empty() {
                continue;
            }
            if self.is_url_seen(&normalized) {
                skipped += 1;
                continue;
            }
            unique.push(link);
        }

        if unique.is_empty() {
            return IngestResult {
                effects: Vec::new(),
                enqueued: 0,
                skipped,
                enqueued_job_ids: Vec::new(),
            };
        }

        let should_start = self.session() == SessionState::Idle;
        if should_start {
            self.start_session();
        }

        let mut effects = Vec::with_capacity(unique.len() + usize::from(should_start));
        if should_start {
            effects.push(Effect::StartSession);
        }
        let mut enqueued = 0;
        let mut enqueued_job_ids = Vec::new();
        for link in unique {
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            enqueued_job_ids.push(job_id);
            self.jobs.insert(
                job_id,
                Self::build_job_state(
                    link.url.clone(),
                    JobOrigin::Indirect {
                        source_job_id: link.source_job_id,
                    },
                ),
            );
            effects.push(Effect::EnqueueUrl {
                job_id,
                url: link.url,
            });
            enqueued += 1;
        }

        IngestResult {
            effects,
            enqueued,
            skipped,
            enqueued_job_ids,
        }
    }

    pub(crate) fn begin_indirect_link_generation(&mut self) {
        self.indirect_link_pool.begin_new_generation();
    }

    pub(crate) fn drain_indirect_links(&mut self) -> Vec<IndirectLink> {
        self.indirect_link_pool.draining_links()
    }

    pub(crate) fn set_indirect_poll_in_progress(&mut self, value: bool) {
        self.indirect_poll_in_progress = value;
    }

    pub(crate) fn indirect_poll_in_progress(&self) -> bool {
        self.indirect_poll_in_progress
    }

    pub(crate) fn has_indirect_links(&self) -> bool {
        !self.indirect_link_pool.is_empty()
    }

    pub(crate) fn apply_progress(
        &mut self,
        job_id: JobId,
        stage: Stage,
        tokens: Option<u32>,
        bytes: Option<u64>,
        content_preview: Option<String>,
    ) {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.stage = stage;
            if let Some(t) = tokens {
                if job.tokens != Some(t) {
                    let previous = job.tokens.unwrap_or(0) as u64;
                    self.metrics.total_tokens = self
                        .metrics
                        .total_tokens
                        .saturating_sub(previous)
                        .saturating_add(t as u64);
                    job.tokens = Some(t);
                }
            }
            if let Some(b) = bytes {
                job.bytes = Some(b);
            }
            if let Some(content) = content_preview {
                let selected = self.ui.selected_job_id() == Some(job_id);
                if selected {
                    self.ui.set_preview_state(PreviewState::InProgress {
                        job_id,
                        content: content.clone(),
                    });
                }
                job.set_preview_content(content);
            }
            self.dirty = true;
        }
    }

    pub(crate) fn apply_done(
        &mut self,
        job_id: JobId,
        result: JobResultKind,
        content_preview: Option<String>,
        extracted_links: Vec<ExtractedLink>,
        msg_fetched_utc: Option<String>,
    ) {
        let job_updated = if let Some(job) = self.jobs.get_mut(&job_id) {
            job.stage = Stage::Done;
            job.outcome = Some(result);
            job.fetched_utc = msg_fetched_utc
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));
            if matches!(job.outcome.as_ref(), Some(JobResultKind::Success)) {
                if let Some(content) = content_preview {
                    job.set_preview_content(content);
                }
                job.attach_extracted_links(extracted_links);
                self.collect_indirect_links_from_job(job_id);
            } else {
                job.clear_preview_content();
                job.clear_links();
            }
            true
        } else {
            false
        };
        if job_updated && self.ui.selected_job_id() == Some(job_id) {
            self.refresh_selected_preview();
        }
        if job_updated {
            self.clear_settled_poll_pipeline_if_complete();
            self.dirty = true;
        }
    }

    pub(crate) fn start_session(&mut self) {
        self.session = SessionState::Running;
        self.dirty = true;
    }

    pub(crate) fn finish_session(&mut self) {
        self.session = SessionState::Finishing;
        self.dirty = true;
    }

    pub(crate) fn set_last_paste_stats(&mut self, enqueued: usize, skipped: usize) {
        self.last_paste_stats = Some(LastPasteStats { enqueued, skipped });
        self.dirty = true;
    }

    /// Check if URL has been seen before. If not, insert it and return false.
    /// If yes, return true (indicating it should be skipped).
    pub(crate) fn is_url_seen(&mut self, normalized_url: &str) -> bool {
        !self.seen_urls.insert(normalized_url.to_owned())
    }

    /// Returns the current left panel width (PANEL_INPUT + PANEL_JOBS combined).
    pub(crate) fn left_panel_width(&self) -> i32 {
        self.ui.left_panel_width()
    }

    pub(crate) fn input_panel_visible(&self) -> bool {
        self.ui.input_panel_visible()
    }

    /// Sets the left panel width.
    pub(crate) fn set_left_panel_width(&mut self, width: i32) {
        self.ui.set_left_panel_width(width);
    }

    pub(crate) fn set_input_panel_visible(&mut self, visible: bool) {
        self.ui.set_input_panel_visible(visible);
    }

    /// Returns the current window width.
    pub(crate) fn window_width(&self) -> i32 {
        self.ui.window_width()
    }

    /// Sets the window width.
    pub(crate) fn set_window_width(&mut self, width: i32) {
        self.ui.set_window_width(width);
    }

    // ------------------------------------------------------------------
    // Prompt Lab command API
    // ------------------------------------------------------------------

    pub(crate) fn prompt_lab(&self) -> &PromptLabState {
        &self.prompt_lab
    }

    // Used in tests; will be used by the reducer when UI override messages are added.
    #[allow(dead_code)]
    pub(crate) fn prompt_lab_mut(&mut self) -> &mut PromptLabState {
        &mut self.prompt_lab
    }

    pub(crate) fn select_tab(&mut self, tab: AppTab) {
        if self.active_tab != tab {
            self.active_tab = tab;
            self.dirty = true;
        }
    }

    pub fn active_tab(&self) -> AppTab {
        self.active_tab
    }

    pub(crate) fn select_left_tab(&mut self, tab: LeftTab) {
        if self.left_tab != tab {
            self.left_tab = tab;
            self.dirty = true;
        }
    }

    pub fn left_tab(&self) -> LeftTab {
        self.left_tab
    }

    pub fn job_list_scope(&self) -> JobListScope {
        self.job_list_scope
    }

    pub(crate) fn set_job_list_scope(&mut self, scope: JobListScope) {
        if self.job_list_scope != scope {
            self.job_list_scope = scope;
            self.dirty = true;
        }
    }

    pub(crate) fn set_active_trend_category(&mut self, category: TrendCategory) {
        self.active_trend_category = category;
        self.dirty = true;
    }

    pub fn active_trend_category(&self) -> TrendCategory {
        self.active_trend_category
    }

    /// Store a freshly loaded or rebuilt entity index and re-compute trend data.
    pub(crate) fn set_entity_index(
        &mut self,
        index: crate::entity_index::EntityIndex,
        window_weeks: u32,
        top_n: usize,
    ) {
        self.entity_trend_data = Some(crate::trends::compute_trends(&index, window_weeks, top_n));
        self.entity_index = Some(index);
        self.dirty = true;
    }

    pub fn entity_trend_data(&self) -> Option<&crate::trends::EntityTrendData> {
        self.entity_trend_data.as_ref()
    }

    /// Set the active left tab unconditionally.
    pub(crate) fn set_left_tab(&mut self, tab: LeftTab) {
        self.select_left_tab(tab);
    }

    pub(crate) fn open_prompt_lab(&mut self) {
        self.select_left_tab(LeftTab::PromptLab);
        self.prompt_lab.open();
    }

    /// Close Prompt Lab internal state (panel state, etc.) without changing `left_tab`.
    pub(crate) fn close_prompt_lab_internals(&mut self) {
        self.prompt_lab.close();
    }

    pub(crate) fn select_prompt_lab_stage(&mut self, stage: PromptLabStage) {
        self.prompt_lab.select_stage(stage);
        self.dirty = true;
    }

    pub(crate) fn set_prompt_lab_input(&mut self, text: String) {
        self.prompt_lab.set_input(text);
    }

    pub(crate) fn allocate_next_prompt_lab_run_id(&mut self) -> PromptLabRunId {
        let id = PromptLabRunId(self.next_prompt_lab_run_id);
        self.next_prompt_lab_run_id = self.next_prompt_lab_run_id.saturating_add(1);
        id
    }

    pub(crate) fn allocate_next_prompt_lab_resolve_id(&mut self) -> u64 {
        let id = self.prompt_lab_next_resolve_id;
        self.prompt_lab_next_resolve_id = self.prompt_lab_next_resolve_id.saturating_add(1);
        id
    }

    pub(crate) fn add_prompt_lab_pending_run(
        &mut self,
        registration: PromptLabPendingRunRegistration,
    ) {
        self.prompt_lab.add_pending_run(
            registration.run_id,
            registration.stage,
            registration.prompt_id,
            registration.input_snapshot,
            registration.request_id,
            registration.overrides,
        );
        if let Some(record) = self.prompt_lab.run_by_id_mut(registration.run_id) {
            record.compare_batch_id = registration.compare_batch_id;
            record.compare_candidate_id = registration.compare_candidate_id;
        }
    }

    pub(crate) fn complete_prompt_lab_run(
        &mut self,
        run_id: PromptLabRunId,
        output_json: String,
        metadata: LlmRunMetadata,
    ) {
        self.prompt_lab.complete_run(run_id, output_json, metadata);
    }

    pub(crate) fn fail_prompt_lab_run(
        &mut self,
        run_id: PromptLabRunId,
        reason: String,
        metadata: Option<LlmRunMetadata>,
    ) {
        self.prompt_lab.fail_run(run_id, reason, metadata);
    }

    pub(crate) fn consume_prompt_lab_ownership(&mut self, request_id: u64) {
        self.prompt_lab.consume_ownership(request_id);
    }

    pub(crate) fn clear_prompt_lab_history(&mut self) {
        self.prompt_lab.clear_history();
        self.dirty = true;
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
