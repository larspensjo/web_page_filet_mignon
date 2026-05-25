use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use harvester_core::OverrideKey;
use harvester_engine::{ensure_output_dir, AtomicFileWriter};
use serde::{Deserialize, Serialize};

const CURRENT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedOverrideKey {
    signal_key: String,
    prompt_id: String,
    prompt_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedFile {
    #[serde(default = "default_version")]
    version: u32,
    overrides: Vec<PersistedOverrideKey>,
}

fn default_version() -> u32 {
    CURRENT_FORMAT_VERSION
}

pub fn save(path: &Path, overrides: &HashSet<OverrideKey>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_output_dir(parent).map_err(io::Error::other)?;
    }
    let persisted = PersistedFile {
        version: CURRENT_FORMAT_VERSION,
        overrides: overrides
            .iter()
            .map(|override_key| PersistedOverrideKey {
                signal_key: override_key.signal_key.clone(),
                prompt_id: override_key.prompt_id.clone(),
                prompt_version: override_key.prompt_version,
            })
            .collect(),
    };
    let ron_text = ron::ser::to_string_pretty(&persisted, ron::ser::PrettyConfig::default())
        .map_err(|err| io::Error::other(err.to_string()))?;
    let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(".signal_candidate_overrides.ron");
    AtomicFileWriter::new(PathBuf::from(parent_dir))
        .write(filename, &ron_text)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(())
}

pub fn load(path: &Path) -> io::Result<HashSet<OverrideKey>> {
    if !path.exists() {
        return Ok(HashSet::new());
    }
    let text = std::fs::read_to_string(path)?;
    let persisted: PersistedFile = ron::from_str(&text)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    if persisted.version != CURRENT_FORMAT_VERSION {
        engine_logging::engine_warn!(
            "[signal-overrides] discarding unknown overrides version {} at {:?}",
            persisted.version,
            path
        );
        return Ok(HashSet::new());
    }
    Ok(persisted
        .overrides
        .into_iter()
        .map(|persisted| OverrideKey {
            signal_key: persisted.signal_key,
            prompt_id: persisted.prompt_id,
            prompt_version: persisted.prompt_version,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn overrides_round_trip_preserves_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".signal_candidate_overrides.ron");

        let mut overrides = HashSet::new();
        overrides.insert(OverrideKey {
            signal_key: "drop-this-cluster".into(),
            prompt_id: "ArticleSignalCandidate".into(),
            prompt_version: 1,
        });

        save(&path, &overrides).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, overrides);
    }

    #[test]
    fn overrides_load_returns_empty_when_file_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.ron");
        assert!(load(&path).unwrap().is_empty());
    }
}
