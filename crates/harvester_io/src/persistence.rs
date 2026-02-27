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
            engine_warn!(
                "Failed to read persisted state from {:?}: {}",
                state_path,
                err
            );
            return Vec::new();
        }
    };

    let state: PersistedState = match ron::from_str(&content) {
        Ok(state) => state,
        Err(err) => {
            engine_warn!(
                "Failed to parse persisted state from {:?}: {}",
                state_path,
                err
            );
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
            engine_warn!(
                "Failed to read persisted state from {:?}: {}",
                state_path,
                err
            );
            return std::collections::HashMap::new();
        }
    };
    let state: PersistedState = match ron::from_str(&content) {
        Ok(state) => state,
        Err(err) => {
            engine_warn!(
                "Failed to parse persisted state from {:?}: {}",
                state_path,
                err
            );
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
    persist_runtime_state(
        state_path,
        completed,
        &std::collections::HashMap::new(),
    );
}

pub fn persist_pre_triage_overrides(
    state_path: &Path,
    pre_triage_overrides: &std::collections::HashMap<ArticleFilterKey, ManualDecision>,
) {
    let completed = load_completed_jobs(state_path);
    persist_runtime_state(state_path, &completed, pre_triage_overrides);
}

pub fn persist_runtime_state(
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

// ──────────────────────────────────────────────────────────────────────────
// Briefing History Persistence
// ──────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedBriefingHistory {
    #[serde(default)]
    entries: Vec<PersistedBriefingEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedBriefingEntry {
    generated_at_utc: String,
    executive_summary: String,
    themes: Vec<PersistedBriefingTheme>,
    #[serde(default)]
    article_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedBriefingTheme {
    name: String,
    description: String,
}

/// Loads briefing history from disk. Returns an empty Vec on missing file or parse error.
pub fn load_briefing_history(path: &Path) -> Vec<harvester_core::BriefingHistoryEntry> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return vec![],
        Err(e) => {
            engine_warn!("[briefing-history] Failed to read {:?}: {}", path, e);
            return vec![];
        }
    };
    let persisted: PersistedBriefingHistory = match ron::from_str(&text) {
        Ok(p) => p,
        Err(e) => {
            engine_warn!("[briefing-history] Failed to parse {:?}: {}", path, e);
            return vec![];
        }
    };
    persisted
        .entries
        .into_iter()
        .filter_map(|e| {
            if e.generated_at_utc.trim().is_empty() {
                engine_warn!("[briefing-history] Dropping entry with empty timestamp");
                return None;
            }
            Some(harvester_core::BriefingHistoryEntry {
                generated_at_utc: e.generated_at_utc,
                executive_summary: e.executive_summary,
                themes: e
                    .themes
                    .into_iter()
                    .map(|t| harvester_core::BriefingHistoryTheme {
                        name: t.name,
                        description: t.description,
                    })
                    .collect(),
                article_count: e.article_count,
            })
        })
        .collect()
}

/// Saves briefing history to disk atomically. Logs on error; never panics.
pub fn save_briefing_history(
    path: &Path,
    entries: &[harvester_core::BriefingHistoryEntry],
) -> Result<(), String> {
    let output_dir = path.parent().unwrap_or(Path::new("."));
    ensure_output_dir(output_dir).map_err(|e| format!("ensure_output_dir: {e}"))?;
    let persisted = PersistedBriefingHistory {
        entries: entries
            .iter()
            .map(|e| PersistedBriefingEntry {
                generated_at_utc: e.generated_at_utc.clone(),
                executive_summary: e.executive_summary.clone(),
                themes: e
                    .themes
                    .iter()
                    .map(|t| PersistedBriefingTheme {
                        name: t.name.clone(),
                        description: t.description.clone(),
                    })
                    .collect(),
                article_count: e.article_count,
            })
            .collect(),
    };
    let pretty = ron::ser::PrettyConfig::new();
    let content = ron::ser::to_string_pretty(&persisted, pretty)
        .map_err(|e| format!("RON serialize: {e}"))?;
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("invalid file path: {:?}", path))?;
    let writer = AtomicFileWriter::new(PathBuf::from(output_dir));
    writer
        .write(filename, &content)
        .map(|_| ())
        .map_err(|e| format!("AtomicFileWriter: {e}"))
}

// ──────────────────────────────────────────────────────────────────────────
// Briefing Checkpoint Persistence
// ──────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedBriefingCheckpoint {
    since_utc: Option<String>,
}

/// Loads the briefing time checkpoint from disk.
/// Returns `None` on missing file (normal), malformed RON, or non-RFC3339 timestamp.
pub fn load_briefing_checkpoint(path: &Path) -> Option<String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            engine_warn!("[briefing-checkpoint] failed to read {:?}: {}", path, e);
            return None;
        }
    };
    let persisted: PersistedBriefingCheckpoint = match ron::from_str(&text) {
        Ok(p) => p,
        Err(e) => {
            engine_warn!("[briefing-checkpoint] malformed RON in {:?}: {}", path, e);
            return None;
        }
    };
    let value = persisted.since_utc?;
    // Validate RFC3339 at IO boundary (defense-in-depth; reducer also validates)
    match chrono::DateTime::parse_from_rfc3339(&value) {
        Ok(_) => {
            engine_info!("[briefing-checkpoint] loaded: {}", value);
            Some(value)
        }
        Err(e) => {
            engine_warn!("[briefing-checkpoint] invalid RFC3339 in {:?}: {}", path, e);
            None
        }
    }
}

/// Saves (or clears) the briefing time checkpoint.
/// `since_utc = None` deletes the file; otherwise the RFC3339 string is written atomically.
pub fn save_briefing_checkpoint(path: &Path, since_utc: Option<&str>) -> Result<(), String> {
    if since_utc.is_none() {
        match fs::remove_file(path) {
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("failed to delete checkpoint {:?}: {}", path, e)),
        }
    }
    let output_dir = path.parent().unwrap_or(Path::new("."));
    ensure_output_dir(output_dir).map_err(|e| format!("ensure_output_dir: {e}"))?;
    let persisted = PersistedBriefingCheckpoint {
        since_utc: since_utc.map(str::to_owned),
    };
    let pretty = ron::ser::PrettyConfig::new();
    let content = ron::ser::to_string_pretty(&persisted, pretty)
        .map_err(|e| format!("RON serialize: {e}"))?;
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("invalid file path: {:?}", path))?;
    let writer = AtomicFileWriter::new(PathBuf::from(output_dir));
    writer
        .write(filename, &content)
        .map(|_| ())
        .map_err(|e| format!("AtomicFileWriter: {e}"))
}

#[cfg(test)]
mod briefing_checkpoint_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn checkpoint_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".briefing_checkpoint.ron");
        save_briefing_checkpoint(&path, Some("2025-12-31T23:00:00Z")).unwrap();
        let loaded = load_briefing_checkpoint(&path);
        assert_eq!(loaded.as_deref(), Some("2025-12-31T23:00:00Z"));
    }

    #[test]
    fn checkpoint_absent_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".briefing_checkpoint.ron");
        assert!(load_briefing_checkpoint(&path).is_none());
    }

    #[test]
    fn checkpoint_clear_deletes_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".briefing_checkpoint.ron");
        save_briefing_checkpoint(&path, Some("2025-12-31T23:00:00Z")).unwrap();
        assert!(path.exists());
        save_briefing_checkpoint(&path, None).unwrap();
        assert!(!path.exists());
        assert!(load_briefing_checkpoint(&path).is_none());
    }

    #[test]
    fn checkpoint_malformed_ron_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".briefing_checkpoint.ron");
        std::fs::write(&path, "{{not valid ron]]").unwrap();
        assert!(load_briefing_checkpoint(&path).is_none());
    }

    #[test]
    fn checkpoint_invalid_timestamp_returns_none() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".briefing_checkpoint.ron");
        std::fs::write(&path, "(since_utc: Some(\"not-a-timestamp\"))").unwrap();
        assert!(load_briefing_checkpoint(&path).is_none());
    }
}

#[cfg(test)]
mod briefing_history_tests {
    use super::*;
    use harvester_core::{BriefingHistoryEntry, BriefingHistoryTheme};
    use tempfile::TempDir;

    fn make_entry(ts: &str) -> BriefingHistoryEntry {
        BriefingHistoryEntry {
            generated_at_utc: ts.to_string(),
            executive_summary: format!("Summary for {ts}"),
            themes: vec![BriefingHistoryTheme {
                name: "Topic".to_string(),
                description: "Details.".to_string(),
            }],
            article_count: 3,
        }
    }

    #[test]
    fn round_trip_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".briefing_history.ron");
        save_briefing_history(&path, &[]).unwrap();
        let loaded = load_briefing_history(&path);
        assert!(loaded.is_empty());
    }

    #[test]
    fn round_trip_three_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".briefing_history.ron");
        let entries: Vec<_> = [
            "2026-02-21T10:00:00Z",
            "2026-02-21T08:00:00Z",
            "2026-02-20T18:00:00Z",
        ]
        .iter()
        .map(|ts| make_entry(ts))
        .collect();
        save_briefing_history(&path, &entries).unwrap();
        let loaded = load_briefing_history(&path);
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].generated_at_utc, "2026-02-21T10:00:00Z");
        assert_eq!(loaded[0].themes[0].name, "Topic");
    }

    #[test]
    fn missing_file_returns_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.ron");
        let loaded = load_briefing_history(&path);
        assert!(loaded.is_empty());
    }

    #[test]
    fn malformed_ron_returns_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".briefing_history.ron");
        std::fs::write(&path, "{{not valid ron]]").unwrap();
        let loaded = load_briefing_history(&path);
        assert!(loaded.is_empty());
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
