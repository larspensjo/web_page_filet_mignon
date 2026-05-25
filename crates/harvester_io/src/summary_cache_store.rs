use std::fs;
use std::path::{Path, PathBuf};

use engine_logging::{engine_info, engine_warn};
use harvester_core::{
    ArticleSummaryResult, SummaryCache, SummaryCacheEntry, SummaryCacheKey, SummaryEntities,
};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::{ensure_output_dir, AtomicFileWriter};
use serde::{Deserialize, Serialize};

/// DTO for persisting SummaryCacheKey
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedCacheKey {
    content_hash: String,
    prompt_id: String, // Serialize as string for forward compatibility
    prompt_version: u32,
    model_id: String,
    context_hash: String,
}

/// DTO for persisting SummaryEntities — backward-compatible with V3 cache files that lack the field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SummaryEntitiesDto {
    #[serde(default)]
    companies: Vec<String>,
    #[serde(default)]
    technologies: Vec<String>,
    #[serde(default)]
    products: Vec<String>,
}

/// DTO for persisting ArticleSummaryResult
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSummaryResult {
    title: String,
    summary: String,
    key_points: Vec<String>,
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    entities: SummaryEntitiesDto,
}

/// DTO for persisting SummaryCacheEntry
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedCacheEntry {
    result: PersistedSummaryResult,
    created_at_utc: String,
}

/// Top-level persisted cache structure
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedCache {
    #[serde(default = "default_version")]
    version: u32,
    entries: Vec<(PersistedCacheKey, PersistedCacheEntry)>,
}

fn default_version() -> u32 {
    1
}

/// Load the summary cache from disk
pub fn load_summary_cache(path: &Path) -> SummaryCache {
    let content = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            engine_info!("[summary-cache] No persisted cache found at {:?}", path);
            return SummaryCache::new();
        }
        Err(err) => {
            engine_warn!(
                "[summary-cache] Failed to read cache from {:?}: {}",
                path,
                err
            );
            return SummaryCache::new();
        }
    };

    let persisted: PersistedCache = match ron::from_str(&content) {
        Ok(cache) => cache,
        Err(err) => {
            engine_warn!(
                "[summary-cache] Failed to parse cache from {:?}: {}",
                path,
                err
            );
            return SummaryCache::new();
        }
    };

    let mut cache = SummaryCache::new();
    for (persisted_key, persisted_entry) in persisted.entries {
        // Convert string prompt_id back to enum
        let prompt_id = match persisted_key.prompt_id.as_str() {
            "ArticleSummary" => PromptId::ArticleSummary,
            "ArticleTriage" => PromptId::ArticleTriage,
            "ArticleSignalCandidate" => PromptId::ArticleSignalCandidate,
            "AggregateBriefing" => PromptId::AggregateBriefing,
            unknown => {
                engine_warn!(
                    "[summary-cache] Unknown prompt_id '{}' in persisted cache, skipping entry",
                    unknown
                );
                continue;
            }
        };

        let key = SummaryCacheKey {
            content_hash: persisted_key.content_hash,
            prompt_id,
            prompt_version: persisted_key.prompt_version,
            model_id: persisted_key.model_id,
            context_hash: persisted_key.context_hash,
        };

        let entry = SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: persisted_entry.result.title,
                summary: persisted_entry.result.summary,
                key_points: persisted_entry.result.key_points,
                input_tokens: persisted_entry.result.input_tokens,
                output_tokens: persisted_entry.result.output_tokens,
                entities: SummaryEntities {
                    companies: persisted_entry.result.entities.companies,
                    technologies: persisted_entry.result.entities.technologies,
                    products: persisted_entry.result.entities.products,
                },
            },
            created_at_utc: persisted_entry.created_at_utc,
        };

        cache.insert(key, entry);
    }

    engine_info!(
        "[summary-cache] Loaded {} entries from {:?}",
        cache.len(),
        path
    );
    cache
}

/// Save the summary cache to disk
pub fn persist_summary_cache(
    cache: &SummaryCache,
    path: &Path,
) -> Result<(), harvester_engine::PersistError> {
    // Ensure output directory exists
    if let Some(parent) = path.parent() {
        ensure_output_dir(parent)?;
    }

    // Convert cache to persisted format
    let entries: Vec<(PersistedCacheKey, PersistedCacheEntry)> = cache
        .iter()
        .map(|(key, entry)| {
            let persisted_key = PersistedCacheKey {
                content_hash: key.content_hash.clone(),
                prompt_id: format!("{:?}", key.prompt_id),
                prompt_version: key.prompt_version,
                model_id: key.model_id.clone(),
                context_hash: key.context_hash.clone(),
            };

            let persisted_entry = PersistedCacheEntry {
                result: PersistedSummaryResult {
                    title: entry.result.title.clone(),
                    summary: entry.result.summary.clone(),
                    key_points: entry.result.key_points.clone(),
                    input_tokens: entry.result.input_tokens,
                    output_tokens: entry.result.output_tokens,
                    entities: SummaryEntitiesDto {
                        companies: entry.result.entities.companies.clone(),
                        technologies: entry.result.entities.technologies.clone(),
                        products: entry.result.entities.products.clone(),
                    },
                },
                created_at_utc: entry.created_at_utc.clone(),
            };

            (persisted_key, persisted_entry)
        })
        .collect();

    let persisted = PersistedCache {
        version: 1,
        entries,
    };

    // Serialize to RON format
    let serialized = ron::ser::to_string_pretty(&persisted, ron::ser::PrettyConfig::default())
        .map_err(|err| std::io::Error::other(format!("Failed to serialize cache: {}", err)))?;

    // Write atomically
    let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let writer = AtomicFileWriter::new(PathBuf::from(parent_dir));
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".summary_cache.ron");
    writer.write(filename, &serialized)?;

    engine_info!(
        "[summary-cache] Saved {} entries to {:?}",
        cache.len(),
        path
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    #[test]
    fn load_missing_file_returns_empty_cache() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing_cache.ron");

        let cache = load_summary_cache(&path);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn load_corrupt_file_returns_empty_cache() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("corrupt_cache.ron");
        fs::write(&path, "this is not valid RON").unwrap();

        let cache = load_summary_cache(&path);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn roundtrip_save_and_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_cache.ron");

        let mut cache = SummaryCache::new();
        let key = SummaryCacheKey {
            content_hash: "test-hash".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "gpt-4".to_string(),
            context_hash: "ctx-hash".to_string(),
        };
        let entry = SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "Test Title".to_string(),
                summary: "Test Summary".to_string(),
                key_points: vec!["Point 1".to_string()],
                input_tokens: 100,
                output_tokens: 50,
                entities: Default::default(),
            },
            created_at_utc: Utc::now().to_rfc3339(),
        };
        cache.insert(key.clone(), entry.clone());

        // Save
        persist_summary_cache(&cache, &path).unwrap();

        // Load
        let loaded = load_summary_cache(&path);

        assert_eq!(loaded.len(), 1);
        let loaded_entry = loaded.lookup(&key).unwrap();
        assert_eq!(loaded_entry.result.title, "Test Title");
        assert_eq!(loaded_entry.result.summary, "Test Summary");
        assert_eq!(loaded_entry.result.key_points.len(), 1);
    }

    #[test]
    fn roundtrip_preserves_entities() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("entities_cache.ron");

        let mut cache = SummaryCache::new();
        let key = SummaryCacheKey {
            content_hash: "hash-v4".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "claude-3".to_string(),
            context_hash: "ctx-hash".to_string(),
        };
        let entry = SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "Entity Test".to_string(),
                summary: "Summary".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
                entities: SummaryEntities {
                    companies: vec!["Nvidia".to_string(), "TSMC".to_string()],
                    technologies: vec!["custom silicon".to_string()],
                    products: vec!["H100".to_string()],
                },
            },
            created_at_utc: Utc::now().to_rfc3339(),
        };
        cache.insert(key.clone(), entry);

        persist_summary_cache(&cache, &path).unwrap();
        let loaded = load_summary_cache(&path);

        let loaded_entry = loaded.lookup(&key).expect("entry present");
        assert_eq!(
            loaded_entry.result.entities.companies,
            vec!["Nvidia", "TSMC"]
        );
        assert_eq!(
            loaded_entry.result.entities.technologies,
            vec!["custom silicon"]
        );
        assert_eq!(loaded_entry.result.entities.products, vec!["H100"]);
    }

    #[test]
    fn load_v3_cache_without_entities_gives_empty_entities() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("v3_cache.ron");

        // Write a V3-style RON file without the entities field.
        let v3_ron = r#"(
    version: 1,
    entries: [
        (
            (
                content_hash: "old-hash",
                prompt_id: "ArticleSummary",
                prompt_version: 3,
                model_id: "gpt-4",
                context_hash: "ctx",
            ),
            (
                result: (
                    title: "Old Article",
                    summary: "Old summary.",
                    key_points: ["Point A"],
                    input_tokens: 50,
                    output_tokens: 25,
                ),
                created_at_utc: "2025-01-01T00:00:00Z",
            ),
        ),
    ],
)"#;
        fs::write(&path, v3_ron).unwrap();

        let cache = load_summary_cache(&path);
        assert_eq!(cache.len(), 1);

        let key = SummaryCacheKey {
            content_hash: "old-hash".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 3,
            model_id: "gpt-4".to_string(),
            context_hash: "ctx".to_string(),
        };
        let entry = cache.lookup(&key).expect("entry present");
        assert_eq!(entry.result.title, "Old Article");
        assert!(
            entry.result.entities.companies.is_empty(),
            "V3 cache should have empty companies"
        );
        assert!(
            entry.result.entities.technologies.is_empty(),
            "V3 cache should have empty technologies"
        );
        assert!(
            entry.result.entities.products.is_empty(),
            "V3 cache should have empty products"
        );
    }
}
