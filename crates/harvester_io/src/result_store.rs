//! Append-only paid-result files and atomic publication of legacy migrations.
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::hash::Hash;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use engine_logging::{engine_info, engine_warn};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

// Readers, tail recovery, migration and appends share this in-process lock.
// A reader in this host cannot mistake its own append for a torn tail. Separate
// hosts still need the folder-level lock planned for later work.
static STORE_IO: Mutex<()> = Mutex::new(());

pub fn jsonl_path(path: &Path) -> PathBuf {
    path.with_extension("jsonl")
}

pub(crate) fn error(path: &Path, operation: &str, reason: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("{}: {operation}: {reason}", path.display()))
}

#[derive(Deserialize)]
pub(crate) struct LegacyFile<K, E> {
    #[serde(default = "legacy_version")]
    pub version: u32,
    pub entries: Vec<(K, E)>,
}

fn legacy_version() -> u32 {
    1
}

/// The legacy DTOs use strings for enums. JSON transcodes those strings to the
/// same domain enums without changing any key dimension or entry meaning.
pub(crate) fn read_ron<
    K: DeserializeOwned,
    E: DeserializeOwned,
    DK: DeserializeOwned + Serialize,
    DE: DeserializeOwned + Serialize,
>(
    path: &Path,
) -> io::Result<Vec<(K, E)>> {
    let bytes = fs::read(path).map_err(|e| error(path, "read RON", e))?;
    let file: LegacyFile<DK, DE> =
        ron::de::from_bytes(&bytes).map_err(|e| error(path, "parse RON", e))?;
    if file.version != 1 {
        return Err(error(
            path,
            "read RON",
            format!("unknown format version {}", file.version),
        ));
    }
    let mut records = Vec::new();
    let mut skipped = 0;
    for (key, entry) in file.entries {
        let key_value = serde_json::to_value(key).map_err(|e| error(path, "decode RON key", e))?;
        let entry_value =
            serde_json::to_value(entry).map_err(|e| error(path, "decode RON entry", e))?;
        let key = match serde_json::from_value(key_value.clone()) {
            Ok(key) => key,
            Err(e) => {
                let unknown_prompt = key_value
                    .get("prompt_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| {
                        serde_json::from_value::<harvester_engine::llm::PromptId>(
                            serde_json::Value::String(id.to_owned()),
                        )
                        .is_err()
                    });
                if !unknown_prompt {
                    return Err(error(path, "decode RON key", e));
                }
                skipped += 1;
                continue;
            }
        };
        let entry =
            serde_json::from_value(entry_value).map_err(|e| error(path, "decode RON entry", e))?;
        records.push((key, entry));
    }
    if skipped > 0 {
        engine_warn!(
            "[results] path={} skipped {} entries with unknown prompt id during RON migration",
            path.display(),
            skipped
        );
    }
    Ok(records)
}

fn artifact_path(path: &Path, operation: &str) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(
        "{stem}.{operation}-{}.jsonl",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%.9f")
    ))
}

fn recover_tail(path: &Path, bytes: &[u8]) -> io::Result<usize> {
    if bytes.is_empty() || bytes.last() == Some(&b'\n') {
        return Ok(bytes.len());
    }
    let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let sidecar = artifact_path(path, "torn");
    let mut backup = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&sidecar)
        .map_err(|e| error(&sidecar, "create torn-tail sidecar", e))?;
    backup
        .write_all(&bytes[complete..])
        .and_then(|()| backup.sync_all())
        .map_err(|e| error(&sidecar, "save torn tail", e))?;
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| error(path, "open for tail recovery", e))?;
    file.set_len(complete as u64)
        .and_then(|()| file.sync_all())
        .map_err(|e| error(path, "truncate torn tail", e))?;
    engine_warn!(
        "[results] store={} path={} recovered {} torn bytes to {}",
        path.file_stem().unwrap_or_default().to_string_lossy(),
        path.display(),
        bytes.len() - complete,
        sidecar.display()
    );
    Ok(complete)
}

fn transcode<T: Serialize, U: DeserializeOwned>(value: T) -> serde_json::Result<U> {
    serde_json::from_value(serde_json::to_value(value)?)
}

fn parse_lines<
    K: DeserializeOwned,
    E: DeserializeOwned,
    DK: DeserializeOwned + Serialize,
    DE: DeserializeOwned + Serialize,
>(
    path: &Path,
    bytes: &[u8],
) -> (Vec<(K, E)>, usize) {
    let mut records = Vec::new();
    let mut skipped = 0;
    for (index, line) in bytes.split_inclusive(|b| *b == b'\n').enumerate() {
        let parsed = serde_json::from_slice::<(DK, DE)>(line).and_then(transcode::<_, (K, E)>);
        match parsed {
            Ok(record) => records.push(record),
            Err(e) => {
                skipped += 1;
                engine_warn!(
                    "[results] store={} path={} skipped line {}: {}",
                    path.file_stem().unwrap_or_default().to_string_lossy(),
                    path.display(),
                    index + 1,
                    e
                );
            }
        }
    }
    if skipped > 0 {
        engine_warn!(
            "[results] store={} path={} skipped {} malformed complete lines; loaded {} records",
            path.file_stem().unwrap_or_default().to_string_lossy(),
            path.display(),
            skipped,
            records.len()
        );
    }
    (records, skipped)
}

fn write_records<
    K: Serialize,
    E: Serialize,
    DK: DeserializeOwned + Serialize,
    DE: DeserializeOwned + Serialize,
>(
    file: &mut File,
    records: &[(K, E)],
) -> io::Result<()> {
    let mut bytes = Vec::new();
    for record in records {
        let persisted: (DK, DE) = transcode(record)?;
        serde_json::to_writer(&mut bytes, &persisted)?;
        bytes.push(b'\n');
    }
    file.write_all(&bytes)?;
    file.sync_all()
}

pub(crate) fn load<
    K: DeserializeOwned + Serialize + Eq + Hash,
    E: DeserializeOwned + Serialize,
    DK: DeserializeOwned + Serialize,
    DE: DeserializeOwned + Serialize,
>(
    path: &Path,
    legacy: impl FnOnce(&Path) -> io::Result<Vec<(K, E)>>,
) -> io::Result<Vec<(K, E)>> {
    let _guard = STORE_IO.lock().unwrap_or_else(|e| e.into_inner());
    load_locked::<K, E, DK, DE>(path, legacy)
}

fn load_locked<
    K: DeserializeOwned + Serialize + Eq + Hash,
    E: DeserializeOwned + Serialize,
    DK: DeserializeOwned + Serialize,
    DE: DeserializeOwned + Serialize,
>(
    path: &Path,
    legacy: impl FnOnce(&Path) -> io::Result<Vec<(K, E)>>,
) -> io::Result<Vec<(K, E)>> {
    let path = jsonl_path(path);
    match fs::read(&path) {
        Ok(bytes) => {
            let complete = recover_tail(&path, &bytes)?;
            return Ok(parse_lines::<K, E, DK, DE>(&path, &bytes[..complete]).0);
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(error(&path, "read JSONL", e)),
    }
    let ron_path = path.with_extension("ron");
    match fs::metadata(&ron_path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(error(&ron_path, "inspect RON", e)),
        Ok(_) => {}
    }
    // Validate the source before any writes, including deleting stale migration files.
    let records = legacy(&ron_path)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let prefix = format!(
        "{}.migrating-",
        path.file_stem().unwrap_or_default().to_string_lossy()
    );
    for entry in fs::read_dir(parent).map_err(|e| error(parent, "find stale migrations", e))? {
        let entry = entry.map_err(|e| error(parent, "read migration directory", e))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".jsonl") {
            fs::remove_file(entry.path())
                .map_err(|e| error(&entry.path(), "delete stale migration", e))?;
            engine_info!(
                "[results] removed stale migration {}",
                entry.path().display()
            );
        }
    }
    let temp = artifact_path(&path, "migrating");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| error(&temp, "create migration", e))?;
    write_records::<K, E, DK, DE>(&mut file, &records)
        .map_err(|e| error(&temp, "write migration", e))?;
    drop(file);
    let bytes = fs::read(&temp).map_err(|e| error(&temp, "verify migration", e))?;
    let (verified, skipped): (Vec<(K, E)>, _) = parse_lines::<K, E, DK, DE>(&temp, &bytes);
    let key_count = records
        .iter()
        .map(|(key, _)| key)
        .collect::<HashSet<_>>()
        .len();
    let verified_count = verified
        .iter()
        .map(|(key, _)| key)
        .collect::<HashSet<_>>()
        .len();
    if skipped != 0
        || verified_count != key_count
        || verified.len() != records.len()
        || (!bytes.is_empty() && bytes.last() != Some(&b'\n'))
    {
        return Err(error(
            &temp,
            "verify migration",
            "record/key count mismatch",
        ));
    }
    fs::rename(&temp, &path).map_err(|e| error(&path, "publish migration", e))?;
    engine_info!(
        "[results] migrated {} -> {} records={} keys={}; RON backup untouched",
        ron_path.display(),
        path.display(),
        records.len(),
        key_count
    );
    Ok(records)
}

/// One writer per store, held by the ordered result sink. Opening validates and
/// recovers the file; subsequent appends cost only the newly arrived records.
pub(crate) struct AppendFile {
    path: PathBuf,
    file: File,
}
impl AppendFile {
    pub fn open<
        K: DeserializeOwned + Serialize + Eq + Hash,
        E: DeserializeOwned + Serialize,
        DK: DeserializeOwned + Serialize,
        DE: DeserializeOwned + Serialize,
    >(
        path: &Path,
        legacy: impl FnOnce(&Path) -> io::Result<Vec<(K, E)>>,
    ) -> io::Result<Self> {
        let _guard = STORE_IO.lock().unwrap_or_else(|e| e.into_inner());
        load_locked::<K, E, DK, DE>(path, legacy)?;
        let path = jsonl_path(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| error(parent, "create store directory", e))?;
        }
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .map_err(|e| error(&path, "open append", e))?;
        Ok(Self { path, file })
    }
    pub fn append<
        K: Serialize,
        E: Serialize,
        DK: DeserializeOwned + Serialize,
        DE: DeserializeOwned + Serialize,
    >(
        &mut self,
        records: &[(K, E)],
    ) -> io::Result<()> {
        let _guard = STORE_IO.lock().unwrap_or_else(|e| e.into_inner());
        write_records::<K, E, DK, DE>(&mut self.file, records)
            .map_err(|e| error(&self.path, "append results", e))
    }
}

#[cfg(test)]
mod tests;
