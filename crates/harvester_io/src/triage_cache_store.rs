#[cfg(test)]
use harvester_core::ArticleTriageResult;
use harvester_core::{TriageCache, TriageCacheEntry, TriageCacheKey};
#[cfg(test)]
use harvester_engine::llm::prompt::PromptId;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::fs;
use std::{io, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PersistedTriageCacheKey {
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
pub(crate) struct PersistedTriageEntry {
    result: PersistedTriageResult,
    created_at_utc: String,
}

pub(crate) fn legacy(path: &Path) -> io::Result<Vec<(TriageCacheKey, TriageCacheEntry)>> {
    crate::result_store::read_ron::<
        TriageCacheKey,
        TriageCacheEntry,
        PersistedTriageCacheKey,
        PersistedTriageEntry,
    >(path)
}

pub fn load_triage_cache(path: &Path) -> io::Result<TriageCache> {
    let records = crate::result_store::load::<
        TriageCacheKey,
        TriageCacheEntry,
        PersistedTriageCacheKey,
        PersistedTriageEntry,
    >(path, legacy)?;
    let mut cache = TriageCache::default();
    for (key, entry) in records {
        cache.insert_entry(key, entry);
    }
    Ok(cache)
}

/// Append the supplied entries. Production completions use the runner's ordered sink.
#[cfg(test)]
pub(crate) fn persist_triage_cache(cache: &TriageCache, path: &Path) -> io::Result<()> {
    let records: Vec<_> = cache.iter().collect();
    crate::result_store::AppendFile::open::<
        TriageCacheKey,
        TriageCacheEntry,
        PersistedTriageCacheKey,
        PersistedTriageEntry,
    >(path, legacy)?
    .append::<_, _, PersistedTriageCacheKey, PersistedTriageEntry>(&records)
}

#[cfg(test)]
#[derive(Serialize)]
struct PersistedTriageCache {
    version: u32,
    entries: Vec<(PersistedTriageCacheKey, PersistedTriageEntry)>,
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

        let cache = load_triage_cache(&path).unwrap();
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
        let loaded = load_triage_cache(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        let (_, loaded_entry) = loaded.lookup(&key).unwrap();
        assert_eq!(loaded_entry.category, result.category);
        assert_eq!(loaded_entry.priority, result.priority);
    }

    #[test]
    fn corrupt_file_returns_empty_and_warns() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("corrupt_cache.ron");
        fs::write(&path, "not valid ron").unwrap();

        let before = fs::read(&path).unwrap();
        let error = load_triage_cache(&path).unwrap_err().to_string();
        assert!(error.contains("corrupt_cache.ron"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!path.with_extension("jsonl").exists());
    }

    #[test]
    fn unknown_prompt_id_entry_is_skipped_with_warning() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("unknown_prompt.ron");
        let mut persisted = PersistedTriageCache {
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
        let mut valid = persisted.entries[0].clone();
        valid.0.prompt_id = "ArticleTriage".to_string();
        valid.0.content_hash = "valid-hash".to_string();
        persisted.entries.push(valid);
        let serialized =
            ron::ser::to_string_pretty(&persisted, ron::ser::PrettyConfig::default()).unwrap();
        fs::write(&path, serialized).unwrap();

        let loaded = load_triage_cache(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.iter().next().unwrap().0.content_hash, "valid-hash");
        assert_eq!(
            fs::read_to_string(path.with_extension("jsonl"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}
