use std::fs;
use std::path::{Path, PathBuf};

use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::{CompletedJobSnapshot, LinkSnapshotRecord, SlimJobRecord};
use harvester_engine::{ensure_output_dir, AtomicFileWriter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedJob {
    url: String,
    tokens: Option<u32>,
    bytes: Option<u64>,
    #[serde(default, skip_serializing)]
    links: Vec<Box<ron::value::RawValue>>,
    #[serde(default)]
    fetched_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedLink {
    url: String,
    #[serde(default)]
    downloaded_path: Option<String>,
    #[serde(default, alias = "anchor_text")]
    text: Option<String>,
    #[serde(default = "hyperlink_kind")]
    kind: harvester_engine::LinkKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedState {
    completed: Vec<PersistedJob>,
    #[serde(default)]
    links_in_store: bool,
    #[serde(default)]
    fetch_time_recovery_done: bool,
    #[serde(default)]
    job_list_mode: Option<harvester_core::JobListMode>,
    #[serde(default)]
    selected_article_url: Option<String>,
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
/// retains no completed jobs; typed consumption avoids RON's slow generic skip.
#[derive(Debug, Deserialize, Default)]
struct PersistedRuntimeSettings {
    // RON's generic IgnoredAny path for an omitted `completed` field is very
    // expensive at corpus scale. Consume typed jobs, discarding each immediately.
    #[serde(
        default,
        rename = "completed",
        deserialize_with = "discard_completed_jobs"
    )]
    _completed: (),
    #[serde(default)]
    links_in_store: bool,
    #[serde(default)]
    fetch_time_recovery_done: bool,
    #[serde(default)]
    job_list_mode: Option<harvester_core::JobListMode>,
    #[serde(default)]
    selected_article_url: Option<String>,
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

fn discard_completed_jobs<'de, D: serde::Deserializer<'de>>(de: D) -> Result<(), D::Error> {
    struct Jobs;
    impl<'de> serde::de::Visitor<'de> for Jobs {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("completed jobs")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
            while seq.next_element::<PersistedJob>()?.is_some() {}
            Ok(())
        }
        fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
            Ok(())
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
            Ok(())
        }
    }
    de.deserialize_any(Jobs)
}

fn parsed_legacy_links(job: &PersistedJob, path: &Path) -> Vec<PersistedLink> {
    job.links.iter().enumerate().filter_map(|(index, raw)| {
        match ron::from_str::<PersistedLink>(raw.get_ron()) {
            Ok(link) => Some(link),
            Err(error) => {
                engine_warn!("[runtime-state] migration skip malformed link path={} article={} record={} error={}", path.display(), job.url, index, error);
                None
            }
        }
    }).collect()
}

fn hyperlink_kind() -> harvester_engine::LinkKind {
    harvester_engine::LinkKind::Hyperlink
}

static RUNTIME_IO: Mutex<()> = Mutex::new(());
const BACKUP_NAME: &str = ".harvester_state.pre-slim.ron";
const BACKUP_TEMP_PREFIX: &str = ".harvester_state.pre-slim-partial-";
const SLIM_TEMP_PREFIX: &str = ".harvester_state.slim-partial-";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationPoint {
    BackupPartial,
    BackupPublished,
    LinkPublished,
    SlimPartial,
    SlimVerified,
}

fn migration_error(path: &Path, step: &str, error: impl std::fmt::Display) -> String {
    format!("{}: migration {step}: {error}", path.display())
}

fn pinned_temp(output: &Path, prefix: &str) -> PathBuf {
    output.join(format!(
        "{prefix}{}.ron",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%.9f")
    ))
}

fn remove_migration_leftovers(output: &Path) -> Result<(), String> {
    for entry in fs::read_dir(output).map_err(|e| migration_error(output, "list leftovers", e))? {
        let entry = entry.map_err(|e| migration_error(output, "inspect leftover", e))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if [BACKUP_TEMP_PREFIX, SLIM_TEMP_PREFIX]
            .iter()
            .any(|prefix| name.starts_with(prefix))
            && name.ends_with(".ron")
        {
            engine_info!(
                "[runtime-state] migration restart discarding temporary {}",
                entry.path().display()
            );
            fs::remove_file(entry.path())
                .map_err(|e| migration_error(&entry.path(), "remove leftover", e))?;
        }
    }
    Ok(())
}

fn byte_identical(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && Sha256::digest(a) == Sha256::digest(b)
}

/// Preserve the original bytes before publishing links and slim runtime state.
/// Returns true only when this call completes a migration; retries are idempotent.
pub fn migrate_runtime_state(state_path: &Path) -> Result<bool, String> {
    let _guard = RUNTIME_IO.lock().map_err(|e| e.to_string())?;
    migrate_with_interrupt(state_path, |_| Ok(()))
}

fn migrate_with_interrupt(
    state_path: &Path,
    checkpoint: impl FnMut(MigrationPoint) -> Result<(), String>,
) -> Result<bool, String> {
    migrate_with_notices(state_path, checkpoint, |_| {})
}

fn migrate_with_notices(
    state_path: &Path,
    mut checkpoint: impl FnMut(MigrationPoint) -> Result<(), String>,
    mut notice: impl FnMut(String),
) -> Result<bool, String> {
    let started = std::time::Instant::now();
    engine_info!(
        "[runtime-state] operation=migration-check path={} start",
        state_path.display()
    );
    let output = state_path.parent().unwrap_or_else(|| Path::new("."));
    if !output.exists() {
        engine_info!(
            "[runtime-state] operation=migration-check path={} elapsed_ms={} result=missing-output",
            state_path.display(),
            started.elapsed().as_millis()
        );
        return Ok(false);
    }
    remove_migration_leftovers(output)?;
    let original = match fs::read(state_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(migration_error(state_path, "read original", e)),
    };
    let parsed = ron::de::from_bytes::<PersistedState>(&original);
    let unparseable = parsed.is_err();
    if let Err(error) = &parsed {
        engine_warn!(
            "[runtime-state] parse original path={} error={}",
            state_path.display(),
            error
        );
    }
    let mut state = parsed.unwrap_or_default();
    if state.links_in_store {
        if let Err(error) = crate::article_links::remove_partial_links(output) {
            engine_warn!(
                "[runtime-state] migration completed; link temporary recovery output={} error={}",
                output.display(),
                error
            );
        }
        engine_info!(
            "[runtime-state] operation=migration-check path={} elapsed_ms={} result=already-slim",
            state_path.display(),
            started.elapsed().as_millis()
        );
        return Ok(false);
    }
    let mut backup = output.join(BACKUP_NAME);
    let backup_matches = match fs::symlink_metadata(&backup) {
        Ok(metadata) => {
            !metadata.file_type().is_symlink()
                && fs::read(&backup).is_ok_and(|saved| byte_identical(&original, &saved))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(migration_error(&backup, "inspect backup", e)),
    };
    let dated_backup = unparseable || (!backup_matches && fs::symlink_metadata(&backup).is_ok());
    if dated_backup {
        // Timestamp plus collision counter: no backup, including an owner-created one,
        // can be overwritten. This namespace is distinct from disposable partials.
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.9f");
        for suffix in 0_u64.. {
            backup = output.join(format!(
                ".harvester_state.pre-slim-{timestamp}-{suffix}.ron"
            ));
            match fs::symlink_metadata(&backup) {
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(migration_error(&backup, "inspect dated backup", error)),
            }
        }
    }
    if !backup_matches || dated_backup {
        let temp = pinned_temp(output, BACKUP_TEMP_PREFIX);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| migration_error(&temp, "create backup", e))?;
        let halfway = original.len() / 2;
        file.write_all(&original[..halfway])
            .map_err(|e| migration_error(&temp, "copy backup", e))?;
        checkpoint(MigrationPoint::BackupPartial)?;
        file.write_all(&original[halfway..])
            .and_then(|_| file.flush())
            .and_then(|_| file.sync_all())
            .map_err(|e| migration_error(&temp, "flush backup", e))?;
        drop(file);
        let copied =
            fs::read(&temp).map_err(|e| migration_error(&temp, "read copied backup", e))?;
        if !byte_identical(&original, &copied) {
            return Err(migration_error(
                &temp,
                "verify backup",
                "length or SHA-256 differs",
            ));
        }
        // A backup appearing after the existence check must also survive publication.
        tempfile::TempPath::try_from_path(temp.clone())
            .map_err(|e| migration_error(&temp, "prepare backup rename", e))?
            .persist_noclobber(&backup)
            .map_err(|e| migration_error(&backup, "publish backup", e.error))?;
    }
    if dated_backup {
        let reason = if unparseable {
            "Unreadable runtime state"
        } else {
            "Restored runtime state"
        };
        let message = format!(
            "{reason} preserved in {}; runtime saving continues.",
            backup.file_name().unwrap().to_string_lossy()
        );
        engine_warn!("[runtime-state] path={} {}", state_path.display(), message);
        notice(message);
    }
    checkpoint(MigrationPoint::BackupPublished)?;
    crate::article_links::remove_partial_links(output)?;
    // Read from the verified original, never from a partially rewritten active state.
    let mut articles =
        std::collections::BTreeMap::<String, Vec<harvester_engine::ExtractedLink>>::new();
    for job in &state.completed {
        let legacy_links = parsed_legacy_links(job, state_path);
        if legacy_links.is_empty() {
            continue;
        }
        let links = articles
            .entry(harvester_engine::archive_url_key(&job.url))
            .or_default();
        for link in legacy_links {
            let record = harvester_engine::ExtractedLink {
                url: link.url.clone(),
                text: link.text.clone(),
                kind: link.kind.clone(),
            };
            links.push(record);
        }
    }
    for (url, links) in articles {
        if links.is_empty() {
            continue;
        }
        // Restored snapshots can be older than the link store. Retain all existing
        // records rather than replacing richer session data with the old snapshot.
        let mut merged = match crate::article_links::try_load_article_links(output, &url) {
            Ok(links) => links,
            Err(error) => {
                engine_warn!(
                    "[runtime-state] retain unreadable link file url={} output={} error={}",
                    url,
                    output.display(),
                    error
                );
                continue;
            }
        };
        for link in links {
            if !merged.contains(&link) {
                merged.push(link);
            }
        }
        crate::write_article_links(output, &url, &merged)
            .map_err(|e| migration_error(state_path, &format!("write links url={url}"), e))?;
        checkpoint(MigrationPoint::LinkPublished)?;
    }
    state.links_in_store = true;
    let slim = ron::ser::to_string_pretty(&state, ron::ser::PrettyConfig::new())
        .map_err(|e| migration_error(state_path, "serialize slim state", e))?;
    let temp = pinned_temp(output, SLIM_TEMP_PREFIX);
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|e| migration_error(&temp, "create slim temporary", e))?;
    let bytes = slim.as_bytes();
    let halfway = bytes.len() / 2;
    file.write_all(&bytes[..halfway])
        .map_err(|e| migration_error(&temp, "write slim state", e))?;
    checkpoint(MigrationPoint::SlimPartial)?;
    file.write_all(&bytes[halfway..])
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all())
        .map_err(|e| migration_error(&temp, "flush slim state", e))?;
    drop(file);
    let verified: PersistedState = ron::de::from_bytes(
        &fs::read(&temp).map_err(|e| migration_error(&temp, "read slim state", e))?,
    )
    .map_err(|e| migration_error(&temp, "verify slim state loads", e))?;
    if !verified.links_in_store {
        return Err(migration_error(
            &temp,
            "verify slim state",
            "missing store marker",
        ));
    }
    checkpoint(MigrationPoint::SlimVerified)?;
    fs::rename(&temp, state_path)
        .map_err(|e| migration_error(state_path, "replace active state", e))?;
    engine_info!(
        "[runtime-state] migration complete path={} jobs={} backup={} elapsed_ms={}",
        state_path.display(),
        state.completed.len(),
        backup.display(),
        started.elapsed().as_millis()
    );
    Ok(true)
}

fn atomic_runtime_write(path: &Path, content: &str) -> Result<(), String> {
    let output = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::Builder::new()
        .prefix(SLIM_TEMP_PREFIX)
        .suffix(".ron")
        .tempfile_in(output)
        .map_err(|e| format!("{}: runtime temporary: {e}", path.display()))?;
    temp.write_all(content.as_bytes())
        .and_then(|_| temp.flush())
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| format!("{}: flush runtime state: {e}", path.display()))?;
    temp.persist(path)
        .map_err(|e| format!("{}: publish runtime state: {}", path.display(), e.error))?;
    Ok(())
}

/// Read-only projection after safe migration; hosts reduce the recovery completion
/// and enqueue its persistence effect before entering their normal loop.
pub struct RuntimeHydration {
    pub jobs: Vec<CompletedJobSnapshot>,
    pub notices: Vec<String>,
    pub recovery_needs_persist: bool,
    pub job_list_mode: Option<harvester_core::JobListMode>,
    pub selected_article_url: Option<String>,
}

pub fn load_runtime_hydration(state_path: &Path, output: &Path) -> RuntimeHydration {
    let _guard = RUNTIME_IO.lock().expect("runtime persistence lock");
    let mut notices = Vec::new();
    if let Err(e) = migrate_with_notices(state_path, |_| Ok(()), |notice| notices.push(notice)) {
        engine_warn!("[runtime-state] startup migration failed: {}", e);
    }
    let settings = runtime_settings(state_path);
    let mut recovery_done = settings.fetch_time_recovery_done;
    let mut jobs = load_completed_jobs(state_path);
    if !recovery_done && jobs.iter().any(|job| job.fetched_utc.is_none()) {
        match harvester_engine::CorpusScanIndex::default().fetch_times(output) {
            Ok(times) => {
                recovery_done = true;
                let mut recovered = 0;
                for job in &mut jobs {
                    if job.fetched_utc.is_none() {
                        job.fetched_utc = times
                            .get(&harvester_engine::archive_url_key(&job.url))
                            .cloned();
                        recovered += usize::from(job.fetched_utc.is_some());
                    }
                }
                engine_info!(
                    "[runtime-state] fetch-time recovery path={} recovered={} remaining_hidden={}",
                    state_path.display(),
                    recovered,
                    jobs.iter().filter(|job| job.fetched_utc.is_none()).count()
                );
            }
            Err(e) => engine_warn!(
                "[runtime-state] fetch-time scan output={} error={}",
                output.display(),
                e
            ),
        }
    } else if !recovery_done {
        recovery_done = true;
    }
    RuntimeHydration {
        jobs,
        notices,
        recovery_needs_persist: recovery_done && !settings.fetch_time_recovery_done,
        job_list_mode: settings.job_list_mode,
        selected_article_url: settings.selected_article_url,
    }
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
        .map(|job| {
            let links = parsed_legacy_links(&job, state_path)
                .into_iter()
                .map(|link| LinkSnapshotRecord {
                    url: link.url,
                    downloaded_path: sanitize_downloaded_path(link.downloaded_path),
                })
                .collect();
            CompletedJobSnapshot {
                url: job.url,
                tokens: job.tokens,
                bytes: job.bytes,
                links,
                fetched_utc: job.fetched_utc,
            }
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
    persist_desktop_window_size_with_notices(state_path, width, height, |_| {});
}

pub(crate) fn persist_desktop_window_size_with_notices(
    state_path: &Path,
    width: i32,
    height: i32,
    notice: impl FnMut(String),
) {
    let _guard = RUNTIME_IO.lock().expect("runtime persistence lock");
    if !runtime_settings(state_path).links_in_store {
        if let Err(error) = migrate_with_notices(state_path, |_| Ok(()), notice) {
            engine_error!(
                "[runtime-state] geometry path={} migration failed: {}",
                state_path.display(),
                error
            );
            return;
        }
    }
    let content = fs::read_to_string(state_path).unwrap_or_default();
    let mut state: PersistedState = ron::from_str(&content).unwrap_or_default();
    state.links_in_store = true;
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
    if let Err(err) = atomic_runtime_write(state_path, &serialized) {
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

pub fn persist_completed_jobs(state_path: &Path, completed: &[SlimJobRecord]) {
    persist_runtime_state(state_path, completed);
}

pub fn persist_runtime_state(state_path: &Path, completed: &[SlimJobRecord]) {
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
    completed: &[SlimJobRecord],
) -> Result<(), String> {
    let _guard = RUNTIME_IO.lock().map_err(|e| e.to_string())?;
    let mut existing = runtime_settings(state_path);
    if !existing.links_in_store {
        migrate_with_interrupt(state_path, |_| Ok(()))?;
        existing = runtime_settings(state_path);
    }
    let pending_intake = existing.pending_intake.clone();
    persist_runtime_state_with_existing(state_path, completed, &pending_intake, existing)
}

/// Persist the completed-job projection and the reducer-owned pending-intake list.
pub fn try_persist_runtime_state_with_pending(
    state_path: &Path,
    completed: &[SlimJobRecord],
    pending_intake: &[String],
) -> Result<(), String> {
    persist_snapshot_with_notices(
        state_path,
        completed,
        pending_intake,
        false,
        None,
        None,
        |_| {},
    )
}

fn runtime_settings(state_path: &Path) -> PersistedRuntimeSettings {
    let started = std::time::Instant::now();
    let settings = fs::read_to_string(state_path)
        .ok()
        .and_then(|text| ron::from_str(&text).ok())
        .unwrap_or_default();
    engine_info!(
        "[runtime-state] operation=read-settings path={} elapsed_ms={}",
        state_path.display(),
        started.elapsed().as_millis()
    );
    settings
}

pub(crate) fn persist_snapshot_with_notices(
    state_path: &Path,
    completed: &[SlimJobRecord],
    pending_intake: &[String],
    fetch_time_recovery_done: bool,
    job_list_mode: Option<harvester_core::JobListMode>,
    selected_article_url: Option<&str>,
    notice: impl FnMut(String),
) -> Result<(), String> {
    let _guard = RUNTIME_IO.lock().map_err(|e| e.to_string())?;
    let output_dir = state_path.parent().unwrap_or_else(|| Path::new("."));
    ensure_output_dir(output_dir).map_err(|err| format!("ensure output dir: {err}"))?;

    // Read settings only: old completed jobs and downloaded paths are not carried forward.
    let mut existing = runtime_settings(state_path);

    if !existing.links_in_store {
        migrate_with_notices(state_path, |_| Ok(()), notice)?;
        existing = runtime_settings(state_path);
    }
    existing.fetch_time_recovery_done |= fetch_time_recovery_done;
    if job_list_mode.is_some() {
        existing.job_list_mode = job_list_mode;
        existing.selected_article_url = selected_article_url.map(str::to_owned);
    }
    persist_runtime_state_with_existing(state_path, completed, pending_intake, existing)
}

fn persist_runtime_state_with_existing(
    state_path: &Path,
    completed: &[SlimJobRecord],
    pending_intake: &[String],
    existing: PersistedRuntimeSettings,
) -> Result<(), String> {
    let output_dir = state_path.parent().unwrap_or_else(|| Path::new("."));
    ensure_output_dir(output_dir).map_err(|err| format!("ensure output dir: {err}"))?;

    let state = PersistedState {
        links_in_store: true,
        fetch_time_recovery_done: existing.fetch_time_recovery_done,
        job_list_mode: existing.job_list_mode,
        selected_article_url: existing.selected_article_url,
        completed: completed
            .iter()
            .map(|job| PersistedJob {
                url: job.url.clone(),
                tokens: job.tokens,
                bytes: job.bytes,
                links: vec![],
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
    let started = std::time::Instant::now();
    let content = ron::ser::to_string_pretty(&state, pretty)
        .map_err(|err| format!("serialize persisted state: {err}"))?;
    engine_info!(
        "[runtime-state] operation=serialize path={} jobs={} bytes={} elapsed_ms={}",
        state_path.display(),
        completed.len(),
        content.len(),
        started.elapsed().as_millis()
    );
    let started = std::time::Instant::now();
    atomic_runtime_write(state_path, &content)?;
    engine_info!(
        "[runtime-state] operation=publish path={} elapsed_ms={}",
        state_path.display(),
        started.elapsed().as_millis()
    );
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

    #[test]
    fn corpus_scale_saves_are_bounded_and_do_not_recover_or_read_backups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".harvester_state.ron");
        let jobs: Vec<_> = (0..12_000)
            .map(|i| SlimJobRecord {
                url: format!("https://example.com/article/{i}"),
                tokens: Some(123),
                bytes: Some(456),
                fetched_utc: Some("2026-10-01T00:00:00Z".into()),
            })
            .collect();
        try_persist_runtime_state_with_pending(&path, &jobs, &[]).unwrap();
        // These would be removed by migration recovery. Normal snapshots must
        // leave both namespaces alone, even with a large existing link store.
        let leftover = dir.path().join(format!("{SLIM_TEMP_PREFIX}sentinel.ron"));
        fs::write(&leftover, "retain").unwrap();
        let store = dir.path().join(".article_links");
        fs::create_dir(&store).unwrap();
        for i in 0..12_000 {
            fs::write(store.join(format!("{i}.json")), "[]").unwrap();
        }
        let partial = store.join(".partial-sentinel");
        fs::write(&partial, "retain").unwrap();
        let backup = dir.path().join(BACKUP_NAME);
        fs::write(&backup, "owner backup").unwrap();
        #[cfg(windows)]
        let _exclusive_backup = {
            use std::os::windows::fs::OpenOptionsExt;
            fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&backup)
                .unwrap()
        };
        let started = std::time::Instant::now();
        for _ in 0..2 {
            try_persist_runtime_state_with_pending(&path, &jobs, &[]).unwrap();
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "two slim saves exceeded the old single-save baseline: {:?}",
            started.elapsed()
        );
        assert!(leftover.exists());
        assert!(partial.exists());
        assert_eq!(load_completed_jobs(&path).len(), jobs.len());
    }
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

        try_persist_runtime_state_with_pending(&path, &slim(&jobs), &pending)
            .expect("persist snapshot");

        assert_eq!(load_pending_intake(&path), pending);
        assert_eq!(load_completed_jobs(&path), jobs);
    }

    #[test]
    fn link_store_roundtrips_links_and_runtime_state_stays_slim() {
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

        let links = snapshot[0]
            .links
            .iter()
            .map(|link| harvester_engine::ExtractedLink {
                url: link.url.clone(),
                text: None,
                kind: hyperlink_kind(),
            })
            .collect::<Vec<_>>();
        crate::write_article_links(temp.path(), &snapshot[0].url, &links).unwrap();
        persist_completed_jobs(&state_path(temp.path()), &slim(&snapshot));
        let loaded = load_completed_jobs(&state_path(temp.path()));

        assert_eq!(loaded[0].url, snapshot[0].url);
        assert!(loaded[0].links.is_empty());
        assert_eq!(
            crate::load_article_links(temp.path(), &snapshot[0].url),
            links
        );
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
        persist_completed_jobs(&path, &slim(&jobs));
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

        persist_completed_jobs(&path, &slim(&jobs));

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
        persist_runtime_state(&path, &slim(&snapshot));
        assert!(load_completed_jobs(&path)[0].links.is_empty());
        assert_eq!(
            crate::load_article_links(dir.path(), &snapshot[0].url)[0].url,
            snapshot[0].links[0].url
        );
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
        fs::write(&path, r#"(completed: "not a job collection", links_in_store: true, pending_intake: ["https://pending.example"], desktop_window_width: Some(960), desktop_window_height: Some(720))"#).unwrap();
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

        try_persist_runtime_state(&path, &slim(&completed)).unwrap();
        let loaded = load_completed_jobs(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].url, completed[0].url);
        assert!(loaded[0].links.is_empty());
        assert_eq!(load_pending_intake(&path), vec!["https://pending.example"]);
        assert_eq!(load_desktop_window_size(&path), Some((960, 720)));

        try_persist_runtime_state_with_pending(&path, &slim(&completed), &[]).unwrap();
        assert!(load_pending_intake(&path).is_empty());
        assert_eq!(load_desktop_window_size(&path), Some((960, 720)));
    }
}

#[cfg(test)]
fn slim(jobs: &[CompletedJobSnapshot]) -> Vec<SlimJobRecord> {
    jobs.iter()
        .map(|job| SlimJobRecord {
            url: job.url.clone(),
            tokens: job.tokens,
            bytes: job.bytes,
            fetched_utc: job.fetched_utc.clone(),
        })
        .collect()
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    const ORIGINAL: &str = r#"(completed: [
        (url: "https://example.com/a", tokens: Some(12), bytes: Some(240), fetched_utc: Some("2026-09-20T09:15:00Z"), links: [(url: "https://example.com/link-a", downloaded_path: Some("../../ignored.md"), text: Some("Read more"), kind: Hyperlink)]),
        (url: "https://example.com/b", tokens: None, bytes: None, links: [(url: "https://example.com/image", downloaded_path: None, kind: Image)])
    ], pending_intake: ["https://example.com/pending"], window_width: Some(1280), window_height: Some(800), desktop_window_width: Some(1512), desktop_window_height: Some(982))"#;

    fn original(dir: &Path) -> PathBuf {
        let path = dir.join(".harvester_state.ron");
        fs::write(&path, ORIGINAL).unwrap();
        path
    }

    fn assert_migrated(dir: &Path, path: &Path) {
        assert_eq!(
            fs::read(dir.join(BACKUP_NAME)).unwrap(),
            ORIGINAL.as_bytes()
        );
        let bytes = fs::read(path).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("links:"));
        assert!(!text.contains("downloaded_path"));
        let state: PersistedState = ron::de::from_bytes(&bytes).unwrap();
        assert!(state.links_in_store);
        assert_eq!(state.completed.len(), 2);
        assert_eq!(
            state.completed[0].fetched_utc.as_deref(),
            Some("2026-09-20T09:15:00Z")
        );
        assert_eq!(load_pending_intake(path), ["https://example.com/pending"]);
        assert_eq!(
            state.window_width.zip(state.window_height),
            Some((1280, 800))
        );
        assert_eq!(load_desktop_window_size(path), Some((1512, 982)));
        let a = crate::load_article_links(dir, "https://example.com/a");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].text.as_deref(), Some("Read more"));
        assert_eq!(a[0].url, "https://example.com/link-a");
        let b = crate::load_article_links(dir, "https://example.com/b");
        assert_eq!(b[0].kind, harvester_engine::LinkKind::Image);
        assert_eq!(b[0].url, "https://example.com/image");
    }

    #[test]
    fn old_state_migrates_with_identical_backup_and_older_reader_can_load_slim_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = original(dir.path());
        assert_eq!(load_completed_jobs(&path).len(), 2);
        assert!(migrate_runtime_state(&path).unwrap());
        assert_migrated(dir.path(), &path);
        #[derive(Deserialize)]
        struct OldState {
            completed: Vec<OldJob>,
        }
        #[derive(Deserialize)]
        struct OldJob {
            url: String,
            #[serde(default)]
            links: Vec<LinkSnapshotRecord>,
        }
        let old: OldState = ron::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(old.completed.len(), 2);
        assert_eq!(old.completed[0].url, "https://example.com/a");
        assert!(old.completed.iter().all(|job| job.links.is_empty()));
    }

    #[test]
    fn second_start_does_not_migrate_or_rewrite_backup_or_link_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = original(dir.path());
        migrate_runtime_state(&path).unwrap();
        let link = crate::article_links::link_path(dir.path(), "https://example.com/a").unwrap();
        let before = fs::metadata(&link).unwrap().modified().unwrap();
        let active = fs::read(&path).unwrap();
        assert!(!migrate_runtime_state(&path).unwrap());
        assert_eq!(fs::read(&path).unwrap(), active);
        assert_eq!(fs::metadata(link).unwrap().modified().unwrap(), before);
        assert_migrated(dir.path(), &path);
    }

    fn interrupted(point: MigrationPoint) {
        let dir = tempfile::tempdir().unwrap();
        let path = original(dir.path());
        let result = migrate_with_interrupt(&path, |step| {
            if step == point {
                Err(format!("simulated interruption at {step:?}"))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read(&path).unwrap(),
            ORIGINAL.as_bytes(),
            "active stays original until publication"
        );
        if point != MigrationPoint::BackupPartial {
            assert_eq!(
                fs::read(dir.path().join(BACKUP_NAME)).unwrap(),
                ORIGINAL.as_bytes()
            );
        }
        assert!(migrate_runtime_state(&path).unwrap());
        assert_migrated(dir.path(), &path);
        assert!(!migrate_runtime_state(&path).unwrap());
    }

    #[test]
    fn restart_after_interrupted_backup_copy() {
        interrupted(MigrationPoint::BackupPartial);
    }
    #[test]
    fn restart_after_backup_before_links() {
        interrupted(MigrationPoint::BackupPublished);
    }
    #[test]
    fn restart_after_partial_link_publication() {
        interrupted(MigrationPoint::LinkPublished);
        let dir = tempfile::tempdir().unwrap();
        let path = original(dir.path());
        assert!(
            migrate_with_interrupt(&path, |step| if step == MigrationPoint::BackupPublished {
                Err("interrupted".into())
            } else {
                Ok(())
            })
            .is_err()
        );
        fs::create_dir_all(dir.path().join(".article_links")).unwrap();
        let torn = dir.path().join(".article_links/.partial-torn");
        fs::write(&torn, "[{\"url\":").unwrap();
        migrate_runtime_state(&path).unwrap();
        assert!(!torn.exists());
        assert_migrated(dir.path(), &path);
    }
    #[test]
    fn restart_after_interrupted_slim_write() {
        interrupted(MigrationPoint::SlimPartial);
    }
    #[test]
    fn restart_after_slim_verification_before_replace() {
        interrupted(MigrationPoint::SlimVerified);
    }

    #[test]
    fn existing_verified_backup_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = original(dir.path());
        let backup = dir.path().join(BACKUP_NAME);
        fs::write(&backup, ORIGINAL).unwrap();
        let modified = fs::metadata(&backup).unwrap().modified().unwrap();
        migrate_runtime_state(&path).unwrap();
        assert_eq!(fs::metadata(backup).unwrap().modified().unwrap(), modified);
        assert_migrated(dir.path(), &path);
        let race_dir = tempfile::tempdir().unwrap();
        let race_path = original(race_dir.path());
        let race_backup = race_dir.path().join(BACKUP_NAME);
        assert!(migrate_with_interrupt(&race_path, |step| {
            if step == MigrationPoint::BackupPartial {
                fs::write(&race_backup, ORIGINAL).unwrap();
            }
            Ok(())
        })
        .is_err());
        assert_eq!(fs::read(&race_path).unwrap(), ORIGINAL.as_bytes());
        let published = fs::metadata(&race_backup).unwrap().modified().unwrap();
        migrate_runtime_state(&race_path).unwrap();
        assert_eq!(
            fs::metadata(race_backup).unwrap().modified().unwrap(),
            published
        );
        assert_migrated(race_dir.path(), &race_path);
    }

    #[test]
    fn mismatched_backup_preserves_both_files_and_allows_runtime_save_with_notice() {
        let dir = tempfile::tempdir().unwrap();
        let path = original(dir.path());
        fs::write(dir.path().join(BACKUP_NAME), "owner backup").unwrap();
        let mut notices = vec![];
        assert!(migrate_with_notices(&path, |_| Ok(()), |message| notices.push(message)).unwrap());
        let dated = dated_backups(dir.path());
        assert_eq!(dated.len(), 1);
        assert_eq!(fs::read(&dated[0]).unwrap(), ORIGINAL.as_bytes());
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains(dated[0].file_name().unwrap().to_str().unwrap()));
        let jobs = slim(&load_completed_jobs(&path));
        try_persist_runtime_state(&path, &jobs).unwrap();
        try_persist_runtime_state_with_pending(&path, &jobs, &[]).unwrap();
        persist_desktop_window_size(&path, 900, 700);
        assert_eq!(load_completed_jobs(&path).len(), 2);
        assert_eq!(
            fs::read(dir.path().join(BACKUP_NAME)).unwrap(),
            b"owner backup"
        );
    }

    fn dated_backups(output: &Path) -> Vec<PathBuf> {
        fs::read_dir(output)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                let name = path.file_name().unwrap().to_str().unwrap();
                name.starts_with(".harvester_state.pre-slim-")
                    && !name.starts_with(BACKUP_TEMP_PREFIX)
            })
            .collect()
    }

    #[test]
    fn unparseable_original_is_backed_up_before_saves_resume() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".harvester_state.ron");
        let corrupt = b"(completed: [\xff truncated";
        fs::write(&path, corrupt).unwrap();
        let hydration = load_runtime_hydration(&path, dir.path());
        let dated = dated_backups(dir.path());
        assert_eq!(dated.len(), 1);
        assert_eq!(fs::read(&dated[0]).unwrap(), corrupt);
        assert_eq!(hydration.notices.len(), 1);
        assert!(hydration.jobs.is_empty());
        assert!(hydration.notices[0].contains(dated[0].file_name().unwrap().to_str().unwrap()));
        try_persist_runtime_state(&path, &[]).unwrap();
        try_persist_runtime_state_with_pending(&path, &[], &["https://pending.example".into()])
            .unwrap();
        assert_eq!(load_pending_intake(&path), ["https://pending.example"]);
        // A second corrupt restore gets another backup; all earlier bytes survive.
        fs::write(&path, b"another truncated restore").unwrap();
        try_persist_runtime_state(&path, &[]).unwrap();
        assert_eq!(dated_backups(dir.path()).len(), 2);
        assert_eq!(fs::read(&dated[0]).unwrap(), corrupt);
    }

    #[test]
    fn remigration_skips_empty_lists_and_merges_restored_links() {
        let dir = tempfile::tempdir().unwrap();
        let path = original(dir.path());
        migrate_runtime_state(&path).unwrap();
        let a_path = crate::article_links::link_path(dir.path(), "https://example.com/a").unwrap();
        let before = fs::read(&a_path).unwrap();
        fs::write(&path, r#"(completed: [(url: "https://example.com/a", tokens: None, bytes: None, links: []), (url: "https://example.com/empty", tokens: None, bytes: None)])"#).unwrap();
        migrate_runtime_state(&path).unwrap();
        assert_eq!(fs::read(&a_path).unwrap(), before);
        assert_eq!(
            fs::read_dir(dir.path().join(".article_links"))
                .unwrap()
                .count(),
            2
        );
        fs::write(&path, r#"(completed: [(url: "https://example.com/a", tokens: None, bytes: None, links: [(url: "https://example.com/new", downloaded_path: None)])])"#).unwrap();
        migrate_runtime_state(&path).unwrap();
        let links = crate::load_article_links(dir.path(), "https://example.com/a");
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].text.as_deref(), Some("Read more"));
        assert_eq!(
            fs::read(dir.path().join(BACKUP_NAME)).unwrap(),
            ORIGINAL.as_bytes()
        );
    }

    #[test]
    fn geometry_save_on_slim_state_does_not_run_migration_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = original(dir.path());
        migrate_runtime_state(&path).unwrap();
        let sentinel = dir.path().join(format!("{BACKUP_TEMP_PREFIX}sentinel.ron"));
        fs::write(&sentinel, "sentinel").unwrap();
        persist_desktop_window_size(&path, 900, 700);
        assert_eq!(fs::read_to_string(sentinel).unwrap(), "sentinel");
        assert_eq!(load_desktop_window_size(&path), Some((900, 700)));
    }

    #[test]
    fn pinned_leftovers_match_marker_and_are_removed_on_old_and_slim_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = original(dir.path());
        let marker = harvester_engine::build_corpus_manifest("2026-10-01T00:00:00Z");
        for slim_active in [false, true] {
            let leftovers = [BACKUP_TEMP_PREFIX, SLIM_TEMP_PREFIX]
                .map(|prefix| pinned_temp(dir.path(), prefix));
            for temp in &leftovers {
                let name = temp.file_name().unwrap().to_str().unwrap();
                assert!(marker["layout"]["internal_state"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|v| v.as_str())
                    .any(
                        |pattern| pattern
                            .split_once('*')
                            .is_some_and(|(prefix, suffix)| name.starts_with(prefix)
                                && name.ends_with(suffix))
                    ));
                fs::write(temp, "torn temporary").unwrap();
            }
            assert_eq!(migrate_runtime_state(&path).unwrap(), !slim_active);
            assert!(leftovers.iter().all(|temp| !temp.exists()));
            assert_migrated(dir.path(), &path);
        }
    }
}
