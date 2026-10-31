use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::SourceId;

/// Minimum gap between successful searches of one Brave source.
///
/// Brave bills per request, and extra same-day runs mostly return URLs the
/// seen-set already holds. 20 hours rather than 24 lets a daily run drift
/// earlier without skipping a day.
pub const BRAVE_MIN_SEARCH_INTERVAL_HOURS: i64 = 20;

/// Records when each Brave source last completed a search Brave accepted.
///
/// Failed requests are never recorded, so a source that failed is searched
/// again on the next run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BraveSearchLog {
    #[serde(default)]
    last_success_utc: BTreeMap<SourceId, DateTime<Utc>>,
}

impl BraveSearchLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_success(&mut self, id: &SourceId, at: DateTime<Utc>) {
        self.last_success_utc.insert(id.clone(), at);
    }

    /// Age of the source's last successful search when it is too recent to
    /// search again. A timestamp in the future (clock change) counts as due.
    pub fn recent_search_age(&self, id: &SourceId, now: DateTime<Utc>) -> Option<Duration> {
        let age = now.signed_duration_since(*self.last_success_utc.get(id)?);
        (age >= Duration::zero() && age < Duration::hours(BRAVE_MIN_SEARCH_INTERVAL_HOURS))
            .then_some(age)
    }
}

/// Human-readable reason for skipping a source searched `age` ago.
pub fn brave_skip_reason(age: Duration) -> String {
    let searched = match age.num_hours() {
        0 => format!("{} min ago", age.num_minutes()),
        1 => "1 hour ago".to_string(),
        hours => format!("{hours} hours ago"),
    };
    format!(
        "searched {searched}; Brave sources are searched at most once per \
         {BRAVE_MIN_SEARCH_INTERVAL_HOURS} hours"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> SourceId {
        SourceId::new(value).expect("valid")
    }

    fn at(hours: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_800_000_000, 0).expect("valid") + Duration::hours(hours)
    }

    #[test]
    fn never_searched_source_is_due() {
        let log = BraveSearchLog::new();
        assert_eq!(log.recent_search_age(&id("brave-a"), at(0)), None);
    }

    #[test]
    fn source_searched_within_interval_is_skipped_with_its_age() {
        let mut log = BraveSearchLog::new();
        log.record_success(&id("brave-a"), at(0));
        assert_eq!(
            log.recent_search_age(&id("brave-a"), at(5)),
            Some(Duration::hours(5))
        );
        assert_eq!(log.recent_search_age(&id("brave-b"), at(5)), None);
    }

    #[test]
    fn source_is_due_again_once_the_interval_has_passed() {
        let mut log = BraveSearchLog::new();
        log.record_success(&id("brave-a"), at(0));
        assert!(log
            .recent_search_age(&id("brave-a"), at(BRAVE_MIN_SEARCH_INTERVAL_HOURS - 1))
            .is_some());
        assert_eq!(
            log.recent_search_age(&id("brave-a"), at(BRAVE_MIN_SEARCH_INTERVAL_HOURS)),
            None
        );
    }

    #[test]
    fn future_timestamp_counts_as_due() {
        let mut log = BraveSearchLog::new();
        log.record_success(&id("brave-a"), at(3));
        assert_eq!(log.recent_search_age(&id("brave-a"), at(0)), None);
    }

    #[test]
    fn roundtrips_through_ron() {
        let mut log = BraveSearchLog::new();
        log.record_success(&id("brave-a"), at(0));
        let text = ron::to_string(&log).expect("serialize");
        let loaded: BraveSearchLog = ron::from_str(&text).expect("deserialize");
        assert_eq!(loaded, log);
    }

    #[test]
    fn skip_reason_names_the_age_and_the_interval() {
        assert_eq!(
            brave_skip_reason(Duration::hours(3) + Duration::minutes(20)),
            "searched 3 hours ago; Brave sources are searched at most once per 20 hours"
        );
        assert!(brave_skip_reason(Duration::minutes(12)).starts_with("searched 12 min ago;"));
        assert!(brave_skip_reason(Duration::minutes(75)).starts_with("searched 1 hour ago;"));
    }
}
