use std::fs;
use std::path::{Path, PathBuf};

use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::{CompletedJobSnapshot, LinkSnapshotRecord};
use harvester_engine::{ensure_output_dir, AtomicFileWriter};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedJob {
    url: String,
    tokens: Option<u32>,
    bytes: Option<u64>,
    #[serde(default)]
    links: Vec<PersistedLink>,
    #[serde(default)]
    fetched_utc: Option<String>,
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
    pending_intake: Vec<String>,
    #[serde(default)]
    window_width: Option<i32>,
    #[serde(default)]
    window_height: Option<i32>,
    #[serde(default)]
    desktop_window_width: Option<i32>,
    #[serde(default)]
    desktop_window_height: Option<i32>,
}

/// Settings retained while replacing the completed-job snapshot. Deliberately
/// omits completed jobs so saves do not deserialize the old link collection.
#[derive(Debug, Deserialize, Default)]
struct PersistedRuntimeSettings {
    #[serde(default)]
    pending_intake: Vec<String>,
    #[serde(default)]
    window_width: Option<i32>,
    #[serde(default)]
    window_height: Option<i32>,
    #[serde(default)]
    desktop_window_width: Option<i32>,
    #[serde(default)]
    desktop_window_height: Option<i32>,
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

    let completed: Vec<CompletedJobSnapshot> = state
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
            fetched_utc: job.fetched_utc,
        })
        .collect();

    if !completed.is_empty() {
        engine_info!("Loaded persisted completed jobs from {:?}", state_path);
        return completed;
    }

    engine_info!(
        "No completed jobs found in persisted state {:?}",
        state_path
    );
    completed
}

/// Load URLs saved for intake by the next Full run. Older runtime-state files
/// omit the optional field and therefore return an empty list.
pub fn load_pending_intake(state_path: &Path) -> Vec<String> {
    let content = match fs::read_to_string(state_path) {
        Ok(text) => text,
        Err(_) => return Vec::new(),
    };
    match ron::from_str::<PersistedState>(&content) {
        Ok(state) => state.pending_intake,
        Err(_) => Vec::new(),
    }
}

pub fn load_desktop_window_size(state_path: &Path) -> Option<(i32, i32)> {
    let content = fs::read_to_string(state_path).ok()?;
    let state: PersistedState = ron::from_str(&content).ok()?;
    match (state.desktop_window_width, state.desktop_window_height) {
        (Some(width), Some(height)) => Some((width, height)),
        _ => None,
    }
}

pub fn persist_desktop_window_size(state_path: &Path, width: i32, height: i32) {
    let content = fs::read_to_string(state_path).unwrap_or_default();
    let mut state: PersistedState = ron::from_str(&content).unwrap_or_default();
    state.desktop_window_width = Some(width);
    state.desktop_window_height = Some(height);

    let output_dir = state_path.parent().unwrap_or_else(|| Path::new("."));
    if let Err(err) = ensure_output_dir(output_dir) {
        engine_error!("Failed to ensure output dir {:?}: {}", output_dir, err);
        return;
    }

    let serialized = match ron::ser::to_string_pretty(&state, ron::ser::PrettyConfig::new()) {
        Ok(text) => text,
        Err(err) => {
            engine_error!("Failed to serialize desktop window size: {}", err);
            return;
        }
    };
    let writer = AtomicFileWriter::new(PathBuf::from(output_dir));
    let filename = state_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(".harvester_state.ron");
    if let Err(err) = writer.write(filename, &serialized) {
        engine_error!(
            "Failed to write desktop window size to {:?}: {}",
            state_path,
            err
        );
    }
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
    persist_runtime_state(state_path, completed);
}

pub fn persist_runtime_state(state_path: &Path, completed: &[CompletedJobSnapshot]) {
    if let Err(err) = try_persist_runtime_state(state_path, completed) {
        engine_error!(
            "Failed to persist runtime state to {:?}: {}",
            state_path,
            err
        );
    }
}

/// Persist a runtime snapshot while reporting failure to observing hosts.
pub fn try_persist_runtime_state(
    state_path: &Path,
    completed: &[CompletedJobSnapshot],
) -> Result<(), String> {
    let existing: PersistedRuntimeSettings = fs::read_to_string(state_path)
        .ok()
        .and_then(|text| ron::from_str(&text).ok())
        .unwrap_or_default();
    let pending_intake = existing.pending_intake.clone();
    persist_runtime_state_with_existing(state_path, completed, &pending_intake, existing)
}

/// Persist the completed-job projection and the reducer-owned pending-intake list.
pub fn try_persist_runtime_state_with_pending(
    state_path: &Path,
    completed: &[CompletedJobSnapshot],
    pending_intake: &[String],
) -> Result<(), String> {
    let output_dir = state_path.parent().unwrap_or_else(|| Path::new("."));
    ensure_output_dir(output_dir).map_err(|err| format!("ensure output dir: {err}"))?;

    // Read settings only: old completed jobs and downloaded paths are not carried forward.
    let existing: PersistedRuntimeSettings = fs::read_to_string(state_path)
        .ok()
        .and_then(|text| ron::from_str(&text).ok())
        .unwrap_or_default();

    persist_runtime_state_with_existing(state_path, completed, pending_intake, existing)
}

fn persist_runtime_state_with_existing(
    state_path: &Path,
    completed: &[CompletedJobSnapshot],
    pending_intake: &[String],
    existing: PersistedRuntimeSettings,
) -> Result<(), String> {
    let output_dir = state_path.parent().unwrap_or_else(|| Path::new("."));
    ensure_output_dir(output_dir).map_err(|err| format!("ensure output dir: {err}"))?;

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
                fetched_utc: job.fetched_utc.clone(),
            })
            .collect(),
        pending_intake: pending_intake.to_vec(),
        window_width: existing.window_width,
        window_height: existing.window_height,
        desktop_window_width: existing.desktop_window_width,
        desktop_window_height: existing.desktop_window_height,
    };

    let pretty = ron::ser::PrettyConfig::new();
    let content = ron::ser::to_string_pretty(&state, pretty)
        .map_err(|err| format!("serialize persisted state: {err}"))?;

    let writer = AtomicFileWriter::new(PathBuf::from(output_dir));
    let filename = state_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".harvester_state.ron");
    writer
        .write(filename, &content)
        .map_err(|err| format!("write persisted state: {err}"))?;
    Ok(())
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
        assert!(load_pending_intake(&state_path(temp.path())).is_empty());
    }

    #[test]
    fn pending_intake_roundtrips_in_runtime_state() {
        let temp = tempdir().expect("tempdir");
        let path = state_path(temp.path());
        let jobs = vec![CompletedJobSnapshot {
            url: "https://example.com/completed".to_string(),
            tokens: Some(10),
            bytes: Some(512),
            links: vec![],
            fetched_utc: None,
        }];
        let pending = vec!["https://example.com/pending".to_string()];

        try_persist_runtime_state_with_pending(&path, &jobs, &pending).expect("persist snapshot");

        assert_eq!(load_pending_intake(&path), pending);
        assert_eq!(load_completed_jobs(&path), jobs);
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
            fetched_utc: None,
        }];

        persist_completed_jobs(&state_path(temp.path()), &snapshot);
        let loaded = load_completed_jobs(&state_path(temp.path()));

        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn load_state_without_window_size_deserializes_to_none() {
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
        let path = state_path(temp.path());
        let text = fs::read_to_string(&path).unwrap();
        let state: super::PersistedState = ron::from_str(&text).unwrap();
        assert_eq!(state.window_width, None);
        assert_eq!(state.window_height, None);
        assert_eq!(state.desktop_window_width, None);
        assert_eq!(state.desktop_window_height, None);
    }

    #[test]
    fn legacy_pre_triage_overrides_are_ignored_on_read() {
        let temp = tempdir().expect("tempdir");
        write_state(
            temp.path(),
            include_str!("../fixtures/legacy_pre_triage_overrides.ron"),
        );
        let path = state_path(temp.path());

        let completed = load_completed_jobs(&path);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].url, "https://example.com/completed");
        assert_eq!(
            {
                let state: PersistedState =
                    ron::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
                state.window_width.zip(state.window_height)
            },
            Some((1200, 900))
        );
    }

    #[test]
    fn persist_runtime_state_preserves_window_size() {
        let temp = tempdir().expect("tempdir");
        let path = state_path(temp.path());
        write_state(
            temp.path(),
            "(completed: [], window_width: Some(1200), window_height: Some(900))",
        );
        let jobs = vec![CompletedJobSnapshot {
            url: "https://example.com".to_string(),
            tokens: Some(10),
            bytes: Some(512),
            links: vec![],
            fetched_utc: None,
        }];
        persist_completed_jobs(&path, &jobs);
        let loaded_size = {
            let state: PersistedState = ron::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            state.window_width.zip(state.window_height)
        };
        assert_eq!(loaded_size, Some((1200, 900)));
    }

    #[test]
    fn desktop_window_size_roundtrips_without_changing_legacy_window_size() {
        let temp = tempdir().expect("tempdir");
        let path = state_path(temp.path());
        write_state(
            temp.path(),
            "(completed: [], window_width: Some(1200), window_height: Some(900))",
        );

        persist_desktop_window_size(&path, 960, 720);

        assert_eq!(
            {
                let state: PersistedState =
                    ron::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
                state.window_width.zip(state.window_height)
            },
            Some((1200, 900))
        );
        assert_eq!(load_desktop_window_size(&path), Some((960, 720)));
    }

    #[test]
    fn persist_runtime_state_preserves_desktop_window_size() {
        let temp = tempdir().expect("tempdir");
        let path = state_path(temp.path());
        persist_desktop_window_size(&path, 1200, 900);
        let jobs = vec![CompletedJobSnapshot {
            url: "https://example.com".to_string(),
            tokens: Some(10),
            bytes: Some(512),
            links: vec![],
            fetched_utc: None,
        }];

        persist_completed_jobs(&path, &jobs);

        assert_eq!(load_desktop_window_size(&path), Some((1200, 900)));
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

#[cfg(test)]
mod compatibility_tests {
    use super::*;

    #[test]
    fn old_link_paths_load_but_runtime_saves_drop_them_and_preserve_geometry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".harvester_state.ron");
        fs::write(&path, r#"(completed: [(url: "https://example.com/article", tokens: Some(42), bytes: Some(1024), links: [(url: "https://example.com/link", downloaded_path: Some("linked/old.md"))], fetched_utc: Some("2026-09-20T09:15:00Z"))], window_width: Some(1280), window_height: Some(800), desktop_window_width: Some(1512), desktop_window_height: Some(982))"#).unwrap();
        let loaded = load_completed_jobs(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded[0].links[0].downloaded_path.as_deref(),
            Some("linked/old.md")
        );
        let (state, _) = harvester_core::update(
            harvester_core::AppState::new(),
            harvester_core::Msg::RestoreCompletedJobs(loaded),
        );
        let (state, _) =
            harvester_core::update(state, harvester_core::Msg::JobSelected { job_id: 1 });
        assert_eq!(
            state.view().desktop_job_list.selected_job.unwrap().links[0].url,
            "https://example.com/link"
        );
        let snapshot = state.completed_jobs_snapshot();
        assert!(snapshot[0].links[0].downloaded_path.is_none());
        persist_runtime_state(&path, &snapshot);
        assert!(load_completed_jobs(&path)[0].links[0]
            .downloaded_path
            .is_none());
        assert_eq!(load_desktop_window_size(&path), Some((1512, 982)));
        let persisted: PersistedState = ron::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            persisted.window_width.zip(persisted.window_height),
            Some((1280, 800))
        );
    }

    #[test]
    fn runtime_save_ignores_old_completed_payload_and_preserves_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".harvester_state.ron");
        fs::write(&path, r#"(completed: "not a job collection", pending_intake: ["https://pending.example"], desktop_window_width: Some(960), desktop_window_height: Some(720))"#).unwrap();
        let completed = vec![CompletedJobSnapshot {
            url: "https://new.example/article".into(),
            tokens: Some(42),
            bytes: Some(1024),
            links: vec![LinkSnapshotRecord {
                url: "https://new.example/link".into(),
                downloaded_path: None,
            }],
            fetched_utc: Some("2026-09-20T09:15:00Z".into()),
        }];

        try_persist_runtime_state(&path, &completed).unwrap();
        let loaded = load_completed_jobs(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].url, completed[0].url);
        assert_eq!(loaded[0].links[0].url, completed[0].links[0].url);
        assert!(loaded[0].links[0].downloaded_path.is_none());
        assert_eq!(load_pending_intake(&path), vec!["https://pending.example"]);
        assert_eq!(load_desktop_window_size(&path), Some((960, 720)));

        try_persist_runtime_state_with_pending(&path, &completed, &[]).unwrap();
        assert!(load_pending_intake(&path).is_empty());
        assert_eq!(load_desktop_window_size(&path), Some((960, 720)));
    }
}
