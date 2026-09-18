//! Pure quality comparison between the recorded baseline and Jev results.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::config::RunConfig;
use crate::manifest::Manifest;
use crate::metrics::operational::{compute as compute_operational, OperationalMetrics};
use crate::metrics::small_sample::Rate;
use crate::runner::store::{select_records, ResultRecord};

pub const CATEGORIES: [&str; 5] = [
    "Business",
    "Technology",
    "Politics & Regulation",
    "Finance & Markets",
    "Science & Research",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComparisonMetrics {
    pub article_count: u64,
    pub attempted_records: u64,
    pub successful_records: u64,
    pub failure_count: u64,
    pub articles_with_successful_record: u64,
    pub articles_without_successful_record: u64,
    pub failed_high_priority_articles: u64,
    pub failed_priority_five_articles: u64,
    pub baseline_priority_histogram: BTreeMap<u8, u64>,
    pub jev_priority_histogram: BTreeMap<u8, u64>,
    pub confusion_matrix: [[u64; 5]; 5],
    pub exact_agreement: Rate,
    pub within_one: Rate,
    pub mean_absolute_error: Option<f64>,
    pub priority_five_precision: Rate,
    pub priority_five_recall: Rate,
    pub high_priority_precision: Rate,
    pub high_priority_recall: Rate,
    pub severe_demotions: u64,
    pub severe_promotions: u64,
    pub category: CategoryMetrics,
    pub tags: TagMetrics,
    pub failures_by_outcome: BTreeMap<String, u64>,
    pub failures_by_error_type: BTreeMap<String, u64>,
    pub failures: Vec<FailureDetail>,
    pub retry_count: u64,
    pub superseded_records: u64,
    pub throughput: Throughput,
    pub operational: OperationalMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CategoryMetrics {
    pub positive_rates: BTreeMap<String, Rate>,
    pub cooccurrence: BTreeMap<String, BTreeMap<String, u64>>,
    pub zero_category_share: Rate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TagMetric {
    pub precision: Rate,
    pub recall: Rate,
    pub support: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TagMetrics {
    pub mean_jaccard: Option<f64>,
    pub articles: u64,
    pub both_empty_excluded: u64,
    pub per_tag: BTreeMap<String, TagMetric>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailureDetail {
    pub article_id: String,
    pub repetition: u32,
    pub outcome: String,
    pub error_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Throughput {
    pub successful_records: u64,
    pub total_elapsed_ms: u64,
    pub articles_per_second: Option<String>,
}

/// Selects the append-order records that feed comparison. A successful record
/// is preferred when it is the newest record for a pair; otherwise the newest
/// record represents that pair as a failure.
pub fn selected_records(records: &[ResultRecord]) -> (Vec<ResultRecord>, u64) {
    select_records(records)
}

pub fn compute(
    manifest: &Manifest,
    records: &[ResultRecord],
    config: &RunConfig,
) -> ComparisonMetrics {
    let articles: BTreeMap<_, _> = manifest
        .articles
        .iter()
        .map(|a| (a.article_id.as_str(), a))
        .collect();
    let (selected, superseded_records) = selected_records(records);
    let mut baseline_histogram = BTreeMap::new();
    let mut jev_histogram = BTreeMap::new();
    let mut matrix = [[0_u64; 5]; 5];
    let mut exact = 0;
    let mut within = 0;
    let mut absolute_error = 0_u64;
    let mut successful = 0_u64;
    let mut successful_articles = BTreeSet::new();
    let mut attempted_articles = BTreeSet::new();
    let mut failures_by_outcome = BTreeMap::new();
    let mut failures_by_error_type = BTreeMap::new();
    let mut failures = Vec::new();
    let mut total_elapsed = 0_u64;
    let mut retries = 0_u64;
    let mut category_counts: BTreeMap<String, u64> =
        CATEGORIES.iter().map(|c| ((*c).into(), 0)).collect();
    let mut cooccurrence: BTreeMap<String, BTreeMap<String, u64>> = CATEGORIES
        .iter()
        .map(|left| {
            (
                (*left).into(),
                CATEGORIES
                    .iter()
                    .map(|right| ((*right).into(), 0))
                    .collect(),
            )
        })
        .collect();
    let mut zero_categories = 0;
    let mut jaccard_sum = 0.0;
    let mut jaccard_count = 0_u64;
    let mut both_empty_excluded = 0_u64;
    let mut tag_stats: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();
    let mut baseline_high = 0;
    let mut jev_high = 0;
    let mut baseline_five = 0;
    let mut jev_five = 0;
    let mut five_true = 0;
    let mut high_true = 0;
    let mut severe_demotions = 0;
    let mut severe_promotions = 0;
    for record in &selected {
        total_elapsed += record.total_elapsed_ms;
        retries += u64::from(record.attempt_count.saturating_sub(1));
        let Some(article) = articles.get(record.article_id.as_str()) else {
            continue;
        };
        attempted_articles.insert(record.article_id.as_str());
        if record.outcome != "ok" || record.priority.is_none() {
            *failures_by_outcome
                .entry(record.outcome.clone())
                .or_default() += 1;
            *failures_by_error_type
                .entry(
                    record
                        .error_type
                        .clone()
                        .unwrap_or_else(|| "unknown".into()),
                )
                .or_default() += 1;
            failures.push(FailureDetail {
                article_id: record.article_id.clone(),
                repetition: record.repetition,
                outcome: record.outcome.clone(),
                error_type: record.error_type.clone(),
            });
            continue;
        }
        successful += 1;
        successful_articles.insert(record.article_id.as_str());
        let priority = record.priority.expect("checked above");
        *baseline_histogram
            .entry(article.baseline.priority)
            .or_default() += 1;
        *jev_histogram.entry(priority).or_default() += 1;
        let baseline = article.baseline.priority;
        if baseline == priority {
            exact += 1;
        }
        if baseline.abs_diff(priority) <= 1 {
            within += 1;
        }
        absolute_error += u64::from(baseline.abs_diff(priority));
        if (1..=5).contains(&baseline) && (1..=5).contains(&priority) {
            matrix[(baseline - 1) as usize][(priority - 1) as usize] += 1;
        }
        baseline_high += u64::from(baseline >= 4);
        jev_high += u64::from(priority >= 4);
        baseline_five += u64::from(baseline == 5);
        jev_five += u64::from(priority == 5);
        five_true += u64::from(baseline == 5 && priority == 5);
        high_true += u64::from(baseline >= 4 && priority >= 4);
        severe_demotions += u64::from(baseline == 5 && priority <= 2);
        severe_promotions += u64::from(baseline <= 2 && priority == 5);
        let selected_categories = record.categories_selected.as_deref().unwrap_or(&[]);
        if selected_categories.is_empty() {
            zero_categories += 1;
        }
        let category_set: BTreeSet<&str> = selected_categories.iter().map(String::as_str).collect();
        for category in &CATEGORIES {
            if category_set.contains(category) {
                *category_counts.entry((*category).into()).or_default() += 1;
            }
        }
        for left in &CATEGORIES {
            for right in &CATEGORIES {
                if category_set.contains(left) && category_set.contains(right) {
                    *cooccurrence
                        .entry((*left).into())
                        .or_default()
                        .entry((*right).into())
                        .or_default() += 1;
                }
            }
        }
        let predicted_tags: BTreeSet<&str> = record
            .tags_selected
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(String::as_str)
            .collect();
        let baseline_tags: BTreeSet<&str> =
            article.baseline.tags.iter().map(String::as_str).collect();
        let union = predicted_tags.union(&baseline_tags).count();
        let intersection = predicted_tags.intersection(&baseline_tags).count();
        if union > 0 {
            jaccard_sum += intersection as f64 / union as f64;
            jaccard_count += 1;
        } else {
            both_empty_excluded += 1;
        }
        for tag in predicted_tags.union(&baseline_tags) {
            let stats = tag_stats.entry((*tag).into()).or_default();
            if predicted_tags.contains(tag) {
                stats.1 += 1;
            }
            if baseline_tags.contains(tag) {
                stats.2 += 1;
            }
            if predicted_tags.contains(tag) && baseline_tags.contains(tag) {
                stats.0 += 1;
            }
        }
    }
    let failed_articles = attempted_articles
        .difference(&successful_articles)
        .copied()
        .collect::<Vec<_>>();
    let failed_high_priority_articles = failed_articles
        .iter()
        .filter(|id| articles.get(**id).is_some_and(|a| a.baseline.priority >= 4))
        .count() as u64;
    let failed_priority_five_articles = failed_articles
        .iter()
        .filter(|id| articles.get(**id).is_some_and(|a| a.baseline.priority == 5))
        .count() as u64;
    let category_rates = CATEGORIES
        .iter()
        .map(|category| {
            (
                (*category).into(),
                Rate::new(*category_counts.get(*category).unwrap_or(&0), successful),
            )
        })
        .collect();
    let mut per_tag = BTreeMap::new();
    for (tag, (intersection, predicted, baseline)) in tag_stats {
        per_tag.insert(
            tag,
            TagMetric {
                precision: Rate::new(intersection, predicted),
                recall: Rate::new(intersection, baseline),
                support: baseline,
            },
        );
    }
    ComparisonMetrics {
        article_count: attempted_articles.len() as u64,
        attempted_records: selected.len() as u64,
        successful_records: successful,
        failure_count: selected.len() as u64 - successful,
        articles_with_successful_record: successful_articles.len() as u64,
        articles_without_successful_record: failed_articles.len() as u64,
        failed_high_priority_articles,
        failed_priority_five_articles,
        baseline_priority_histogram: baseline_histogram,
        jev_priority_histogram: jev_histogram,
        confusion_matrix: matrix,
        exact_agreement: Rate::new(exact, successful),
        within_one: Rate::new(within, successful),
        mean_absolute_error: (successful > 0).then(|| absolute_error as f64 / successful as f64),
        priority_five_precision: Rate::new(five_true, jev_five),
        priority_five_recall: Rate::new(five_true, baseline_five),
        high_priority_precision: Rate::new(high_true, jev_high),
        high_priority_recall: Rate::new(high_true, baseline_high),
        severe_demotions,
        severe_promotions,
        category: CategoryMetrics {
            positive_rates: category_rates,
            cooccurrence,
            zero_category_share: Rate::new(zero_categories, successful),
        },
        tags: TagMetrics {
            mean_jaccard: (jaccard_count > 0).then(|| jaccard_sum / jaccard_count as f64),
            articles: jaccard_count,
            both_empty_excluded,
            per_tag,
        },
        failures_by_outcome,
        failures_by_error_type,
        failures,
        retry_count: retries,
        superseded_records,
        throughput: Throughput {
            successful_records: successful,
            total_elapsed_ms: total_elapsed,
            articles_per_second: (total_elapsed > 0)
                .then(|| format!("{:.3}", successful as f64 * 1000.0 / total_elapsed as f64)),
        },
        operational: compute_operational(manifest, records, config),
    }
}
