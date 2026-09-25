use crate::cache_utils::model_ids_compatible;
use crate::context_hash;
use crate::summary_cache::DEFAULT_CACHE_CAPACITY;
use crate::triage::ArticleTriageResult;
use chrono::Utc;
use engine_logging::engine_info;
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Cache key for article triage results.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageCacheEntry {
    pub result: ArticleTriageResult,
    pub created_at_utc: String,
}

/// In-memory cache for article triage results.
#[derive(Debug, Clone, Serialize)]
pub struct TriageCache {
    entries: HashMap<TriageCacheKey, TriageCacheEntry>,
    #[serde(skip)]
    aliases: HashMap<String, Vec<TriageCacheAlias>>,
}

#[derive(Debug, Clone)]
struct TriageCacheAlias {
    key: TriageCacheKey,
    priority: u8,
}

impl PartialEq for TriageCache {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl Eq for TriageCache {}

impl<'de> Deserialize<'de> for TriageCache {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            entries: HashMap<TriageCacheKey, TriageCacheEntry>,
        }
        let mut cache = Self {
            entries: Wire::deserialize(deserializer)?.entries,
            aliases: HashMap::new(),
        };
        cache.rebuild_alias_index();
        Ok(cache)
    }
}

impl TriageCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    /// Returns both the result and its stored key so consumers retain provenance
    /// when a compatible model alias satisfies the lookup.
    pub fn lookup<'a>(
        &'a self,
        key: &TriageCacheKey,
    ) -> Option<(&'a TriageCacheKey, &'a ArticleTriageResult)> {
        if let Some((stored_key, entry)) = self.entries.get_key_value(key) {
            return Some((stored_key, &entry.result));
        }
        self.lookup_current_key_parts(
            &key.content_hash,
            key.prompt_id,
            key.prompt_version,
            &key.model_id,
            &key.context_hash,
        )
    }

    /// Look up a current metadata key without allocating an owned cache-key tuple for each
    /// article. Exact model matches retain the same precedence as `lookup`; compatible model
    /// aliases retain their insertion order within the matching prompt/version/context group.
    pub(crate) fn lookup_current_key_parts(
        &self,
        content_hash: &str,
        prompt_id: PromptId,
        prompt_version: PromptVersion,
        model_id: &str,
        context_hash: &str,
    ) -> Option<(&TriageCacheKey, &ArticleTriageResult)> {
        let alias = self.current_alias(
            content_hash,
            prompt_id,
            prompt_version,
            model_id,
            context_hash,
        )?;
        let (stored_key, entry) = self.entries.get_key_value(&alias.key)?;
        Some((stored_key, &entry.result))
    }

    pub(crate) fn lookup_current_priority_parts(
        &self,
        content_hash: &str,
        prompt_id: PromptId,
        prompt_version: PromptVersion,
        model_id: &str,
        context_hash: &str,
    ) -> Option<u8> {
        self.current_alias(
            content_hash,
            prompt_id,
            prompt_version,
            model_id,
            context_hash,
        )
        .map(|alias| alias.priority)
    }

    fn current_alias(
        &self,
        content_hash: &str,
        prompt_id: PromptId,
        prompt_version: PromptVersion,
        model_id: &str,
        context_hash: &str,
    ) -> Option<&TriageCacheAlias> {
        let candidates = self.aliases.get(content_hash)?;
        let matches_metadata = |candidate: &&TriageCacheAlias| {
            candidate.key.prompt_id == prompt_id
                && candidate.key.prompt_version == prompt_version
                && candidate.key.context_hash == context_hash
        };
        let exact = candidates
            .iter()
            .filter(matches_metadata)
            .find(|candidate| candidate.key.model_id == model_id);
        let candidate = exact.or_else(|| {
            candidates
                .iter()
                .filter(matches_metadata)
                .find(|candidate| {
                    model_ids_compatible(&candidate.key.model_id, model_id)
                        || model_ids_compatible(model_id, &candidate.key.model_id)
                })
        })?;
        Some(candidate)
    }

    pub fn insert(&mut self, key: TriageCacheKey, result: ArticleTriageResult) -> Vec<String> {
        let entry = TriageCacheEntry {
            result,
            created_at_utc: Utc::now().to_rfc3339(),
        };
        self.insert_entry(key, entry)
    }

    pub fn insert_entry(&mut self, key: TriageCacheKey, entry: TriageCacheEntry) -> Vec<String> {
        let priority = entry.result.priority;
        if self.entries.contains_key(&key) {
            if let Some(alias) = self
                .aliases
                .get_mut(&key.content_hash)
                .and_then(|candidates| candidates.iter_mut().find(|candidate| candidate.key == key))
            {
                alias.priority = priority;
            }
        } else {
            self.aliases
                .entry(key.content_hash.clone())
                .or_default()
                .push(TriageCacheAlias {
                    key: key.clone(),
                    priority,
                });
        }
        self.entries.insert(key, entry);
        self.enforce_capacity()
    }

    pub(crate) fn rebuild_alias_index(&mut self) {
        self.aliases.clear();
        for (key, entry) in &self.entries {
            self.aliases
                .entry(key.content_hash.clone())
                .or_default()
                .push(TriageCacheAlias {
                    key: key.clone(),
                    priority: entry.result.priority,
                });
        }
    }

    fn enforce_capacity(&mut self) -> Vec<String> {
        if self.entries.len() <= DEFAULT_CACHE_CAPACITY {
            return Vec::new();
        }
        let evicted = self.evict_to_limit(DEFAULT_CACHE_CAPACITY);
        if !evicted.is_empty() {
            engine_info!(
                "[triage-cache] Evicted {} oldest entries (capacity: {})",
                evicted.len(),
                DEFAULT_CACHE_CAPACITY
            );
        }
        evicted
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

    fn evict_to_limit(&mut self, limit: usize) -> Vec<String> {
        if self.entries.len() <= limit {
            return Vec::new();
        }
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.created_at_utc.clone()))
            .collect();
        entries.sort_by(|a, b| a.1.cmp(&b.1));
        let to_remove = self.entries.len() - limit;
        let mut evicted = Vec::with_capacity(to_remove);
        for (key, _) in entries.iter().take(to_remove) {
            self.entries.remove(key);
            evicted.push(key.content_hash.clone());
        }
        self.rebuild_alias_index();
        evicted
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
    use crate::triage::ArticleTriageResult;

    const TEST_MODEL_ID: &str = "test-model-mini";
    const TEST_MODEL_VARIANT_ID: &str = "test-model-mini-2026-03-31";

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
            Some(TEST_MODEL_ID),
            &context,
        )
        .unwrap();
        cache.insert(key.clone(), sample_result());
        let (_, retrieved) = cache.lookup(&key).unwrap();
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
                Some(TEST_MODEL_ID),
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
        let stored_key = build_key("hash", TEST_MODEL_ID, &context_hash);
        cache.insert(stored_key, sample_result());
        let lookup_key = build_key("hash", TEST_MODEL_VARIANT_ID, &context_hash);
        let (stored_key, _) = cache.lookup(&lookup_key).expect("compatible cache hit");
        assert_eq!(stored_key.model_id, TEST_MODEL_ID);
        assert_eq!(
            cache.lookup_current_priority_parts(
                "hash",
                PromptId::ArticleTriage,
                1,
                TEST_MODEL_VARIANT_ID,
                &context_hash,
            ),
            Some(sample_result().priority)
        );
    }

    #[test]
    fn current_priority_index_tracks_cache_writes_hydration_and_metadata() {
        let context_hash = "triage-context";
        let key = build_key("hash", TEST_MODEL_ID, context_hash);
        let mut cache = TriageCache::new();

        let mut result = sample_result();
        result.priority = 2;
        cache.insert_entry(
            key.clone(),
            TriageCacheEntry {
                result,
                created_at_utc: "2026-09-25T12:00:00Z".into(),
            },
        );
        assert_eq!(
            cache.lookup_current_priority_parts(
                "hash",
                PromptId::ArticleTriage,
                1,
                TEST_MODEL_ID,
                context_hash,
            ),
            cache.lookup(&key).map(|(_, result)| result.priority)
        );

        let mut replacement = sample_result();
        replacement.priority = 5;
        cache.insert_entry(
            key.clone(),
            TriageCacheEntry {
                result: replacement,
                created_at_utc: "2026-09-25T12:01:00Z".into(),
            },
        );
        assert_eq!(
            cache.lookup_current_priority_parts(
                "hash",
                PromptId::ArticleTriage,
                1,
                TEST_MODEL_ID,
                context_hash,
            ),
            cache.lookup(&key).map(|(_, result)| result.priority)
        );

        let mut hydrated = cache.clone();
        hydrated.aliases.clear();
        hydrated.rebuild_alias_index();
        assert_eq!(
            hydrated.lookup_current_priority_parts(
                "hash",
                PromptId::ArticleTriage,
                1,
                TEST_MODEL_ID,
                context_hash,
            ),
            hydrated.lookup(&key).map(|(_, result)| result.priority)
        );
        assert_eq!(
            hydrated.lookup_current_priority_parts(
                "hash",
                PromptId::ArticleTriage,
                2,
                TEST_MODEL_ID,
                context_hash,
            ),
            None
        );
        assert_eq!(
            hydrated.lookup_current_priority_parts(
                "hash",
                PromptId::ArticleTriage,
                1,
                TEST_MODEL_ID,
                "changed-context",
            ),
            None
        );
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
