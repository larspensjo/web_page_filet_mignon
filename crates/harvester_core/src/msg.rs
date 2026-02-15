use std::collections::HashMap;
use std::path::PathBuf;

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
    /// User clicked Stop/Finish.
    StopFinishClicked,
    /// User clicked Archive.
    ArchiveClicked,
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
    /// Polling completed for a source.
    SourcePollCompleted {
        source_id: harvester_engine::SourceId,
        urls: Vec<String>,
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
        articles: Vec<LoadedArticle>,
    },
    /// Loader failed for triage.
    TriageArticlesLoadFailed {
        reason: String,
    },
    /// Briefing prerequisite articles prepared by the loader.
    BriefingPrereqArticlesLoaded {
        articles: Vec<LoadedArticle>,
    },
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
    /// User opened the Prompt Lab template editor.
    PromptLabTemplateEditorOpened,
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
