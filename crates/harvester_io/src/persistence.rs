use std::fs;
use std::path::{Path, PathBuf};

use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::{ArticleFilterKey, CompletedJobSnapshot, LinkSnapshotRecord, ManualDecision};
use harvester_engine::{ensure_output_dir, AtomicFileWriter};
use serde::{Deserialize, Serialize};

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

pub fn load_completed_jobs(state_path: &Path) -> Vec<CompletedJobSnapshot> {
    let content = match fs::read_to_string(state_path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Vec::new();
        }
        Err(err) => {
            engine_warn!("Failed to read persisted state from {:?}: {}", state_path, err);
            return Vec::new();
        }
    };

    let state: PersistedState = match ron::from_str(&content) {
        Ok(state) => state,
        Err(err) => {
            engine_warn!("Failed to parse persisted state from {:?}: {}", state_path, err);
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

    engine_info!("Loaded persisted completed jobs from {:?}", state_path);
    completed
}

pub fn load_pre_triage_overrides(
    state_path: &Path,
) -> std::collections::HashMap<ArticleFilterKey, ManualDecision> {
    let content = match fs::read_to_string(state_path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return std::collections::HashMap::new();
        }
        Err(err) => {
            engine_warn!("Failed to read persisted state from {:?}: {}", state_path, err);
            return std::collections::HashMap::new();
        }
    };
    let state: PersistedState = match ron::from_str(&content) {
        Ok(state) => state,
        Err(err) => {
            engine_warn!("Failed to parse persisted state from {:?}: {}", state_path, err);
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

pub fn persist_completed_jobs(state_path: &Path, completed: &[CompletedJobSnapshot]) {
    persist_state(state_path, completed, &std::collections::HashMap::new());
}

pub fn persist_pre_triage_overrides(
    state_path: &Path,
    pre_triage_overrides: &std::collections::HashMap<ArticleFilterKey, ManualDecision>,
) {
    let completed = load_completed_jobs(state_path);
    persist_state(state_path, &completed, pre_triage_overrides);
}

fn persist_state(
    state_path: &Path,
    completed: &[CompletedJobSnapshot],
    pre_triage_overrides: &std::collections::HashMap<ArticleFilterKey, ManualDecision>,
) {
    let output_dir = state_path.parent().unwrap_or_else(|| Path::new("."));
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
    let filename = state_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".harvester_state.ron");
    if let Err(err) = writer.write(filename, &content) {
        engine_error!(
            "Failed to write persisted state to {:?}: {}",
            state_path,
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
        let path = dir.join(".harvester_state.ron");
        fs::write(&path, content).expect("write state");
    }

    fn state_path(dir: &Path) -> PathBuf {
        dir.join(".harvester_state.ron")
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

        let snapshot = load_completed_jobs(&state_path(temp.path()));
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

        persist_completed_jobs(&state_path(temp.path()), &snapshot);
        let loaded = load_completed_jobs(&state_path(temp.path()));

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

        let snapshot = load_completed_jobs(&state_path(temp.path()));
        assert_eq!(snapshot.len(), 1);
        assert!(snapshot[0].links.len() == 1);
        assert!(snapshot[0].links[0].downloaded_path.is_none());
    }
}
