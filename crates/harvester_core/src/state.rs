use crate::briefing::BriefingSession;
use crate::context_hash;
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
use crate::triage::{ArticleTriageResult, ArticleTriageState, TriagePhase, TriageSession};
use crate::triage_cache::{TriageCache, TriageCacheKey};
use crate::url_age::{guess_age_from_url, AgeEstimate};
use crate::view_model::{
    AppViewModel, JobFilterStatus, JobRowView, LastPasteStats, LinkRowView, PreviewHeaderView,
    TriageAnnotationView, DEFAULT_JOBS_PANEL_WIDTH, DEFAULT_WINDOW_WIDTH, TOKEN_LIMIT,
};
use crate::Effect;
use harvester_engine::llm::prompt::{PromptId, PromptRegistry, PromptVersion};
use harvester_engine::llm::run_metadata::LlmRunMetadata;
use harvester_engine::{
    truncate_to_char_boundary, ExtractedLink, ImportedArchiveRef, LinkKind, SourceId,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use url::Url;

pub type JobId = u64;

const MAX_EXTRACTED_LINKS: usize = 5_000;
const LINK_ROW_LIMIT: usize = 200;
const LINK_LABEL_MAX: usize = 80;
const LINK_LABEL_TRUNCATE_MARKER: &str = "…";

fn default_prompt_template_snapshots() -> HashMap<PromptId, PromptLabTemplateSnapshot> {
    let registry = PromptRegistry::with_defaults();
    let prompt_ids = [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetadataLoadState {
    Idle,
    Pending,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryCacheMetadataSnapshot {
    prompt_version: PromptVersion,
    model_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TriageCacheMetadataSnapshot {
    prompt_version: PromptVersion,
    model_id: String,
    context_hash: String,
}

pub(crate) enum TriageCacheLookupResult<'a> {
    Hit(&'a ArticleTriageResult),
    Miss,
    KeyUnavailable,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BriefingOrchestration {
    requested: bool,
    skip_aggregate_briefing: bool,
    priority_cutoff_exclusive: u8,
    prereq_articles: Option<Vec<crate::briefing::LoadedArticle>>,
}

impl Default for BriefingOrchestration {
    fn default() -> Self {
        Self {
            requested: false,
            skip_aggregate_briefing: false,
            priority_cutoff_exclusive: 1,
            prereq_articles: None,
        }
    }
}

impl BriefingOrchestration {
    fn request(&mut self, skip_aggregate_briefing: bool) {
        self.requested = true;
        self.skip_aggregate_briefing = skip_aggregate_briefing;
    }

    fn store_prereq(&mut self, articles: Vec<crate::briefing::LoadedArticle>) {
        self.prereq_articles = Some(articles);
    }

    fn take_prereq(&mut self) -> Option<Vec<crate::briefing::LoadedArticle>> {
        self.prereq_articles.take()
    }

    fn clear(&mut self) {
        self.requested = false;
        self.skip_aggregate_briefing = false;
        self.prereq_articles = None;
    }

    fn is_requested(&self) -> bool {
        self.requested
    }

    fn policy(&self) -> crate::briefing::TriageSelectionPolicy {
        crate::briefing::TriageSelectionPolicy {
            cutoff_exclusive: self.priority_cutoff_exclusive,
            exclude_untriaged: true,
        }
    }

    fn clear_request(&mut self) {
        self.requested = false;
    }

    fn skip_aggregate_briefing(&self) -> bool {
        self.skip_aggregate_briefing
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SummaryCacheMetrics {
    hits: usize,
    misses: usize,
    key_unavailable: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TriageCacheRunMetrics {
    hits: u32,
    misses: u32,
    key_unavailable: u32,
}

impl TriageCacheRunMetrics {
    pub(crate) fn hits(&self) -> u32 {
        self.hits
    }

    pub(crate) fn misses(&self) -> u32 {
        self.misses
    }

    pub(crate) fn key_unavailable(&self) -> u32 {
        self.key_unavailable
    }

    pub(crate) fn total(&self) -> u32 {
        self.hits + self.misses + self.key_unavailable
    }
}

impl SummaryCacheMetrics {
    pub(crate) fn hits(&self) -> usize {
        self.hits
    }

    pub(crate) fn misses(&self) -> usize {
        self.misses
    }

    pub(crate) fn key_unavailable(&self) -> usize {
        self.key_unavailable
    }

    pub(crate) fn total(&self) -> usize {
        self.hits + self.misses + self.key_unavailable
    }
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
    llm_requests: LlmResultIndex,
    briefing: BriefingSession,
    briefing_history: Vec<crate::briefing::BriefingHistoryEntry>,
    briefing_since_utc: Option<chrono::DateTime<chrono::Utc>>,
    triage: TriageSession,
    pre_triage: PreTriageSession,
    pre_triage_manual_overrides: HashMap<ArticleFilterKey, ManualDecision>,
    source_states: SourceStateIndex,
    prompt_contexts: HashMap<PromptId, Vec<(String, String)>>,
    active_prompt_versions: HashMap<PromptId, PromptVersion>,
    effective_models: HashMap<PromptId, String>,
    prompt_lab_templates: HashMap<PromptId, PromptLabTemplateSnapshot>,
    summary_cache: SummaryCache,
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
            llm_requests: LlmResultIndex::new(),
            briefing: BriefingSession::default(),
            briefing_history: vec![],
            briefing_since_utc: None,
            triage: TriageSession::default(),
            pre_triage: PreTriageSession::default(),
            pre_triage_manual_overrides: HashMap::new(),
            source_states: SourceStateIndex::default(),
            prompt_contexts: HashMap::new(),
            active_prompt_versions: HashMap::new(),
            effective_models: HashMap::new(),
            summary_cache: SummaryCache::new(),
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
            active_tab: AppTab::default(),
            left_tab: LeftTab::default(),
            job_list_scope: JobListScope::default(),
            active_trend_category: TrendCategory::default(),
            entity_index: None,
            entity_trend_data: None,
            #[cfg(test)]
            next_triage_request_id: 1,
            triage_in_flight_request_id: None,
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
        }
    }

    pub fn view(&self) -> AppViewModel {
        let since = self.briefing_since_utc();
        let mut jobs: Vec<JobRowView> = self
            .jobs
            .iter()
            .map(|(id, job)| {
                // UI tab uses strict semantics: unknown fetch time → exclude.
                // (Archive/briefing use the inclusive passes_since_filter helper instead.)
                let is_since = match (job.fetched_utc, since) {
                    (_, None) => true,
                    (None, Some(_)) => false,
                    (Some(t), Some(s)) => t >= s,
                };
                job.to_view(*id, is_since)
            })
            .collect();
        for job_view in &mut jobs {
            if let Some(result) = self.triage.result_for_url(&job_view.url) {
                job_view.triage_annotation = Some(TriageAnnotationView {
                    priority: result.priority,
                    category: result.category.clone(),
                    tags: result.tags.clone(),
                });
            }
        }
        // Block 2: summary availability and headline projection.
        for job_view in &mut jobs {
            if let Some(summary) = self.briefing.summary_for_url(&job_view.url) {
                job_view.has_summary = true;
                job_view.summary_title = Some(summary.title.clone());
            } else {
                job_view.has_summary = false;
                job_view.summary_title = None;
            }
        }
        if matches!(
            self.pre_triage.phase(),
            PreTriagePhase::Reviewing | PreTriagePhase::ReadyToTriage
        ) {
            for job_view in &mut jobs {
                job_view.filter_status = self
                    .pre_triage
                    .entry_for_url(&job_view.url)
                    .map(map_job_filter_status);
            }
        }

        // Block 3: has_analysis — true if summary, triage, or exclusion data exists
        for job_view in &mut jobs {
            job_view.has_analysis = job_view.has_summary
                || job_view.triage_annotation.is_some()
                || matches!(
                    job_view.filter_status,
                    Some(JobFilterStatus::HardExcluded { .. })
                        | Some(JobFilterStatus::ReviewNeeded { .. })
                        | Some(JobFilterStatus::ManuallyExcluded)
                );
        }

        // Jobs remain in BTreeMap (job_id) insertion order — stable regardless of triage.
        // Tab-specific ordering (e.g. triage-priority for TriageResults) is applied in
        // the render layer (build_job_tree) so the Jobs tab is never affected.

        // Derive selected_url — expose for any selected job
        let selected_url = self
            .ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id))
            .map(|job| job.url.clone());
        let briefing_preview = self.briefing.format_preview();
        let preview_text = match self.ui.preview_mode() {
            PreviewMode::SelectedJob => self.ui.preview_content().map(ToOwned::to_owned),
            PreviewMode::Briefing => briefing_preview
                .clone()
                .or_else(|| self.ui.preview_content().map(ToOwned::to_owned)),
        };
        let preview_header = self
            .ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id))
            .map(|job| {
                let quality = job.preview_quality.unwrap_or_default();
                PreviewHeaderView {
                    domain: domain_from_url(&job.url),
                    tokens: job.tokens,
                    bytes: job.bytes,
                    stage: job.stage,
                    outcome: job.outcome.clone(),
                    heading_count: quality.heading_count,
                    link_density: quality.link_density,
                    nav_heavy: quality.nav_heavy(),
                }
            });
        let selected_triage_article_available = self
            .ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id))
            .and_then(|job| {
                let selected_norm = normalize_url_for_dedupe(&job.url);
                self.triage()
                    .articles()
                    .iter()
                    .find(|article| {
                        normalize_url_for_dedupe(&article.url) == selected_norm
                            && matches!(article.triage_state, ArticleTriageState::Completed { .. })
                    })
                    .map(|_| ())
            })
            .is_some();
        let preview_source = self.ui.preview.content_kind();
        AppViewModel {
            session: self.session,
            queued_urls: self.ui.urls.clone(),
            job_count: self.jobs.len(),
            jobs,
            last_paste_stats: self.last_paste_stats.clone(),
            dirty: self.dirty,
            total_tokens: self.metrics.total_tokens,
            token_limit: TOKEN_LIMIT,
            preview_text,
            preview_header,
            preview_source,
            briefing_can_start: self.briefing.can_start(),
            briefing_progress: self.briefing.progress_text(),
            briefing_preview,
            triage_can_start: (!self.briefing_orchestration.is_requested())
                && self.triage.can_start()
                && matches!(self.pre_triage.phase(), PreTriagePhase::ReadyToTriage),
            triage_progress: self
                .triage
                .progress_text()
                .or_else(|| self.pre_triage_progress_text()),
            poll_sources_enabled: matches!(
                self.session,
                SessionState::Idle | SessionState::Running
            ) && !self.source_states.is_poll_in_progress(),
            left_panel_width: self.ui.left_panel_width(),
            input_panel_visible: self.ui.input_panel_visible(),
            window_width: self.ui.window_width(),
            selected_url,
            left_pane: crate::view_model::LeftPaneView {
                left_tab: self.left_tab,
                job_list_scope: self.job_list_scope,
                prompt_lab: crate::view_model::PromptLabView::from_state(
                    &self.prompt_lab,
                    &self.prompt_contexts,
                    &self.prompt_lab_templates,
                    selected_triage_article_available,
                ),
            },
            is_pre_triage_reviewing: self.pre_triage.is_interactive(),
            llm_usage_by_model: self.llm_usage_rows(),
            right_pane: self.build_right_pane_view(selected_triage_article_available),
        }
    }

    fn build_right_pane_view(
        &self,
        selected_triage_article_available: bool,
    ) -> crate::view_model::RightPaneView {
        use crate::view_model::RightPaneView;

        let selected_url = self
            .ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id))
            .map(|job| job.url.as_str());

        // Build triage markdown for the selected job.
        let triage_markdown = selected_url.and_then(|url| {
            self.triage.result_for_url(url).map(|r| {
                let tags_line = if r.tags.is_empty() {
                    "none".to_string()
                } else {
                    r.tags.join(", ")
                };
                format!(
                    "**Category:** {}\n**Priority:** P{}\n**Tags:** {}\n\n---\n\n{}\n",
                    r.category, r.priority, tags_line, r.rationale
                )
            })
        });

        // Build summary markdown for the selected job.
        let summary_markdown = selected_url.and_then(|url| {
            self.briefing.summary_for_url(url).map(|s| {
                let kp_lines: String = s.key_points.iter().map(|kp| format!("- {kp}\n")).collect();
                format!(
                    "# {}\n\n{}\n\n**Key Points:**\n\n{}\n",
                    s.title, s.summary, kp_lines
                )
            })
        });

        // Briefing markdown — use the existing briefing preview.
        let briefing_markdown = self.briefing.format_preview();

        let prompt_lab = crate::view_model::PromptLabView::from_state(
            &self.prompt_lab,
            &self.prompt_contexts,
            &self.prompt_lab_templates,
            selected_triage_article_available,
        );

        // When the left pane is showing the Prompt Lab, route the latest completed lab
        // run result into the matching right-pane viewer so the user can see it inline.
        let (effective_triage_markdown, effective_summary_markdown, effective_briefing_markdown) =
            if self.left_tab == LeftTab::PromptLab {
                let lab_triage = prompt_lab.latest_run.as_ref().and_then(|run| {
                    use crate::prompt_lab::PromptLabStage;
                    if run.stage == PromptLabStage::Triage {
                        run.output_json.as_deref().map(format_lab_triage_markdown)
                    } else {
                        None
                    }
                });
                let lab_summary = prompt_lab.latest_run.as_ref().and_then(|run| {
                    use crate::prompt_lab::PromptLabStage;
                    if run.stage == PromptLabStage::Summary {
                        run.output_json.as_deref().map(format_lab_summary_markdown)
                    } else {
                        None
                    }
                });
                let lab_briefing = prompt_lab.latest_run.as_ref().and_then(|run| {
                    use crate::prompt_lab::PromptLabStage;
                    if run.stage == PromptLabStage::Briefing {
                        run.output_json.as_deref().map(format_lab_briefing_markdown)
                    } else {
                        None
                    }
                });
                (
                    lab_triage.or(triage_markdown),
                    lab_summary.or(summary_markdown),
                    lab_briefing.or(briefing_markdown),
                )
            } else {
                (triage_markdown, summary_markdown, briefing_markdown)
            };

        let _ = prompt_lab; // moved into LeftPaneView via view()

        let trends = crate::view_model::build_trends_tab_view(
            self.entity_trend_data.as_ref(),
            self.active_trend_category,
        );

        RightPaneView {
            active_tab: self.active_tab,
            triage_markdown: effective_triage_markdown,
            summary_markdown: effective_summary_markdown,
            briefing_markdown: effective_briefing_markdown,
            trends,
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

    pub(crate) fn set_pre_triage(&mut self, pre_triage: PreTriageSession) {
        self.pre_triage = pre_triage;
        self.dirty = true;
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

    fn pre_triage_progress_text(&self) -> Option<String> {
        let total = self.pre_triage.entries().len();
        if total == 0 {
            return None;
        }
        let include = self.pre_triage.resolved_included_articles().len();
        let review = self
            .pre_triage
            .entries()
            .iter()
            .filter(|entry| {
                matches!(entry.auto_verdict, crate::AutoVerdict::Review)
                    && entry.manual_decision.is_none()
            })
            .count();
        let filtered = total.saturating_sub(include + review);
        match self.pre_triage.phase() {
            PreTriagePhase::LoadingArticles => Some("Pre-triage loading...".to_string()),
            PreTriagePhase::Reviewing | PreTriagePhase::ReadyToTriage => Some(format!(
                "Pre-triage: {} include, {} review, {} filtered",
                include, review, filtered
            )),
            PreTriagePhase::Failed { reason } => Some(format!("Pre-triage failed: {reason}")),
            PreTriagePhase::Idle => None,
        }
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

    #[allow(dead_code)]
    pub(crate) fn record_source_error(&mut self, id: &SourceId, error: String) {
        self.source_states.record_source_error(id, error);
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub(crate) fn start_poll(&mut self) -> bool {
        let started = self.source_states.start_poll();
        if started {
            self.dirty = true;
        }
        started
    }

    #[allow(dead_code)]
    pub(crate) fn end_poll(&mut self) {
        self.source_states.end_poll();
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub(crate) fn is_poll_in_progress(&self) -> bool {
        self.source_states.is_poll_in_progress()
    }

    pub fn ordered_completed_job_urls_snapshot(&self) -> Vec<String> {
        self.jobs
            .iter()
            .filter_map(|(_, job)| {
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
    }

    /// Get the active prompt version for a specific prompt.
    pub fn active_version_for(&self, prompt_id: PromptId) -> Option<PromptVersion> {
        self.active_prompt_versions.get(&prompt_id).copied()
    }

    /// Get the effective model for a specific prompt.
    pub fn effective_model_for(&self, prompt_id: PromptId) -> Option<&str> {
        self.effective_models.get(&prompt_id).map(|s| s.as_str())
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

    /// Try to reuse a cached summary result for the given cache key.
    /// Returns None if there is no cached entry for this key.
    pub(crate) fn try_reuse_summary(
        &self,
        key: &crate::summary_cache::SummaryCacheKey,
    ) -> Option<&crate::briefing::ArticleSummaryResult> {
        self.summary_cache.lookup(key).map(|entry| &entry.result)
    }

    /// Store a summary result in the cache with the given key.
    pub(crate) fn store_summary_result(
        &mut self,
        key: crate::summary_cache::SummaryCacheKey,
        result: crate::briefing::ArticleSummaryResult,
        created_at_utc: String,
    ) {
        let entry = crate::summary_cache::SummaryCacheEntry {
            result,
            created_at_utc,
        };
        self.summary_cache.insert(key, entry);
    }

    /// Replace the entire summary cache (used for hydration).
    pub(crate) fn set_summary_cache(&mut self, cache: SummaryCache) {
        self.summary_cache = cache;
    }

    pub(crate) fn start_summary_cache_run(&mut self) {
        self.summary_cache_metrics = SummaryCacheMetrics::default();
        self.summary_cache_metadata_snapshot = None;
        self.summary_cache_warmup_logged = false;
        self.briefing_metadata_state = MetadataLoadState::Pending;
    }

    pub(crate) fn mark_briefing_metadata_ready(&mut self) {
        if self.briefing_metadata_state != MetadataLoadState::Pending {
            return;
        }
        let snapshot = match (
            self.active_prompt_versions
                .get(&PromptId::ArticleSummary)
                .copied(),
            self.effective_models
                .get(&PromptId::ArticleSummary)
                .cloned(),
        ) {
            (Some(version), Some(model_id)) => Some(SummaryCacheMetadataSnapshot {
                prompt_version: version,
                model_id,
            }),
            _ => None,
        };
        self.summary_cache_metadata_snapshot = snapshot;
        self.briefing_metadata_state = MetadataLoadState::Ready;
    }

    pub(crate) fn is_briefing_metadata_ready(&self) -> bool {
        matches!(self.briefing_metadata_state, MetadataLoadState::Ready)
    }

    pub(crate) fn summary_cache_metadata(&self) -> Option<(PromptVersion, &str)> {
        self.summary_cache_metadata_snapshot
            .as_ref()
            .map(|snapshot| (snapshot.prompt_version, snapshot.model_id.as_str()))
    }

    pub(crate) fn summary_cache_warmup_logged(&self) -> bool {
        self.summary_cache_warmup_logged
    }

    pub(crate) fn mark_summary_cache_warmup_logged(&mut self) {
        self.summary_cache_warmup_logged = true;
    }

    pub(crate) fn record_summary_cache_hit(&mut self) {
        self.summary_cache_metrics.hits += 1;
    }

    pub(crate) fn record_summary_cache_miss(&mut self) {
        self.summary_cache_metrics.misses += 1;
    }

    pub(crate) fn record_summary_cache_key_unavailable(&mut self) {
        self.summary_cache_metrics.key_unavailable += 1;
    }

    pub(crate) fn summary_cache_metrics(&self) -> SummaryCacheMetrics {
        self.summary_cache_metrics
    }

    pub(crate) fn finalize_summary_cache_run(&mut self) {
        self.briefing_metadata_state = MetadataLoadState::Idle;
        self.summary_cache_metadata_snapshot = None;
        self.summary_cache_warmup_logged = false;
    }

    pub(crate) fn briefing_orchestration_requested(&self) -> bool {
        self.briefing_orchestration.is_requested()
    }

    pub(crate) fn request_briefing_orchestration(&mut self) {
        self.briefing_orchestration.request(false);
    }

    pub(crate) fn request_summary_preparation(&mut self) {
        self.briefing_orchestration.request(true);
    }

    pub(crate) fn store_briefing_prereq_articles(
        &mut self,
        articles: Vec<crate::briefing::LoadedArticle>,
    ) {
        self.briefing_orchestration.store_prereq(articles);
    }

    pub(crate) fn take_briefing_prereq_articles(
        &mut self,
    ) -> Option<Vec<crate::briefing::LoadedArticle>> {
        self.briefing_orchestration.take_prereq()
    }

    pub(crate) fn clear_briefing_orchestration(&mut self) {
        self.briefing_orchestration.clear();
    }

    pub(crate) fn clear_briefing_orchestration_request(&mut self) {
        self.briefing_orchestration.clear_request();
    }

    pub(crate) fn briefing_triage_policy(&self) -> crate::briefing::TriageSelectionPolicy {
        self.briefing_orchestration.policy()
    }

    pub(crate) fn briefing_orchestration_skip_aggregate(&self) -> bool {
        self.briefing_orchestration.skip_aggregate_briefing()
    }

    /// Get an immutable reference to the summary cache.
    pub(crate) fn summary_cache(&self) -> &SummaryCache {
        &self.summary_cache
    }
    pub(crate) fn set_triage_cache(&mut self, cache: TriageCache) {
        self.triage_cache = cache;
    }

    pub fn triage_cache(&self) -> &TriageCache {
        &self.triage_cache
    }

    pub(crate) fn start_triage_cache_run(&mut self) {
        self.triage_cache_run_metrics = TriageCacheRunMetrics::default();
        self.triage_cache_run_start_logged = false;
    }

    pub(crate) fn mark_triage_metadata_pending(&mut self) {
        self.triage_metadata_state = MetadataLoadState::Pending;
    }

    pub(crate) fn mark_triage_metadata_ready(&mut self) {
        let snapshot = match (
            self.active_prompt_versions
                .get(&PromptId::ArticleTriage)
                .copied(),
            self.effective_models.get(&PromptId::ArticleTriage).cloned(),
        ) {
            (Some(prompt_version), Some(model_id)) => Some(TriageCacheMetadataSnapshot {
                prompt_version,
                model_id,
                context_hash: context_hash(self.context_for(PromptId::ArticleTriage)),
            }),
            _ => {
                self.triage_metadata_state = MetadataLoadState::Pending;
                None
            }
        };
        if snapshot.is_some() {
            self.triage_metadata_state = MetadataLoadState::Ready;
        }
        self.triage_cache_metadata_snapshot = snapshot;
    }

    pub(crate) fn triage_metadata_ready(&self) -> bool {
        matches!(self.triage_metadata_state, MetadataLoadState::Ready)
    }

    pub(crate) fn triage_cache_metadata(&self) -> Option<(PromptVersion, &str, &str)> {
        self.triage_cache_metadata_snapshot
            .as_ref()
            .map(|snapshot| {
                (
                    snapshot.prompt_version,
                    snapshot.model_id.as_str(),
                    snapshot.context_hash.as_str(),
                )
            })
    }

    pub(crate) fn try_reuse_triage(&self, content_hash: &str) -> TriageCacheLookupResult<'_> {
        let snapshot = match &self.triage_cache_metadata_snapshot {
            Some(snapshot) => snapshot,
            None => return TriageCacheLookupResult::KeyUnavailable,
        };
        let key = match TriageCacheKey::try_new_with_context_hash(
            content_hash,
            PromptId::ArticleTriage,
            Some(snapshot.prompt_version),
            Some(snapshot.model_id.as_str()),
            &snapshot.context_hash,
        ) {
            Ok(key) => key,
            Err(_) => return TriageCacheLookupResult::KeyUnavailable,
        };
        match self.triage_cache.lookup(&key) {
            Some(result) => TriageCacheLookupResult::Hit(result),
            None => TriageCacheLookupResult::Miss,
        }
    }

    pub(crate) fn store_triage_result(&mut self, content_hash: &str, result: ArticleTriageResult) {
        let snapshot = match &self.triage_cache_metadata_snapshot {
            Some(snapshot) => snapshot,
            None => return,
        };
        let key = match TriageCacheKey::try_new_with_context_hash(
            content_hash,
            PromptId::ArticleTriage,
            Some(snapshot.prompt_version),
            Some(snapshot.model_id.as_str()),
            &snapshot.context_hash,
        ) {
            Ok(key) => key,
            Err(_) => return,
        };

        self.triage_cache.insert(key, result);
    }

    pub(crate) fn record_triage_cache_hit(&mut self) {
        self.triage_cache_run_metrics.hits = self.triage_cache_run_metrics.hits.saturating_add(1);
    }

    pub(crate) fn record_triage_cache_miss(&mut self) {
        self.triage_cache_run_metrics.misses =
            self.triage_cache_run_metrics.misses.saturating_add(1);
    }

    pub(crate) fn record_triage_cache_key_unavailable(&mut self) {
        self.triage_cache_run_metrics.key_unavailable = self
            .triage_cache_run_metrics
            .key_unavailable
            .saturating_add(1);
    }

    pub(crate) fn triage_cache_metrics(&self) -> &TriageCacheRunMetrics {
        &self.triage_cache_run_metrics
    }

    pub(crate) fn triage_cache_run_start_logged(&self) -> bool {
        self.triage_cache_run_start_logged
    }

    pub(crate) fn mark_triage_cache_run_started(&mut self) {
        self.triage_cache_run_start_logged = true;
    }

    pub(crate) fn finalize_triage_cache_run(&mut self) {
        self.triage_cache_run_start_logged = false;
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
            return (
                PreviewContentKind::Triage,
                preview::format_triage_for_preview(triage_result),
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

    pub(crate) fn enqueue_jobs_from_ui(&mut self) -> Vec<(JobId, String)> {
        let mut enqueued = Vec::new();
        for url in self.ui.urls.iter() {
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            self.jobs.insert(
                job_id,
                JobState {
                    url: url.clone(),
                    stage: Stage::Queued,
                    outcome: None,
                    tokens: None,
                    bytes: None,
                    content_preview: None,
                    preview_quality: None,
                    links: Vec::new(),
                    fetched_utc: None,
                },
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
        for (job_id, url) in enqueued {
            effects.push(Effect::EnqueueUrl { job_id, url });
        }

        IngestResult {
            effects,
            enqueued: enqueued_count,
            skipped,
        }
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
pub fn normalize_url_for_dedupe(url: &str) -> String {
    let trimmed = url.trim();
    let lowercased = trimmed.to_lowercase();
    lowercased.trim_end_matches('/').to_owned()
}

fn normalize_extracted_link(link: &str) -> String {
    let trimmed = link.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if let Ok(mut parsed) = Url::parse(trimmed) {
        parsed.set_fragment(None);
        if let Some(port) = parsed.port() {
            let normalized_port = match parsed.scheme() {
                "http" if port == 80 => None,
                "https" if port == 443 => None,
                _ => Some(port),
            };
            let _ = parsed.set_port(normalized_port);
        }
        parsed.into()
    } else {
        trimmed.to_string()
    }
}

/// Format a lab triage JSON output as readable markdown for the right-pane Triage viewer.
fn format_lab_triage_markdown(output_json: &str) -> String {
    use harvester_engine::llm::validation::validate_triage;
    match validate_triage(output_json) {
        Ok(result) => {
            let tags_line = if result.tags.is_empty() {
                "none".to_string()
            } else {
                result.tags.join(", ")
            };
            format!(
                "**\\[Lab\\]** **Category:** {}\n**Priority:** P{}\n**Tags:** {}\n\n---\n\n{}\n",
                result.category,
                result.priority.value(),
                tags_line,
                result.rationale
            )
        }
        Err(_) => format!("**\\[Lab Triage\\]**\n\n```json\n{output_json}\n```\n"),
    }
}

/// Format a lab summary JSON output as readable markdown for the right-pane Summary viewer.
fn format_lab_summary_markdown(output_json: &str) -> String {
    use harvester_engine::llm::validation::validate_summary;
    match validate_summary(output_json) {
        Ok(result) => {
            let kp_lines: String = result
                .key_points
                .iter()
                .map(|kp| format!("- {kp}\n"))
                .collect();
            format!(
                "# \\[Lab\\] {}\n\n{}\n\n**Key Points:**\n\n{}\n",
                result.title, result.summary, kp_lines
            )
        }
        Err(_) => format!("**\\[Lab Summary\\]**\n\n```json\n{output_json}\n```\n"),
    }
}

/// Format a lab briefing JSON output as readable markdown for the right-pane Briefing viewer.
fn format_lab_briefing_markdown(output_json: &str) -> String {
    format!("**\\[Lab Briefing\\]**\n\n```json\n{output_json}\n```\n")
}

fn domain_from_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .find("://")
        .map(|pos| &trimmed[pos + 3..])
        .unwrap_or(trimmed);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    if host.is_empty() {
        trimmed.to_string()
    } else {
        host.to_string()
    }
}

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

#[derive(Debug, Clone, PartialEq, Default)]
struct JobState {
    url: String,
    stage: Stage,
    outcome: Option<JobResultKind>,
    tokens: Option<u32>,
    bytes: Option<u64>,
    content_preview: Option<String>,
    preview_quality: Option<PreviewQuality>,
    links: Vec<LinkRecord>,
    fetched_utc: Option<chrono::DateTime<chrono::Utc>>,
}

impl JobState {
    fn to_view(&self, id: JobId, is_since_checkpoint: bool) -> JobRowView {
        let links = build_link_rows(&self.links);
        let downloaded_link_count = self
            .links
            .iter()
            .filter(|link| matches!(link.download_state, LinkDownloadState::Downloaded { .. }))
            .count();
        JobRowView {
            job_id: id,
            url: self.url.clone(),
            stage: self.stage,
            outcome: self.outcome.clone(),
            tokens: self.tokens,
            bytes: self.bytes,
            link_count: self.links.len(),
            downloaded_link_count,
            links,
            triage_annotation: None,
            has_summary: false,
            summary_title: None,
            filter_status: None,
            has_analysis: false,
            is_since_checkpoint,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn content_preview(&self) -> Option<&str> {
        self.content_preview.as_deref()
    }

    fn set_preview_content(&mut self, content: String) {
        self.preview_quality = Some(PreviewQuality::from_markdown(&content));
        self.content_preview = Some(content);
    }

    fn clear_preview_content(&mut self) {
        self.preview_quality = None;
        self.content_preview = None;
    }
    #[allow(dead_code)]
    fn links(&self) -> &[LinkRecord] {
        &self.links
    }

    #[allow(dead_code)]
    fn clear_links(&mut self) {
        self.links.clear();
    }

    fn attach_extracted_links(&mut self, links: Vec<ExtractedLink>) {
        self.links.clear();
        let mut seen = HashSet::new();
        for (idx, link) in links.into_iter().enumerate() {
            if self.links.len() >= MAX_EXTRACTED_LINKS {
                break;
            }
            let canonical = normalize_extracted_link(&link.url);
            if canonical.is_empty() {
                continue;
            }
            if !seen.insert(canonical.clone()) {
                continue;
            }
            self.links.push(LinkRecord {
                index: idx as u32,
                url: canonical.clone(),
                anchor_text: link.text,
                kind: link.kind,
                download_state: LinkDownloadState::NotDownloaded,
                age_estimate: guess_age_from_url(&canonical),
            });
        }
    }

    fn apply_link_snapshots(&mut self, snapshots: &[LinkSnapshotRecord]) {
        for snapshot in snapshots {
            if let Some(path) = snapshot.downloaded_path.as_ref() {
                let canonical = normalize_extracted_link(&snapshot.url);
                if canonical.is_empty() {
                    continue;
                }
                if let Some(record) = self.links.iter_mut().find(|record| record.url == canonical) {
                    record.download_state = LinkDownloadState::Downloaded {
                        path: PathBuf::from(path),
                    };
                }
            }
        }
    }

    #[allow(dead_code)]
    fn find_link_mut(&mut self, link_index: u32) -> Option<&mut LinkRecord> {
        self.links
            .iter_mut()
            .find(|record| record.index == link_index)
    }

    #[allow(dead_code)]
    fn mark_link_download_requested(&mut self, link_index: u32) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::Downloading;
        }
    }

    #[allow(dead_code)]
    fn mark_link_download_completed(&mut self, link_index: u32, path: PathBuf) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::Downloaded { path };
        }
    }

    #[allow(dead_code)]
    fn mark_link_download_failed(&mut self, link_index: u32, error: String) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::Failed { error };
        }
    }

    #[allow(dead_code)]
    fn mark_link_deleted(&mut self, link_index: u32) {
        if let Some(record) = self.find_link_mut(link_index) {
            record.download_state = LinkDownloadState::NotDownloaded;
        }
    }
}

fn map_job_filter_status(entry: &crate::ArticleFilterEntry) -> JobFilterStatus {
    match entry.manual_decision {
        Some(crate::ManualDecision::Exclude) => JobFilterStatus::ManuallyExcluded,
        Some(crate::ManualDecision::Include) => JobFilterStatus::ManuallyIncluded,
        None => match entry.auto_verdict {
            crate::AutoVerdict::HardExclude => JobFilterStatus::HardExcluded {
                reasons: entry.reasons.clone(),
            },
            crate::AutoVerdict::Review => JobFilterStatus::ReviewNeeded {
                reasons: entry.reasons.clone(),
            },
            crate::AutoVerdict::Include => JobFilterStatus::AutoIncluded,
        },
    }
}

fn build_link_rows(records: &[LinkRecord]) -> Vec<LinkRowView> {
    records
        .iter()
        .take(LINK_ROW_LIMIT)
        .map(|record| LinkRowView {
            index: record.index,
            url: record.url.clone(),
            label: link_label_for_record(record),
            kind: record.kind.clone(),
            download_state: record.download_state.clone(),
            age_suspect: record.age_estimate.is_some(),
        })
        .collect()
}

fn link_label_for_record(record: &LinkRecord) -> String {
    if let Some(text) = record
        .anchor_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        text.to_string()
    } else {
        truncate_link_url(&record.url)
    }
}

fn truncate_link_url(url: &str) -> String {
    if url.chars().count() <= LINK_LABEL_MAX {
        url.to_string()
    } else {
        let max_chars = LINK_LABEL_MAX
            .saturating_sub(LINK_LABEL_TRUNCATE_MARKER.len())
            .max(1);
        let truncated = truncate_to_char_boundary(url, max_chars);
        format!("{truncated}{LINK_LABEL_TRUNCATE_MARKER}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PreviewQuality {
    heading_count: usize,
    link_density: f64,
}

impl Default for PreviewQuality {
    fn default() -> Self {
        Self {
            heading_count: 0,
            link_density: 0.0,
        }
    }
}

impl PreviewQuality {
    const NAV_HEAVY_THRESHOLD: f64 = 0.3;

    fn from_markdown(content: &str) -> Self {
        let heading_count = content
            .lines()
            .filter(|line| line.trim_start().starts_with('#'))
            .count();
        let link_count = content
            .split('[')
            .skip(1)
            .filter(|segment| segment.contains("]("))
            .count();
        let word_count = content.split_whitespace().count();
        let link_density = if word_count > 0 {
            link_count as f64 / word_count as f64
        } else {
            0.0
        };
        Self {
            heading_count,
            link_density,
        }
    }

    fn nav_heavy(&self) -> bool {
        self.link_density > Self::NAV_HEAVY_THRESHOLD
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct MetricsState {
    total_urls: usize,
    total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum PreviewState {
    #[default]
    Empty,
    Available {
        job_id: JobId,
        content: String,
        kind: PreviewContentKind,
    },
    InProgress {
        job_id: JobId,
        content: String,
    },
    Unavailable {
        job_id: JobId,
    },
}

impl PreviewState {
    fn job_id(&self) -> Option<JobId> {
        match self {
            PreviewState::Empty => None,
            PreviewState::Available { job_id, .. }
            | PreviewState::InProgress { job_id, .. }
            | PreviewState::Unavailable { job_id } => Some(*job_id),
        }
    }

    fn content(&self) -> Option<&str> {
        match self {
            PreviewState::Available { content, .. } | PreviewState::InProgress { content, .. } => {
                Some(content.as_str())
            }
            PreviewState::Empty | PreviewState::Unavailable { .. } => None,
        }
    }

    fn content_kind(&self) -> Option<PreviewContentKind> {
        match self {
            PreviewState::Available { kind, .. } => Some(*kind),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PreviewMode {
    #[default]
    Briefing,
    SelectedJob,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UiState {
    urls: Vec<String>,
    input_buffer: String,
    preview: PreviewState,
    preview_mode: PreviewMode,
    left_panel_width: i32,
    input_panel_visible: bool,
    window_width: i32,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            urls: Vec::new(),
            input_buffer: String::new(),
            preview: PreviewState::default(),
            preview_mode: PreviewMode::default(),
            left_panel_width: DEFAULT_JOBS_PANEL_WIDTH,
            input_panel_visible: false,
            window_width: DEFAULT_WINDOW_WIDTH,
        }
    }
}

impl UiState {
    fn preview_content(&self) -> Option<&str> {
        self.preview.content()
    }

    fn preview_mode(&self) -> PreviewMode {
        self.preview_mode
    }

    fn set_preview_mode(&mut self, mode: PreviewMode) {
        self.preview_mode = mode;
    }

    fn selected_job_id(&self) -> Option<JobId> {
        self.preview.job_id()
    }

    fn select_job(&mut self, job_id: JobId, content: Option<(&str, PreviewContentKind)>) -> bool {
        let next_state = match content {
            Some((text, kind)) => PreviewState::Available {
                job_id,
                content: text.to_owned(),
                kind,
            },
            None => PreviewState::Unavailable { job_id },
        };
        self.set_preview_state(next_state)
    }

    fn clear_preview(&mut self) -> bool {
        self.set_preview_state(PreviewState::Empty)
    }

    fn set_preview_state(&mut self, next: PreviewState) -> bool {
        if self.preview == next {
            false
        } else {
            self.preview = next;
            true
        }
    }

    fn set_input_buffer(&mut self, text: String) {
        self.input_buffer = text;
    }

    fn input_buffer(&self) -> &str {
        &self.input_buffer
    }

    fn clear_input_buffer(&mut self) {
        self.input_buffer.clear();
    }

    fn left_panel_width(&self) -> i32 {
        self.left_panel_width
    }

    fn input_panel_visible(&self) -> bool {
        self.input_panel_visible
    }

    fn set_left_panel_width(&mut self, width: i32) {
        self.left_panel_width = width;
    }

    fn set_input_panel_visible(&mut self, visible: bool) {
        self.input_panel_visible = visible;
    }

    fn window_width(&self) -> i32 {
        self.window_width
    }

    fn set_window_width(&mut self, width: i32) {
        self.window_width = width;
    }
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
mod tests {
    use super::*;
    use crate::{update, Msg};
    use harvester_engine::{ExtractedLink, LinkKind};

    #[test]
    fn job_done_success_stores_preview() {
        let mut state = AppState::new();
        state.jobs.insert(
            1,
            JobState {
                url: "https://example.com".to_string(),
                stage: Stage::Queued,
                ..Default::default()
            },
        );
        state.apply_done(
            1,
            JobResultKind::Success,
            Some("preview content".to_string()),
            Vec::new(),
            None,
        );
        let job = state.jobs.get(&1).expect("job exists");
        assert_eq!(job.content_preview(), Some("preview content"));
    }

    #[test]
    fn batch_observation_poll_in_progress_tracks_source_poll_state() {
        let mut state = AppState::new();
        assert!(!state.batch_observation().poll_in_progress);

        assert!(state.start_poll());
        assert!(state.batch_observation().poll_in_progress);

        state.end_poll();
        assert!(!state.batch_observation().poll_in_progress);
    }

    #[test]
    fn batch_observation_counts_queued_jobs_as_in_flight() {
        let mut state = AppState::new();
        state.jobs.insert(
            1,
            JobState {
                url: "https://queued.example".to_string(),
                stage: Stage::Queued,
                outcome: None,
                ..Default::default()
            },
        );
        state.jobs.insert(
            2,
            JobState {
                url: "https://done.example".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );
        state.jobs.insert(
            3,
            JobState {
                url: "https://failed.example".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Failed {
                    reason: "boom".to_string(),
                }),
                ..Default::default()
            },
        );

        let obs = state.batch_observation();
        assert_eq!(obs.jobs_total, 3);
        assert_eq!(obs.jobs_in_flight, 1);
        assert_eq!(obs.jobs_done, 1);
        assert_eq!(obs.jobs_failed, 1);
    }

    #[test]
    fn job_done_failure_clears_preview() {
        let mut state = AppState::new();
        state.jobs.insert(
            2,
            JobState {
                url: "https://example.com".to_string(),
                stage: Stage::Queued,
                content_preview: Some("old preview".to_string()),
                ..Default::default()
            },
        );
        state.apply_done(
            2,
            JobResultKind::Failed {
                reason: "ignored".to_string(),
            },
            Some("ignored".to_string()),
            Vec::new(),
            None,
        );
        let job = state.jobs.get(&2).expect("job exists");
        assert_eq!(job.content_preview(), None);
    }

    #[test]
    fn selecting_job_with_preview_updates_view_model() {
        let mut state = AppState::new();
        state.jobs.insert(
            3,
            JobState {
                url: "https://example.com/path".to_string(),
                stage: Stage::Done,
                content_preview: Some("preview content".to_string()),
                ..Default::default()
            },
        );
        let (state, _) = update(state, Msg::JobSelected { job_id: 3 });
        let view = state.view();
        // No briefing session, so shows fallback message
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("No Analysis Available Yet"));
        assert_eq!(view.preview_header.as_ref().unwrap().domain, "example.com");
    }

    #[test]
    fn selecting_job_without_preview_only_sets_header() {
        let mut state = AppState::new();
        state.jobs.insert(
            4,
            JobState {
                url: "http://sub.example.net/a".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );
        let (state, _) = update(state, Msg::JobSelected { job_id: 4 });
        let view = state.view();
        // No briefing, so shows fallback message text (not None)
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("No Analysis Available Yet"));
        let header = view.preview_header.expect("header should exist");
        assert_eq!(header.domain, "sub.example.net");
        assert_eq!(header.stage, Stage::Downloading);
    }

    #[test]
    fn selecting_same_job_twice_only_sets_dirty_once() {
        let mut state = AppState::new();
        state.jobs.insert(
            5,
            JobState {
                url: "https://repeat.example".to_string(),
                stage: Stage::Done,
                content_preview: Some("d".to_string()),
                ..Default::default()
            },
        );
        let (state, _) = update(state, Msg::JobSelected { job_id: 5 });
        let mut state = state;
        assert!(state.consume_dirty());
        let (state, _) = update(state, Msg::JobSelected { job_id: 5 });
        let mut state = state;
        assert!(!state.consume_dirty());
    }

    #[test]
    fn domain_from_url_handles_various_inputs() {
        assert_eq!(domain_from_url("https://example.com/"), "example.com");
        assert_eq!(domain_from_url("http://foo.bar/baz?qux"), "foo.bar");
        assert_eq!(domain_from_url("example.org/path"), "example.org");
        assert_eq!(domain_from_url(""), "");
    }

    #[test]
    fn job_progress_with_preview_updates_selected_preview() {
        let mut state = AppState::new();
        state.jobs.insert(
            6,
            JobState {
                url: "https://partial.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let (state, _) = update(state, Msg::JobSelected { job_id: 6 });
        let (state, _) = update(
            state,
            Msg::JobProgress {
                job_id: 6,
                stage: Stage::Converting,
                tokens: None,
                bytes: None,
                content_preview: Some("live content".to_string()),
            },
        );

        let view = state.view();
        assert_eq!(view.preview_text, Some("live content".to_string()));
        let job = state.jobs.get(&6).expect("job exists");
        assert_eq!(job.content_preview(), Some("live content"));
    }

    #[test]
    fn job_progress_with_preview_stores_content_when_not_selected() {
        let mut state = AppState::new();
        state.jobs.insert(
            7,
            JobState {
                url: "https://unselected.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let (state, _) = update(
            state,
            Msg::JobProgress {
                job_id: 7,
                stage: Stage::Converting,
                tokens: None,
                bytes: None,
                content_preview: Some("background content".to_string()),
            },
        );

        let view = state.view();
        assert_eq!(view.preview_text, None);
        let job = state.jobs.get(&7).expect("job exists");
        assert_eq!(job.content_preview(), Some("background content"));
    }

    #[test]
    fn job_done_after_inprogress_promotes_preview_to_available() {
        let mut state = AppState::new();
        state.jobs.insert(
            8,
            JobState {
                url: "https://final.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let (state, _) = update(state, Msg::JobSelected { job_id: 8 });
        let (state, _) = update(
            state,
            Msg::JobProgress {
                job_id: 8,
                stage: Stage::Converting,
                tokens: None,
                bytes: None,
                content_preview: Some("partial".to_string()),
            },
        );
        let (state, _) = update(
            state,
            Msg::JobDone {
                job_id: 8,
                result: JobResultKind::Success,
                content_preview: Some("final".to_string()),
                extracted_links: Vec::new(),
                fetched_utc: None,
            },
        );

        let view = state.view();
        // With no summary or triage, shows fallback message
        assert!(view
            .preview_text
            .unwrap()
            .contains("No Analysis Available Yet"));
        let header = view.preview_header.expect("header present");
        assert_eq!(header.stage, Stage::Done);
    }

    #[test]
    fn preview_quality_counts_headings_and_skips_nav_indicator_when_low_density() {
        let content =
            "# Title\n## Section\nBody text with a [link](http://example.com).\nMore words here.";
        let quality = PreviewQuality::from_markdown(content);
        assert_eq!(quality.heading_count, 2);
        assert!(!quality.nav_heavy());
    }

    #[test]
    fn preview_quality_marks_nav_heavy_when_link_density_high() {
        let content = "[a](x) [b](x) [c](x) [d](x) [e](x)";
        let quality = PreviewQuality::from_markdown(content);
        assert!(quality.nav_heavy());
    }

    #[test]
    fn job_done_success_stores_normalized_links() {
        let mut state = AppState::new();
        state.jobs.insert(
            9,
            JobState {
                url: "https://link.example".to_string(),
                stage: Stage::Downloading,
                ..Default::default()
            },
        );

        let links = vec![
            ExtractedLink {
                url: "HTTP://EXAMPLE.com".to_string(),
                text: None,
                kind: LinkKind::Hyperlink,
            },
            ExtractedLink {
                url: "http://example.com/".to_string(),
                text: None,
                kind: LinkKind::Hyperlink,
            },
            ExtractedLink {
                url: "https://other.example:443/path".to_string(),
                text: None,
                kind: LinkKind::Hyperlink,
            },
        ];
        let (state, _) = update(
            state,
            Msg::JobDone {
                job_id: 9,
                result: JobResultKind::Success,
                content_preview: None,
                extracted_links: links,
                fetched_utc: None,
            },
        );

        let job = state.jobs.get(&9).expect("job exists");
        let stored_links = job.links();
        assert_eq!(stored_links.len(), 2);
        assert_eq!(stored_links[0].url, "http://example.com/".to_string());
        assert_eq!(stored_links[0].index, 0);
        assert_eq!(
            stored_links[1].url,
            "https://other.example/path".to_string()
        );
        assert_eq!(stored_links[1].index, 2);
    }

    #[test]
    fn cache_starts_empty() {
        let state = AppState::new();
        assert_eq!(state.summary_cache().len(), 0);
    }

    #[test]
    fn store_and_retrieve_summary_result() {
        use crate::briefing::ArticleSummaryResult;
        use crate::summary_cache::SummaryCacheKey;
        use harvester_engine::llm::prompt::PromptId;

        let mut state = AppState::new();
        let key = SummaryCacheKey {
            content_hash: "hash1".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model1".to_string(),
            context_hash: "ctx1".to_string(),
        };
        let result = ArticleSummaryResult {
            title: "Test Title".to_string(),
            summary: "Test Summary".to_string(),
            key_points: vec!["Point 1".to_string()],
            input_tokens: 100,
            output_tokens: 50,
            entities: Default::default(),
        };

        state.store_summary_result(
            key.clone(),
            result.clone(),
            "2026-01-01T00:00:00Z".to_string(),
        );

        let retrieved = state.try_reuse_summary(&key);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().title, "Test Title");
        assert_eq!(retrieved.unwrap().summary, "Test Summary");
        assert_eq!(state.summary_cache().len(), 1);
    }

    #[test]
    fn briefing_complete_then_job_selected_shows_summary_not_briefing() {
        use crate::briefing::{
            ArticleSummaryResult, BriefingResult, BriefingStoryResult, LoadedArticle,
        };

        let mut state = AppState::new();
        state.jobs.insert(
            1,
            JobState {
                url: "https://example.com/article".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );

        // Simulate a completed briefing session
        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            vec![LoadedArticle {
                url: "https://example.com/article".to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
                fetched_utc: None,
            }],
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        briefing.start_article(0, 1);
        briefing.complete_article(
            0,
            ArticleSummaryResult {
                title: "Article Title".to_string(),
                summary: "Article summary text".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
        );
        briefing.set_briefing_request_id(2);
        briefing.complete_briefing(BriefingResult {
            executive_summary: "Executive summary".to_string(),
            top_stories: vec![BriefingStoryResult {
                headline: "Story 1".to_string(),
                body: "desc".to_string(),
            }],
            article_count: 1,
            input_tokens: 20,
            output_tokens: 10,
        });
        state.set_briefing(briefing);

        // After briefing completes, view should show briefing text
        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Executive Briefing"));

        // After job selected, view should show summary not briefing
        state.select_job(1);
        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Article Title"));
        assert!(!view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Executive Briefing"));
    }

    #[test]
    fn job_selected_then_briefing_completes_shows_briefing() {
        use crate::briefing::{
            ArticleSummaryResult, BriefingResult, BriefingStoryResult, LoadedArticle,
        };

        let mut state = AppState::new();
        state.jobs.insert(
            1,
            JobState {
                url: "https://example.com/article".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );

        // Select job first (summary exists via briefing session we'll add)
        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            vec![LoadedArticle {
                url: "https://example.com/article".to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
                fetched_utc: None,
            }],
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        briefing.start_article(0, 1);
        briefing.complete_article(
            0,
            ArticleSummaryResult {
                title: "Article Title".to_string(),
                summary: "Article summary text".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
        );
        briefing.set_briefing_request_id(2);
        briefing.complete_briefing(BriefingResult {
            executive_summary: "Executive summary".to_string(),
            top_stories: vec![BriefingStoryResult {
                headline: "Story 1".to_string(),
                body: "desc".to_string(),
            }],
            article_count: 1,
            input_tokens: 20,
            output_tokens: 10,
        });
        state.set_briefing(briefing);

        state.select_job(1);
        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Article Title"));

        // Now revert to briefing mode (simulating briefing completing again)
        state.revert_preview_to_briefing();
        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Executive Briefing"));
    }

    #[test]
    fn no_selection_shows_briefing_when_complete() {
        use crate::briefing::{BriefingResult, BriefingStoryResult};

        let mut state = AppState::new();
        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(vec![], "collection".to_string());
        // Force complete state directly
        briefing.set_briefing_request_id(1);
        briefing.complete_briefing(BriefingResult {
            executive_summary: "Executive summary text".to_string(),
            top_stories: vec![BriefingStoryResult {
                headline: "Story".to_string(),
                body: "desc".to_string(),
            }],
            article_count: 0,
            input_tokens: 10,
            output_tokens: 5,
        });
        state.set_briefing(briefing);

        let view = state.view();
        assert!(view
            .preview_text
            .as_deref()
            .unwrap_or("")
            .contains("Executive Briefing"));
    }

    #[test]
    fn cache_miss_returns_none() {
        use crate::summary_cache::SummaryCacheKey;
        use harvester_engine::llm::prompt::PromptId;

        let state = AppState::new();
        let key = SummaryCacheKey {
            content_hash: "nonexistent".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        };

        assert!(state.try_reuse_summary(&key).is_none());
    }

    #[test]
    fn set_summary_cache_replaces_entire_cache() {
        use crate::briefing::ArticleSummaryResult;
        use crate::summary_cache::{SummaryCache, SummaryCacheEntry, SummaryCacheKey};
        use harvester_engine::llm::prompt::PromptId;

        let mut state = AppState::new();

        // Store initial entry
        let key1 = SummaryCacheKey {
            content_hash: "hash1".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model1".to_string(),
            context_hash: "ctx1".to_string(),
        };
        let result1 = ArticleSummaryResult {
            title: "Title1".to_string(),
            summary: "Summary1".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        };
        state.store_summary_result(key1.clone(), result1, "2026-01-01T00:00:00Z".to_string());
        assert_eq!(state.summary_cache().len(), 1);

        // Replace with new cache containing different entry
        let mut new_cache = SummaryCache::new();
        let key2 = SummaryCacheKey {
            content_hash: "hash2".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model2".to_string(),
            context_hash: "ctx2".to_string(),
        };
        let entry2 = SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "Title2".to_string(),
                summary: "Summary2".to_string(),
                key_points: vec![],
                input_tokens: 20,
                output_tokens: 10,
                entities: Default::default(),
            },
            created_at_utc: "2026-01-02T00:00:00Z".to_string(),
        };
        new_cache.insert(key2.clone(), entry2);

        state.set_summary_cache(new_cache);

        // Old entry should be gone, new entry should be present
        assert_eq!(state.summary_cache().len(), 1);
        assert!(state.try_reuse_summary(&key1).is_none());
        assert_eq!(state.try_reuse_summary(&key2).unwrap().title, "Title2");
    }

    fn make_state_with_summarized_job() -> AppState {
        use crate::briefing::{ArticleSummaryResult, LoadedArticle};
        let mut state = AppState::new();
        state.jobs.insert(
            10,
            JobState {
                url: "https://summarized.example/article".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );
        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            vec![LoadedArticle {
                url: "https://summarized.example/article".to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
                fetched_utc: None,
            }],
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        briefing.start_article(0, 1);
        briefing.complete_article(
            0,
            ArticleSummaryResult {
                title: "My Title".to_string(),
                summary: "My summary".to_string(),
                key_points: vec!["Point A".to_string()],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
        );
        state.set_briefing(briefing);
        state
    }

    #[test]
    fn selecting_job_with_summary_shows_formatted_summary() {
        let mut state = make_state_with_summarized_job();
        state.select_job(10);
        let view = state.view();
        let text = view.preview_text.unwrap_or_default();
        assert!(text.contains("My Title"));
        assert!(text.contains("My summary"));
        assert!(text.contains("Point A"));
    }

    #[test]
    fn selecting_job_without_summary_shows_placeholder() {
        let mut state = AppState::new();
        state.jobs.insert(
            11,
            JobState {
                url: "https://no-summary.example".to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                ..Default::default()
            },
        );
        state.select_job(11);
        let view = state.view();
        let text = view.preview_text.unwrap_or_default();
        assert!(text.contains("No Analysis Available Yet"));
    }

    #[test]
    fn selecting_job_sets_preview_mode_to_selected_job_summary() {
        let mut state = make_state_with_summarized_job();
        state.select_job(10);
        // Confirm mode is SelectedJobSummary by verifying briefing text is NOT shown
        // even though we manually put a complete briefing in state
        use crate::briefing::{BriefingResult, BriefingStoryResult};
        let mut s2 = make_state_with_summarized_job();
        s2.briefing_mut().set_briefing_request_id(99);
        s2.briefing_mut().complete_briefing(BriefingResult {
            executive_summary: "Exec summary".to_string(),
            top_stories: vec![BriefingStoryResult {
                headline: "T".to_string(),
                body: "d".to_string(),
            }],
            article_count: 1,
            input_tokens: 10,
            output_tokens: 5,
        });
        s2.select_job(10);
        let view = s2.view();
        let text = view.preview_text.unwrap_or_default();
        assert!(
            !text.contains("Exec summary"),
            "should not show briefing text when job selected"
        );
        assert!(text.contains("My Title"), "should show summary");
    }

    #[test]
    fn format_summary_includes_title_summary_and_key_points() {
        use crate::briefing::ArticleSummaryResult;
        let result = ArticleSummaryResult {
            title: "Test Title".to_string(),
            summary: "Test summary body".to_string(),
            key_points: vec!["KP1".to_string(), "KP2".to_string()],
            input_tokens: 0,
            output_tokens: 0,
            entities: Default::default(),
        };
        let formatted = preview::format_summary_for_preview(&result);
        assert!(formatted.contains("Test Title"));
        assert!(formatted.contains("Test summary body"));
        assert!(formatted.contains("KP1"));
        assert!(formatted.contains("KP2"));
        assert!(formatted.contains("Key Points"));
    }

    #[test]
    fn format_summary_omits_key_points_section_when_empty() {
        use crate::briefing::ArticleSummaryResult;
        let result = ArticleSummaryResult {
            title: "Title Only".to_string(),
            summary: "Summary only".to_string(),
            key_points: vec![],
            input_tokens: 0,
            output_tokens: 0,
            entities: Default::default(),
        };
        let formatted = preview::format_summary_for_preview(&result);
        assert!(formatted.contains("Title Only"));
        assert!(formatted.contains("Summary only"));
        assert!(!formatted.contains("Key Points"));
    }

    #[test]
    fn selected_article_url_returns_url_when_summarized_job_selected() {
        let mut state = make_state_with_summarized_job();
        state.select_job(10);
        let url = state.selected_article_url();
        assert_eq!(url, Some("https://summarized.example/article".to_string()));
    }

    #[test]
    fn selected_article_url_returns_none_when_no_summary() {
        let mut state = AppState::new();
        state.jobs.insert(
            12,
            JobState {
                url: "https://no-summary.example".to_string(),
                stage: Stage::Done,
                ..Default::default()
            },
        );
        state.select_job(12);
        assert!(state.selected_article_url().is_none());
    }

    #[test]
    fn selected_article_url_returns_none_when_no_selection() {
        let state = AppState::new();
        assert!(state.selected_article_url().is_none());
    }

    #[test]
    fn view_has_summary_true_for_completed_articles() {
        let state = make_state_with_summarized_job();
        let view = state.view();
        let job = view
            .jobs
            .iter()
            .find(|j| j.job_id == 10)
            .expect("job 10 exists");
        assert!(job.has_summary);
        assert_eq!(job.summary_title.as_deref(), Some("My Title"));
    }

    #[test]
    fn view_has_summary_false_before_briefing() {
        let mut state = AppState::new();
        state.jobs.insert(
            13,
            JobState {
                url: "https://no-briefing.example".to_string(),
                stage: Stage::Done,
                ..Default::default()
            },
        );
        let view = state.view();
        let job = view
            .jobs
            .iter()
            .find(|j| j.job_id == 13)
            .expect("job 13 exists");
        assert!(!job.has_summary);
        assert!(job.summary_title.is_none());
    }

    #[test]
    fn view_selected_url_populated_when_summarized_job_selected() {
        let mut state = make_state_with_summarized_job();
        state.select_job(10);
        let view = state.view();
        assert_eq!(
            view.selected_url,
            Some("https://summarized.example/article".to_string())
        );
    }

    #[test]
    fn view_selected_url_none_when_unsummarized_job_selected() {
        let mut state = AppState::new();
        state.jobs.insert(
            14,
            JobState {
                url: "https://unsummarized.example".to_string(),
                stage: Stage::Done,
                ..Default::default()
            },
        );
        state.select_job(14);
        let view = state.view();
        // Phase 3: selected_url is now available for any selected job
        assert_eq!(
            view.selected_url,
            Some("https://unsummarized.example".to_string())
        );
    }

    #[test]
    fn view_selected_url_none_when_no_selection() {
        let state = make_state_with_summarized_job();
        let view = state.view();
        assert!(view.selected_url.is_none());
    }

    // ------------------------------------------------------------------
    // Substep B: Prompt Lab AppState integration tests
    // ------------------------------------------------------------------

    #[test]
    fn default_app_state_has_closed_empty_prompt_lab() {
        let state = AppState::new();
        let lab = state.prompt_lab();
        assert!(!lab.is_visible());
        assert_eq!(lab.run_count(), 0);
        assert!(!lab.has_in_flight_run());
    }

    #[test]
    fn allocate_prompt_lab_run_id_is_monotonic_starting_at_one() {
        let mut state = AppState::new();
        let id1 = state.allocate_next_prompt_lab_run_id();
        let id2 = state.allocate_next_prompt_lab_run_id();
        let id3 = state.allocate_next_prompt_lab_run_id();
        use crate::prompt_lab::PromptLabRunId;
        assert_eq!(id1, PromptLabRunId(1));
        assert_eq!(id2, PromptLabRunId(2));
        assert_eq!(id3, PromptLabRunId(3));
    }

    #[test]
    fn prompt_lab_and_llm_request_id_counters_are_independent() {
        let mut state = AppState::new();
        // Allocate some LLM request IDs
        let llm1 = state.allocate_next_llm_request_id();
        let llm2 = state.allocate_next_llm_request_id();
        // Allocate some Prompt Lab run IDs
        let lab1 = state.allocate_next_prompt_lab_run_id();
        let lab2 = state.allocate_next_prompt_lab_run_id();
        // Both start at 1, but they are independent counters on distinct types
        assert_eq!(llm1, 1u64);
        assert_eq!(llm2, 2u64);
        use crate::prompt_lab::PromptLabRunId;
        assert_eq!(lab1, PromptLabRunId(1));
        assert_eq!(lab2, PromptLabRunId(2));
    }

    #[test]
    fn clear_prompt_lab_history_preserves_pending_entries() {
        use harvester_engine::llm::prompt::PromptId;
        let mut state = AppState::new();
        state.open_prompt_lab();

        // Add a pending run
        let req_id = state.allocate_next_llm_request_id();
        let run_id = state.allocate_next_prompt_lab_run_id();
        state.add_prompt_lab_pending_run(PromptLabPendingRunRegistration {
            run_id,
            stage: crate::prompt_lab::PromptLabStage::Triage,
            prompt_id: PromptId::ArticleTriage,
            input_snapshot: "input".to_string(),
            request_id: req_id,
            overrides: PromptLabRunOverrides::default(),
            compare_batch_id: None,
            compare_candidate_id: None,
        });

        // Add a completed run
        let req_id2 = state.allocate_next_llm_request_id();
        let run_id2 = state.allocate_next_prompt_lab_run_id();
        state.add_prompt_lab_pending_run(PromptLabPendingRunRegistration {
            run_id: run_id2,
            stage: crate::prompt_lab::PromptLabStage::Triage,
            prompt_id: PromptId::ArticleTriage,
            input_snapshot: "input2".to_string(),
            request_id: req_id2,
            overrides: PromptLabRunOverrides::default(),
            compare_batch_id: None,
            compare_candidate_id: None,
        });
        state.complete_prompt_lab_run(
            run_id2,
            "{}".to_string(),
            harvester_engine::llm::run_metadata::LlmRunMetadata::stub(),
        );
        state.consume_prompt_lab_ownership(req_id2);

        state.clear_prompt_lab_history();

        // Pending run survives
        assert_eq!(state.prompt_lab().run_count(), 1);
        assert!(state.prompt_lab().ownership_for(req_id).is_some());
    }

    #[test]
    fn resolve_preview_prefers_summary_over_triage() {
        use crate::briefing::{ArticleSummaryResult, LoadedArticle};
        use crate::triage::ArticleTriageResult;

        let mut state = AppState::new();
        let url = "https://test.example/article";

        // Add both summary and triage result
        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            vec![LoadedArticle {
                url: url.to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
                fetched_utc: None,
            }],
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        briefing.start_article(0, 1);
        briefing.complete_article(
            0,
            ArticleSummaryResult {
                title: "Test Summary".to_string(),
                summary: "Summary text".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
        );
        state.set_briefing(briefing);

        let mut triage = crate::triage::TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash".to_string(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        triage.start_article(0, 1);
        triage.complete_article(
            0,
            ArticleTriageResult {
                category: "Security".to_string(),
                priority: 7,
                tags: vec![],
                rationale: "Test rationale".to_string(),
                input_tokens: 10,
                output_tokens: 5,
            },
        );
        state.set_triage(triage);

        let (kind, content) = state.resolve_best_preview(url);
        assert_eq!(kind, PreviewContentKind::Summary);
        assert!(content.contains("Test Summary"));
    }

    #[test]
    fn resolve_preview_uses_triage_when_summary_missing() {
        use crate::briefing::LoadedArticle;
        use crate::triage::ArticleTriageResult;

        let mut state = AppState::new();
        let url = "https://test.example/article";

        // Add only triage result, no summary
        let mut triage = crate::triage::TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash".to_string(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        triage.start_article(0, 1);
        triage.complete_article(
            0,
            ArticleTriageResult {
                category: "Security".to_string(),
                priority: 7,
                tags: vec!["test-tag".to_string()],
                rationale: "Test rationale".to_string(),
                input_tokens: 10,
                output_tokens: 5,
            },
        );
        state.set_triage(triage);

        let (kind, content) = state.resolve_best_preview(url);
        assert_eq!(kind, PreviewContentKind::Triage);
        assert!(content.contains("Triage Assessment"));
        assert!(content.contains("7/10"));
        assert!(content.contains("Test rationale"));
    }

    #[test]
    fn resolve_preview_uses_fallback_when_nothing_available() {
        let state = AppState::new();
        let url = "https://test.example/article";

        let (kind, content) = state.resolve_best_preview(url);
        assert_eq!(kind, PreviewContentKind::Fallback);
        assert!(content.contains("No Analysis Available Yet"));
    }

    #[test]
    fn resolve_preview_returns_correct_kind() {
        use crate::briefing::{ArticleSummaryResult, LoadedArticle};

        let mut state = AppState::new();
        let url = "https://test.example/article";

        // Test with summary
        let mut briefing = crate::briefing::BriefingSession::new_loading(None);
        briefing.set_articles(
            vec![LoadedArticle {
                url: url.to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
                fetched_utc: None,
            }],
            "collection".to_string(),
        );
        briefing.transition_to_summarizing();
        briefing.start_article(0, 1);
        briefing.complete_article(
            0,
            ArticleSummaryResult {
                title: "Test".to_string(),
                summary: "Summary".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
        );
        state.set_briefing(briefing);

        let (kind, _) = state.resolve_best_preview(url);
        assert_eq!(kind, PreviewContentKind::Summary);
    }
}

#[cfg(test)]
mod briefing_history_state_tests {
    use super::*;
    use crate::briefing::BriefingHistoryEntry;

    fn entry(ts: &str) -> BriefingHistoryEntry {
        BriefingHistoryEntry {
            generated_at_utc: ts.to_string(),
            executive_summary: format!("Summary {ts}"),
            top_stories: vec![],
            article_count: 1,
        }
    }

    #[test]
    fn starts_empty() {
        let state = AppState::new();
        assert!(state.briefing_history().is_empty());
    }

    #[test]
    fn push_adds_newest_first() {
        let mut state = AppState::new();
        state.push_briefing_history(entry("2026-02-20T00:00:00Z"));
        state.push_briefing_history(entry("2026-02-21T00:00:00Z"));
        assert_eq!(
            state.briefing_history()[0].generated_at_utc,
            "2026-02-21T00:00:00Z"
        );
        assert_eq!(
            state.briefing_history()[1].generated_at_utc,
            "2026-02-20T00:00:00Z"
        );
    }

    #[test]
    fn push_caps_at_three() {
        let mut state = AppState::new();
        for i in 1..=4 {
            state.push_briefing_history(entry(&format!("2026-02-2{}T00:00:00Z", i)));
        }
        assert_eq!(state.briefing_history().len(), 3);
        // Oldest (day 1) was dropped; the 4th push (day 4) is now at index 0
        assert_eq!(
            state.briefing_history()[0].generated_at_utc,
            "2026-02-24T00:00:00Z"
        );
    }
}
