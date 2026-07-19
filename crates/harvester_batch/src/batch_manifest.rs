//! Durable, fail-closed bookkeeping for paid OpenAI Batch API work.
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use harvester_core::{CollectedEntry, FrozenBatchKey, StageKind};
use harvester_engine::AtomicFileWriter;
use serde::{Deserialize, Serialize};

const MANIFEST_FILE: &str = ".batch_manifest.ron";
const MANIFEST_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatchState {
    Created,
    Submitted,
    Collected,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEntry {
    pub custom_id: String,
    pub key: FrozenBatchKey,
    pub stage: StageKind,
    pub attempts: u32,
    pub collected: Option<CollectedEntryRecord>,
}

/// Serializable collected snapshot.  The runner converts it to the core
/// contract only after this record has reached disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectedEntryRecord {
    pub custom_id: String,
    pub created_at_utc: String,
    pub outcome: CollectedOutcomeRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollectedOutcomeRecord {
    Success {
        raw_output_json: String,
        input_tokens: u32,
        output_tokens: u32,
        cached_input_tokens: u32,
        resolved_model: String,
    },
    LineError {
        detail: String,
    },
}

impl PendingEntry {
    pub fn collected_entry(&self, batch_id: &str) -> Option<CollectedEntry> {
        use harvester_core::CollectedOutcome;
        use harvester_engine::llm::TokenUsage;
        let collected = self.collected.as_ref()?;
        let outcome = match &collected.outcome {
            CollectedOutcomeRecord::Success {
                raw_output_json,
                input_tokens,
                output_tokens,
                cached_input_tokens,
                resolved_model,
            } => CollectedOutcome::Success {
                raw_output_json: raw_output_json.clone(),
                usage: TokenUsage {
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                    cached_input_tokens: *cached_input_tokens,
                },
                resolved_model: resolved_model.clone(),
            },
            CollectedOutcomeRecord::LineError { detail } => CollectedOutcome::LineError {
                detail: detail.clone(),
            },
        };
        Some(CollectedEntry {
            batch_id: batch_id.to_string(),
            custom_id: collected.custom_id.clone(),
            stage: self.stage,
            key: self.key.clone(),
            created_at_utc: collected.created_at_utc.clone(),
            outcome,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingBatch {
    pub input_file_id: String,
    pub batch_id: Option<String>,
    pub stage: String,
    pub completion_window: String,
    pub submitted_at_utc: String,
    pub status: BatchState,
    pub entries: Vec<PendingEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchManifest {
    pub version: u32,
    pub batches: Vec<PendingBatch>,
    /// Durable failure count retained after terminal batches are pruned.
    #[serde(default)]
    pub failed_attempts: HashMap<String, u32>,
}

impl Default for BatchManifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            batches: Vec::new(),
            failed_attempts: HashMap::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("batch manifest is unreadable; refusing to submit to avoid duplicate paid work: {0}")]
    Unreadable(String),
    #[error("batch manifest persistence failed: {0}")]
    Persist(String),
}

pub struct BatchManifestStore {
    output_dir: PathBuf,
    manifest: BatchManifest,
}

impl BatchManifestStore {
    pub fn load(output_dir: PathBuf) -> Result<Self, ManifestError> {
        let path = output_dir.join(MANIFEST_FILE);
        let mut manifest: BatchManifest = if path.exists() {
            let raw = fs::read_to_string(&path)
                .map_err(|err| ManifestError::Unreadable(err.to_string()))?;
            ron::from_str(&raw).map_err(|err| ManifestError::Unreadable(err.to_string()))?
        } else {
            BatchManifest::default()
        };
        let failed_batches: Vec<_> = manifest
            .batches
            .iter()
            .filter(|batch| batch.status == BatchState::Failed)
            .cloned()
            .collect();
        for batch in failed_batches {
            for entry in batch.entries {
                let next = entry.attempts.saturating_add(1);
                manifest
                    .failed_attempts
                    .entry(entry.custom_id)
                    .and_modify(|attempts| *attempts = (*attempts).max(next))
                    .or_insert(next);
            }
        }
        manifest
            .batches
            .retain(|batch| batch.status != BatchState::Failed);
        manifest.version = MANIFEST_VERSION;
        Ok(Self {
            output_dir,
            manifest,
        })
    }
    pub fn manifest(&self) -> &BatchManifest {
        &self.manifest
    }
    pub fn pending_custom_ids(&self) -> HashSet<String> {
        self.manifest
            .batches
            .iter()
            .filter(|batch| batch.status != BatchState::Failed)
            .flat_map(|batch| batch.entries.iter())
            .filter(|entry| {
                !matches!(
                    entry.collected.as_ref().map(|record| &record.outcome),
                    Some(CollectedOutcomeRecord::LineError { .. })
                )
            })
            .map(|entry| entry.custom_id.clone())
            .collect()
    }
    pub fn failed_attempts_for(&self, custom_id: &str) -> u32 {
        let retained = self
            .manifest
            .failed_attempts
            .get(custom_id)
            .copied()
            .unwrap_or_default();
        retained.max(
            self.manifest
                .batches
                .iter()
                .filter(|batch| {
                    batch.status == BatchState::Failed || batch.status == BatchState::Collected
                })
                .flat_map(|batch| batch.entries.iter())
                .filter(|entry| {
                    entry.custom_id == custom_id
                        && (entry.collected.is_none()
                            || matches!(
                                entry.collected.as_ref().map(|record| &record.outcome),
                                Some(CollectedOutcomeRecord::LineError { .. })
                            ))
                })
                .map(|entry| entry.attempts.saturating_add(1))
                .max()
                .unwrap_or_default(),
        )
    }
    pub fn reserve(&mut self, batch: PendingBatch) -> Result<(), ManifestError> {
        self.manifest.batches.push(batch);
        self.save()
    }
    pub fn attach_batch_id(
        &mut self,
        input_file_id: &str,
        batch_id: String,
    ) -> Result<(), ManifestError> {
        if let Some(batch) = self
            .manifest
            .batches
            .iter_mut()
            .find(|batch| batch.input_file_id == input_file_id)
        {
            batch.batch_id = Some(batch_id);
            batch.status = BatchState::Submitted;
        }
        self.save()
    }
    pub fn remove_batch(&mut self, input_file_id: &str) -> Result<(), ManifestError> {
        self.manifest
            .batches
            .retain(|batch| batch.input_file_id != input_file_id);
        self.save()
    }

    pub fn mark_failed(&mut self, input_file_id: &str) -> Result<(), ManifestError> {
        if let Some(batch) = self
            .manifest
            .batches
            .iter()
            .find(|batch| batch.input_file_id == input_file_id)
        {
            for entry in &batch.entries {
                let next = entry.attempts.saturating_add(1);
                self.manifest
                    .failed_attempts
                    .entry(entry.custom_id.clone())
                    .and_modify(|attempts| *attempts = (*attempts).max(next))
                    .or_insert(next);
            }
        }
        self.manifest
            .batches
            .retain(|batch| batch.input_file_id != input_file_id);
        self.save()
    }
    pub fn mark_collected(
        &mut self,
        input_file_id: &str,
        entries: Vec<CollectedEntryRecord>,
    ) -> Result<(), ManifestError> {
        let mut failed_attempts = Vec::new();
        if let Some(batch) = self
            .manifest
            .batches
            .iter_mut()
            .find(|batch| batch.input_file_id == input_file_id)
        {
            for entry in &mut batch.entries {
                entry.collected = entries
                    .iter()
                    .find(|record| record.custom_id == entry.custom_id)
                    .cloned();
                if matches!(
                    entry.collected.as_ref().map(|record| &record.outcome),
                    Some(CollectedOutcomeRecord::LineError { .. })
                ) {
                    failed_attempts
                        .push((entry.custom_id.clone(), entry.attempts.saturating_add(1)));
                }
            }
            batch.status = BatchState::Collected;
        }
        for (custom_id, next) in failed_attempts {
            self.manifest
                .failed_attempts
                .entry(custom_id)
                .and_modify(|attempts| *attempts = (*attempts).max(next))
                .or_insert(next);
        }
        self.save()
    }
    pub fn adopt_batch_id(
        &mut self,
        input_file_id: &str,
        batch_id: String,
    ) -> Result<(), ManifestError> {
        self.attach_batch_id(input_file_id, batch_id)
    }
    pub fn mark_entries_line_error(
        &mut self,
        custom_ids: &std::collections::HashSet<String>,
        detail: &str,
    ) -> Result<(), ManifestError> {
        for batch in &mut self.manifest.batches {
            if batch.status != BatchState::Collected {
                continue;
            }
            for entry in &mut batch.entries {
                if custom_ids.contains(&entry.custom_id) {
                    let next = entry.attempts.saturating_add(1);
                    self.manifest
                        .failed_attempts
                        .entry(entry.custom_id.clone())
                        .and_modify(|attempts| *attempts = (*attempts).max(next))
                        .or_insert(next);
                    entry.collected = Some(CollectedEntryRecord {
                        custom_id: entry.custom_id.clone(),
                        created_at_utc: entry
                            .collected
                            .as_ref()
                            .map(|record| record.created_at_utc.clone())
                            .unwrap_or_default(),
                        outcome: CollectedOutcomeRecord::LineError {
                            detail: detail.to_string(),
                        },
                    });
                }
            }
        }
        self.save()
    }
    pub fn save(&self) -> Result<(), ManifestError> {
        let content = ron::ser::to_string_pretty(&self.manifest, ron::ser::PrettyConfig::new())
            .map_err(|err| ManifestError::Persist(err.to_string()))?;
        AtomicFileWriter::new(self.output_dir.clone())
            .write(MANIFEST_FILE, &content)
            .map_err(|err| ManifestError::Persist(err.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
pub fn manifest_path(output_dir: &std::path::Path) -> PathBuf {
    output_dir.join(MANIFEST_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    fn entry() -> PendingEntry {
        PendingEntry {
            custom_id: "id".into(),
            key: FrozenBatchKey {
                content_hash: "key".into(),
                prompt_id: harvester_engine::llm::PromptId::ArticleTriage,
                prompt_version: 1,
                model_id: "model".into(),
                context_hash: "context".into(),
                stage: StageKind::Triage,
                url: "https://example.test".into(),
                rendered_system: "s".into(),
                rendered_user: "u".into(),
            },
            stage: StageKind::Triage,
            attempts: 0,
            collected: None,
        }
    }
    #[test]
    fn round_trip_and_reserve_without_attach_are_durable() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().to_path_buf();
        let mut store = BatchManifestStore::load(dir.clone()).unwrap();
        store
            .reserve(PendingBatch {
                input_file_id: "file_1".into(),
                batch_id: None,
                stage: "triage".into(),
                completion_window: "24h".into(),
                submitted_at_utc: "now".into(),
                status: BatchState::Created,
                entries: vec![entry()],
            })
            .unwrap();
        let loaded = BatchManifestStore::load(dir).unwrap();
        assert_eq!(loaded.manifest().batches[0].batch_id, None);
        assert!(loaded.pending_custom_ids().contains("id"));
    }
    #[test]
    fn corrupt_manifest_fails_closed() {
        let temp = TempDir::new().unwrap();
        fs::write(manifest_path(temp.path()), "not ron").unwrap();
        assert!(matches!(
            BatchManifestStore::load(temp.path().to_path_buf()),
            Err(ManifestError::Unreadable(_))
        ));
    }

    #[test]
    fn failed_and_line_error_batches_are_pruned_without_losing_attempt_count() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().to_path_buf();
        let mut store = BatchManifestStore::load(dir.clone()).unwrap();
        store
            .reserve(PendingBatch {
                input_file_id: "failed-file".into(),
                batch_id: Some("failed-batch".into()),
                stage: "triage".into(),
                completion_window: "24h".into(),
                submitted_at_utc: "now".into(),
                status: BatchState::Submitted,
                entries: vec![entry()],
            })
            .unwrap();
        store.mark_failed("failed-file").unwrap();
        assert!(store.manifest().batches.is_empty());
        assert_eq!(store.failed_attempts_for("id"), 1);

        let mut retried = entry();
        retried.attempts = 1;
        store
            .reserve(PendingBatch {
                input_file_id: "line-file".into(),
                batch_id: Some("line-batch".into()),
                stage: "triage".into(),
                completion_window: "24h".into(),
                submitted_at_utc: "later".into(),
                status: BatchState::Submitted,
                entries: vec![retried],
            })
            .unwrap();
        store
            .mark_collected(
                "line-file",
                vec![CollectedEntryRecord {
                    custom_id: "id".into(),
                    created_at_utc: "later".into(),
                    outcome: CollectedOutcomeRecord::LineError {
                        detail: "bad line".into(),
                    },
                }],
            )
            .unwrap();
        store.remove_batch("line-file").unwrap();

        let reloaded = BatchManifestStore::load(dir).unwrap();
        assert!(reloaded.manifest().batches.is_empty());
        assert_eq!(reloaded.failed_attempts_for("id"), 2);
    }
}
