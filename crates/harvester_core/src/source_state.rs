use chrono::{DateTime, Utc};
use harvester_engine::{SourceId, SourceKind};
use std::collections::BTreeMap;

/// Per-source poll statistics for a single poll cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePollStat {
    pub source_id: SourceId,
    pub kind: SourceKind,
    /// Raw count from the API or feed before any filtering.
    pub parsed: usize,
    /// Count filtered by the seen-set (cross-cycle dedup).
    pub dedup_filtered: usize,
    /// Final count emitted into the pipeline (after dedup + limit cap).
    pub emitted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceInstanceState {
    pub last_polled: Option<DateTime<Utc>>,
    pub last_url_count: usize,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceStateIndex {
    states: BTreeMap<SourceId, SourceInstanceState>,
    poll_in_progress: bool,
    poll_total: Option<usize>,
    poll_completed: usize,
    /// Stats accumulated for the current poll cycle; cleared when a new poll starts.
    poll_stats: Vec<SourcePollStat>,
    /// Snapshot of stats from the last *completed* poll cycle. Persists across poll starts.
    last_completed_poll_stats: Vec<SourcePollStat>,
}

impl SourceStateIndex {
    pub fn record_source_poll(&mut self, id: &SourceId, url_count: usize) {
        let entry = self.states.entry(id.clone()).or_default();
        entry.last_polled = Some(Utc::now());
        entry.last_url_count = url_count;
        entry.last_error = None;
    }

    pub fn record_poll_stat(&mut self, stat: SourcePollStat) {
        self.poll_stats.push(stat);
        self.poll_completed += 1;
    }

    pub fn poll_stats(&self) -> &[SourcePollStat] {
        &self.poll_stats
    }

    pub fn record_source_error(&mut self, id: &SourceId, error: String) {
        let entry = self.states.entry(id.clone()).or_default();
        entry.last_polled = Some(Utc::now());
        entry.last_error = Some(error);
        if self.poll_in_progress {
            self.poll_completed += 1;
        }
    }

    pub fn source_state(&self, id: &SourceId) -> Option<&SourceInstanceState> {
        self.states.get(id)
    }

    pub fn start_poll(&mut self) -> bool {
        if self.poll_in_progress {
            false
        } else {
            self.poll_in_progress = true;
            self.poll_total = None;
            self.poll_completed = 0;
            self.poll_stats.clear();
            true
        }
    }

    pub fn set_poll_total(&mut self, total: usize) {
        self.poll_total = Some(total);
    }

    pub fn end_poll(&mut self) {
        self.last_completed_poll_stats = self.poll_stats.clone();
        self.poll_in_progress = false;
        self.poll_total = None;
    }

    pub fn is_poll_in_progress(&self) -> bool {
        self.poll_in_progress
    }

    pub fn last_completed_poll_stats(&self) -> &[SourcePollStat] {
        &self.last_completed_poll_stats
    }

    pub fn poll_progress(&self) -> Option<(usize, usize)> {
        let total = self.poll_total?;
        if !self.poll_in_progress || total == 0 {
            return None;
        }
        Some((self.poll_completed.min(total), total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::SourceId;

    #[test]
    fn poll_guard_allows_once() {
        let mut index = SourceStateIndex::default();
        assert!(index.start_poll());
        assert!(!index.start_poll());
        index.end_poll();
        assert!(index.start_poll());
    }

    #[test]
    fn records_poll_stats() {
        let mut index = SourceStateIndex::default();
        let id = SourceId::new("source").expect("valid");
        index.record_source_poll(&id, 5);
        let state = index.source_state(&id).expect("present");
        assert_eq!(state.last_url_count, 5);
        assert!(state.last_polled.is_some());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn records_error_message() {
        let mut index = SourceStateIndex::default();
        let id = SourceId::new("source").expect("valid");
        index.record_source_error(&id, "boom".to_string());
        let state = index.source_state(&id).expect("present");
        assert_eq!(state.last_error.as_deref(), Some("boom"));
        assert!(state.last_polled.is_some());
    }

    fn make_stat(source_id: &str, emitted: usize) -> SourcePollStat {
        SourcePollStat {
            source_id: SourceId::new(source_id).expect("valid"),
            kind: SourceKind::Rss,
            parsed: emitted,
            dedup_filtered: 0,
            emitted,
        }
    }

    #[test]
    fn last_completed_poll_stats_empty_before_any_poll() {
        let index = SourceStateIndex::default();
        assert!(index.last_completed_poll_stats().is_empty());
    }

    #[test]
    fn last_completed_poll_stats_set_after_end_poll() {
        let mut index = SourceStateIndex::default();
        index.start_poll();
        index.record_poll_stat(make_stat("s1", 3));
        index.end_poll();
        let stats = index.last_completed_poll_stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].emitted, 3);
    }

    #[test]
    fn last_completed_poll_stats_preserved_during_next_poll() {
        let mut index = SourceStateIndex::default();
        // First poll
        index.start_poll();
        index.record_poll_stat(make_stat("s1", 3));
        index.end_poll();
        // Second poll starts — live accumulator clears, snapshot must remain
        index.start_poll();
        assert!(
            index.poll_stats().is_empty(),
            "live accumulator should be cleared"
        );
        let stats = index.last_completed_poll_stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].emitted, 3);
    }

    #[test]
    fn last_completed_poll_stats_replaced_after_second_end_poll() {
        let mut index = SourceStateIndex::default();
        // First poll
        index.start_poll();
        index.record_poll_stat(make_stat("s1", 3));
        index.end_poll();
        // Second poll
        index.start_poll();
        index.record_poll_stat(make_stat("s2", 7));
        index.end_poll();
        let stats = index.last_completed_poll_stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].source_id, SourceId::new("s2").expect("valid"));
        assert_eq!(stats[0].emitted, 7);
    }

    #[test]
    fn poll_progress_none_when_idle() {
        let index = SourceStateIndex::default();
        assert_eq!(index.poll_progress(), None);
    }

    #[test]
    fn poll_progress_available_after_set_total() {
        let mut index = SourceStateIndex::default();
        index.start_poll();
        index.set_poll_total(4);
        assert_eq!(index.poll_progress(), Some((0, 4)));
    }

    #[test]
    fn poll_progress_increments_on_stat() {
        let mut index = SourceStateIndex::default();
        index.start_poll();
        index.set_poll_total(3);
        index.record_poll_stat(make_stat("s1", 1));
        assert_eq!(index.poll_progress(), Some((1, 3)));
    }

    #[test]
    fn poll_progress_increments_on_error() {
        let mut index = SourceStateIndex::default();
        let id = SourceId::new("source").expect("valid");
        index.start_poll();
        index.set_poll_total(2);
        index.record_source_error(&id, "boom".to_string());
        assert_eq!(index.poll_progress(), Some((1, 2)));
    }

    #[test]
    fn poll_progress_not_incremented_on_error_outside_poll() {
        let mut index = SourceStateIndex::default();
        let id = SourceId::new("source").expect("valid");
        index.record_source_error(&id, "boom".to_string());
        assert_eq!(index.poll_progress(), None);
    }

    #[test]
    fn poll_progress_none_when_total_is_zero() {
        let mut index = SourceStateIndex::default();
        index.start_poll();
        index.set_poll_total(0);
        assert_eq!(index.poll_progress(), None);
    }

    #[test]
    fn poll_progress_none_after_end_poll() {
        let mut index = SourceStateIndex::default();
        index.start_poll();
        index.set_poll_total(2);
        index.record_poll_stat(make_stat("s1", 1));
        index.end_poll();
        assert_eq!(index.poll_progress(), None);
    }
}
