use crate::briefing::ArticleSummaryResult;
use chrono::{DateTime, Utc};

/// Mirrors the engine's default `max_input_bytes` (see harvester_app/runner config).
pub const BRIEFING_SNAPSHOT_BUDGET_BYTES: usize = 100_000;

/// One candidate article for the snapshot, in corpus order.
pub struct SnapshotArticle<'a> {
    pub url: &'a str,
    /// RFC3339 timestamp from triage metadata; `None`/malformed => always in-window.
    pub fetched_utc: Option<&'a str>,
    /// Completed summary, or `None` if this in-window article has no settled summary.
    pub summary: Option<&'a ArticleSummaryResult>,
}

/// The frozen snapshot text plus the counts surfaced in Session Info.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingSnapshot {
    pub text: String,
    pub included_count: usize,
    pub skipped_count: usize,
    pub dropped_count: usize,
    pub truncated: bool,
    pub coverage_window_label: String,
}

/// Build the frozen snapshot.
///
/// - Coverage window: with `since_utc = Some`, articles whose `fetched_utc` parses and is
///   strictly older are excluded entirely (not counted as skipped). Missing/malformed
///   `fetched_utc` is always included (matches the existing briefing loader policy).
/// - In-window articles with no completed summary increment `skipped_count`.
/// - Whole `[A#]` entries are appended in order until the next entry would exceed
///   `budget_bytes`; remaining in-window-with-summary entries increment `dropped_count`
///   and `truncated` becomes true. If the first entry alone exceeds the budget, it is
///   still emitted whole and marked truncated. Entries are never split (UTF-8 safe by construction).
pub fn build_briefing_snapshot(
    articles: &[SnapshotArticle<'_>],
    since_utc: Option<DateTime<Utc>>,
    budget_bytes: usize,
    coverage_window_label: String,
) -> BriefingSnapshot {
    let mut text = String::new();
    let mut included_count = 0usize;
    let mut skipped_count = 0usize;
    let mut dropped_count = 0usize;
    let mut budget_reached = false;
    let mut oversized_first_entry = false;

    for article in articles {
        if !in_coverage_window(article.fetched_utc, since_utc) {
            continue;
        }
        let Some(summary) = article.summary else {
            skipped_count += 1;
            continue;
        };
        if budget_reached {
            dropped_count += 1;
            continue;
        }
        let entry = format_entry(included_count + 1, summary);
        let separator_len = if text.is_empty() { 0 } else { 2 };
        if !text.is_empty() && text.len() + separator_len + entry.len() > budget_bytes {
            budget_reached = true;
            dropped_count += 1;
            continue;
        }
        if text.is_empty() && entry.len() > budget_bytes {
            text.push_str(&entry);
            included_count += 1;
            budget_reached = true;
            oversized_first_entry = true;
            continue;
        }
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&entry);
        included_count += 1;
    }

    BriefingSnapshot {
        text,
        included_count,
        skipped_count,
        dropped_count,
        truncated: dropped_count > 0 || oversized_first_entry,
        coverage_window_label,
    }
}

fn format_entry(index: usize, summary: &ArticleSummaryResult) -> String {
    format!(
        "[A{index}] {}\n{}",
        summary.title.trim(),
        summary.summary.trim()
    )
}

fn in_coverage_window(fetched_utc: Option<&str>, since_utc: Option<DateTime<Utc>>) -> bool {
    let Some(since) = since_utc else {
        return true;
    };
    match fetched_utc {
        None => true,
        Some(raw) => match DateTime::parse_from_rfc3339(raw) {
            Ok(dt) => dt.with_timezone(&Utc) >= since,
            Err(_) => true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefing::ArticleSummaryResult;

    fn summary(title: &str, body: &str) -> ArticleSummaryResult {
        ArticleSummaryResult {
            title: title.to_string(),
            summary: body.to_string(),
            key_points: vec![],
            input_tokens: 0,
            output_tokens: 0,
            entities: Default::default(),
        }
    }

    #[test]
    fn includes_duplicates_in_corpus_order_with_stable_labels() {
        let first = summary("Alpha", "First.");
        let second = summary("Alpha", "First.");
        let articles = vec![
            SnapshotArticle {
                url: "u1",
                fetched_utc: None,
                summary: Some(&first),
            },
            SnapshotArticle {
                url: "u2",
                fetched_utc: None,
                summary: Some(&second),
            },
        ];
        let snap = build_briefing_snapshot(&articles, None, 100_000, "all".to_string());
        assert_eq!(snap.included_count, 2);
        assert!(snap.text.starts_with("[A1] Alpha"));
        assert!(snap.text.contains("[A2] Alpha"));
        assert_eq!(snap.skipped_count, 0);
        assert_eq!(snap.dropped_count, 0);
        assert!(!snap.truncated);
    }

    #[test]
    fn skips_in_window_articles_without_summary() {
        let first = summary("Alpha", "First.");
        let articles = vec![
            SnapshotArticle {
                url: "u1",
                fetched_utc: None,
                summary: Some(&first),
            },
            SnapshotArticle {
                url: "u2",
                fetched_utc: None,
                summary: None,
            },
        ];
        let snap = build_briefing_snapshot(&articles, None, 100_000, "all".to_string());
        assert_eq!(snap.included_count, 1);
        assert_eq!(snap.skipped_count, 1);
    }

    #[test]
    fn excludes_articles_before_coverage_window() {
        let old = summary("Old", "stale.");
        let new = summary("New", "fresh.");
        let articles = vec![
            SnapshotArticle {
                url: "u1",
                fetched_utc: Some("2026-01-01T00:00:00Z"),
                summary: Some(&old),
            },
            SnapshotArticle {
                url: "u2",
                fetched_utc: Some("2026-06-10T00:00:00Z"),
                summary: Some(&new),
            },
        ];
        let since = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let snap = build_briefing_snapshot(&articles, Some(since), 100_000, "win".to_string());
        assert_eq!(snap.included_count, 1);
        assert!(snap.text.contains("New"));
        assert!(!snap.text.contains("Old"));
        assert_eq!(snap.skipped_count, 0);
    }

    #[test]
    fn malformed_or_missing_fetched_utc_is_included() {
        let no_timestamp = summary("NoTs", "x.");
        let bad_timestamp = summary("BadTs", "y.");
        let articles = vec![
            SnapshotArticle {
                url: "u1",
                fetched_utc: None,
                summary: Some(&no_timestamp),
            },
            SnapshotArticle {
                url: "u2",
                fetched_utc: Some("not-a-date"),
                summary: Some(&bad_timestamp),
            },
        ];
        let since = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let snap = build_briefing_snapshot(&articles, Some(since), 100_000, "win".to_string());
        assert_eq!(snap.included_count, 2);
    }

    #[test]
    fn drops_whole_entries_over_budget_and_marks_truncated() {
        let first = summary("A", &"x".repeat(50));
        let second = summary("B", &"y".repeat(50));
        let articles = vec![
            SnapshotArticle {
                url: "u1",
                fetched_utc: None,
                summary: Some(&first),
            },
            SnapshotArticle {
                url: "u2",
                fetched_utc: None,
                summary: Some(&second),
            },
        ];
        let first_len = format!("[A1] A\n{}", "x".repeat(50)).len();
        let snap = build_briefing_snapshot(&articles, None, first_len + 1, "all".to_string());
        assert_eq!(snap.included_count, 1);
        assert_eq!(snap.dropped_count, 1);
        assert!(snap.truncated);
        assert!(snap.text.contains("[A1] A"));
        assert!(!snap.text.contains("[A2]"));
    }

    #[test]
    fn exact_fit_budget_includes_separator_bytes() {
        let first = summary("A", &"x".repeat(20));
        let second = summary("B", &"y".repeat(20));
        let articles = vec![
            SnapshotArticle {
                url: "u1",
                fetched_utc: None,
                summary: Some(&first),
            },
            SnapshotArticle {
                url: "u2",
                fetched_utc: None,
                summary: Some(&second),
            },
        ];
        let entry_a = format!("[A1] A\n{}", "x".repeat(20));
        let entry_b = format!("[A2] B\n{}", "y".repeat(20));
        let budget = entry_a.len() + entry_b.len();
        let snap = build_briefing_snapshot(&articles, None, budget, "all".to_string());
        assert_eq!(snap.included_count, 1);
        assert_eq!(snap.dropped_count, 1);
        assert!(snap.truncated);
        assert!(
            snap.text.len() <= budget,
            "snapshot must never exceed the byte budget"
        );
    }

    #[test]
    fn utf8_multibyte_entries_are_never_split() {
        let first = summary("Caf\u{00e9}", &"\u{00e9}".repeat(40));
        let second = summary("Na\u{00ef}ve", &"\u{00fc}".repeat(40));
        let articles = vec![
            SnapshotArticle {
                url: "u1",
                fetched_utc: None,
                summary: Some(&first),
            },
            SnapshotArticle {
                url: "u2",
                fetched_utc: None,
                summary: Some(&second),
            },
        ];
        let snap = build_briefing_snapshot(&articles, None, 10, "all".to_string());
        assert!(snap.text.is_char_boundary(snap.text.len()));
        assert_eq!(snap.included_count, 1);
        assert_eq!(snap.dropped_count, 1);
    }

    #[test]
    fn oversized_first_entry_is_emitted_whole_and_marked_truncated() {
        let first = summary("Large", &"x".repeat(50));
        let articles = vec![SnapshotArticle {
            url: "u1",
            fetched_utc: None,
            summary: Some(&first),
        }];
        let snap = build_briefing_snapshot(&articles, None, 10, "all".to_string());
        assert_eq!(snap.included_count, 1);
        assert_eq!(snap.dropped_count, 0);
        assert!(snap.truncated);
        assert!(snap.text.len() > 10);
        assert!(snap.text.contains("[A1] Large"));
    }

    #[test]
    fn empty_when_no_completed_summaries() {
        let articles = vec![SnapshotArticle {
            url: "u1",
            fetched_utc: None,
            summary: None,
        }];
        let snap = build_briefing_snapshot(&articles, None, 100_000, "all".to_string());
        assert_eq!(snap.included_count, 0);
        assert!(snap.text.is_empty());
    }
}
