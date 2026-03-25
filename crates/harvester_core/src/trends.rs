//! Archive-level entity trend computation.
//!
//! Computes top-N entity mentions per category over a rolling window of ISO weeks.

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};

use crate::entity_index::{EntityIndex, EntityIndexEntry};

/// Trend data for all entity categories across the window.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityTrendData {
    pub companies: CategoryTrend,
    pub technologies: CategoryTrend,
    pub products: CategoryTrend,
    pub themes: CategoryTrend,
}

/// Trend data for a single entity category.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryTrend {
    /// Sorted ISO weeks in the window, oldest first.
    pub weeks: Vec<IsoWeek>,
    /// Top-N entities by total mention count.
    pub top_entities: Vec<EntityLine>,
    /// Total number of distinct entities (before top-N truncation).
    pub total_entity_count: usize,
}

/// A single entity's mention counts across the week window.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityLine {
    /// Canonical display form (most frequent original form; ties resolved lexically).
    pub display_label: String,
    /// Per-week mention counts, parallel to `CategoryTrend::weeks`.
    pub weekly_counts: Vec<u32>,
    /// Sum of `weekly_counts`.
    pub total_count: u32,
}

/// One ISO week in the trend window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoWeek {
    /// Monday of this ISO week (UTC).
    pub week_start: NaiveDate,
    /// Short display label, e.g. "Feb 3".
    pub label: String,
}

/// Compute trend data from the entity index.
///
/// `window_weeks` is the number of consecutive ISO weeks to include, ending with
/// the week containing `now`.  `top_n` limits how many entities are returned per
/// category.
pub fn compute_trends(index: &EntityIndex, window_weeks: u32, top_n: usize) -> EntityTrendData {
    let as_of = chrono::Utc::now().date_naive();
    compute_trends_as_of(index, window_weeks, top_n, as_of)
}

/// Like `compute_trends` but with an explicit reference date for deterministic testing.
pub(crate) fn compute_trends_as_of(
    index: &EntityIndex,
    window_weeks: u32,
    top_n: usize,
    as_of: NaiveDate,
) -> EntityTrendData {
    let weeks = build_week_buckets(window_weeks, as_of);

    EntityTrendData {
        companies: compute_category_trend(index, |e| &e.companies, &weeks, top_n),
        technologies: compute_category_trend(index, |e| &e.technologies, &weeks, top_n),
        products: compute_category_trend(index, |e| &e.products, &weeks, top_n),
        themes: compute_category_trend(index, |e| &e.themes, &weeks, top_n),
    }
}

/// Normalize an entity string for grouping: trim, collapse internal whitespace, lowercase.
pub fn normalize_entity_key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Choose the canonical display label from a list of `(form, occurrence_count)` pairs.
/// Returns the most frequent form; ties resolved by lexically smallest form.
pub fn choose_display_label(occurrences: &[(String, u32)]) -> String {
    if occurrences.is_empty() {
        return String::new();
    }
    let max_count = occurrences.iter().map(|(_, c)| *c).max().unwrap_or(0);
    occurrences
        .iter()
        .filter(|(_, c)| *c == max_count)
        .map(|(label, _)| label.as_str())
        .min() // lexically smallest among tied forms
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compute a recency-weighted score for ranking entities.
///
/// Weights the most recent weeks more heavily so that currently active entities
/// rank above entities that had a spike in the distant past.
///
/// Formula: `latest*100 + prev1*40 + prev2*20 + total`
pub(crate) fn compute_recency_score(counts: &[u32]) -> u64 {
    let n = counts.len();
    let latest = counts.last().copied().unwrap_or(0) as u64;
    let prev1 = if n >= 2 { counts[n - 2] as u64 } else { 0 };
    let prev2 = if n >= 3 { counts[n - 3] as u64 } else { 0 };
    let total: u64 = counts.iter().map(|&c| c as u64).sum();
    latest * 100 + prev1 * 40 + prev2 * 20 + total
}

/// Return the ISO Monday (week start) for the given date.
fn week_start_for_date(date: NaiveDate) -> NaiveDate {
    let days_from_monday = date.weekday().num_days_from_monday() as i64;
    date - chrono::Duration::days(days_from_monday)
}

/// Build `window_weeks` consecutive week buckets ending with the week containing `as_of`,
/// oldest first.
fn build_week_buckets(window_weeks: u32, as_of: NaiveDate) -> Vec<IsoWeek> {
    if window_weeks == 0 {
        return Vec::new();
    }
    let current_monday = week_start_for_date(as_of);
    (0..window_weeks)
        .rev()
        .map(|weeks_ago| {
            let week_start = current_monday - chrono::Duration::weeks(weeks_ago as i64);
            let month = week_start.format("%b").to_string();
            let day = week_start.day();
            let label = format!("{month} {day}");
            IsoWeek { week_start, label }
        })
        .collect()
}

/// Compute the `CategoryTrend` for one entity category extracted by `get_entities`.
fn compute_category_trend<F>(
    index: &EntityIndex,
    get_entities: F,
    weeks: &[IsoWeek],
    top_n: usize,
) -> CategoryTrend
where
    F: Fn(&EntityIndexEntry) -> &Vec<String>,
{
    let n_weeks = weeks.len();

    // Map week start date → index in the weeks slice.
    let week_to_idx: HashMap<NaiveDate, usize> = weeks
        .iter()
        .enumerate()
        .map(|(i, w)| (w.week_start, i))
        .collect();

    // normalized_key → per-week counts (len == n_weeks)
    let mut entity_counts: HashMap<String, Vec<u32>> = HashMap::new();
    // normalized_key → original_form → occurrence_count (for display label election)
    let mut entity_forms: HashMap<String, HashMap<String, u32>> = HashMap::new();

    for entry in index.entries.values() {
        // Parse fetched_utc — skip articles with missing or unparseable timestamps.
        let article_date = match entry.fetched_utc.as_deref() {
            Some(s) => match chrono::DateTime::parse_from_rfc3339(s) {
                Ok(dt) => dt.naive_utc().date(),
                Err(_) => continue,
            },
            None => continue,
        };

        let week_start = week_start_for_date(article_date);
        let week_idx = match week_to_idx.get(&week_start) {
            Some(&idx) => idx,
            None => continue, // outside the trend window
        };

        let entities = get_entities(entry);

        // Count at most one mention per entity per article (presence, not frequency).
        let mut seen_in_article: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for entity_str in entities {
            let key = normalize_entity_key(entity_str);
            if key.is_empty() {
                continue;
            }
            if !seen_in_article.insert(key.clone()) {
                continue; // already counted for this article
            }

            entity_counts
                .entry(key.clone())
                .or_insert_with(|| vec![0u32; n_weeks])[week_idx] += 1;

            *entity_forms
                .entry(key)
                .or_default()
                .entry(entity_str.clone())
                .or_insert(0) += 1;
        }
    }

    let total_entity_count = entity_counts.len();

    let mut lines: Vec<EntityLine> = entity_counts
        .iter()
        .map(|(key, week_counts)| {
            let total: u32 = week_counts.iter().sum();
            let forms: Vec<(String, u32)> = entity_forms
                .get(key)
                .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
                .unwrap_or_default();
            EntityLine {
                display_label: choose_display_label(&forms),
                weekly_counts: week_counts.clone(),
                total_count: total,
            }
        })
        .collect();

    // Sort: recency-weighted score desc → display_label asc (deterministic tie-break).
    lines.sort_by(|a, b| {
        let score_a = compute_recency_score(&a.weekly_counts);
        let score_b = compute_recency_score(&b.weekly_counts);
        score_b
            .cmp(&score_a)
            .then_with(|| a.display_label.cmp(&b.display_label))
    });

    lines.truncate(top_n);

    CategoryTrend {
        weeks: weeks.to_vec(),
        top_entities: lines,
        total_entity_count,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_index::{EntityIndex, EntityIndexEntry};
    use std::collections::BTreeMap;

    fn make_index(entries: Vec<(&str, EntityIndexEntry)>) -> EntityIndex {
        EntityIndex {
            schema_version: 1,
            entries: entries
                .into_iter()
                .map(|(url, e)| (url.to_string(), e))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    fn entry_with_companies(fetched_utc: &str, companies: Vec<&str>) -> EntityIndexEntry {
        EntityIndexEntry {
            fetched_utc: Some(fetched_utc.to_string()),
            content_hash: None,
            companies: companies.iter().map(|s| s.to_string()).collect(),
            technologies: vec![],
            products: vec![],
            themes: vec![],
        }
    }

    // -----------------------------------------------------------------------
    // normalize_entity_key
    // -----------------------------------------------------------------------

    #[test]
    fn normalize_trims_and_lowercases() {
        assert_eq!(normalize_entity_key("  Nvidia  "), "nvidia");
    }

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(
            normalize_entity_key("Large  Language  Models"),
            "large language models"
        );
    }

    #[test]
    fn normalize_empty_stays_empty() {
        assert_eq!(normalize_entity_key("   "), "");
    }

    // -----------------------------------------------------------------------
    // choose_display_label
    // -----------------------------------------------------------------------

    #[test]
    fn display_label_most_frequent() {
        let forms = vec![
            ("Nvidia".to_string(), 5),
            ("NVIDIA".to_string(), 2),
            ("nvidia".to_string(), 1),
        ];
        assert_eq!(choose_display_label(&forms), "Nvidia");
    }

    #[test]
    fn display_label_tie_resolves_lexically() {
        let forms = vec![
            ("TSMC".to_string(), 3),
            ("Tsmc".to_string(), 3),
            ("tsmc".to_string(), 3),
        ];
        // Lexically smallest: "TSMC" (uppercase letters come before lowercase in ASCII)
        assert_eq!(choose_display_label(&forms), "TSMC");
    }

    #[test]
    fn display_label_empty_input() {
        assert_eq!(choose_display_label(&[]), "");
    }

    // -----------------------------------------------------------------------
    // compute_trends_as_of
    // -----------------------------------------------------------------------

    /// 2025-02-10 is a Monday (ISO week 2025-W07).
    const REF_DATE_STR: &str = "2025-02-10";

    fn ref_date() -> NaiveDate {
        NaiveDate::parse_from_str(REF_DATE_STR, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn trends_week_count() {
        let index = make_index(vec![]);
        let data = compute_trends_as_of(&index, 13, 10, ref_date());
        assert_eq!(data.companies.weeks.len(), 13);
        assert_eq!(data.technologies.weeks.len(), 13);
    }

    #[test]
    fn trends_current_week_is_last() {
        let index = make_index(vec![]);
        let data = compute_trends_as_of(&index, 4, 10, ref_date());
        // Ref date is 2025-02-10 (Monday). Last week in window should also be 2025-02-10.
        let last = data.companies.weeks.last().unwrap();
        assert_eq!(last.week_start, ref_date());
    }

    #[test]
    fn trends_oldest_week_is_window_minus_1_weeks_back() {
        let index = make_index(vec![]);
        let data = compute_trends_as_of(&index, 4, 10, ref_date());
        // 4 weeks window: oldest = 3 weeks before 2025-02-10 = 2025-01-20
        let first = data.companies.weeks.first().unwrap();
        let expected = ref_date() - chrono::Duration::weeks(3);
        assert_eq!(first.week_start, expected);
    }

    #[test]
    fn trends_buckets_article_in_correct_week() {
        // 2025-02-12 (Wed) is in the week starting 2025-02-10
        let index = make_index(vec![(
            "https://a.example.com",
            entry_with_companies("2025-02-12T10:00:00Z", vec!["Nvidia"]),
        )]);
        let data = compute_trends_as_of(&index, 4, 10, ref_date());
        let last_week_idx = data.companies.weeks.len() - 1;
        let nvidia_line = data
            .companies
            .top_entities
            .iter()
            .find(|e| e.display_label == "Nvidia");
        let nvidia_line = nvidia_line.expect("Nvidia should appear");
        assert_eq!(nvidia_line.weekly_counts[last_week_idx], 1);
        assert_eq!(nvidia_line.total_count, 1);
    }

    #[test]
    fn trends_article_outside_window_is_skipped() {
        // 2025-01-01 is before any of our 4-week window starting 2025-01-20
        let index = make_index(vec![(
            "https://a.example.com",
            entry_with_companies("2025-01-01T00:00:00Z", vec!["Nvidia"]),
        )]);
        let data = compute_trends_as_of(&index, 4, 10, ref_date());
        assert!(data.companies.top_entities.is_empty());
    }

    #[test]
    fn trends_missing_fetched_utc_skipped_no_panic() {
        let entry = EntityIndexEntry {
            fetched_utc: None,
            content_hash: None,
            companies: vec!["Nvidia".to_string()],
            technologies: vec![],
            products: vec![],
            themes: vec![],
        };
        let index = make_index(vec![("https://a.example.com", entry)]);
        let data = compute_trends_as_of(&index, 4, 10, ref_date());
        assert!(data.companies.top_entities.is_empty());
    }

    #[test]
    fn trends_counts_at_most_once_per_article() {
        // Same entity listed twice in one article
        let entry = EntityIndexEntry {
            fetched_utc: Some("2025-02-12T00:00:00Z".to_string()),
            content_hash: None,
            companies: vec!["Nvidia".to_string(), "nvidia".to_string()], // both normalize to "nvidia"
            technologies: vec![],
            products: vec![],
            themes: vec![],
        };
        let index = make_index(vec![("https://a.example.com", entry)]);
        let data = compute_trends_as_of(&index, 4, 10, ref_date());
        assert_eq!(data.companies.top_entities.len(), 1);
        assert_eq!(data.companies.top_entities[0].total_count, 1);
    }

    #[test]
    fn trends_top_n_tie_breaking_deterministic() {
        // Three entities each with total_count=1, same latest-week count.
        // They should be sorted by display_label asc.
        let index = make_index(vec![
            (
                "https://a.example.com",
                entry_with_companies("2025-02-12T00:00:00Z", vec!["Zeta"]),
            ),
            (
                "https://b.example.com",
                entry_with_companies("2025-02-12T00:00:00Z", vec!["Alpha"]),
            ),
            (
                "https://c.example.com",
                entry_with_companies("2025-02-12T00:00:00Z", vec!["Midway"]),
            ),
        ]);
        let data = compute_trends_as_of(&index, 4, 3, ref_date());
        let labels: Vec<_> = data
            .companies
            .top_entities
            .iter()
            .map(|e| e.display_label.as_str())
            .collect();
        assert_eq!(labels, vec!["Alpha", "Midway", "Zeta"]);
    }

    #[test]
    fn trends_month_year_boundary_week() {
        // 2024-12-30 is Monday of the week spanning Dec 30 – Jan 5 (ISO year 2025 W01)
        // Ref date: 2025-01-06 (Monday) → 1-week window starts 2025-01-06
        // Use 2-week window so the Dec 30 week is in range
        let as_of = NaiveDate::parse_from_str("2025-01-06", "%Y-%m-%d").unwrap();
        // Article on Dec 31 (which is in the Dec 30 week)
        let index = make_index(vec![(
            "https://a.example.com",
            entry_with_companies("2024-12-31T00:00:00Z", vec!["TSMC"]),
        )]);
        let data = compute_trends_as_of(&index, 2, 10, as_of);
        // First week should be 2024-12-30
        let first_week = &data.companies.weeks[0];
        assert_eq!(
            first_week.week_start,
            NaiveDate::parse_from_str("2024-12-30", "%Y-%m-%d").unwrap()
        );
        // TSMC should appear in first week
        let tsmc = data
            .companies
            .top_entities
            .iter()
            .find(|e| e.display_label == "TSMC")
            .expect("TSMC should appear");
        assert_eq!(tsmc.weekly_counts[0], 1);
    }

    #[test]
    fn trends_total_entity_count_is_full_population() {
        // 5 distinct entities, top_n=2 → total_entity_count=5 but only 2 in top_entities
        let entries: Vec<_> = ["Apple", "Google", "Meta", "Nvidia", "TSMC"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let url = format!("https://example.com/{i}");
                let entry = entry_with_companies("2025-02-12T00:00:00Z", vec![name]);
                (url, entry)
            })
            .collect();
        let index = EntityIndex {
            schema_version: 1,
            entries: entries.into_iter().collect::<BTreeMap<_, _>>(),
        };
        let data = compute_trends_as_of(&index, 4, 2, ref_date());
        assert_eq!(data.companies.total_entity_count, 5);
        assert_eq!(data.companies.top_entities.len(), 2);
    }

    // -----------------------------------------------------------------------
    // compute_recency_score
    // -----------------------------------------------------------------------

    #[test]
    fn recent_mover_outranks_stale_spike() {
        // "Stale" had a spike 5 weeks ago; "Recent" has activity in the last 2 weeks.
        // With total-count-first sorting, "Stale" would win (total=100 vs 15).
        // With recency-weighted scoring, "Recent" should win because the stale spike
        // is in week0 (oldest) and contributes only to `total` (weight=1),
        // while Recent's 10 in the latest week contributes 10*100=1000.
        //
        // Window: 5 weeks ending 2025-02-10 (ref_date).
        //   Week 0: 2025-01-13, Week 1: 2025-01-20, Week 2: 2025-01-27,
        //   Week 3: 2025-02-03, Week 4: 2025-02-10
        //
        // "Stale": spike of 100 in week 0 (2025-01-13), zero elsewhere.
        //   score = 0*100 + 0*40 + 0*20 + 100 = 100
        // "Recent": 5 in week 3, 10 in week 4.
        //   score = 10*100 + 5*40 + 0*20 + 15 = 1215

        // Build "Stale": 100 articles in 2025-01-14 (week starting 2025-01-13)
        let mut stale_entries: Vec<(String, EntityIndexEntry)> = (0..100)
            .map(|i| {
                let url = format!("https://stale.example.com/{i}");
                let entry = entry_with_companies("2025-01-14T00:00:00Z", vec!["Stale"]);
                (url, entry)
            })
            .collect();

        // Build "Recent": 5 articles in 2025-02-04 (week starting 2025-02-03)
        let recent_w3: Vec<(String, EntityIndexEntry)> = (0..5)
            .map(|i| {
                let url = format!("https://recent-w3.example.com/{i}");
                let entry = entry_with_companies("2025-02-04T00:00:00Z", vec!["Recent"]);
                (url, entry)
            })
            .collect();

        // 10 articles in 2025-02-11 (week starting 2025-02-10)
        let recent_w4: Vec<(String, EntityIndexEntry)> = (0..10)
            .map(|i| {
                let url = format!("https://recent-w4.example.com/{i}");
                let entry = entry_with_companies("2025-02-11T00:00:00Z", vec!["Recent"]);
                (url, entry)
            })
            .collect();

        stale_entries.extend(recent_w3);
        stale_entries.extend(recent_w4);

        let index = EntityIndex {
            schema_version: 1,
            entries: stale_entries.into_iter().collect::<BTreeMap<_, _>>(),
        };

        // ref_date() is 2025-02-10 (Monday). Use 5-week window.
        // The week containing 2025-02-11 starts 2025-02-10, which IS within the window.
        let data = compute_trends_as_of(&index, 5, 5, ref_date());
        let labels: Vec<_> = data
            .companies
            .top_entities
            .iter()
            .map(|e| e.display_label.as_str())
            .collect();
        assert_eq!(
            labels[0], "Recent",
            "Recent mover should outrank stale spike; got order: {labels:?}"
        );
    }

    #[test]
    fn identical_latest_falls_back_to_weighted_recent() {
        // "A" and "B" both have latest=3, but A has a larger prev1 (5 vs 1).
        // Recency score: A = 3*100 + 5*40 + 10*20 + 18 = 300+200+200+18 = 718
        //                B = 3*100 + 1*40 + 0*20 + 4  = 300+40+0+4   = 344
        // So A should rank higher.
        //
        // Window: 5 weeks ending ref_date. counts are [w0, w1, w2, w3, w4].
        // A: [0, 0, 10, 5, 3]  — 10 articles in w2, 5 in w3, 3 in w4
        // B: [0, 0, 0,  1, 3]  — 1 article in w3, 3 in w4

        let mut entries: Vec<(String, EntityIndexEntry)> = Vec::new();

        // A: 10 in week2 (2025-01-27), 5 in week3 (2025-02-03), 3 in week4 (2025-02-10)
        for i in 0..10 {
            entries.push((
                format!("https://a-w2.example.com/{i}"),
                entry_with_companies("2025-01-28T00:00:00Z", vec!["A"]),
            ));
        }
        for i in 0..5 {
            entries.push((
                format!("https://a-w3.example.com/{i}"),
                entry_with_companies("2025-02-04T00:00:00Z", vec!["A"]),
            ));
        }
        for i in 0..3 {
            entries.push((
                format!("https://a-w4.example.com/{i}"),
                entry_with_companies("2025-02-11T00:00:00Z", vec!["A"]),
            ));
        }

        // B: 1 in week3, 3 in week4
        entries.push((
            "https://b-w3.example.com/0".to_string(),
            entry_with_companies("2025-02-04T00:00:00Z", vec!["B"]),
        ));
        for i in 0..3 {
            entries.push((
                format!("https://b-w4.example.com/{i}"),
                entry_with_companies("2025-02-11T00:00:00Z", vec!["B"]),
            ));
        }

        let index = EntityIndex {
            schema_version: 1,
            entries: entries.into_iter().collect::<BTreeMap<_, _>>(),
        };

        let data = compute_trends_as_of(&index, 5, 5, ref_date());
        let labels: Vec<_> = data
            .companies
            .top_entities
            .iter()
            .map(|e| e.display_label.as_str())
            .collect();
        assert_eq!(
            labels[0], "A",
            "A should rank above B due to larger prev1; got: {labels:?}"
        );
    }

    #[test]
    fn ordering_is_deterministic_on_equal_scores() {
        // "Alpha" and "Beta" have identical counts → same recency score.
        // Tie-break should put "Alpha" before "Beta" (label ascending).
        let mut entries: Vec<(String, EntityIndexEntry)> = Vec::new();
        for i in 0..5 {
            entries.push((
                format!("https://alpha.example.com/{i}"),
                entry_with_companies("2025-02-11T00:00:00Z", vec!["Alpha"]),
            ));
        }
        for i in 0..5 {
            entries.push((
                format!("https://beta.example.com/{i}"),
                entry_with_companies("2025-02-11T00:00:00Z", vec!["Beta"]),
            ));
        }

        let index = EntityIndex {
            schema_version: 1,
            entries: entries.into_iter().collect::<BTreeMap<_, _>>(),
        };

        let data = compute_trends_as_of(&index, 5, 5, ref_date());
        let labels: Vec<_> = data
            .companies
            .top_entities
            .iter()
            .map(|e| e.display_label.as_str())
            .collect();
        assert_eq!(
            labels[0], "Alpha",
            "Alpha should come before Beta on equal scores; got: {labels:?}"
        );
    }

    #[test]
    fn short_window_ranks_deterministically() {
        // 2-week window. "X" has count in week0, "Y" has count in week1 (latest).
        // Y should rank higher because latest > 0 while X's latest == 0.
        //
        // 2-week window ending ref_date (2025-02-10):
        //   week0: 2025-02-03, week1: 2025-02-10
        // X: article on 2025-02-04 → week0
        // Y: article on 2025-02-11 → week1

        let index = make_index(vec![
            (
                "https://x.example.com",
                entry_with_companies("2025-02-04T00:00:00Z", vec!["X"]),
            ),
            (
                "https://y.example.com",
                entry_with_companies("2025-02-11T00:00:00Z", vec!["Y"]),
            ),
        ]);

        let data = compute_trends_as_of(&index, 2, 5, ref_date());
        let labels: Vec<_> = data
            .companies
            .top_entities
            .iter()
            .map(|e| e.display_label.as_str())
            .collect();
        assert_eq!(
            labels[0], "Y",
            "Y (latest week activity) should rank above X (older week); got: {labels:?}"
        );
    }

    // -----------------------------------------------------------------------
    // compute_recency_score unit tests (direct)
    // -----------------------------------------------------------------------

    #[test]
    fn recency_score_weights_latest_most() {
        // A large spike in the oldest position only adds to total (score=100).
        // Modest recent activity scores much higher due to latest/prev1 weights.
        let stale = compute_recency_score(&[100, 0, 0, 0, 0]);
        let recent = compute_recency_score(&[0, 0, 0, 5, 10]);
        assert!(
            recent > stale,
            "recent score {recent} should exceed stale score {stale}"
        );
    }

    #[test]
    fn recency_score_short_slice() {
        // 2 elements: latest=10, no prev2
        let score = compute_recency_score(&[5, 10]);
        // latest*100 + prev1*40 + total = 1000 + 200 + 15 = 1215
        assert_eq!(score, 1215);
    }

    #[test]
    fn recency_score_single_element() {
        let score = compute_recency_score(&[7]);
        // latest*100 + total = 700 + 7 = 707
        assert_eq!(score, 707);
    }

    #[test]
    fn recency_score_empty_slice() {
        assert_eq!(compute_recency_score(&[]), 0);
    }
}
