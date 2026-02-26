use std::fs;
use std::path::{Path, PathBuf};

use engine_logging::{engine_debug, engine_info, engine_warn};
use harvester_core::entity_index::{EntityIndex, EntityIndexEntry};
use harvester_core::SummaryEntities;
use harvester_engine::{ensure_output_dir, AtomicFileWriter};

/// A partial update to be merged into an `EntityIndexEntry`.
/// `None` fields are preserved; `Some` fields overwrite.
#[derive(Debug, Clone)]
pub struct EntityIndexPatch {
    pub fetched_utc: Option<String>,
    pub content_hash: Option<String>,
    pub summary_entities: Option<SummaryEntities>,
    pub themes: Option<Vec<String>>,
}

/// Load the entity index from disk.
/// Returns `EntityIndex::default()` if the file does not exist or cannot be parsed.
pub fn load_entity_index(path: &Path) -> EntityIndex {
    let content = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            engine_info!("[entity-index] No persisted index found at {:?}", path);
            return EntityIndex::default();
        }
        Err(err) => {
            engine_warn!(
                "[entity-index] Failed to read index from {:?}: {}",
                path,
                err
            );
            return EntityIndex::default();
        }
    };

    match ron::from_str::<EntityIndex>(&content) {
        Ok(index) => {
            engine_info!(
                "[entity-index] Loaded {} entries from {:?}",
                index.entries.len(),
                path
            );
            index
        }
        Err(err) => {
            engine_warn!(
                "[entity-index] Failed to parse index from {:?}: {}",
                path,
                err
            );
            EntityIndex::default()
        }
    }
}

/// Save the entity index to disk atomically.
pub fn save_entity_index(
    path: &Path,
    index: &EntityIndex,
) -> Result<(), harvester_engine::PersistError> {
    if let Some(parent) = path.parent() {
        ensure_output_dir(parent)?;
    }

    let serialized = ron::ser::to_string_pretty(index, ron::ser::PrettyConfig::default())
        .map_err(|err| std::io::Error::other(format!("Failed to serialize entity index: {err}")))?;

    let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let writer = AtomicFileWriter::new(PathBuf::from(parent_dir));
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".entity_index.ron");
    writer.write(filename, &serialized)?;

    engine_debug!(
        "[entity-index] Saved {} entries to {:?}",
        index.entries.len(),
        path
    );
    Ok(())
}

/// Merge a patch into the entity index entry for `url`.
///
/// - If no entry exists for `url`, one is created with the patch fields.
/// - `None` patch fields leave the existing entry fields unchanged.
/// - `Some` patch fields overwrite the existing entry fields.
/// - Entity lists from `summary_entities` are deduplicated before storing.
pub fn upsert_entry(index: &mut EntityIndex, url: &str, patch: EntityIndexPatch) {
    let entry = index.entries.entry(url.to_string()).or_default();

    if let Some(ts) = patch.fetched_utc {
        entry.fetched_utc = Some(ts);
    }
    if let Some(hash) = patch.content_hash {
        entry.content_hash = Some(hash);
    }
    if let Some(entities) = patch.summary_entities {
        entry.companies = dedup_strings(entities.companies);
        entry.technologies = dedup_strings(entities.technologies);
        entry.products = dedup_strings(entities.products);
    }
    if let Some(themes) = patch.themes {
        entry.themes = dedup_strings(themes);
    }
}

/// Deduplicate a list of strings case-insensitively, preserving first occurrence order.
fn dedup_strings(items: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut result: Vec<String> = Vec::new();
    for item in items {
        let lower = item.to_lowercase();
        if !seen.contains(&lower) {
            seen.push(lower);
            result.push(item);
        }
    }
    result
}

/// Create an empty `EntityIndexEntry` for use in tests or initialization.
#[allow(dead_code)]
pub(crate) fn empty_entry() -> EntityIndexEntry {
    EntityIndexEntry::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn make_patch_entities(
        companies: &[&str],
        technologies: &[&str],
        products: &[&str],
    ) -> EntityIndexPatch {
        EntityIndexPatch {
            fetched_utc: None,
            content_hash: None,
            summary_entities: Some(SummaryEntities {
                companies: companies.iter().map(|s| s.to_string()).collect(),
                technologies: technologies.iter().map(|s| s.to_string()).collect(),
                products: products.iter().map(|s| s.to_string()).collect(),
            }),
            themes: None,
        }
    }

    fn make_patch_themes(themes: &[&str]) -> EntityIndexPatch {
        EntityIndexPatch {
            fetched_utc: None,
            content_hash: None,
            summary_entities: None,
            themes: Some(themes.iter().map(|s| s.to_string()).collect()),
        }
    }

    // ── upsert_entry tests ───────────────────────────────────────────────────

    #[test]
    fn upsert_entry_partial_summary_preserves_existing_themes() {
        let mut index = EntityIndex::default();
        let url = "https://example.com/article";

        // First: add themes via triage patch.
        upsert_entry(&mut index, url, make_patch_themes(&["AI", "enterprise"]));
        // Then: add entities via summary patch.
        upsert_entry(&mut index, url, make_patch_entities(&["Nvidia"], &[], &[]));

        let entry = &index.entries[url];
        assert_eq!(entry.themes, vec!["AI", "enterprise"], "themes preserved");
        assert_eq!(entry.companies, vec!["Nvidia"], "companies added");
    }

    #[test]
    fn upsert_entry_partial_themes_preserves_existing_entities() {
        let mut index = EntityIndex::default();
        let url = "https://example.com/article";

        // First: add entities via summary patch.
        upsert_entry(
            &mut index,
            url,
            make_patch_entities(&["Nvidia"], &[], &["H100"]),
        );
        // Then: add themes via triage patch.
        upsert_entry(&mut index, url, make_patch_themes(&["AI"]));

        let entry = &index.entries[url];
        assert_eq!(entry.companies, vec!["Nvidia"], "companies preserved");
        assert_eq!(entry.products, vec!["H100"], "products preserved");
        assert_eq!(entry.themes, vec!["AI"], "themes added");
    }

    #[test]
    fn upsert_entry_idempotent_same_patch_twice() {
        let mut index = EntityIndex::default();
        let url = "https://example.com/article";

        let patch = make_patch_entities(&["Nvidia", "TSMC"], &["custom silicon"], &["H100"]);
        upsert_entry(&mut index, url, patch.clone());
        upsert_entry(&mut index, url, patch);

        let entry = &index.entries[url];
        assert_eq!(entry.companies, vec!["Nvidia", "TSMC"]);
        assert_eq!(entry.technologies, vec!["custom silicon"]);
        assert_eq!(entry.products, vec!["H100"]);
    }

    #[test]
    fn upsert_entry_deduplicates_company_names_in_patch() {
        let mut index = EntityIndex::default();
        let url = "https://example.com/article";

        upsert_entry(
            &mut index,
            url,
            make_patch_entities(&["Nvidia", "nvidia", "NVIDIA", "TSMC"], &[], &[]),
        );

        let entry = &index.entries[url];
        assert_eq!(entry.companies, vec!["Nvidia", "TSMC"]);
    }

    // ── load/save round-trip tests ───────────────────────────────────────────

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".entity_index.ron");
        let index = load_entity_index(&path);
        assert_eq!(index.schema_version, 1);
        assert!(index.entries.is_empty());
    }

    #[test]
    fn load_corrupt_file_returns_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".entity_index.ron");
        fs::write(&path, "not valid RON at all!").unwrap();
        let index = load_entity_index(&path);
        assert!(index.entries.is_empty());
    }

    #[test]
    fn roundtrip_save_and_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".entity_index.ron");

        let mut index = EntityIndex::default();
        upsert_entry(
            &mut index,
            "https://example.com/a",
            EntityIndexPatch {
                fetched_utc: Some("2026-01-01T00:00:00Z".to_string()),
                content_hash: Some("abc123".to_string()),
                summary_entities: Some(SummaryEntities {
                    companies: vec!["Nvidia".to_string()],
                    technologies: vec!["custom silicon".to_string()],
                    products: vec!["H100".to_string()],
                }),
                themes: Some(vec!["AI".to_string(), "enterprise".to_string()]),
            },
        );

        save_entity_index(&path, &index).expect("save succeeded");
        let loaded = load_entity_index(&path);

        assert_eq!(loaded.schema_version, 1);
        assert_eq!(loaded.entries.len(), 1);
        let entry = &loaded.entries["https://example.com/a"];
        assert_eq!(entry.fetched_utc.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(entry.content_hash.as_deref(), Some("abc123"));
        assert_eq!(entry.companies, vec!["Nvidia"]);
        assert_eq!(entry.technologies, vec!["custom silicon"]);
        assert_eq!(entry.products, vec!["H100"]);
        assert_eq!(entry.themes, vec!["AI", "enterprise"]);
    }
}
