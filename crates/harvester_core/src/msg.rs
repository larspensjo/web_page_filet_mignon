use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use harvester_engine::llm::prompt::{PromptId, PromptTemplateOwned, PromptVersion};
use harvester_engine::llm::run_metadata::LlmRunMetadata;
use harvester_engine::llm::types::ModelId;
use harvester_engine::ExtractedLink;

use crate::prompt_lab::{
    PromptLabCompareBatchId, PromptLabInputSource, PromptLabRunId, PromptLabStage,
    PromptLabTemplateSnapshot,
};

use crate::briefing::LoadedArticle;
use crate::pre_triage_filter::{ArticleFilterKey, ManualDecision};
use crate::state::AiAvailability;
use crate::tabs::{AppTab, JobListScope, LeftTab, TrendCategory};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// User edited the URL input box (debounced text).
    InputChanged(String),
    /// App startup hook for reducer-owned metadata hydration.
    StartupHydrationRequested,
    /// User submitted the current URL input for ingestion.
    UrlsSubmitted,
    /// Restore previously completed jobs from persisted state.
    RestoreCompletedJobs(Vec<crate::CompletedJobSnapshot>),
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
    },
    /// Archive dialog was confirmed by the user.
    ArchiveDialogSubmitted {
        request_id: u64,
        basename: String,
        set_checkpoint: bool,
        submitted_at: DateTime<Utc>,
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
    /// User toggled visibility of the URL input/dropbox panel.
    ToggleInputPanel,
    /// UI/render tick to coalesce rendering.
    Tick,
    /// Engine progress for a job.
    JobProgress {
        job_id: crate::JobId,
        stage: crate::Stage,
        tokens: Option<u32>,
        bytes: Option<u64>,
        content_preview: Option<String>,
    },
    /// Engine completion for a job.
    JobDone {
        job_id: crate::JobId,
        result: crate::JobResultKind,
        content_preview: Option<String>,
        extracted_links: Vec<ExtractedLink>,
        fetched_utc: Option<String>,
    },
    LinkToggleRequested {
        job_id: crate::JobId,
        link_index: u32,
        checked: bool,
    },
    LinkDownloadStarted {
        job_id: crate::JobId,
        link_index: u32,
    },
    LinkDownloadCompleted {
        job_id: crate::JobId,
        link_index: u32,
        path: PathBuf,
    },
    LinkDownloadFailed {
        job_id: crate::JobId,
        link_index: u32,
        error: String,
    },
    LinkDeleted {
        job_id: crate::JobId,
        link_index: u32,
    },
    /// User selected a job from the tree view.
    JobSelected {
        job_id: crate::JobId,
    },
    /// User dragged the splitter to resize the left panels.
    SplitterMoved {
        desired_left_width_px: i32,
    },
    /// Window was resized.
    WindowResized {
        window_width: i32,
    },
    /// Window resize drag completed. Carries outer (frame) dimensions for persistence.
    WindowResizeCompleted {
        outer_width: i32,
        outer_height: i32,
    },
    /// Fallback for placeholder wiring.
    NoOp,
    /// User requested an LLM completion.
    RequestLlmCompletion {
        prompt_id: PromptId,
        prompt_version: Option<PromptVersion>,
        /// Per-run model override; `None` means use the stage/default model.
        model_override: Option<ModelId>,
        input_content: String,
        context: Vec<(String, String)>,
        template_override: Option<PromptTemplateOwned>,
    },
    /// A completion result came back from the worker.
    LlmCompleted {
        request_id: u64,
        result: LlmResultKind,
        /// Full run metadata. `None` only for pre-flight errors that fire
        /// before timing/model info is available (e.g. `PromptNotFound`).
        metadata: Option<LlmRunMetadata>,
    },
    /// User requested generation of a briefing.
    GenerateBriefingClicked,
    /// Headless batch flow: run triage + per-article summaries but skip aggregate briefing.
    PrepareSummariesClicked,
    /// User requested triage.
    TriageClicked,
    PreTriageDecisionSet {
        key: ArticleFilterKey,
        decision: ManualDecision,
    },
    PreTriageApplyClicked,
    PreTriageResetClicked,
    /// User requested polling all configured sources.
    PollSourcesClicked,
    /// User requested polling the indirect-link pool.
    PollIndirectLinks,
    /// Effect runner reports the total number of enabled sources to poll.
    PollStarted {
        total: usize,
    },
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
    /// Articles prepared by the loader.
    ArticlesLoaded {
        articles: Vec<LoadedArticle>,
        collection_text: String,
    },
    /// Loader failed.
    ArticlesLoadFailed {
        reason: String,
    },
    /// Triage-specific articles prepared by the loader.
    TriageArticlesLoaded {
        request_id: u64,
        articles: Vec<LoadedArticle>,
    },
    /// Incremental loader progress for a triage-specific article load.
    TriageArticlesLoadProgress {
        request_id: u64,
        files_scanned: usize,
        files_total: usize,
        matched_urls: usize,
    },
    /// Loader failed for triage.
    TriageArticlesLoadFailed {
        request_id: u64,
        reason: String,
    },
    /// Briefing prerequisite articles prepared by the loader.
    BriefingPrereqArticlesLoaded {
        articles: Vec<LoadedArticle>,
    },
    /// Briefing history loaded from disk at startup.
    /// On IO or parse failure, the effect runner sends this with an empty Vec
    /// rather than a separate failure message — keeps the reducer simple and avoids dead variants.
    BriefingHistoryLoaded {
        entries: Vec<crate::briefing::BriefingHistoryEntry>,
    },
    /// Briefing time checkpoint loaded from disk at startup.
    /// Raw wire type; the reducer parses the string into `DateTime<Utc>`.
    BriefingCheckpointLoaded {
        since_utc: Option<String>,
    },
    /// Briefing time checkpoint persisted successfully.
    BriefingCheckpointSaveSucceeded {
        save_id: u64,
    },
    /// Briefing time checkpoint persistence failed.
    BriefingCheckpointSaveFailed {
        save_id: u64,
        reason: String,
    },
    /// Request to update the in-memory briefing checkpoint (and persist it).
    /// Raw wire type; the reducer validates the string before storing.
    BriefingCheckpointSet(Option<String>),
    /// Loader failed for briefing prerequisites.
    BriefingPrereqLoadFailed {
        reason: String,
    },
    /// Prompt contexts loaded from disk.
    PromptContextsLoaded {
        contexts: HashMap<PromptId, Vec<(String, String)>>,
    },
    /// Prompt contexts failed to load.
    PromptContextsLoadFailed {
        reason: String,
    },
    /// LLM metadata (active prompt versions and effective models) loaded.
    LlmMetadataLoaded {
        active_versions: std::collections::HashMap<PromptId, PromptVersion>,
        effective_models: std::collections::HashMap<PromptId, String>,
        templates: std::collections::HashMap<PromptId, PromptLabTemplateSnapshot>,
    },
    /// Startup/effect boundary detected whether AI-backed workflows are available.
    AiAvailabilityDetected {
        availability: AiAvailability,
    },
    /// Summary cache hydrated from persisted store at startup.
    SummaryCacheHydrated {
        cache: crate::SummaryCache,
    },
    /// Triage cache hydrated from persisted store at startup.
    TriageCacheHydrated {
        cache: crate::TriageCache,
    },
    /// Pre-triage manual overrides hydrated from persisted store at startup.
    PreTriageOverridesHydrated {
        overrides: HashMap<ArticleFilterKey, ManualDecision>,
    },
    /// User requested to open the currently selected article URL in the default browser.
    OpenInBrowserClicked,
    /// User requested to open the Prompt Lab panel.
    PromptLabOpenRequested,
    /// User requested to close the Prompt Lab panel.
    PromptLabCloseRequested,
    /// User selected a different stage in the Prompt Lab.
    PromptLabStageSelected {
        stage: PromptLabStage,
    },
    /// User selected a different input source for Prompt Lab.
    PromptLabInputSourceSelected {
        source: PromptLabInputSource,
    },
    /// User edited the Prompt Lab input text.
    PromptLabInputChanged {
        text: String,
    },
    /// User edited the URL input used for TypeUrl runs.
    PromptLabUrlInputChanged {
        url: String,
    },
    /// User requested the TypeUrl resolver to fetch the URL content.
    PromptLabResolveRequested,
    /// Background effect finished resolving the URL input.
    PromptLabInputResolved {
        resolve_id: u64,
        result: Result<String, String>,
    },
    /// User opened the Prompt Lab context editor.
    PromptLabContextEditorOpened,
    /// User changed the context draft text.
    PromptLabContextDraftChanged {
        text: String,
    },
    /// User requested the draft to be applied.
    PromptLabContextApplyRequested,
    /// User requested apply and rerun.
    PromptLabContextApplyAndRerunRequested,
    /// User requested to revert draft edits.
    PromptLabContextRevertRequested,
    /// User requested saving the applied context to disk.
    PromptLabContextSaveRequested,
    /// User requested reloading the context from disk.
    PromptLabContextReloadRequested,
    /// Save effect succeeded.
    PromptLabContextSaved {
        prompt_id: PromptId,
        path: String,
        version: u64,
    },
    /// Save effect failed.
    PromptLabContextSaveFailed {
        prompt_id: PromptId,
        reason: String,
    },
    /// User toggled the Prompt Lab template editor open/closed.
    PromptLabTemplateEditorToggled,
    /// User changed the system template draft text.
    PromptLabTemplateSystemDraftChanged {
        text: String,
    },
    /// User changed the user template draft text.
    PromptLabTemplateUserDraftChanged {
        text: String,
    },
    /// User requested the template draft to be validated/applied.
    PromptLabTemplateApplyRequested,
    /// User requested to apply the template draft and rerun immediately.
    PromptLabTemplateApplyAndRerunRequested,
    /// User requested to revert template edits.
    PromptLabTemplateRevertRequested,
    /// User requested saving the applied template to disk.
    PromptLabTemplateSaveRequested,
    /// Template save effect succeeded.
    PromptLabTemplateSaved {
        prompt_id: PromptId,
        version: PromptVersion,
        path: String,
    },
    /// Template save effect failed.
    PromptLabTemplateSaveFailed {
        prompt_id: PromptId,
        reason: String,
    },
    /// User requested a Prompt Lab LLM run with the current input and stage.
    PromptLabRunRequested,
    /// User requested rerunning using the latest completed/final run parameters.
    PromptLabRerunRequested,
    /// User requested to clear completed/failed runs from Prompt Lab history.
    PromptLabHistoryCleared,
    PromptLabCompareDraftReset,
    PromptLabCompareCurrentSettingsCaptured,
    PromptLabCompareBaselineCaptured,
    PromptLabCompareCandidateRemoved {
        candidate_id: u64,
    },
    PromptLabCompareCandidateLabelChanged {
        candidate_id: u64,
        label: String,
    },
    PromptLabCompareBatchStartRequested,
    PromptLabCompareBatchConfirmedStart,
    PromptLabCompareBatchCancelRequested,
    PromptLabCompareWinnerSelected {
        run_id: PromptLabRunId,
    },
    PromptLabCompareWinnerCleared,
    PromptLabCompareRunRated {
        run_id: PromptLabRunId,
        rating: u8,
    },
    PromptLabAdvancedModeSet {
        enabled: bool,
    },
    PromptLabModelCatalogLoaded {
        models: Vec<ModelId>,
        source: crate::prompt_lab::ModelCatalogSource,
    },
    PromptLabModelOverrideSet {
        model: Option<ModelId>,
    },
    PromptLabCompareSectionToggled,
    PromptLabContextSectionToggled,
    PromptLabTemplateSectionToggled,
    PromptLabRunDetailsSectionToggled,
    PromptLabComparePolicyUpdated {
        require_parse_ok: Option<bool>,
        max_cost_microdollars: Option<Option<u64>>,
        max_wall_ms: Option<Option<u64>>,
        rating_beats_cost: Option<bool>,
    },
    PromptLabCompareAutoSelectRequested,
    PromptLabCompareBatchSetWarning {
        batch_id: PromptLabCompareBatchId,
        warning: Option<String>,
    },
    /// User selected a tab in the right pane.
    TabSelected {
        tab: AppTab,
    },
    /// User selected a tab in the left pane.
    LeftTabSelected {
        tab: LeftTab,
    },
    /// User changed the job list scope (All vs SinceCheckpoint).
    JobListScopeSet {
        scope: JobListScope,
    },
    /// User selected a trend category in the Trends tab.
    TrendCategorySelected {
        category: TrendCategory,
    },
    /// Entity index successfully loaded from disk.
    EntityIndexLoaded {
        index: crate::entity_index::EntityIndex,
    },
    /// Entity index failed to load from disk (parse error or IO error).
    EntityIndexLoadFailed {
        reason: String,
    },
    /// Entity index successfully rebuilt from the archive.
    EntityIndexRebuilt {
        index: crate::entity_index::EntityIndex,
    },
    /// Entity index rebuild failed.
    EntityIndexRebuildFailed {
        reason: String,
    },

    // --- Import saved webpages ---
    /// Request to import browser-saved .htm/.html files from `dir`.
    ImportSavedWebpagesRequested {
        dir: PathBuf,
    },
    /// Import batch completed (may include per-file failures).
    ImportSavedWebpagesCompleted {
        request_id: u64,
        report: harvester_engine::ImportReport,
    },
    /// Import batch failed at the directory level (scan or setup failure).
    ImportSavedWebpagesFailed {
        request_id: u64,
        reason: String,
    },
    /// Clear / reset the current imported corpus session.
    ImportedCorpusCleared,
}

/// Result payload returned by the LLM worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmResultKind {
    Success {
        output_json: String,
        input_tokens: u32,
        output_tokens: u32,
        prompt_version: PromptVersion,
        model_id: String,
    },
    ValidationFailed {
        reason: String,
        raw_response: String,
    },
    QuotaExhausted {
        reason: String,
    },
    Failed {
        reason: String,
    },
}
