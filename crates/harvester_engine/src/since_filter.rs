use chrono::{DateTime, Utc};

/// Returns `true` if the job/article should be included in the Since Checkpoint view.
///
/// Used by Archive, Briefing, and the UI tab to keep semantics consistent.
pub fn passes_since_filter_dt(
    fetched_utc: Option<DateTime<Utc>>,
    since: Option<DateTime<Utc>>,
) -> bool {
    match (fetched_utc, since) {
        (_, None) => true,
        (None, Some(_)) => true, // include if missing (consistent with archive/briefing)
        (Some(t), Some(s)) => t >= s,
    }
}

/// Convenience wrapper for callers that hold a raw string.
///
/// Encapsulates parse-and-fallback so callers don't repeat the rfc3339 logic.
/// A malformed string is treated as `None` (include by default).
pub fn passes_since_filter_str(
    fetched_utc_str: Option<&str>,
    since: Option<DateTime<Utc>>,
) -> bool {
    let dt = fetched_utc_str
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    passes_since_filter_dt(dt, since)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    // passes_since_filter_dt

    #[test]
    fn dt_since_none_always_true() {
        assert!(passes_since_filter_dt(None, None));
        assert!(passes_since_filter_dt(Some(t(2024, 1, 1)), None));
    }

    #[test]
    fn dt_fetched_none_since_some_includes() {
        assert!(passes_since_filter_dt(None, Some(t(2024, 6, 1))));
    }

    #[test]
    fn dt_fetched_at_or_after_since_includes() {
        let since = t(2024, 6, 1);
        assert!(passes_since_filter_dt(Some(t(2024, 6, 1)), Some(since)));
        assert!(passes_since_filter_dt(Some(t(2024, 6, 2)), Some(since)));
    }

    #[test]
    fn dt_fetched_before_since_excludes() {
        let since = t(2024, 6, 1);
        assert!(!passes_since_filter_dt(Some(t(2024, 5, 31)), Some(since)));
    }

    // passes_since_filter_str

    #[test]
    fn str_since_none_always_true() {
        assert!(passes_since_filter_str(None, None));
        assert!(passes_since_filter_str(Some("2024-01-01T00:00:00Z"), None));
    }

    #[test]
    fn str_fetched_none_since_some_includes() {
        assert!(passes_since_filter_str(None, Some(t(2024, 6, 1))));
    }

    #[test]
    fn str_valid_rfc3339_same_result_as_dt() {
        let since = t(2024, 6, 1);
        assert!(passes_since_filter_str(
            Some("2024-06-01T00:00:00Z"),
            Some(since)
        ));
        assert!(passes_since_filter_str(
            Some("2024-06-02T00:00:00Z"),
            Some(since)
        ));
        assert!(!passes_since_filter_str(
            Some("2024-05-31T00:00:00Z"),
            Some(since)
        ));
    }

    #[test]
    fn str_malformed_treats_as_none_includes() {
        assert!(passes_since_filter_str(
            Some("not-a-date"),
            Some(t(2024, 6, 1))
        ));
        assert!(passes_since_filter_str(Some(""), Some(t(2024, 6, 1))));
    }
}
