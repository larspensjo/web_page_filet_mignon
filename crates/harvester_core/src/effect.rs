use std::collections::HashMap;
use std::path::PathBuf;

use harvester_engine::llm::dto::SummaryEntities;
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
        since_utc: Option<chrono::DateTime<chrono::Utc>>,
    },
    LoadArticlesForTriage {
        request_id: u64,
        ordered_urls: Vec<String>,
        since_utc: Option<chrono::DateTime<chrono::Utc>>,
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
        /// Extra key-value pairs inserted as individual template variables ({{key}}).
        /// NOT concatenated into the {{context}} block.
        extra_template_vars: Vec<(String, String)>,
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
    ArchiveRequested {
        request_id: u64,
        basename: String,
        ordered_urls: Vec<String>,
        since_utc: Option<chrono::DateTime<chrono::Utc>>,
        requested_checkpoint: Option<chrono::DateTime<chrono::Utc>>,
        use_summaries: bool,
        summaries: HashMap<String, String>,
    },
    OpenArchiveDialog {
        request_id: u64,
        article_count: usize,
        since_utc: Option<chrono::DateTime<chrono::Utc>>,
        default_basename: String,
        pending_pre_triage_count: usize,
        token_estimates: crate::ArchiveTokenEstimates,
        signal_candidate_default: crate::signal_candidate::SignalCandidateDialogDefault,
        signal_candidate_count: usize,
        signal_candidate_scoring_done: u32,
        signal_candidate_scoring_total: u32,
        signal_candidate_token_estimates: crate::ArchiveTokenEstimates,
    },
    ShowArchiveDialog {
        request_id: u64,
        article_count: usize,
        since_utc: Option<chrono::DateTime<chrono::Utc>>,
        default_basename: String,
        default_file_exists: bool,
        export_dir: PathBuf,
        pending_pre_triage_count: usize,
        token_estimates: crate::ArchiveTokenEstimates,
        signal_candidate_default: crate::signal_candidate::SignalCandidateDialogDefault,
        signal_candidate_count: usize,
        signal_candidate_scoring_done: u32,
        signal_candidate_scoring_total: u32,
        signal_candidate_token_estimates: crate::ArchiveTokenEstimates,
    },
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
    PersistSignalCandidateCache {
        cache: crate::signal_candidate_cache::SignalCandidateCache,
    },
    PersistSignalCandidateOverrides {
        overrides: std::collections::HashSet<crate::signal_candidate::OverrideKey>,
    },
    PersistTriageCache {
        cache: crate::TriageCache,
    },
    /// Load briefing history from disk at startup.
    LoadBriefingHistory,
    /// Save briefing history to disk after a successful briefing.
    SaveBriefingHistory {
        entries: Vec<crate::briefing::BriefingHistoryEntry>,
    },
    /// Load the briefing time checkpoint from disk at startup.
    LoadBriefingCheckpoint,
    /// Save (or clear) the briefing time checkpoint.
    SaveBriefingCheckpoint {
        save_id: u64,
        since_utc: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// Open a URL in the user's default web browser.
    OpenUrlInBrowser {
        url: String,
    },
    /// Load the entity index from disk (full loading in Slice 3).
    LoadEntityIndex,
    /// Rebuild the entity index from scratch from the article archive (full implementation Slice 4).
    RebuildEntityIndex,
    /// Upsert one article's entity data into the entity index (full implementation Slice 3).
    UpsertEntityIndexEntry {
        url: String,
        fetched_utc: Option<String>,
        content_hash: Option<String>,
        summary_entities: Option<SummaryEntities>,
        themes: Option<Vec<String>>,
    },

    // --- Import saved webpages ---
    /// Scan and import browser-saved .htm/.html files from `dir`.
    /// The effect runner resolves the archive dir from its own `RuntimePaths`.
    ImportSavedWebpages {
        dir: PathBuf,
        request_id: u64,
    },
    /// Persist the window's outer dimensions to disk.
    PersistWindowSize {
        width: i32,
        height: i32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPolicy {
    Finish,
    Immediate,
}
