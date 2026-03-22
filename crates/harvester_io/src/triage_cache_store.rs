use std::fs;
use std::path::{Path, PathBuf};

use engine_logging::{engine_info, engine_warn};
use harvester_core::{ArticleTriageResult, TriageCache, TriageCacheEntry, TriageCacheKey};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::{ensure_output_dir, AtomicFileWriter, PersistError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedTriageCacheKey {
    content_hash: String,
    prompt_id: String,
    prompt_version: u32,
    model_id: String,
    context_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedTriageResult {
    category: String,
    priority: u8,
    tags: Vec<String>,
    rationale: String,
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedTriageEntry {
    result: PersistedTriageResult,
    created_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedTriageCache {
    #[serde(default = "default_version")]
    version: u32,
    entries: Vec<(PersistedTriageCacheKey, PersistedTriageEntry)>,
}

fn default_version() -> u32 {
    1
}

pub fn load_triage_cache(path: &Path) -> TriageCache {
    let text = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            engine_info!("[triage-cache] No persisted cache found at {:?}", path);
            return TriageCache::new();
        }
        Err(err) => {
            engine_warn!(
                "[triage-cache] Failed to read cache from {:?}: {}",
                path,
                err
            );
            return TriageCache::new();
        }
    };

    let persisted: PersistedTriageCache = match ron::from_str(&text) {
        Ok(cache) => cache,
        Err(err) => {
            engine_warn!(
                "[triage-cache] Failed to parse cache from {:?}: {}",
                path,
                err
            );
            return TriageCache::new();
        }
    };

    let mut cache = TriageCache::new();
    for (persisted_key, persisted_entry) in persisted.entries {
        let prompt_id = match persisted_key.prompt_id.as_str() {
            "ArticleTriage" => PromptId::ArticleTriage,
            unknown => {
                engine_warn!(
                    "[triage-cache] Unknown prompt_id '{}' in persisted cache, skipping entry",
                    unknown
                );
                continue;
            }
        };

        let key = match TriageCacheKey::try_new_with_context_hash(
            &persisted_key.content_hash,
            prompt_id,
            Some(persisted_key.prompt_version),
            Some(persisted_key.model_id.as_str()),
            &persisted_key.context_hash,
        ) {
            Ok(key) => key,
            Err(err) => {
                engine_warn!(
                    "[triage-cache] Skipping entry due to invalid metadata: {:?}",
                    err
                );
                continue;
            }
        };

        let entry = TriageCacheEntry {
            result: ArticleTriageResult {
                category: persisted_entry.result.category,
                priority: persisted_entry.result.priority,
                tags: persisted_entry.result.tags,
                rationale: persisted_entry.result.rationale,
                input_tokens: persisted_entry.result.input_tokens,
                output_tokens: persisted_entry.result.output_tokens,
            },
            created_at_utc: persisted_entry.created_at_utc,
        };

        cache.insert_entry(key, entry);
    }

    engine_info!(
        "[triage-cache] Loaded {} entries from {:?}",
        cache.len(),
        path
    );
    cache
}

pub fn persist_triage_cache(cache: &TriageCache, path: &Path) -> Result<(), PersistError> {
    if let Some(parent) = path.parent() {
        ensure_output_dir(parent)?;
    }

    let entries: Vec<(PersistedTriageCacheKey, PersistedTriageEntry)> = cache
        .iter()
        .map(|(key, entry)| {
            (
                PersistedTriageCacheKey {
                    content_hash: key.content_hash.clone(),
                    prompt_id: format!("{:?}", key.prompt_id),
                    prompt_version: key.prompt_version,
                    model_id: key.model_id.clone(),
                    context_hash: key.context_hash.clone(),
                },
                PersistedTriageEntry {
                    result: PersistedTriageResult {
                        category: entry.result.category.clone(),
                        priority: entry.result.priority,
                        tags: entry.result.tags.clone(),
                        rationale: entry.result.rationale.clone(),
                        input_tokens: entry.result.input_tokens,
                        output_tokens: entry.result.output_tokens,
                    },
                    created_at_utc: entry.created_at_utc.clone(),
                },
            )
        })
        .collect();

    let persisted = PersistedTriageCache {
        version: 1,
        entries,
    };

    let serialized = ron::ser::to_string_pretty(&persisted, ron::ser::PrettyConfig::default())
        .map_err(|err| std::io::Error::other(format!("Failed to serialize cache: {}", err)))?;

    let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let writer = AtomicFileWriter::new(PathBuf::from(parent_dir));
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".triage_cache.ron");
    writer.write(filename, &serialized)?;

    engine_info!("[triage-cache] Saved {} entries to {:?}", cache.len(), path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI;
    use tempfile::tempdir;

    #[test]
    fn load_missing_file_returns_empty_cache() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing_cache.ron");

        let cache = load_triage_cache(&path);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_triage_cache.ron");

        let mut cache = TriageCache::new();
        let key = TriageCacheKey::try_new(
            "hash",
            PromptId::ArticleTriage,
            Some(1),
            Some(OPENAI_MODEL_GPT_4O_MINI),
            &[("k".to_string(), "v".to_string())],
        )
        .unwrap();
        let result = ArticleTriageResult {
            category: "category".to_string(),
            priority: 5,
            tags: vec!["tag".to_string()],
            rationale: "reason".to_string(),
            input_tokens: 50,
            output_tokens: 25,
        };
        cache.insert(key.clone(), result.clone());

        persist_triage_cache(&cache, &path).unwrap();
        let loaded = load_triage_cache(&path);
        assert_eq!(loaded.len(), 1);
        let loaded_entry = loaded.lookup(&key).unwrap();
        assert_eq!(loaded_entry.category, result.category);
        assert_eq!(loaded_entry.priority, result.priority);
    }

    #[test]
    fn corrupt_file_returns_empty_and_warns() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("corrupt_cache.ron");
        fs::write(&path, "not valid ron").unwrap();

        let cache = load_triage_cache(&path);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn unknown_prompt_id_entry_is_skipped_with_warning() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("unknown_prompt.ron");
        let persisted = PersistedTriageCache {
            version: 1,
            entries: vec![(
                PersistedTriageCacheKey {
                    content_hash: "hash".to_string(),
                    prompt_id: "UnknownPrompt".to_string(),
                    prompt_version: 1,
                    model_id: "model".to_string(),
                    context_hash: "ctx".to_string(),
                },
                PersistedTriageEntry {
                    result: PersistedTriageResult {
                        category: "category".to_string(),
                        priority: 1,
                        tags: vec![],
                        rationale: "reason".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                    created_at_utc: "2026-01-01T00:00:00Z".to_string(),
                },
            )],
        };
        let serialized =
            ron::ser::to_string_pretty(&persisted, ron::ser::PrettyConfig::default()).unwrap();
        fs::write(&path, serialized).unwrap();

        let cache = load_triage_cache(&path);
        assert!(cache.is_empty());
    }
}
