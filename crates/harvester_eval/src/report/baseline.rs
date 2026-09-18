//! Baseline-only metrics from frozen OpenAI replay records.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::identity::PathKind;
use crate::manifest::{load_manifest, Manifest};
use crate::metrics::small_sample::{rate_label as shared_rate_label, DEFAULT_FLOOR};

pub use crate::metrics::operational::LatencySummary;

/// Stable, machine-readable baseline figures.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BaselineMetrics {
    pub manifest_hash: String,
    pub article_count: u64,
    pub priority_histogram: BTreeMap<u8, u64>,
    pub category_histogram: BTreeMap<String, u64>,
    pub tag_histogram: BTreeMap<String, u64>,
    pub tag_cardinality_histogram: BTreeMap<usize, u64>,
    pub cost_microdollars_total: u64,
    pub cost_microdollars_sync: u64,
    pub cost_microdollars_batch: u64,
    pub cost_microdollars_per_article: f64,
    pub cost_microdollars_per_sync_article: f64,
    pub cost_microdollars_per_batch_article: f64,
    pub sync_records: u64,
    pub batch_records: u64,
    pub batch_latency_excluded: u64,
    pub sync_zero_latency_excluded: u64,
    pub historical_sync_latency: Option<LatencySummary>,
}

/// Computes figures without IO.
pub fn compute(manifest: &Manifest) -> BaselineMetrics {
    let mut priority_histogram = BTreeMap::new();
    let mut category_histogram = BTreeMap::new();
    let mut tag_histogram = BTreeMap::new();
    let mut tag_cardinality_histogram = BTreeMap::new();
    let mut sync_latencies = Vec::new();
    let mut total = 0;
    let mut sync_cost = 0;
    let mut batch_cost = 0;
    let mut sync_records = 0;
    let mut batch_records = 0;
    let mut batch_latency_excluded = 0;
    let mut sync_zero_latency_excluded = 0;
    for article in &manifest.articles {
        let baseline = &article.baseline;
        *priority_histogram.entry(baseline.priority).or_default() += 1;
        *category_histogram
            .entry(baseline.category.clone())
            .or_default() += 1;
        *tag_cardinality_histogram
            .entry(baseline.tags.len())
            .or_default() += 1;
        for tag in &baseline.tags {
            *tag_histogram.entry(tag.clone()).or_default() += 1;
        }
        total += baseline.cost_microdollars;
        match baseline.path_kind {
            PathKind::Sync => {
                sync_records += 1;
                sync_cost += baseline.cost_microdollars;
                if baseline.wall_ms > 0 {
                    sync_latencies.push(baseline.wall_ms);
                } else {
                    sync_zero_latency_excluded += 1;
                }
            }
            PathKind::Batch => {
                batch_records += 1;
                batch_cost += baseline.cost_microdollars;
                batch_latency_excluded += 1;
            }
        }
    }
    let article_count = manifest.articles.len() as u64;
    BaselineMetrics {
        manifest_hash: manifest.manifest_hash.clone(),
        article_count,
        priority_histogram,
        category_histogram,
        tag_histogram,
        tag_cardinality_histogram,
        cost_microdollars_total: total,
        cost_microdollars_sync: sync_cost,
        cost_microdollars_batch: batch_cost,
        cost_microdollars_per_article: if article_count == 0 {
            0.0
        } else {
            total as f64 / article_count as f64
        },
        cost_microdollars_per_sync_article: per_article(sync_cost, sync_records),
        cost_microdollars_per_batch_article: per_article(batch_cost, batch_records),
        sync_records,
        batch_records,
        batch_latency_excluded,
        sync_zero_latency_excluded,
        historical_sync_latency: crate::metrics::operational::latency_summary(&mut sync_latencies),
    }
}

fn per_article(cost: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        cost as f64 / count as f64
    }
}

/// Writes metrics into the baseline report directory and returns it.
pub fn write_baseline_report(manifest_path: &Path, out: Option<&Path>) -> anyhow::Result<PathBuf> {
    let manifest = load_manifest(manifest_path)?;
    let metrics = compute(&manifest);
    let directory = out.map(Path::to_path_buf).unwrap_or_else(|| {
        manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("reports")
            .join(format!("baseline-{}", &manifest.manifest_hash[..16]))
    });
    fs::create_dir_all(&directory)?;
    fs::write(
        directory.join("metrics.json"),
        serde_json::to_vec_pretty(&metrics)?,
    )?;
    fs::write(directory.join("metrics.md"), render_markdown(&metrics))?;
    Ok(directory)
}

fn render_markdown(metrics: &BaselineMetrics) -> String {
    let mut markdown = format!("# OpenAI baseline metrics\n\nManifest: `{}`\n\n## Quality reference distribution\n\nPriority histogram:\n\n", metrics.manifest_hash);
    for (priority, count) in &metrics.priority_histogram {
        markdown.push_str(&format!(
            "- {priority}: {}\n",
            rate_label(*count, metrics.article_count)
        ));
    }
    markdown.push_str("\nCategory histogram:\n\n");
    for (category, count) in &metrics.category_histogram {
        markdown.push_str(&format!(
            "- {category}: {}\n",
            rate_label(*count, metrics.article_count)
        ));
    }
    markdown.push_str("\nTag histogram:\n\n");
    for (tag, count) in &metrics.tag_histogram {
        markdown.push_str(&format!(
            "- {tag}: {}\n",
            rate_label(*count, metrics.article_count)
        ));
    }
    markdown.push_str("\nTag cardinality histogram:\n\n");
    for (cardinality, count) in &metrics.tag_cardinality_histogram {
        markdown.push_str(&format!(
            "- {cardinality} tags: {}\n",
            rate_label(*count, metrics.article_count)
        ));
    }
    markdown.push_str("\n## Operational observations\n\n");
    markdown.push_str(&format!(
        "Cost: {} microdollars total; {:.2} microdollars/article. Sync: {} total, {:.2}/sync article. Batch: {} total, {:.2}/batch article.\n\n",
        metrics.cost_microdollars_total,
        metrics.cost_microdollars_per_article,
        metrics.cost_microdollars_sync,
        metrics.cost_microdollars_per_sync_article,
        metrics.cost_microdollars_batch,
        metrics.cost_microdollars_per_batch_article
    ));
    markdown.push_str(&format!("Historical, recorded under past conditions: OpenAI latency is calculated only from non-zero sync observations. {} batch rows are excluded; {} sync rows with wall_ms = 0 are also excluded.\n", metrics.batch_latency_excluded, metrics.sync_zero_latency_excluded));
    if let Some(latency) = &metrics.historical_sync_latency {
        if latency.count < DEFAULT_FLOOR {
            markdown.push_str(&format!(
                "Sync historical latency: n = {}, percentiles not reported below the small-sample floor of {}.\n",
                latency.count, DEFAULT_FLOOR
            ));
        } else {
            markdown.push_str(&format!(
                "Sync historical latency: median {} ms, p95 {} ms (n = {}).\n",
                latency.median_ms, latency.p95_ms, latency.count
            ));
        }
    }
    markdown
}

/// Formats a rate only when it has sufficient support.
pub fn rate_label(numerator: u64, denominator: u64) -> String {
    shared_rate_label(numerator, denominator, DEFAULT_FLOOR)
}
