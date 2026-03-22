use crate::cache_utils::model_ids_compatible;
use crate::context_hash;
use crate::summary_cache::DEFAULT_CACHE_CAPACITY;
use crate::triage::ArticleTriageResult;
use chrono::Utc;
use engine_logging::engine_info;
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use std::collections::HashMap;

/// Cache key for article triage results.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TriageCacheKey {
    pub content_hash: String,
    pub prompt_id: PromptId,
    pub prompt_version: PromptVersion,
    pub model_id: String,
    pub context_hash: String,
}

impl TriageCacheKey {
    pub fn try_new(
        content_hash: &str,
        prompt_id: PromptId,
        prompt_version: Option<PromptVersion>,
        model_id: Option<&str>,
        context: &[(String, String)],
    ) -> Result<Self, TriageCacheKeyError> {
        let context_hash = context_hash(context);
        Self::try_new_with_context_hash(
            content_hash,
            prompt_id,
            prompt_version,
            model_id,
            &context_hash,
        )
    }

    pub fn try_new_with_context_hash(
        content_hash: &str,
        prompt_id: PromptId,
        prompt_version: Option<PromptVersion>,
        model_id: Option<&str>,
        context_hash: &str,
    ) -> Result<Self, TriageCacheKeyError> {
        if prompt_id != PromptId::ArticleTriage {
            return Err(TriageCacheKeyError::InvalidPromptId);
        }
        if content_hash.is_empty() {
            return Err(TriageCacheKeyError::EmptyContentHash);
        }
        let prompt_version = prompt_version.ok_or(TriageCacheKeyError::MissingPromptVersion)?;
        let model_id = model_id.ok_or(TriageCacheKeyError::MissingModelId)?;
        Ok(Self {
            content_hash: content_hash.to_string(),
            prompt_id,
            prompt_version,
            model_id: model_id.to_string(),
            context_hash: context_hash.to_string(),
        })
    }
}

/// Errors returned when constructing a triage cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriageCacheKeyError {
    MissingPromptVersion,
    MissingModelId,
    EmptyContentHash,
    InvalidPromptId,
}

/// Cached entry for an article triage result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriageCacheEntry {
    pub result: ArticleTriageResult,
    pub created_at_utc: String,
}

/// In-memory cache for article triage results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriageCache {
    entries: HashMap<TriageCacheKey, TriageCacheEntry>,
}

impl TriageCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn lookup(&self, key: &TriageCacheKey) -> Option<&ArticleTriageResult> {
        if let Some(entry) = self.entries.get(key) {
            return Some(&entry.result);
        }
        self.entries.iter().find_map(|(stored_key, entry)| {
            let model_match = model_ids_compatible(&stored_key.model_id, &key.model_id)
                || model_ids_compatible(&key.model_id, &stored_key.model_id);
            if stored_key.content_hash == key.content_hash
                && stored_key.prompt_id == key.prompt_id
                && stored_key.prompt_version == key.prompt_version
                && stored_key.context_hash == key.context_hash
                && model_match
            {
                Some(&entry.result)
            } else {
                None
            }
        })
    }

    pub fn insert(&mut self, key: TriageCacheKey, result: ArticleTriageResult) {
        let entry = TriageCacheEntry {
            result,
            created_at_utc: Utc::now().to_rfc3339(),
        };
        self.insert_entry(key, entry);
    }

    pub fn insert_entry(&mut self, key: TriageCacheKey, entry: TriageCacheEntry) {
        self.entries.insert(key, entry);
        self.enforce_capacity();
    }

    fn enforce_capacity(&mut self) {
        if self.entries.len() <= DEFAULT_CACHE_CAPACITY {
            return;
        }
        let before = self.entries.len();
        self.evict_to_limit(DEFAULT_CACHE_CAPACITY);
        let evicted = before - self.entries.len();
        if evicted > 0 {
            engine_info!(
                "[triage-cache] Evicted {} oldest entries (capacity: {})",
                evicted,
                DEFAULT_CACHE_CAPACITY
            );
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&TriageCacheKey, &TriageCacheEntry)> {
        self.entries.iter()
    }

    fn evict_to_limit(&mut self, limit: usize) {
        if self.entries.len() <= limit {
            return;
        }
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.created_at_utc.clone()))
            .collect();
        entries.sort_by(|a, b| a.1.cmp(&b.1));
        let to_remove = self.entries.len() - limit;
        for (key, _) in entries.iter().take(to_remove) {
            self.entries.remove(key);
        }
    }
}

impl Default for TriageCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI;
    use crate::triage::ArticleTriageResult;
    fn sample_result() -> ArticleTriageResult {
        ArticleTriageResult {
            category: "cat".to_string(),
            priority: 1,
            tags: vec!["tag".to_string()],
            rationale: "reason".to_string(),
            input_tokens: 10,
            output_tokens: 5,
        }
    }

    fn build_key(content_hash: &str, model_id: &str, context_hash: &str) -> TriageCacheKey {
        TriageCacheKey::try_new_with_context_hash(
            content_hash,
            PromptId::ArticleTriage,
            Some(1),
            Some(model_id),
            context_hash,
        )
        .unwrap()
    }

    #[test]
    fn insert_and_lookup_roundtrip() {
        let mut cache = TriageCache::new();
        let context = vec![("k".to_string(), "v".to_string())];
        let key = TriageCacheKey::try_new(
            "hash",
            PromptId::ArticleTriage,
            Some(1),
            Some(OPENAI_MODEL_GPT_4O_MINI),
            &context,
        )
        .unwrap();
        cache.insert(key.clone(), sample_result());
        let retrieved = cache.lookup(&key).unwrap();
        assert_eq!(retrieved.category, "cat");
    }

    #[test]
    fn unknown_prompt_id_rejected() {
        let context = Vec::new();
        assert_eq!(
            TriageCacheKey::try_new(
                "hash",
                PromptId::ArticleSummary,
                Some(1),
                Some(OPENAI_MODEL_GPT_4O_MINI),
                &context,
            ),
            Err(TriageCacheKeyError::InvalidPromptId)
        );
    }

    #[test]
    fn model_variant_compatibility_allows_alias_match() {
        let mut cache = TriageCache::new();
        let context = vec![("k".to_string(), "v".to_string())];
        let context_hash = context_hash(&context);
        let stored_key = build_key("hash", "gpt-4o-mini-2024-07-18", &context_hash);
        cache.insert(stored_key, sample_result());
        let lookup_key = build_key("hash", OPENAI_MODEL_GPT_4O_MINI, &context_hash);
        assert!(cache.lookup(&lookup_key).is_some());
    }

    #[test]
    fn different_context_hash_is_a_miss() {
        let mut cache = TriageCache::new();
        let key = TriageCacheKey::try_new(
            "hash",
            PromptId::ArticleTriage,
            Some(1),
            Some("model"),
            &[("k".to_string(), "v".to_string())],
        )
        .unwrap();
        cache.insert(key, sample_result());
        let miss_key = TriageCacheKey::try_new(
            "hash",
            PromptId::ArticleTriage,
            Some(1),
            Some("model"),
            &[("k".to_string(), "different".to_string())],
        )
        .unwrap();
        assert!(cache.lookup(&miss_key).is_none());
    }

    #[test]
    fn different_prompt_version_is_a_miss() {
        let mut cache = TriageCache::new();
        let key = TriageCacheKey::try_new(
            "hash",
            PromptId::ArticleTriage,
            Some(1),
            Some("model"),
            &[("k".to_string(), "v".to_string())],
        )
        .unwrap();
        cache.insert(key, sample_result());
        let miss_key = TriageCacheKey::try_new(
            "hash",
            PromptId::ArticleTriage,
            Some(2),
            Some("model"),
            &[("k".to_string(), "v".to_string())],
        )
        .unwrap();
        assert!(cache.lookup(&miss_key).is_none());
    }

    #[test]
    fn capacity_guard_evicts_oldest_entry() {
        let mut cache = TriageCache::new();
        let context = vec![("k".to_string(), "v".to_string())];
        for i in 0..=DEFAULT_CACHE_CAPACITY {
            let key = TriageCacheKey::try_new(
                &format!("hash-{i}"),
                PromptId::ArticleTriage,
                Some(1),
                Some("model"),
                &context,
            )
            .unwrap();
            cache.insert(key.clone(), sample_result());
            if i == DEFAULT_CACHE_CAPACITY {
                assert_eq!(cache.len(), DEFAULT_CACHE_CAPACITY);
            }
        }
        assert_eq!(cache.len(), DEFAULT_CACHE_CAPACITY);
    }
}
