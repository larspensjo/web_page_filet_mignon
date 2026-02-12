use std::fs;
use std::path::{Path, PathBuf};

use engine_logging::{engine_info, engine_warn};
use harvester_core::{ArticleSummaryResult, SummaryCache, SummaryCacheEntry, SummaryCacheKey};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::{ensure_output_dir, AtomicFileWriter};
use serde::{Deserialize, Serialize};

const CACHE_FILENAME: &str = ".summary_cache.ron";

/// DTO for persisting SummaryCacheKey
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedCacheKey {
    content_hash: String,
    prompt_id: String, // Serialize as string for forward compatibility
    prompt_version: u32,
    model_id: String,
    context_hash: String,
}

/// DTO for persisting ArticleSummaryResult
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSummaryResult {
    title: String,
    summary: String,
    key_points: Vec<String>,
    input_tokens: u32,
    output_tokens: u32,
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

/// Get the default path for the summary cache file
pub(crate) fn default_summary_cache_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("output")
        .join(CACHE_FILENAME)
}

/// Load the summary cache from disk
pub(crate) fn load_summary_cache(path: &Path) -> SummaryCache {
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
pub(crate) fn save_summary_cache(
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
        .unwrap_or(CACHE_FILENAME);
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
            },
            created_at_utc: Utc::now().to_rfc3339(),
        };
        cache.insert(key.clone(), entry.clone());

        // Save
        save_summary_cache(&cache, &path).unwrap();

        // Load
        let loaded = load_summary_cache(&path);

        assert_eq!(loaded.len(), 1);
        let loaded_entry = loaded.lookup(&key).unwrap();
        assert_eq!(loaded_entry.result.title, "Test Title");
        assert_eq!(loaded_entry.result.summary, "Test Summary");
        assert_eq!(loaded_entry.result.key_points.len(), 1);
    }
}
