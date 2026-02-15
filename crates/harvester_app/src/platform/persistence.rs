use std::fs;
use std::path::{Path, PathBuf};

use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::{ArticleFilterKey, CompletedJobSnapshot, LinkSnapshotRecord, ManualDecision};
use harvester_engine::{ensure_output_dir, AtomicFileWriter};
use serde::{Deserialize, Serialize};

const STATE_FILENAME: &str = ".harvester_state.ron";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedJob {
    url: String,
    tokens: Option<u32>,
    bytes: Option<u64>,
    #[serde(default)]
    links: Vec<PersistedLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedLink {
    url: String,
    downloaded_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedState {
    completed: Vec<PersistedJob>,
    #[serde(default)]
    pre_triage_overrides: Vec<PersistedPreTriageOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedPreTriageOverride {
    url: String,
    content_hash: u64,
    include: bool,
}

pub(crate) fn load_completed_jobs(output_dir: &Path) -> Vec<CompletedJobSnapshot> {
    let path = output_dir.join(STATE_FILENAME);
    let content = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Vec::new();
        }
        Err(err) => {
            engine_warn!("Failed to read persisted state from {:?}: {}", path, err);
            return Vec::new();
        }
    };

    let state: PersistedState = match ron::from_str(&content) {
        Ok(state) => state,
        Err(err) => {
            engine_warn!("Failed to parse persisted state from {:?}: {}", path, err);
            return Vec::new();
        }
    };

    let completed = state
        .completed
        .into_iter()
        .map(|job| CompletedJobSnapshot {
            url: job.url,
            tokens: job.tokens,
            bytes: job.bytes,
            links: job
                .links
                .into_iter()
                .map(|link| LinkSnapshotRecord {
                    url: link.url,
                    downloaded_path: sanitize_downloaded_path(link.downloaded_path),
                })
                .collect(),
        })
        .collect();

    engine_info!("Loaded persisted completed jobs from {:?}", path);
    completed
}

pub(crate) fn load_pre_triage_overrides(
    output_dir: &Path,
) -> std::collections::HashMap<ArticleFilterKey, ManualDecision> {
    let path = output_dir.join(STATE_FILENAME);
    let content = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return std::collections::HashMap::new();
        }
        Err(err) => {
            engine_warn!("Failed to read persisted state from {:?}: {}", path, err);
            return std::collections::HashMap::new();
        }
    };
    let state: PersistedState = match ron::from_str(&content) {
        Ok(state) => state,
        Err(err) => {
            engine_warn!("Failed to parse persisted state from {:?}: {}", path, err);
            return std::collections::HashMap::new();
        }
    };
    state
        .pre_triage_overrides
        .into_iter()
        .map(|item| {
            (
                ArticleFilterKey {
                    url: item.url,
                    content_hash: item.content_hash,
                },
                if item.include {
                    ManualDecision::Include
                } else {
                    ManualDecision::Exclude
                },
            )
        })
        .collect()
}

fn sanitize_downloaded_path(path: Option<String>) -> Option<String> {
    match path {
        Some(value) if is_safe_downloaded_path(&value) => Some(value),
        Some(value) => {
            engine_warn!("Discarding unsafe persisted downloaded_path: {}", value);
            None
        }
        None => None,
    }
}

fn is_safe_downloaded_path(value: &str) -> bool {
    if value.contains("..") {
        return false;
    }
    if value.starts_with('/') || value.starts_with('\\') {
        return false;
    }
    let mut chars = value.chars();
    if let (Some(first), Some(second)) = (chars.next(), chars.next()) {
        if first.is_ascii_alphabetic() && second == ':' {
            return false;
        }
    }
    true
}

pub(crate) fn save_completed_jobs(output_dir: &Path, completed: &[CompletedJobSnapshot]) {
    save_state(output_dir, completed, &std::collections::HashMap::new());
}

pub(crate) fn save_state(
    output_dir: &Path,
    completed: &[CompletedJobSnapshot],
    pre_triage_overrides: &std::collections::HashMap<ArticleFilterKey, ManualDecision>,
) {
    if let Err(err) = ensure_output_dir(output_dir) {
        engine_error!("Failed to ensure output dir {:?}: {}", output_dir, err);
        return;
    }

    let state = PersistedState {
        completed: completed
            .iter()
            .map(|job| PersistedJob {
                url: job.url.clone(),
                tokens: job.tokens,
                bytes: job.bytes,
                links: job
                    .links
                    .iter()
                    .map(|link| PersistedLink {
                        url: link.url.clone(),
                        downloaded_path: link.downloaded_path.clone(),
                    })
                    .collect(),
            })
            .collect(),
        pre_triage_overrides: pre_triage_overrides
            .iter()
            .map(|(key, decision)| PersistedPreTriageOverride {
                url: key.url.clone(),
                content_hash: key.content_hash,
                include: matches!(decision, ManualDecision::Include),
            })
            .collect(),
    };

    let pretty = ron::ser::PrettyConfig::new();
    let content = match ron::ser::to_string_pretty(&state, pretty) {
        Ok(text) => text,
        Err(err) => {
            engine_error!("Failed to serialize persisted state: {}", err);
            return;
        }
    };

    let writer = AtomicFileWriter::new(PathBuf::from(output_dir));
    if let Err(err) = writer.write(STATE_FILENAME, &content) {
        engine_error!(
            "Failed to write persisted state to {:?}: {}",
            output_dir,
            err
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_state(dir: &Path, content: &str) {
        let path = dir.join(STATE_FILENAME);
        fs::write(&path, content).expect("write state");
    }

    #[test]
    fn load_state_without_links_still_parses_snapshot() {
        let temp = tempdir().expect("tempdir");
        let content = r#"
(
  completed: [
    (
      url: "https://example.com",
      tokens: Some(42u32),
      bytes: Some(1024u64),
    ),
  ],
)
"#;

        write_state(temp.path(), content);

        let snapshot = load_completed_jobs(temp.path());
        assert_eq!(snapshot.len(), 1);
        assert!(snapshot[0].links.is_empty());
    }

    #[test]
    fn save_and_load_roundtrips_links() {
        let temp = tempdir().expect("tempdir");
        let snapshot = vec![CompletedJobSnapshot {
            url: "https://example.com".to_string(),
            tokens: Some(10),
            bytes: Some(512),
            links: vec![
                LinkSnapshotRecord {
                    url: "https://a".to_string(),
                    downloaded_path: None,
                },
                LinkSnapshotRecord {
                    url: "https://b".to_string(),
                    downloaded_path: Some("linked/alpha.md".to_string()),
                },
            ],
        }];

        save_completed_jobs(temp.path(), &snapshot);
        let loaded = load_completed_jobs(temp.path());

        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn load_state_discards_poisoned_downloaded_path() {
        let temp = tempdir().expect("tempdir");
        let content = r#"
(
  completed: [
    (
      url: "https://example.com",
      tokens: Some(42u32),
      bytes: Some(1024u64),
      links: [
        (
          url: "https://attacker.com",
          downloaded_path: Some("../../etc/passwd"),
        ),
      ],
    ),
  ],
)
"#;

        write_state(temp.path(), content);

        let snapshot = load_completed_jobs(temp.path());
        assert_eq!(snapshot.len(), 1);
        assert!(snapshot[0].links.len() == 1);
        assert!(snapshot[0].links[0].downloaded_path.is_none());
    }
}
