use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp;
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
    /// Measured wall time in milliseconds for this run. Defaults to 0 for
    /// records written before this field was introduced.
    #[serde(default)]
    pub wall_ms: u64,
    /// Cache status string (`"miss"` or `"hit_validated"`). Defaults to
    /// `"miss"` for records written before this field was introduced.
    #[serde(default = "default_cache_status")]
    pub cache_status: String,
}

fn default_cache_status() -> String {
    "miss".to_string()
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

/// Returns the sanitized request id encoded in a replay record filename
/// produced by [`persist_replay_record`], or `None` for any other file.
///
/// This allows callers to index a replay directory by request id from the
/// directory listing alone, without opening or parsing any record.
pub fn replay_filename_request_id(file_name: &str) -> Option<&str> {
    let stem = file_name.strip_suffix(".json")?;
    // The hash prefix after the last "--" is hex, so the id is everything
    // before it even when the id itself contains "--".
    let (request_id, _) = stem.rsplit_once("--")?;
    (!request_id.is_empty()).then_some(request_id)
}

/// Returns the sanitized form of a request id as it appears in replay record
/// filenames. Matches what [`replay_filename_request_id`] extracts.
pub fn sanitize_replay_request_id(value: &str) -> String {
    sanitize_request_id(value)
}

/// Load a replay record from disk.
pub fn load_replay_record(path: &Path) -> Result<ReplayRecord, String> {
    let content =
        fs::read_to_string(path).map_err(|err| format!("reading {}: {}", path.display(), err))?;
    serde_json::from_str(&content).map_err(|err| format!("{}: {}", path.display(), err))
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
