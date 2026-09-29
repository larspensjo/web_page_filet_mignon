#[cfg(test)]
use harvester_core::{ArticleSummaryResult, SummaryEntities};
use harvester_core::{SummaryCache, SummaryCacheEntry, SummaryCacheKey};
#[cfg(test)]
use harvester_engine::llm::prompt::PromptId;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::fs;
use std::{io, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PersistedCacheKey {
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
pub(crate) struct PersistedCacheEntry {
    result: PersistedSummaryResult,
    created_at_utc: String,
}

pub(crate) fn legacy(path: &Path) -> io::Result<Vec<(SummaryCacheKey, SummaryCacheEntry)>> {
    crate::result_store::read_ron::<
        SummaryCacheKey,
        SummaryCacheEntry,
        PersistedCacheKey,
        PersistedCacheEntry,
    >(path)
}

pub fn load_summary_cache(path: &Path) -> io::Result<SummaryCache> {
    let records = crate::result_store::load::<
        SummaryCacheKey,
        SummaryCacheEntry,
        PersistedCacheKey,
        PersistedCacheEntry,
    >(path, legacy)?;
    let mut cache = SummaryCache::default();
    for (key, entry) in records {
        cache.insert(key, entry);
    }
    Ok(cache)
}

/// Append the supplied entries. Production completions use the runner's ordered sink.
#[cfg(test)]
pub(crate) fn persist_summary_cache(cache: &SummaryCache, path: &Path) -> io::Result<()> {
    let records: Vec<_> = cache.iter().collect();
    crate::result_store::AppendFile::open::<
        SummaryCacheKey,
        SummaryCacheEntry,
        PersistedCacheKey,
        PersistedCacheEntry,
    >(path, legacy)?
    .append::<_, _, PersistedCacheKey, PersistedCacheEntry>(&records)
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

        let cache = load_summary_cache(&path).unwrap();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn load_corrupt_file_returns_empty_cache() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("corrupt_cache.ron");
        fs::write(&path, "this is not valid RON").unwrap();

        let before = fs::read(&path).unwrap();
        let error = load_summary_cache(&path).unwrap_err().to_string();
        assert!(error.contains("corrupt_cache.ron"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!path.with_extension("jsonl").exists());
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
        let loaded = load_summary_cache(&path).unwrap();

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
        let loaded = load_summary_cache(&path).unwrap();

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

        let cache = load_summary_cache(&path).unwrap();
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
