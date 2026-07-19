//! Cross-layer contract for asynchronous LLM batch collection.
//!
//! Persistence remains owned by `harvester_batch`; this module deliberately
//! contains only immutable data crossing the runner/reducer boundary.
use harvester_engine::llm::{PromptId, TokenUsage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StageKind {
    Triage,
    Summary,
    SignalCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrozenBatchKey {
    pub content_hash: String,
    pub prompt_id: PromptId,
    pub prompt_version: u32,
    pub model_id: String,
    pub context_hash: String,
    pub stage: StageKind,
    pub url: String,
    pub rendered_system: String,
    pub rendered_user: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectedOutcome {
    Success {
        raw_output_json: String,
        usage: TokenUsage,
        resolved_model: String,
    },
    LineError {
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectedEntry {
    pub batch_id: String,
    pub custom_id: String,
    pub stage: StageKind,
    pub key: FrozenBatchKey,
    pub created_at_utc: String,
    pub outcome: CollectedOutcome,
}
