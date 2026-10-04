use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::llm::run_metadata::LlmRunMetadata;
use harvester_engine::llm::QuotaOrigin;
use harvester_engine::ExtractedLink;
use serde::{Deserialize, Serialize};

use crate::state::{AiAvailability, ArchiveTokenEstimates};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[rustfmt::skip]
pub enum Msg {
    ValidatedResultReceived { record: Box<crate::SavedResult> },
    ResultStoreUnavailable { reason: String },
    /// User edited the URL input box (debounced text).
    InputChanged(String),
    /// User typed in the Jobs search box.
    JobsSearchQueryChanged(String),
    /// User pressed Esc inside the Jobs search box.
    JobsSearchCleared,
    /// App startup hook for reducer-owned metadata hydration.
    RestoreDesktopView { mode: Option<crate::JobListMode>, selected_article_url: Option<String>, now: chrono::DateTime<chrono::Utc> },
    StartupHydrationRequested,
    SavedArticlesLoaded { request_id: u64, articles: Vec<harvester_engine::WindowArticle> },
    /// User submitted the current URL input for ingestion.
    UrlsSubmitted,
    /// Restore previously completed jobs from persisted state.
    RestoreCompletedJobs(Vec<crate::CompletedJobSnapshot>),
    /// Restore URLs whose intake must be retried by the next Full run.
    RestorePendingIntake(Vec<String>),
    /// Startup recovery has completed (including jobs with no recoverable date).
    FetchTimeRecoveryCompleted,
    /// I/O preserved an additional owner backup; render through the status notice.
    RuntimeStateNotice { message: String },
    /// App-loop boundary action: evaluate pre-triage refresh demand with a
    /// single snapshot of currently completed URLs.
    EvaluatePreTriageRefresh {
        ordered_urls: Vec<String>,
        triggered_by_job_done: bool,
    },
    /// User clicked Stop/Finish.
    StopFinishClicked,
    /// User clicked Archive.
    ArchiveClicked,
    /// Archive dialog data is ready for the UI to render.
    ArchiveDialogReady {
        request_id: u64,
        article_count: usize,
        since_utc: Option<DateTime<Utc>>,
        default_basename: String,
        default_file_exists: bool,
        export_dir: PathBuf,
        pending_pre_triage_count: usize,
        token_estimates: ArchiveTokenEstimates,
        signal_candidate_default: crate::signal_candidate::SignalCandidateDialogDefault,
        signal_candidate_count: usize,
        signal_candidate_scoring_done: u32,
        signal_candidate_scoring_total: u32,
        signal_candidate_token_estimates: ArchiveTokenEstimates,
    },
    /// Archive dialog was confirmed by the user.
    ArchiveDialogSubmitted {
        request_id: u64,
        basename: String,
        set_checkpoint: bool,
        submitted_at: DateTime<Utc>,
        use_summaries: bool,
        use_signal_candidates: bool,
    },
    /// Archive export completed successfully.
    ArchiveExportCompleted {
        request_id: u64,
        path: PathBuf,
        doc_count: usize,
        requested_checkpoint: Option<DateTime<Utc>>,
    },
    /// Archive export failed.
    ArchiveExportFailed {
        request_id: u64,
        basename: String,
        reason: String,
    },
    /// Toggle manual exclusion for a signal-candidate cluster.
    ToggleSignalCandidateExclusion { signal_key: String },
    /// UI/render tick to coalesce rendering and observe host time.
    Tick { now: DateTime<Utc> },
    /// Desktop job-list mode.
    JobListModeSet { mode: crate::JobListMode },
    /// Host request to run the merged triage + summary pipeline.
    PipelineRunRequested { scope: crate::PipelineRunScope },
    /// Reducer-owned orchestration pulse, sent by either host while a run is active.
    PipelineRunAdvance,
    /// Dismiss the desktop completion notice.
    RunFinishedNoticeDismissed,
    /// Resolve an extracted link by its core-owned job/index pair.
    ExtractedLinkOpenRequested {
        job_id: crate::JobId,
        link_index: u32,
    },
    /// Engine progress for a job.
    JobProgress {
        job_id: crate::JobId,
        stage: crate::Stage,
        tokens: Option<u32>,
        bytes: Option<u64>,
    },
    /// Engine completion for a job.
    JobDone {
        job_id: crate::JobId,
        result: crate::JobResultKind,
        extracted_links: Vec<ExtractedLink>,
        fetched_utc: Option<String>,
    },
    /// Classification of a completed fetch for the domain blacklist. Emitted
    /// alongside `JobDone` from the engine→message translation, before `JobDone`
    /// so the job's URL is still resolvable when reduced. `recorded_at` is the
    /// time captured at that translation site, so the reducer stays pure.
    FetchOutcomeClassified {
        job_id: crate::JobId,
        class: harvester_engine::FetchOutcomeClass,
        failure_label: Option<String>,
        recorded_at: chrono::DateTime<chrono::Utc>,
    },
    /// Replaces the in-memory blacklist with persisted state at startup.
    BlacklistHydrated {
        state: crate::blacklist::BlacklistState,
    },
    /// User selected a job from the tree view.
    JobSelected { job_id: crate::JobId },
    ArticleLinksLoaded { job_id: crate::JobId, url: String, links: Result<Vec<ExtractedLink>, String> },
    /// Tauri desktop window resize debounce completed. Carries logical inner dimensions.
    DesktopWindowResizeCompleted { inner_width: i32, inner_height: i32 },
    /// A completion result came back from the worker.
    LlmCompleted {
        request_id: u64,
        result: LlmResultKind,
        /// Full run metadata. `None` only for pre-flight errors that fire
        /// before timing/model info is available (e.g. `PromptNotFound`).
        metadata: Option<LlmRunMetadata>,
    },
    /// Startup/effect boundary configured session LLM quota limits.
    LlmQuotaConfigured { limits: crate::LlmQuotaLimits },
    /// Authoritative session LLM quota usage snapshot from the worker.
    LlmQuotaUsageUpdated { usage: crate::LlmQuotaUsage },
    /// Effect runner reports the total number of enabled sources to poll.
    PollStarted { total: usize },
    /// Polling completed for a source.
    SourcePollCompleted {
        source_id: harvester_engine::SourceId,
        urls: Vec<String>,
        kind: harvester_engine::SourceKind,
        /// Raw count from the API or feed before any filtering.
        parsed: usize,
        /// Count filtered by the seen-set (cross-cycle dedup).
        dedup_filtered: usize,
    },
    /// Polling failed for a source.
    SourcePollFailed {
        source_id: harvester_engine::SourceId,
        error: String,
    },
    /// All configured sources finished polling.
    AllSourcesPollEnded,
    /// Triage-specific articles prepared by the loader.
    TriageArticlesLoaded {
        request_id: u64,
        delta: harvester_engine::TriageArticleDelta,
    },
    /// Incremental loader progress for a triage-specific article load.
    TriageArticlesLoadProgress {
        request_id: u64,
        files_scanned: usize,
        files_total: usize,
    },
    /// Loader failed for triage.
    TriageArticlesLoadFailed { request_id: u64, reason: String },
    /// Briefing time checkpoint loaded from disk at startup.
    /// Raw wire type; the reducer parses the string into `DateTime<Utc>`.
    BriefingCheckpointLoaded { since_utc: Option<String> },
    /// Briefing time checkpoint persisted successfully.
    BriefingCheckpointSaveSucceeded { save_id: u64 },
    /// Briefing time checkpoint persistence failed.
    BriefingCheckpointSaveFailed { save_id: u64, reason: String },
    /// Request to update the in-memory briefing checkpoint (and persist it).
    /// Raw wire type; the reducer validates the string before storing.
    BriefingCheckpointSet(Option<String>),
    ProcessingConfigurationLoaded {
        request_id: u64,
        contexts: HashMap<PromptId, Vec<(String, String)>>,
        active_versions: HashMap<PromptId, PromptVersion>,
        effective_models: HashMap<PromptId, String>,
        preparation_budget: usize,
    },
    ProcessingConfigurationFailed {
        request_id: u64,
        reason: String,
    },
    /// Prompt contexts loaded from disk.
    PromptContextsLoaded {
        contexts: HashMap<PromptId, Vec<(String, String)>>,
    },
    /// Prompt contexts failed to load.
    PromptContextsLoadFailed { reason: String },
    /// Saved prompt template overlays have been loaded into the prompt registry.
    PromptTemplateFilesLoaded,
    /// LLM metadata (active prompt versions and effective models) loaded.
    LlmMetadataLoaded {
        active_versions: std::collections::HashMap<PromptId, PromptVersion>,
        effective_models: std::collections::HashMap<PromptId, String>,
    },
    /// Startup/effect boundary detected whether AI-backed workflows are available.
    AiAvailabilityDetected { availability: AiAvailability },
    /// Summary cache hydrated from persisted store at startup.
    SummaryCacheHydrated { cache: crate::SummaryCache },
    /// Signal-candidate cache hydrated from persisted store at startup.
    SignalCandidateCacheLoaded {
        cache: crate::signal_candidate_cache::SignalCandidateCache,
    },
    /// Signal-candidate manual overrides hydrated from persisted store at startup.
    SignalCandidateOverridesLoaded {
        overrides: std::collections::HashSet<crate::signal_candidate::OverrideKey>,
    },
    /// Triage cache hydrated from persisted store at startup.
    TriageCacheHydrated { cache: crate::TriageCache },
    /// User requested to open the currently selected article URL in the default browser.
    OpenInBrowserClicked,

    // --- Import saved webpages ---
    /// Request to import browser-saved .htm/.html files from `dir`.
    ImportSavedWebpagesRequested { dir: PathBuf },
    /// Import batch completed (may include per-file failures).
    ImportSavedWebpagesCompleted {
        request_id: u64,
        report: harvester_engine::ImportReport,
    },
    /// Import batch failed at the directory level (scan or setup failure).
    ImportSavedWebpagesFailed { request_id: u64, reason: String },
}

impl Msg {
    /// Cheap message name for host measurements; never formats payloads.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ValidatedResultReceived { .. } => "ValidatedResultReceived",
            Self::ResultStoreUnavailable { .. } => "ResultStoreUnavailable",
            Self::InputChanged(..) => "InputChanged",
            Self::JobsSearchQueryChanged(..) => "JobsSearchQueryChanged",
            Self::JobsSearchCleared => "JobsSearchCleared",
            Self::RestoreDesktopView { .. } => "RestoreDesktopView",
            Self::SavedArticlesLoaded { .. } => "SavedArticlesLoaded",
            Self::StartupHydrationRequested => "StartupHydrationRequested",
            Self::UrlsSubmitted => "UrlsSubmitted",
            Self::RestoreCompletedJobs(..) => "RestoreCompletedJobs",
            Self::RestorePendingIntake(..) => "RestorePendingIntake",
            Self::EvaluatePreTriageRefresh { .. } => "EvaluatePreTriageRefresh",
            Self::StopFinishClicked => "StopFinishClicked",
            Self::ArchiveClicked => "ArchiveClicked",
            Self::ArchiveDialogReady { .. } => "ArchiveDialogReady",
            Self::ArchiveDialogSubmitted { .. } => "ArchiveDialogSubmitted",
            Self::ArchiveExportCompleted { .. } => "ArchiveExportCompleted",
            Self::ArchiveExportFailed { .. } => "ArchiveExportFailed",
            Self::ToggleSignalCandidateExclusion { .. } => "ToggleSignalCandidateExclusion",
            Self::Tick { .. } => "Tick",
            Self::JobListModeSet { .. } => "JobListModeSet",
            Self::PipelineRunRequested { .. } => "PipelineRunRequested",
            Self::PipelineRunAdvance => "PipelineRunAdvance",
            Self::RunFinishedNoticeDismissed => "RunFinishedNoticeDismissed",
            Self::ExtractedLinkOpenRequested { .. } => "ExtractedLinkOpenRequested",
            Self::JobProgress { .. } => "JobProgress",
            Self::JobDone { .. } => "JobDone",
            Self::FetchOutcomeClassified { .. } => "FetchOutcomeClassified",
            Self::BlacklistHydrated { .. } => "BlacklistHydrated",
            Self::JobSelected { .. } => "JobSelected",
            Self::ArticleLinksLoaded { .. } => "ArticleLinksLoaded",
            Self::FetchTimeRecoveryCompleted => "FetchTimeRecoveryCompleted",
            Self::RuntimeStateNotice { .. } => "RuntimeStateNotice",
            Self::DesktopWindowResizeCompleted { .. } => "DesktopWindowResizeCompleted",
            Self::LlmCompleted { .. } => "LlmCompleted",
            Self::LlmQuotaConfigured { .. } => "LlmQuotaConfigured",
            Self::LlmQuotaUsageUpdated { .. } => "LlmQuotaUsageUpdated",
            Self::PollStarted { .. } => "PollStarted",
            Self::SourcePollCompleted { .. } => "SourcePollCompleted",
            Self::SourcePollFailed { .. } => "SourcePollFailed",
            Self::AllSourcesPollEnded => "AllSourcesPollEnded",
            Self::TriageArticlesLoaded { .. } => "TriageArticlesLoaded",
            Self::TriageArticlesLoadProgress { .. } => "TriageArticlesLoadProgress",
            Self::TriageArticlesLoadFailed { .. } => "TriageArticlesLoadFailed",
            Self::BriefingCheckpointLoaded { .. } => "BriefingCheckpointLoaded",
            Self::BriefingCheckpointSaveSucceeded { .. } => "BriefingCheckpointSaveSucceeded",
            Self::BriefingCheckpointSaveFailed { .. } => "BriefingCheckpointSaveFailed",
            Self::BriefingCheckpointSet(..) => "BriefingCheckpointSet",
            Self::ProcessingConfigurationLoaded { .. } => "ProcessingConfigurationLoaded",
            Self::ProcessingConfigurationFailed { .. } => "ProcessingConfigurationFailed",
            Self::PromptContextsLoaded { .. } => "PromptContextsLoaded",
            Self::PromptContextsLoadFailed { .. } => "PromptContextsLoadFailed",
            Self::PromptTemplateFilesLoaded => "PromptTemplateFilesLoaded",
            Self::LlmMetadataLoaded { .. } => "LlmMetadataLoaded",
            Self::AiAvailabilityDetected { .. } => "AiAvailabilityDetected",
            Self::SummaryCacheHydrated { .. } => "SummaryCacheHydrated",
            Self::SignalCandidateCacheLoaded { .. } => "SignalCandidateCacheLoaded",
            Self::SignalCandidateOverridesLoaded { .. } => "SignalCandidateOverridesLoaded",
            Self::TriageCacheHydrated { .. } => "TriageCacheHydrated",
            Self::OpenInBrowserClicked => "OpenInBrowserClicked",
            Self::ImportSavedWebpagesRequested { .. } => "ImportSavedWebpagesRequested",
            Self::ImportSavedWebpagesCompleted { .. } => "ImportSavedWebpagesCompleted",
            Self::ImportSavedWebpagesFailed { .. } => "ImportSavedWebpagesFailed",
        }
    }

    pub fn tick_at(now: DateTime<Utc>) -> Self {
        Self::Tick { now }
    }
}

/// Result payload returned by the LLM worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LlmResultKind {
    Success {
        output_json: String,
        input_tokens: u32,
        output_tokens: u32,
        prompt_version: PromptVersion,
        /// Provider-resolved model string (may be a dated variant like `gpt-5.4-mini-2026-03-17`).
        /// Do NOT use for cache key construction; use the canonical alias captured at enqueue time.
        resolved_model: String,
    },
    ValidationFailed {
        reason: String,
        raw_response: String,
    },
    QuotaExhausted {
        reason: String,
        origin: QuotaOrigin,
    },
    /// Provider refused the call with a rate-limit response (HTTP 429 that is
    /// not insufficient_quota). Kept distinct from Failed so the reducer can
    /// detect systemic provider refusal and stop the run.
    RateLimited {
        reason: String,
    },
    Failed {
        reason: String,
    },
}
