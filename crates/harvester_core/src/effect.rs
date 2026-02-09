use std::path::PathBuf;

use harvester_engine::llm::prompt::{PromptId, PromptVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    EnqueueUrl {
        job_id: crate::JobId,
        url: String,
    },
    LoadArticlesForBriefing,
    RequestLlmCompletion {
        request_id: u64,
        prompt_id: PromptId,
        prompt_version: Option<PromptVersion>,
        input_content: String,
        context: Vec<(String, String)>,
    },
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPolicy {
    Finish,
    Immediate,
}
