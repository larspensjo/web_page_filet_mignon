use engine_logging::{engine_info, engine_warn};
use harvester_engine::{AtomicFileWriter, RssSeenSet};
use ron::ser::PrettyConfig;
use std::fs;
use std::io;
use std::path::Path;

/// Load the seen-set from disk. Missing/corrupt files yield an empty set.
pub fn load_seen_set(path: &Path) -> RssSeenSet {
    match fs::read_to_string(path) {
        Ok(contents) => match ron::from_str::<RssSeenSet>(&contents) {
            Ok(set) => {
                engine_info!("[rss-seen] loaded seen set from {:?}", path);
                set
            }
            Err(err) => {
                engine_warn!("[rss-seen] failed to parse {:?}: {}", path, err);
                RssSeenSet::new()
            }
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => RssSeenSet::new(),
        Err(err) => {
            engine_warn!("[rss-seen] failed to read {:?}: {}", path, err);
            RssSeenSet::new()
        }
    }
}

/// Save the seen-set atomically. Returns IO error on failure.
pub fn persist_seen_set(set: &RssSeenSet, path: &Path) -> io::Result<()> {
    let pretty = PrettyConfig::new();
    let content = ron::ser::to_string_pretty(set, pretty)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid seen set path"))?;

    let writer = AtomicFileWriter::new(dir.to_path_buf());
    writer
        .write(file_name, &content)
        .map(|_| {
            engine_info!("[rss-seen] saved seen set to {:?}", path);
        })
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_logging::initialize_for_tests;
    use harvester_engine::FeedEntry;
    use std::fs;
    use tempfile::TempDir;

    fn init_logging() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(initialize_for_tests);
    }

    #[test]
    fn load_missing_file_returns_empty_set() {
        init_logging();
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("missing.ron");
        let set = load_seen_set(&path);
        assert!(!set.is_seen("source", "guid"));
    }

    #[test]
    fn load_corrupt_file_returns_empty_set() {
        init_logging();
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("corrupt.ron");
        fs::write(&path, "not ron").expect("write corrupt");
        let set = load_seen_set(&path);
        assert!(!set.is_seen("source", "guid"));
    }

    #[test]
    fn save_and_load_roundtrips() {
        init_logging();
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("seen.ron");

        let mut set = RssSeenSet::new();
        let entries = vec![
            FeedEntry {
                guid: "guid-1".to_string(),
                url: Some("https://example.com/a".to_string()),
                title: None,
                published: None,
            },
            FeedEntry {
                guid: "guid-2".to_string(),
                url: Some("https://example.com/b".to_string()),
                title: None,
                published: None,
            },
        ];
        let _ = set.filter_unseen_entries("source", entries);

        persist_seen_set(&set, &path).expect("save");
        let loaded = load_seen_set(&path);
        assert_eq!(set, loaded);
    }

    #[test]
    fn save_failure_reports_error_when_parent_not_dir() {
        init_logging();
        let temp = TempDir::new().expect("tempdir");
        let parent = temp.path().join("notadir");
        fs::write(&parent, "should be directory").expect("write file");
        let path = parent.join("seen.ron");
        let set = RssSeenSet::new();

        let err = persist_seen_set(&set, &path).unwrap_err();
        assert!(matches!(err.kind(), io::ErrorKind::Other));
    }
}
