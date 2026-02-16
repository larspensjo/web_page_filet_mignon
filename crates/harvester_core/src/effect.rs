use std::path::PathBuf;

use harvester_engine::llm::prompt::{PromptId, PromptTemplateOwned, PromptVersion};
use harvester_engine::llm::types::ModelId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    EnqueueUrl {
        job_id: crate::JobId,
        url: String,
    },
    LoadArticlesForBriefing {
        ordered_urls: Vec<String>,
    },
    LoadArticlesForBriefingPrereq {
        ordered_urls: Vec<String>,
    },
    LoadArticlesForTriage {
        ordered_urls: Vec<String>,
    },
    LoadPromptContexts,
    SavePromptContextFile {
        prompt_id: PromptId,
        context_pairs: Vec<(String, String)>,
    },
    SavePromptTemplateFile {
        prompt_id: PromptId,
        system_template: String,
        user_template: String,
        description: String,
        expected_format: String,
    },
    LoadPromptTemplateFiles,
    LoadLlmMetadata,
    PollAllSources,
    RequestLlmCompletion {
        request_id: u64,
        prompt_id: PromptId,
        prompt_version: Option<PromptVersion>,
        /// Per-run model override; `None` means use the stage/default model.
        model_override: Option<ModelId>,
        input_content: String,
        context: Vec<(String, String)>,
        template_override: Option<PromptTemplateOwned>,
    },
    ResolvePromptLabInputFromUrl {
        resolve_id: u64,
        url: String,
    },
    LoadPromptLabModelCatalog,
    StartSession,
    StopFinish {
        policy: StopPolicy,
    },
    ArchiveRequested,
    DownloadLinkedPage {
        job_id: crate::JobId,
        link_index: u32,
        url: String,
    },
    DeleteLinkedPage {
        job_id: crate::JobId,
        link_index: u32,
        path: PathBuf,
    },
    PersistSummaryCache {
        cache: crate::SummaryCache,
    },
    PersistTriageCache {
        cache: crate::TriageCache,
    },
    /// Open a URL in the user's default web browser.
    OpenUrlInBrowser {
        url: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPolicy {
    Finish,
    Immediate,
}
