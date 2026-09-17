//! Source and evidence identities for replay recordings.

use harvester_engine::{llm::ReplayRecord, TRUNCATION_MARKER};
use serde::{Deserialize, Serialize};

use crate::{prompt_identity::sha256, recovery::RecoveredText};

/// The recording path that produced a replay entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    Sync,
    Batch,
}

/// A source-hash inconsistency that needs visibility in preconditions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityAnomaly {
    SyncHashDoesNotMatchEvidence,
    BatchUntruncatedHashDoesNotMatchEvidence,
}

/// Verification result that deliberately accounts for batch's clean-text hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceIdentity {
    pub matches_evidence: bool,
    pub expected_meaning: String,
    pub anomaly: Option<IdentityAnomaly>,
}

pub fn classify_path(record: &ReplayRecord) -> PathKind {
    if record.cache_status == "batch_collected" {
        PathKind::Batch
    } else {
        PathKind::Sync
    }
}

pub fn evidence_hash(recovered_text: &str) -> String {
    sha256(recovered_text)
}

pub fn is_truncated(text: &str) -> bool {
    text.ends_with(TRUNCATION_MARKER)
}

pub fn verify_source_identity(
    path_kind: PathKind,
    record: &ReplayRecord,
    recovered: &RecoveredText,
) -> SourceIdentity {
    let matches_evidence = record.input_content_hash == evidence_hash(&recovered.text);
    let anomaly = match path_kind {
        PathKind::Sync if !matches_evidence => Some(IdentityAnomaly::SyncHashDoesNotMatchEvidence),
        PathKind::Batch if !matches_evidence && !is_truncated(&recovered.text) => {
            Some(IdentityAnomaly::BatchUntruncatedHashDoesNotMatchEvidence)
        }
        _ => None,
    };
    SourceIdentity {
        matches_evidence,
        expected_meaning: match path_kind {
            PathKind::Sync => "hash of sent text".to_string(),
            PathKind::Batch => "hash of untruncated clean text".to_string(),
        },
        anomaly,
    }
}
