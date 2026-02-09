use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::{
    path_policy::is_confined_to,
    persist::{ensure_output_dir, AtomicFileWriter, PersistError},
};

use super::prompt::{PromptId, PromptVersion};
use super::types::TokenUsage;

/// Replay record persisted after each LLM completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayRecord {
    pub request_id: String,
    pub input_content_hash: String,
    pub prompt_id: PromptId,
    pub prompt_version: PromptVersion,
    pub model_id: String,
    pub timestamp_utc: String,
    pub rendered_system_message: String,
    pub rendered_user_message: String,
    pub raw_response: String,
    pub usage: TokenUsage,
    pub validated_output: Option<Value>,
    pub validation_error: Option<String>,
    pub cost_microdollars: u64,
}

/// Returns a SHA-256 hex digest for the provided input.
pub fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// Persist a replay record to disk with append-only semantics.
pub fn persist_replay_record(
    output_dir: &Path,
    record: &ReplayRecord,
) -> Result<PathBuf, PersistError> {
    ensure_output_dir(output_dir)?;
    let serialized = serde_json::to_string_pretty(record)
        .map_err(|err| PersistError::Io(io::Error::other(err)))?;

    let writer = AtomicFileWriter::new(output_dir.to_path_buf());
    let base_name = record_filename_base(record);

    for suffix in 0.. {
        let candidate = candidate_filename(&base_name, suffix);
        validate_candidate(&candidate)?;

        let target = output_dir.join(&candidate);
        if target.exists() {
            continue;
        }

        let path = writer.write(&candidate, &serialized)?;
        if !is_confined_to(Path::new(&candidate), output_dir) {
            fs::remove_file(&path).map_err(PersistError::Io)?;
            return Err(PersistError::OutputDir(
                "filename escapes output directory".into(),
            ));
        }

        return Ok(path);
    }

    unreachable!("append-only filename generation should not loop forever")
}

/// Load a replay record from disk.
pub fn load_replay_record(path: &Path) -> Result<ReplayRecord, String> {
    let content =
        fs::read_to_string(path).map_err(|err| format!("reading {}: {}", path.display(), err))?;
    serde_json::from_str(&content).map_err(|err| format!("{}: {}", path.display(), err))
}

/// In-memory cache of replayed completions.
pub struct ReplayProvider {
    records: HashMap<String, ReplayRecord>,
}

impl ReplayProvider {
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    pub fn from_records(records: HashMap<String, ReplayRecord>) -> Self {
        Self { records }
    }

    /// Load every JSON file in `dir`.
    pub fn load_from_dir(dir: &Path) -> Result<Self, String> {
        let mut provider = Self::new();
        if !dir.exists() {
            return Ok(provider);
        }

        for entry in fs::read_dir(dir).map_err(|err| err.to_string())? {
            let entry = entry.map_err(|err| err.to_string())?;
            let path = entry.path();
            if !entry.file_type().map_err(|err| err.to_string())?.is_file() {
                continue;
            }

            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            let record =
                load_replay_record(&path).map_err(|err| format!("{}: {}", path.display(), err))?;
            let key = lookup_key(
                &record.input_content_hash,
                record.prompt_id,
                record.prompt_version,
            );
            provider.records.entry(key).or_insert(record);
        }

        Ok(provider)
    }

    pub fn lookup(
        &self,
        input_hash: &str,
        prompt_id: PromptId,
        prompt_version: PromptVersion,
    ) -> Option<&ReplayRecord> {
        let key = lookup_key(input_hash, prompt_id, prompt_version);
        self.records.get(&key)
    }

    pub fn insert(&mut self, record: ReplayRecord) {
        let key = lookup_key(
            &record.input_content_hash,
            record.prompt_id,
            record.prompt_version,
        );
        self.records.insert(key, record);
    }
}

impl Default for ReplayProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn lookup_key(input_hash: &str, prompt_id: PromptId, prompt_version: PromptVersion) -> String {
    format!("{input_hash}::{prompt_id:?}::{prompt_version}")
}

fn record_filename_base(record: &ReplayRecord) -> String {
    let sanitized_id = sanitize_request_id(&record.request_id);
    let prefix_len = cmp::min(8, record.input_content_hash.len());
    let hash_prefix = &record.input_content_hash[..prefix_len];
    format!("{sanitized_id}--{hash_prefix}")
}

fn candidate_filename(base: &str, suffix: u32) -> String {
    if suffix == 0 {
        format!("{base}.json")
    } else {
        format!("{base}-{suffix}.json")
    }
}

fn validate_candidate(candidate: &str) -> Result<(), PersistError> {
    let path = Path::new(candidate);
    if path.is_absolute() {
        return Err(PersistError::OutputDir(
            "absolute filenames forbidden".into(),
        ));
    }

    for component in path.components() {
        match component {
            Component::ParentDir | Component::RootDir => {
                return Err(PersistError::OutputDir("invalid filename component".into()))
            }
            _ => {}
        }
    }

    Ok(())
}

fn sanitize_request_id(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}
