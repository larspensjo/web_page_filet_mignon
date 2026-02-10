use chrono::{DateTime, Utc};
use harvester_engine::SourceId;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInstanceState {
    pub last_polled: Option<DateTime<Utc>>,
    pub last_url_count: usize,
    pub last_error: Option<String>,
}

impl Default for SourceInstanceState {
    fn default() -> Self {
        Self {
            last_polled: None,
            last_url_count: 0,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStateIndex {
    states: BTreeMap<SourceId, SourceInstanceState>,
    poll_in_progress: bool,
}

impl Default for SourceStateIndex {
    fn default() -> Self {
        Self {
            states: BTreeMap::new(),
            poll_in_progress: false,
        }
    }
}

impl SourceStateIndex {
    pub fn record_source_poll(&mut self, id: &SourceId, url_count: usize) {
        let entry = self.states.entry(id.clone()).or_default();
        entry.last_polled = Some(Utc::now());
        entry.last_url_count = url_count;
        entry.last_error = None;
    }

    pub fn record_source_error(&mut self, id: &SourceId, error: String) {
        let entry = self.states.entry(id.clone()).or_default();
        entry.last_polled = Some(Utc::now());
        entry.last_error = Some(error);
    }

    pub fn source_state(&self, id: &SourceId) -> Option<&SourceInstanceState> {
        self.states.get(id)
    }

    pub fn start_poll(&mut self) -> bool {
        if self.poll_in_progress {
            false
        } else {
            self.poll_in_progress = true;
            true
        }
    }

    pub fn end_poll(&mut self) {
        self.poll_in_progress = false;
    }

    pub fn is_poll_in_progress(&self) -> bool {
        self.poll_in_progress
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
}
