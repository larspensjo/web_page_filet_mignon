use engine_logging::engine_info;
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::briefing::ArticleSummaryResult;

/// Maximum number of entries allowed in the cache before eviction.
/// Prevents unbounded growth in memory and on disk.
pub const DEFAULT_CACHE_CAPACITY: usize = 10_000;

/// Cache key for article summary results.
/// Includes all dimensions that could affect the summary output.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SummaryCacheKey {
    pub content_hash: String,
    pub prompt_id: PromptId,
    pub prompt_version: PromptVersion,
    pub model_id: String,
    pub context_hash: String,
}

impl SummaryCacheKey {
    pub fn try_new(
        content_hash: &str,
        prompt_id: PromptId,
        prompt_version: Option<PromptVersion>,
        model_id: Option<&str>,
        context: &[(String, String)],
    ) -> Result<Self, SummaryCacheKeyError> {
        if content_hash.is_empty() {
            return Err(SummaryCacheKeyError::EmptyContentHash);
        }
        let prompt_version = prompt_version.ok_or(SummaryCacheKeyError::MissingPromptVersion)?;
        let model_id = model_id.ok_or(SummaryCacheKeyError::MissingModelId)?;
        let context_hash = context_hash(context);
        Ok(Self {
            content_hash: content_hash.to_string(),
            prompt_id,
            prompt_version,
            model_id: model_id.to_string(),
            context_hash,
        })
    }
}

/// Error returned when a summary cache key cannot be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryCacheKeyError {
    MissingPromptVersion,
    MissingModelId,
    EmptyContentHash,
}

/// Cached entry for an article summary result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryCacheEntry {
    pub result: ArticleSummaryResult,
    pub created_at_utc: String,
}

/// In-memory cache for article summary results.
/// Provides lookup and insertion with optional eviction by size limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryCache {
    entries: HashMap<SummaryCacheKey, SummaryCacheEntry>,
}

impl SummaryCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Look up a cached summary result by key.
    pub fn lookup(&self, key: &SummaryCacheKey) -> Option<&SummaryCacheEntry> {
        self.entries.get(key)
    }

    /// Insert a new cache entry, replacing any existing entry with the same key.
    /// Automatically evicts oldest entries if capacity limit is exceeded.
    pub fn insert(&mut self, key: SummaryCacheKey, entry: SummaryCacheEntry) {
        self.entries.insert(key, entry);

        // Enforce capacity limit
        if self.entries.len() > DEFAULT_CACHE_CAPACITY {
            let before = self.entries.len();
            self.evict_to_limit(DEFAULT_CACHE_CAPACITY);
            let evicted = before - self.entries.len();
            if evicted > 0 {
                engine_info!(
                    "[summary-cache] Evicted {} oldest entries (capacity: {})",
                    evicted,
                    DEFAULT_CACHE_CAPACITY
                );
            }
        }
    }

    /// Get the number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Iterate over all cache entries (key, entry pairs).
    pub fn iter(&self) -> impl Iterator<Item = (&SummaryCacheKey, &SummaryCacheEntry)> {
        self.entries.iter()
    }

    /// Evict the oldest entries to keep the cache size at or below the limit.
    /// Uses created_at_utc to determine age (lexicographic ordering of ISO 8601 timestamps).
    pub fn evict_to_limit(&mut self, limit: usize) {
        if self.entries.len() <= limit {
            return;
        }

        // Collect all entries with timestamps
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.created_at_utc.clone()))
            .collect();

        // Sort by timestamp (oldest first)
        entries.sort_by(|a, b| a.1.cmp(&b.1));

        // Remove oldest entries until we're at the limit
        let to_remove = self.entries.len() - limit;
        for (key, _) in entries.iter().take(to_remove) {
            self.entries.remove(key);
        }
    }

    /// Evict entries older than the given UTC timestamp.
    /// Uses lexicographic comparison of ISO 8601 timestamps.
    /// Returns the number of entries evicted.
    pub fn evict_older_than(&mut self, cutoff_utc: &str) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|_, entry| entry.created_at_utc.as_str() >= cutoff_utc);
        before - self.entries.len()
    }

    /// Evict entries older than the specified TTL (in seconds).
    /// Returns the number of entries evicted.
    pub fn evict_by_ttl(&mut self, ttl_seconds: u64) -> usize {
        use chrono::{Duration, Utc};

        let now = Utc::now();
        let cutoff = now - Duration::seconds(ttl_seconds as i64);
        let cutoff_str = cutoff.to_rfc3339();

        self.evict_older_than(&cutoff_str)
    }
}

impl Default for SummaryCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a deterministic hash of context key-value pairs.
/// Sorting ensures the hash is independent of insertion order.
pub fn context_hash(context: &[(String, String)]) -> String {
    if context.is_empty() {
        // Consistent hash for empty context
        return "empty".to_string();
    }

    // Sort by key, then by value to ensure deterministic ordering
    let mut sorted: Vec<_> = context.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let mut hasher = Sha256::new();
    for (key, value) in sorted {
        hasher.update(key.as_bytes());
        hasher.update(b"=");
        hasher.update(value.as_bytes());
        hasher.update(b"\n");
    }

    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn cache_starts_empty() {
        let cache = SummaryCache::new();
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn insert_and_lookup() {
        let mut cache = SummaryCache::new();
        let key = SummaryCacheKey {
            content_hash: "hash1".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model1".to_string(),
            context_hash: "ctx1".to_string(),
        };
        let result = ArticleSummaryResult {
            title: "Title".to_string(),
            summary: "Summary".to_string(),
            key_points: vec!["Point 1".to_string()],
            input_tokens: 100,
            output_tokens: 50,
            entities: Default::default(),
        };
        let entry = SummaryCacheEntry {
            result: result.clone(),
            created_at_utc: Utc::now().to_rfc3339(),
        };

        cache.insert(key.clone(), entry.clone());

        assert_eq!(cache.len(), 1);
        let retrieved = cache.lookup(&key);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().result, result);
    }

    #[test]
    fn different_keys_dont_collide() {
        let mut cache = SummaryCache::new();
        let key1 = SummaryCacheKey {
            content_hash: "hash1".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model1".to_string(),
            context_hash: "ctx1".to_string(),
        };
        let key2 = SummaryCacheKey {
            content_hash: "hash2".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model1".to_string(),
            context_hash: "ctx1".to_string(),
        };

        let entry1 = SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "Title1".to_string(),
                summary: "Summary1".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
        };
        let entry2 = SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "Title2".to_string(),
                summary: "Summary2".to_string(),
                key_points: vec![],
                input_tokens: 20,
                output_tokens: 10,
                entities: Default::default(),
            },
            created_at_utc: "2026-01-01T00:00:01Z".to_string(),
        };

        cache.insert(key1.clone(), entry1.clone());
        cache.insert(key2.clone(), entry2.clone());

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.lookup(&key1).unwrap().result.title, "Title1");
        assert_eq!(cache.lookup(&key2).unwrap().result.title, "Title2");
    }

    #[test]
    fn context_hash_empty_is_deterministic() {
        let hash1 = context_hash(&[]);
        let hash2 = context_hash(&[]);
        assert_eq!(hash1, hash2);
        assert!(!hash1.is_empty());
        assert_ne!(
            hash1,
            context_hash(&[("key".to_string(), "value".to_string())])
        );
    }

    #[test]
    fn context_hash_single_entry() {
        let ctx = vec![("key".to_string(), "value".to_string())];
        let hash = context_hash(&ctx);
        assert!(!hash.is_empty());
        assert_ne!(hash, "empty");
    }

    #[test]
    fn context_hash_order_independent() {
        let ctx1 = vec![
            ("key1".to_string(), "value1".to_string()),
            ("key2".to_string(), "value2".to_string()),
        ];
        let ctx2 = vec![
            ("key2".to_string(), "value2".to_string()),
            ("key1".to_string(), "value1".to_string()),
        ];
        assert_eq!(context_hash(&ctx1), context_hash(&ctx2));
    }

    #[test]
    fn context_hash_different_for_different_values() {
        let ctx1 = vec![("key".to_string(), "value1".to_string())];
        let ctx2 = vec![("key".to_string(), "value2".to_string())];
        assert_ne!(context_hash(&ctx1), context_hash(&ctx2));
    }

    #[test]
    fn context_hash_different_for_different_keys() {
        let ctx1 = vec![("key1".to_string(), "value".to_string())];
        let ctx2 = vec![("key2".to_string(), "value".to_string())];
        assert_ne!(context_hash(&ctx1), context_hash(&ctx2));
    }

    #[test]
    fn summary_cache_key_rejects_empty_content_hash() {
        let err =
            SummaryCacheKey::try_new("", PromptId::ArticleSummary, Some(1), Some("model"), &[]);
        assert_eq!(err, Err(SummaryCacheKeyError::EmptyContentHash));
    }

    #[test]
    fn summary_cache_key_requires_metadata() {
        let ctx: Vec<(String, String)> = Vec::new();
        assert_eq!(
            SummaryCacheKey::try_new("hash", PromptId::ArticleSummary, None, Some("model"), &ctx,),
            Err(SummaryCacheKeyError::MissingPromptVersion)
        );
        assert_eq!(
            SummaryCacheKey::try_new("hash", PromptId::ArticleSummary, Some(1), None, &ctx,),
            Err(SummaryCacheKeyError::MissingModelId)
        );
    }

    #[test]
    fn summary_cache_key_changes_when_prompt_version_changes() {
        let ctx: Vec<(String, String)> = Vec::new();
        let key_v1 = SummaryCacheKey::try_new(
            "hash",
            PromptId::ArticleSummary,
            Some(1),
            Some("model"),
            &ctx,
        )
        .unwrap();
        let key_v2 = SummaryCacheKey::try_new(
            "hash",
            PromptId::ArticleSummary,
            Some(2),
            Some("model"),
            &ctx,
        )
        .unwrap();
        assert_ne!(key_v1, key_v2);
    }

    #[test]
    fn summary_cache_key_changes_when_model_changes() {
        let ctx: Vec<(String, String)> = Vec::new();
        let key_a = SummaryCacheKey::try_new(
            "hash",
            PromptId::ArticleSummary,
            Some(1),
            Some("model-a"),
            &ctx,
        )
        .unwrap();
        let key_b = SummaryCacheKey::try_new(
            "hash",
            PromptId::ArticleSummary,
            Some(1),
            Some("model-b"),
            &ctx,
        )
        .unwrap();
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn summary_cache_key_changes_when_context_changes() {
        let ctx1 = vec![("alpha".to_string(), "1".to_string())];
        let ctx2 = vec![("alpha".to_string(), "2".to_string())];
        let key1 = SummaryCacheKey::try_new(
            "hash",
            PromptId::ArticleSummary,
            Some(1),
            Some("model"),
            &ctx1,
        )
        .unwrap();
        let key2 = SummaryCacheKey::try_new(
            "hash",
            PromptId::ArticleSummary,
            Some(1),
            Some("model"),
            &ctx2,
        )
        .unwrap();
        assert_ne!(key1, key2);
    }

    #[test]
    fn evict_to_limit_removes_oldest() {
        let mut cache = SummaryCache::new();
        let base_key = SummaryCacheKey {
            content_hash: "base".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        };
        let base_result = ArticleSummaryResult {
            title: "Title".to_string(),
            summary: "Summary".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        };

        // Insert entries with different timestamps
        for i in 0..5 {
            let key = SummaryCacheKey {
                content_hash: format!("hash{}", i),
                ..base_key.clone()
            };
            let entry = SummaryCacheEntry {
                result: base_result.clone(),
                created_at_utc: format!("2026-01-01T00:00:0{}Z", i),
            };
            cache.insert(key, entry);
        }

        assert_eq!(cache.len(), 5);

        // Evict to limit of 3
        cache.evict_to_limit(3);
        assert_eq!(cache.len(), 3);

        // The oldest two (hash0 and hash1) should be removed
        let key0 = SummaryCacheKey {
            content_hash: "hash0".to_string(),
            ..base_key.clone()
        };
        let key4 = SummaryCacheKey {
            content_hash: "hash4".to_string(),
            ..base_key.clone()
        };
        assert!(cache.lookup(&key0).is_none());
        assert!(cache.lookup(&key4).is_some());
    }

    #[test]
    fn evict_to_limit_does_nothing_when_below_limit() {
        let mut cache = SummaryCache::new();
        let key = SummaryCacheKey {
            content_hash: "hash".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        };
        let entry = SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "Title".to_string(),
                summary: "Summary".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
        };
        cache.insert(key.clone(), entry);

        cache.evict_to_limit(100);
        assert_eq!(cache.len(), 1);
        assert!(cache.lookup(&key).is_some());
    }

    #[test]
    fn evict_older_than_removes_old_entries() {
        let mut cache = SummaryCache::new();
        let base_key = SummaryCacheKey {
            content_hash: "base".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        };
        let base_result = ArticleSummaryResult {
            title: "Title".to_string(),
            summary: "Summary".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        };

        let key_old = SummaryCacheKey {
            content_hash: "old".to_string(),
            ..base_key.clone()
        };
        let entry_old = SummaryCacheEntry {
            result: base_result.clone(),
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
        };
        cache.insert(key_old.clone(), entry_old);

        let key_new = SummaryCacheKey {
            content_hash: "new".to_string(),
            ..base_key.clone()
        };
        let entry_new = SummaryCacheEntry {
            result: base_result.clone(),
            created_at_utc: "2026-01-02T00:00:00Z".to_string(),
        };
        cache.insert(key_new.clone(), entry_new);

        cache.evict_older_than("2026-01-01T12:00:00Z");

        assert_eq!(cache.len(), 1);
        assert!(cache.lookup(&key_old).is_none());
        assert!(cache.lookup(&key_new).is_some());
    }

    #[test]
    fn clear_removes_all_entries() {
        let mut cache = SummaryCache::new();
        let key = SummaryCacheKey {
            content_hash: "hash".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        };
        let entry = SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "Title".to_string(),
                summary: "Summary".to_string(),
                key_points: vec![],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
        };
        cache.insert(key, entry);

        assert_eq!(cache.len(), 1);
        cache.clear();
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn capacity_enforcement_evicts_oldest() {
        engine_logging::initialize_for_tests();

        let mut cache = SummaryCache::new();
        let base_key = SummaryCacheKey {
            content_hash: "base".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        };
        let base_result = ArticleSummaryResult {
            title: "Title".to_string(),
            summary: "Summary".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        };

        // Insert DEFAULT_CACHE_CAPACITY + 10 entries with sequential timestamps
        for i in 0..(DEFAULT_CACHE_CAPACITY + 10) {
            let key = SummaryCacheKey {
                content_hash: format!("hash{}", i),
                ..base_key.clone()
            };
            // Use microseconds to ensure unique ordering
            let entry = SummaryCacheEntry {
                result: base_result.clone(),
                created_at_utc: format!("2026-01-01T00:00:00.{:06}Z", i),
            };
            cache.insert(key, entry);
        }

        // Should have been evicted to capacity
        assert_eq!(cache.len(), DEFAULT_CACHE_CAPACITY);

        // Oldest 10 should be gone
        for i in 0..10 {
            let key = SummaryCacheKey {
                content_hash: format!("hash{}", i),
                ..base_key.clone()
            };
            assert!(cache.lookup(&key).is_none(), "hash{} should be evicted", i);
        }

        // Newest should remain
        let key_last = SummaryCacheKey {
            content_hash: format!("hash{}", DEFAULT_CACHE_CAPACITY + 9),
            ..base_key.clone()
        };
        assert!(cache.lookup(&key_last).is_some());
    }

    #[test]
    fn evict_by_ttl_removes_expired_entries() {
        use chrono::{Duration, Utc};

        let mut cache = SummaryCache::new();
        let base_key = SummaryCacheKey {
            content_hash: "base".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 1,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        };
        let base_result = ArticleSummaryResult {
            title: "Title".to_string(),
            summary: "Summary".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        };

        // Insert old entry (2 hours old)
        let key_old = SummaryCacheKey {
            content_hash: "old".to_string(),
            ..base_key.clone()
        };
        let old_time = Utc::now() - Duration::hours(2);
        let entry_old = SummaryCacheEntry {
            result: base_result.clone(),
            created_at_utc: old_time.to_rfc3339(),
        };
        cache.insert(key_old.clone(), entry_old);

        // Insert recent entry (30 minutes old)
        let key_recent = SummaryCacheKey {
            content_hash: "recent".to_string(),
            ..base_key.clone()
        };
        let recent_time = Utc::now() - Duration::minutes(30);
        let entry_recent = SummaryCacheEntry {
            result: base_result.clone(),
            created_at_utc: recent_time.to_rfc3339(),
        };
        cache.insert(key_recent.clone(), entry_recent);

        // Evict entries older than 1 hour
        let evicted = cache.evict_by_ttl(3600);

        assert_eq!(evicted, 1);
        assert_eq!(cache.len(), 1);
        assert!(cache.lookup(&key_old).is_none());
        assert!(cache.lookup(&key_recent).is_some());
    }
}
