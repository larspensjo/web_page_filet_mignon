use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

const MAX_ENTRIES: usize = 10_000;
const EVICT_BATCH: usize = MAX_ENTRIES / 5;

/// Tracks seen URLs for Brave News sources to prevent reprocessing.
///
/// URLs are normalized via `normalize_for_dedupe` which mirrors
/// `harvester_core::normalize_url_for_dedupe` — the same logic used by
/// the reducer. If that function changes, this one must be updated too.
/// Capacity is bounded with FIFO eviction.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BraveSeenSet {
    #[serde(default)]
    entries: VecDeque<String>,
    #[serde(skip)]
    lookup: HashSet<String>,
}

impl BraveSeenSet {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            lookup: HashSet::new(),
        }
    }

    /// Rebuild the lookup set from the entries deque.
    /// Must be called after deserialization.
    pub fn rebuild_lookup(&mut self) {
        self.lookup = self.entries.iter().cloned().collect();
    }

    /// Check whether a normalized URL has been seen.
    pub fn is_seen(&self, normalized_url: &str) -> bool {
        self.lookup.contains(normalized_url)
    }

    /// Mark a normalized URL as seen. Returns `true` if the URL was new.
    pub fn mark_seen(&mut self, normalized_url: &str) -> bool {
        if self.lookup.contains(normalized_url) {
            return false;
        }
        if self.entries.len() >= MAX_ENTRIES {
            self.evict_oldest();
        }
        let owned = normalized_url.to_string();
        self.entries.push_back(owned.clone());
        self.lookup.insert(owned);
        true
    }

    /// Filter a list of URLs, returning only those not previously seen.
    /// Normalizes each URL before checking. All new URLs are marked seen.
    pub fn filter_unseen(&mut self, urls: Vec<String>) -> Vec<String> {
        let mut unseen = Vec::new();
        for url in urls {
            let normalized = normalize_for_dedupe(&url);
            if self.mark_seen(&normalized) {
                unseen.push(url);
            }
        }
        unseen
    }

    fn evict_oldest(&mut self) {
        let to_remove = EVICT_BATCH.min(self.entries.len());
        for _ in 0..to_remove {
            if let Some(old) = self.entries.pop_front() {
                self.lookup.remove(&old);
            }
        }
    }
}

/// Mirrors `harvester_core::normalize_url_for_dedupe` exactly.
/// Must be kept in sync if that function changes.
pub fn normalize_for_dedupe(url: &str) -> String {
    url.trim().to_lowercase().trim_end_matches('/').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_seen_returns_true_for_new_url() {
        let mut set = BraveSeenSet::new();
        assert!(set.mark_seen("https://example.com/article"));
    }

    #[test]
    fn mark_seen_returns_false_for_duplicate() {
        let mut set = BraveSeenSet::new();
        set.mark_seen("https://example.com/article");
        assert!(!set.mark_seen("https://example.com/article"));
    }

    #[test]
    fn filter_unseen_removes_duplicates() {
        let mut set = BraveSeenSet::new();
        set.mark_seen(&normalize_for_dedupe("https://example.com/old"));
        let urls = vec![
            "https://example.com/old".to_string(),
            "https://example.com/new".to_string(),
        ];
        let unseen = set.filter_unseen(urls);
        assert_eq!(unseen, vec!["https://example.com/new"]);
    }

    #[test]
    fn filter_unseen_uses_same_normalization_as_reducer() {
        let mut set = BraveSeenSet::new();
        set.mark_seen(&normalize_for_dedupe("https://Example.COM/Article/"));
        let urls = vec!["https://example.com/article".to_string()];
        let unseen = set.filter_unseen(urls);
        assert!(unseen.is_empty(), "should match despite casing/slash differences");
    }

    #[test]
    fn eviction_removes_oldest_at_capacity() {
        let mut set = BraveSeenSet::new();
        for i in 0..MAX_ENTRIES + 1 {
            set.mark_seen(&format!("https://example.com/{}", i));
        }
        assert!(!set.is_seen("https://example.com/0"));
        assert!(set.is_seen(&format!("https://example.com/{}", MAX_ENTRIES)));
    }

    #[test]
    fn roundtrip_serialization() {
        let mut set = BraveSeenSet::new();
        set.mark_seen("https://example.com/1");
        set.mark_seen("https://example.com/2");

        let ron_str = ron::to_string(&set).expect("serialize");
        let mut loaded: BraveSeenSet = ron::from_str(&ron_str).expect("deserialize");
        loaded.rebuild_lookup();

        assert!(loaded.is_seen("https://example.com/1"));
        assert!(loaded.is_seen("https://example.com/2"));
        assert!(!loaded.is_seen("https://example.com/3"));
    }

    #[test]
    fn filter_unseen_then_limit_matches_rss_semantics() {
        // 4 results, 2 already seen, max=1 → should still emit 1 (not 0).
        let mut set = BraveSeenSet::new();
        set.mark_seen(&normalize_for_dedupe("https://example.com/old-1"));
        set.mark_seen(&normalize_for_dedupe("https://example.com/old-2"));

        let urls = vec![
            "https://example.com/old-1".to_string(),
            "https://example.com/old-2".to_string(),
            "https://example.com/new-1".to_string(),
            "https://example.com/new-2".to_string(),
        ];
        let deduped = set.filter_unseen(urls);
        assert_eq!(deduped.len(), 2);

        let emitted: Vec<_> = deduped.into_iter().take(1).collect();
        assert_eq!(emitted, vec!["https://example.com/new-1"]);
    }
}
