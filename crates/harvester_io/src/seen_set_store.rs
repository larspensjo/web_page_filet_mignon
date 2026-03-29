use engine_logging::{engine_info, engine_warn};
use harvester_engine::{AtomicFileWriter, BraveNewsItem, BraveSeenSet, RssSeenSet, SourceId};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};
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

/// Load the Brave seen-set from disk. Missing/corrupt files yield an empty set.
pub fn load_brave_seen_set(path: &Path) -> BraveSeenSet {
    match fs::read_to_string(path) {
        Ok(contents) => match ron::from_str::<BraveSeenSet>(&contents) {
            Ok(mut set) => {
                set.rebuild_lookup();
                engine_info!("[brave-seen] loaded seen set from {:?}", path);
                set
            }
            Err(err) => {
                engine_warn!("[brave-seen] failed to parse {:?}: {}", path, err);
                BraveSeenSet::new()
            }
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => BraveSeenSet::new(),
        Err(err) => {
            engine_warn!("[brave-seen] failed to read {:?}: {}", path, err);
            BraveSeenSet::new()
        }
    }
}

/// Save the Brave seen-set atomically.
pub fn persist_brave_seen_set(set: &BraveSeenSet, path: &Path) -> io::Result<()> {
    let pretty = PrettyConfig::new();
    let content = ron::ser::to_string_pretty(set, pretty)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid brave seen set path")
        })?;

    let writer = AtomicFileWriter::new(dir.to_path_buf());
    writer
        .write(file_name, &content)
        .map(|_| engine_info!("[brave-seen] saved seen set to {:?}", path))
        .map_err(io::Error::other)
}

/// Persist Brave metadata for emitted items (title, description, age) as a sidecar.
/// Appends to existing entries, bounded to the last 5 000.
pub fn persist_brave_metadata(
    items: &[&BraveNewsItem],
    source_id: &SourceId,
    path: &Path,
) -> io::Result<()> {
    if items.is_empty() {
        return Ok(());
    }

    let mut entries: Vec<BraveMetadataEntry> = match fs::read_to_string(path) {
        Ok(contents) => ron::from_str(&contents).unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    for item in items {
        entries.push(BraveMetadataEntry {
            url: item.url.clone(),
            title: item.title.clone(),
            description: item.description.clone(),
            age: item.age.clone(),
            source_id: source_id.clone(),
        });
    }

    const MAX_METADATA_ENTRIES: usize = 5_000;
    if entries.len() > MAX_METADATA_ENTRIES {
        entries.drain(..entries.len() - MAX_METADATA_ENTRIES);
    }

    let pretty = PrettyConfig::new();
    let content = ron::ser::to_string_pretty(&entries, pretty)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid metadata path"))?;

    let writer = AtomicFileWriter::new(dir.to_path_buf());
    writer
        .write(file_name, &content)
        .map(|_| ())
        .map_err(io::Error::other)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BraveMetadataEntry {
    pub url: String,
    pub title: String,
    pub description: String,
    pub age: Option<String>,
    pub source_id: SourceId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_logging::initialize_for_tests;
    use harvester_engine::{BraveSeenSet, FeedEntry};
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
    fn brave_seen_set_save_and_load_roundtrips() {
        init_logging();
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("brave_seen.ron");

        let mut set = BraveSeenSet::new();
        set.mark_seen("https://example.com/article-1");
        set.mark_seen("https://example.com/article-2");

        persist_brave_seen_set(&set, &path).expect("save");
        let loaded = load_brave_seen_set(&path);
        assert!(loaded.is_seen("https://example.com/article-1"));
        assert!(loaded.is_seen("https://example.com/article-2"));
        assert!(!loaded.is_seen("https://example.com/unseen"));
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
