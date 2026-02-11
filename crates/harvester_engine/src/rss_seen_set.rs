use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};

const MAX_GUIDS_PER_FEED: usize = 10_000;
const EVICT_BATCH: usize = MAX_GUIDS_PER_FEED / 5;

/// Tracks seen GUIDs per RSS source to prevent reprocessing.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RssSeenSet {
    map: BTreeMap<String, SeenGuids>,
}

impl RssSeenSet {
    /// Creates an empty seen-set.
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    /// Returns whether the GUID is recorded for `source_id`.
    pub fn is_seen(&self, source_id: &str, guid: &str) -> bool {
        self.map
            .get(source_id)
            .map(|guids| guids.contains(guid))
            .unwrap_or(false)
    }

    /// Marks a GUID as seen for `source_id`. Returns true when the GUID was new.
    pub fn mark_seen(&mut self, source_id: &str, guid: &str) -> bool {
        self.map
            .entry(source_id.to_string())
            .or_default()
            .mark(guid)
    }

    /// Filters out entries whose GUIDs were previously marked seen.
    /// All GUIDs are marked seen regardless of the return value.
    pub fn filter_unseen_entries(
        &mut self,
        source_id: &str,
        entries: Vec<crate::rss_parse::FeedEntry>,
    ) -> Vec<crate::rss_parse::FeedEntry> {
        let mut unseen = Vec::new();

        for entry in entries {
            if self.mark_seen(source_id, &entry.guid) {
                unseen.push(entry);
            }
        }

        unseen
    }
}

impl PartialEq for RssSeenSet {
    fn eq(&self, other: &Self) -> bool {
        self.map == other.map
    }
}

#[derive(Clone, Debug, Default)]
struct SeenGuids {
    guids: VecDeque<String>,
    lookup: HashSet<String>,
}

impl SeenGuids {
    fn contains(&self, guid: &str) -> bool {
        self.lookup.contains(guid)
    }

    fn mark(&mut self, guid: &str) -> bool {
        if self.lookup.contains(guid) {
            return false;
        }

        let owned_guid = guid.to_string();
        if self.guids.len() >= MAX_GUIDS_PER_FEED {
            self.evict_oldest();
        }

        self.guids.push_back(owned_guid.clone());
        self.lookup.insert(owned_guid);
        true
    }

    fn evict_oldest(&mut self) {
        let to_remove = EVICT_BATCH.min(self.guids.len());
        for _ in 0..to_remove {
            if let Some(old_guid) = self.guids.pop_front() {
                self.lookup.remove(&old_guid);
            } else {
                break;
            }
        }
    }
}

impl Serialize for SeenGuids {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.guids.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SeenGuids {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let guids = VecDeque::deserialize(deserializer)?;
        let lookup = guids.iter().cloned().collect();
        Ok(SeenGuids { guids, lookup })
    }
}

impl PartialEq for SeenGuids {
    fn eq(&self, other: &Self) -> bool {
        self.guids == other.guids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rss_parse::FeedEntry;

    fn feed_entry(guid: &str, url: Option<&str>) -> FeedEntry {
        FeedEntry {
            guid: guid.to_string(),
            url: url.map(|value| value.to_string()),
            title: None,
            published: None,
        }
    }

    #[test]
    fn filter_unseen_entries_returns_all_first_time() {
        let mut set = RssSeenSet::new();
        let entries = vec![
            feed_entry("guid-1", Some("https://example.com/1")),
            feed_entry("guid-2", Some("https://example.com/2")),
        ];

        let unseen = set.filter_unseen_entries("source-a", entries.clone());
        assert_eq!(unseen.len(), entries.len());
        assert!(set.is_seen("source-a", "guid-1"));
        assert!(set.is_seen("source-a", "guid-2"));
    }

    #[test]
    fn filter_unseen_entries_returns_empty_for_same_entries() {
        let mut set = RssSeenSet::new();
        let entries = vec![feed_entry("guid-1", Some("https://example.com/1"))];
        let _ = set.filter_unseen_entries("source-a", entries.clone());
        let second = set.filter_unseen_entries("source-a", entries);
        assert!(second.is_empty());
    }

    #[test]
    fn filter_unseen_entries_handles_mixed_seen_and_new() {
        let mut set = RssSeenSet::new();
        let first_batch = vec![feed_entry("guid-1", Some("https://example.com/1"))];
        let _ = set.filter_unseen_entries("source-a", first_batch);

        let second_batch = vec![
            feed_entry("guid-1", Some("https://example.com/1")),
            feed_entry("guid-2", Some("https://example.com/2")),
        ];
        let unseen = set.filter_unseen_entries("source-a", second_batch);
        assert_eq!(unseen.len(), 1);
        assert_eq!(unseen[0].guid, "guid-2");
    }

    #[test]
    fn no_url_entries_are_still_marked_seen() {
        let mut set = RssSeenSet::new();
        let no_url = feed_entry("no-url", None);
        let _ = set.filter_unseen_entries("source-a", vec![no_url.clone()]);
        let second = set.filter_unseen_entries("source-a", vec![no_url]);
        assert!(second.is_empty());
    }

    #[test]
    fn eviction_removes_oldest_guids_at_capacity() {
        let mut set = RssSeenSet::new();
        let source_id = "source-evict";
        let total = MAX_GUIDS_PER_FEED + 1;
        let entries: Vec<FeedEntry> = (0..total)
            .map(|index| feed_entry(&format!("guid-{}", index), Some("https://example.com/")))
            .collect();

        let _ = set.filter_unseen_entries(source_id, entries);
        assert!(!set.is_seen(source_id, "guid-0"));
        assert!(set.is_seen(source_id, &format!("guid-{}", total - 1)));
    }

    #[test]
    fn different_sources_have_independent_sets() {
        let mut set = RssSeenSet::new();
        let entry_a = feed_entry("shared", Some("https://example.com/"));
        let entry_b = feed_entry("shared", Some("https://example.com/"));

        let _ = set.filter_unseen_entries("source-a", vec![entry_a]);
        let unseen = set.filter_unseen_entries("source-b", vec![entry_b]);
        assert_eq!(unseen.len(), 1);
    }
}
