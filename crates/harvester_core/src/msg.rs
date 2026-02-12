use std::collections::HashMap;
use std::path::PathBuf;

use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::ExtractedLink;

use crate::briefing::LoadedArticle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// User edited the URL input box (debounced text).
    InputChanged(String),
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
    JobSelected { job_id: crate::JobId },
    /// User dragged the splitter to resize the left panels.
    SplitterMoved { desired_left_width_px: i32 },
    /// Window was resized.
    WindowResized { window_width: i32 },
    /// Fallback for placeholder wiring.
    NoOp,
    /// User requested an LLM completion.
    RequestLlmCompletion {
        prompt_id: PromptId,
        prompt_version: Option<PromptVersion>,
        input_content: String,
        context: Vec<(String, String)>,
    },
    /// A completion result came back from the worker.
    LlmCompleted {
        request_id: u64,
        result: LlmResultKind,
    },
    /// User requested generation of a briefing.
    GenerateBriefingClicked,
    /// User requested triage.
    TriageClicked,
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
    ArticlesLoadFailed { reason: String },
    /// Triage-specific articles prepared by the loader.
    TriageArticlesLoaded { articles: Vec<LoadedArticle> },
    /// Loader failed for triage.
    TriageArticlesLoadFailed { reason: String },
    /// Prompt contexts loaded from disk.
    PromptContextsLoaded {
        contexts: HashMap<PromptId, Vec<(String, String)>>,
    },
    /// Prompt contexts failed to load.
    PromptContextsLoadFailed { reason: String },
    /// LLM metadata (active prompt versions and effective models) loaded.
    LlmMetadataLoaded {
        active_versions: std::collections::HashMap<PromptId, PromptVersion>,
        effective_models: std::collections::HashMap<PromptId, String>,
    },
    /// Summary cache hydrated from persisted store at startup.
    SummaryCacheHydrated { cache: crate::SummaryCache },
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
