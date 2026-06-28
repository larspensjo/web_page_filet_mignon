use std::io;
use std::path::{Path, PathBuf};

use engine_logging::{engine_info, engine_warn};
use harvester_core::blacklist::BlacklistState;
use harvester_engine::{ensure_output_dir, AtomicFileWriter};

pub fn default_blacklist_path(output_dir: &Path) -> PathBuf {
    output_dir.join(".domain_blacklist.ron")
}

pub fn load_blacklist(path: &Path) -> BlacklistState {
    let content = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return BlacklistState::default();
        }
        Err(err) => {
            engine_warn!("[blacklist] failed to read {:?}: {}", path, err);
            return BlacklistState::default();
        }
    };

    match ron::from_str(&content) {
        Ok(state) => {
            engine_info!("[blacklist] loaded from {:?}", path);
            state
        }
        Err(err) => {
            engine_warn!("[blacklist] failed to parse {:?}: {}", path, err);
            BlacklistState::default()
        }
    }
}

pub fn save_blacklist(path: &Path, state: &BlacklistState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_output_dir(parent).map_err(io::Error::other)?;
    }
    let ron_text = ron::ser::to_string_pretty(state, ron::ser::PrettyConfig::default())
        .map_err(|err| io::Error::other(err.to_string()))?;
    let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(".domain_blacklist.ron");
    AtomicFileWriter::new(PathBuf::from(parent_dir))
        .write(filename, &ron_text)
        .map_err(|err| io::Error::other(err.to_string()))?;
    engine_info!("[blacklist] saved to {:?}", path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::blacklist::BlacklistState;
    use harvester_engine::FetchOutcomeClass;

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = default_blacklist_path(dir.path());
        let mut state = BlacklistState::default();
        for _ in 0..3 {
            state.record_outcome(
                "bloomberg.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 403"),
                chrono::Utc::now(),
            );
        }
        save_blacklist(&path, &state).unwrap();
        let loaded = load_blacklist(&path);
        assert_eq!(loaded, state);
    }

    #[test]
    fn missing_file_yields_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = default_blacklist_path(dir.path());
        assert!(load_blacklist(&path).is_empty());
    }
}
