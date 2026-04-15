/// Check whether fetched_utc matches the date range filters.
///
/// Returns `true` when no filters are specified. When filters are present, an
/// article without a `fetched_utc` value never matches.
pub(crate) fn date_in_range(
    fetched_utc: Option<&str>,
    date_from: Option<&str>,
    date_to: Option<&str>,
) -> bool {
    if date_from.is_none() && date_to.is_none() {
        return true;
    }
    match fetched_utc {
        None => false,
        Some(ts) => {
            if let Some(from) = date_from {
                if ts < from {
                    return false;
                }
            }
            if let Some(to) = date_to {
                if ts > to {
                    return false;
                }
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_filters_always_matches() {
        assert!(date_in_range(None, None, None));
        assert!(date_in_range(Some("2026-01-01T00:00:00Z"), None, None));
    }

    #[test]
    fn missing_timestamp_never_matches_filtered_range() {
        assert!(!date_in_range(None, Some("2026-01-01"), None));
        assert!(!date_in_range(None, None, Some("2026-12-31")));
    }

    #[test]
    fn inclusive_bounds() {
        assert!(date_in_range(
            Some("2026-04-01T00:00:00Z"),
            Some("2026-04-01"),
            Some("2026-04-30")
        ));
        assert!(!date_in_range(
            Some("2026-03-31T23:59:59Z"),
            Some("2026-04-01"),
            None
        ));
        assert!(!date_in_range(
            Some("2026-05-01T00:00:00Z"),
            None,
            Some("2026-04-30")
        ));
    }
}
