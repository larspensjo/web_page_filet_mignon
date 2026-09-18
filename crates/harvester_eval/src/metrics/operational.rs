//! Operational aggregates for the two experiment arms.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::config::RunConfig;
use crate::identity::PathKind;
use crate::manifest::Manifest;
use crate::metrics::small_sample::Rate;
use crate::runner::store::{select_records, ResultRecord};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LatencySummary {
    pub count: u64,
    pub median_ms: u64,
    pub p95_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CostSummary {
    pub total_microdollars: u64,
    pub per_article_microdollars: f64,
    pub per_1000_articles_microdollars: f64,
    pub article_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalMetrics {
    pub attempted_article_count: u64,
    pub successful_article_count: u64,
    pub openai_cost: CostSummary,
    pub openai_sync_cost: CostSummary,
    pub openai_batch_cost: CostSummary,
    pub openai_sync_latency_historical: Option<LatencySummary>,
    pub openai_batch_latency_excluded: u64,
    pub openai_sync_zero_latency_excluded: u64,
    pub jev_cost: CostSummary,
    pub jev_latency_model: Option<LatencySummary>,
    pub jev_latency_total: Option<LatencySummary>,
    pub jev_pricing_version: String,
    pub retry_count: u64,
    pub stability: StabilityMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StabilityMetrics {
    pub repeated_articles: u64,
    pub changed_articles: u64,
    pub choice_change_rate: Rate,
}

pub fn compute(
    manifest: &Manifest,
    records: &[ResultRecord],
    config: &RunConfig,
) -> OperationalMetrics {
    let (selected_records, _) = select_records(records);
    let records = selected_records;
    let attempted_article_ids = records
        .iter()
        .map(|record| record.article_id.as_str())
        .collect::<BTreeSet<_>>();
    let successful_article_ids = records
        .iter()
        .filter(|record| record.outcome == "ok" && record.priority.is_some())
        .map(|record| record.article_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut openai_total = 0;
    let mut openai_sync = 0;
    let mut openai_batch = 0;
    let mut sync_count = 0;
    let mut batch_count = 0;
    let mut sync_latencies = Vec::new();
    let mut batch_excluded = 0;
    let mut sync_zero = 0;
    for article in manifest
        .articles
        .iter()
        .filter(|article| successful_article_ids.contains(article.article_id.as_str()))
    {
        let cost = article.baseline.cost_microdollars;
        openai_total += cost;
        match article.baseline.path_kind {
            PathKind::Sync => {
                openai_sync += cost;
                sync_count += 1;
                if article.baseline.wall_ms > 0 {
                    sync_latencies.push(article.baseline.wall_ms);
                } else {
                    sync_zero += 1;
                }
            }
            PathKind::Batch => {
                openai_batch += cost;
                batch_count += 1;
                batch_excluded += 1;
            }
        }
    }
    let ok: Vec<&ResultRecord> = records
        .iter()
        .filter(|record| record.outcome == "ok" && record.priority.is_some())
        .collect();
    let mut model_latencies = Vec::new();
    let mut total_latencies = Vec::new();
    let mut jev_cost = 0;
    let mut retries = 0;
    for record in &ok {
        if let Some(value) = record.model_latency_ms {
            model_latencies.push(value);
        }
        total_latencies.push(record.total_elapsed_ms);
        jev_cost += record.input_tokens.map_or_else(
            || record.estimated_cost_microdollars.unwrap_or_default(),
            |input_tokens| {
                crate::pricing::jev_cost_microdollars(
                    input_tokens,
                    config.pricing.input_microdollars_per_million,
                )
            },
        );
        retries += u64::from(record.attempt_count.saturating_sub(1));
    }
    let mut by_article: BTreeMap<&str, Vec<(u32, u8)>> = BTreeMap::new();
    for record in &ok {
        if let Some(priority) = record.priority {
            by_article
                .entry(&record.article_id)
                .or_default()
                .push((record.repetition, priority));
        }
    }
    let mut repeated = 0;
    let mut changed = 0;
    for values in by_article.values() {
        if values.len() > 1 {
            repeated += 1;
            if values.iter().any(|(_, priority)| *priority != values[0].1) {
                changed += 1;
            }
        }
    }
    let successful_article_count = successful_article_ids.len() as u64;
    OperationalMetrics {
        attempted_article_count: attempted_article_ids.len() as u64,
        successful_article_count,
        openai_cost: cost_summary(openai_total, sync_count + batch_count),
        openai_sync_cost: cost_summary(openai_sync, sync_count),
        openai_batch_cost: cost_summary(openai_batch, batch_count),
        openai_sync_latency_historical: latency_summary(&mut sync_latencies),
        openai_batch_latency_excluded: batch_excluded,
        openai_sync_zero_latency_excluded: sync_zero,
        jev_cost: cost_summary(jev_cost, successful_article_count),
        jev_latency_model: latency_summary(&mut model_latencies),
        jev_latency_total: latency_summary(&mut total_latencies),
        jev_pricing_version: config.pricing.pricing_version.clone(),
        retry_count: retries,
        stability: StabilityMetrics {
            repeated_articles: repeated,
            changed_articles: changed,
            choice_change_rate: Rate::new(changed, repeated),
        },
    }
}

fn cost_summary(total: u64, count: u64) -> CostSummary {
    let per = if count == 0 {
        0.0
    } else {
        total as f64 / count as f64
    };
    CostSummary {
        total_microdollars: total,
        per_article_microdollars: per,
        per_1000_articles_microdollars: per * 1000.0,
        article_count: count,
    }
}

pub fn latency_summary(values: &mut [u64]) -> Option<LatencySummary> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let percentile = |p: f64| values[((values.len() as f64 * p).ceil() as usize).saturating_sub(1)];
    Some(LatencySummary {
        count: values.len() as u64,
        median_ms: percentile(0.50),
        p95_ms: percentile(0.95),
    })
}
