//! Full Jev-vs-baseline report writer.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Transport;
use crate::metrics::compare::{compute, ComparisonMetrics};
use crate::metrics::small_sample::DEFAULT_FLOOR;
use crate::runner::load_run_data;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonReport {
    pub manifest_hash: String,
    pub config_hash: String,
    pub rubric_hash: String,
    pub split: String,
    pub run_id: String,
    pub transport: Transport,
    pub tool_version: String,
    pub metrics: ComparisonMetrics,
}

pub fn write_comparison_report(
    experiment_dir: &Path,
    run_id: &str,
    out: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    let data = load_run_data(experiment_dir, run_id)?;
    let metrics = compute(&data.manifest, &data.records, &data.file.resolved_config);
    let report = ComparisonReport {
        manifest_hash: data.file.identity.manifest_hash,
        config_hash: data.file.identity.config_hash,
        rubric_hash: data.file.identity.rubric_sha256,
        split: format!("{:?}", data.file.identity.split).to_ascii_lowercase(),
        run_id: run_id.into(),
        transport: data.file.transport_kind,
        tool_version: data.file.identity.tool_version,
        metrics,
    };
    let directory = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| experiment_dir.join("reports").join(run_id));
    fs::create_dir_all(&directory)?;
    let json_path = directory.join("metrics.json");
    fs::write(&json_path, serde_json::to_vec_pretty(&report)?)?;
    engine_logging::engine_info!(
        "run_id={} operation=report artifact={}",
        run_id,
        json_path.display()
    );
    let markdown_path = directory.join("metrics.md");
    fs::write(&markdown_path, render_markdown(&report))?;
    engine_logging::engine_info!(
        "run_id={} operation=report artifact={}",
        run_id,
        markdown_path.display()
    );
    Ok(directory)
}

pub fn render_markdown(report: &ComparisonReport) -> String {
    let m = &report.metrics;
    let mut text = format!(
        "# Jev comparison metrics\n\nManifest: `{}`  \nConfig: `{}`  \nRubric: `{}`  \nSplit: `{}`  \nRun: `{}`  \nTransport: `{:?}`  \nTool version: `{}`\n\n",
        report.manifest_hash, report.config_hash, report.rubric_hash, report.split, report.run_id, report.transport, report.tool_version
    );
    text.push_str("> OpenAI is a recorded reference, not ground truth. OpenAI batch latency is unknowable and excluded. Agreements were not reviewed.\n\n");
    text.push_str(&format!("## Quality\n\nSuccessful article/repetition pairs: {} / {} attempted pairs.\n\nArticles with at least one successful Jev record: {} / {} attempted articles. Articles with no successful Jev record: {} (baseline priority 4–5: {}; baseline priority 5: {}).\n\n", m.successful_records, m.attempted_records, m.articles_with_successful_record, m.article_count, m.articles_without_successful_record, m.failed_high_priority_articles, m.failed_priority_five_articles));
    text.push_str("### Class counts (successful pairs only)\n\n|Priority|OpenAI reference|Jev|\n|---:|---:|---:|\n");
    for priority in 1..=5 {
        text.push_str(&format!(
            "|{priority}|{}|{}|\n",
            m.baseline_priority_histogram
                .get(&priority)
                .copied()
                .unwrap_or_default(),
            m.jev_priority_histogram
                .get(&priority)
                .copied()
                .unwrap_or_default()
        ));
    }
    text.push_str(&format!(
        "\nExact agreement: {}\n\nWithin one level: {}\n\nMean absolute priority error: {}\n\n",
        m.exact_agreement.label(DEFAULT_FLOOR),
        m.within_one.label(DEFAULT_FLOOR),
        m.mean_absolute_error
            .map_or_else(|| "not available".into(), |v| format!("{v:.3}"))
    ));
    text.push_str("### Confusion matrix (OpenAI rows, Jev columns)\n\n| |1|2|3|4|5|\n|---|---:|---:|---:|---:|---:|\n");
    for (index, row) in m.confusion_matrix.iter().enumerate() {
        text.push_str(&format!(
            "|{}|{}|{}|{}|{}|{}|\n",
            index + 1,
            row[0],
            row[1],
            row[2],
            row[3],
            row[4]
        ));
    }
    text.push_str(&format!("\nPriority 5 precision: {}. Priority 5 recall: {}.\n\nHigh-priority (4+5) precision: {}. High-priority recall: {}.\n\nSevere demotions (5 → 1–2): {}. Severe promotions (1–2 → 5): {}.\n\n", m.priority_five_precision.label(DEFAULT_FLOOR), m.priority_five_recall.label(DEFAULT_FLOOR), m.high_priority_precision.label(DEFAULT_FLOOR), m.high_priority_recall.label(DEFAULT_FLOOR), m.severe_demotions, m.severe_promotions));
    text.push_str("### Categories\n\nCategories are diagnostic only; there is no baseline category score.\n\n");
    for (category, rate) in &m.category.positive_rates {
        text.push_str(&format!("- {category}: {}\n", rate.label(DEFAULT_FLOOR)));
    }
    text.push_str(&format!(
        "- none of the five: {}\n\n",
        m.category.zero_category_share.label(DEFAULT_FLOOR)
    ));
    text.push_str(&format!(
        "### Tags\n\nMean Jaccard: {} ({} contributing pairs; {} pairs with both tag sets empty excluded).\n\n",
        m.tags
            .mean_jaccard
            .map_or_else(|| "not available".into(), |v| format!("{v:.3}")),
        m.tags.articles,
        m.tags.both_empty_excluded
    ));
    for (tag, metric) in &m.tags.per_tag {
        text.push_str(&format!(
            "- {tag}: precision {}, recall {}, support {}\n",
            metric.precision.label(DEFAULT_FLOOR),
            metric.recall.label(DEFAULT_FLOOR),
            metric.support
        ));
    }
    text.push_str("\n## Operational observations\n\n");
    text.push_str(&format!("Failures: {}. By outcome: {:?}. By error type: {:?}. Superseded records: {}. Retries: {}.\n\n", m.failure_count, m.failures_by_outcome, m.failures_by_error_type, m.superseded_records, m.retry_count));
    text.push_str("### Failed article/repetition pairs\n\n|Article ID|Repetition|Outcome|Error type|\n|---|---:|---|---|\n");
    if m.failures.is_empty() {
        text.push_str("|_none_|—|—|—|\n");
    } else {
        for failure in &m.failures {
            text.push_str(&format!(
                "|`{}`|{}|{}|{}|\n",
                failure.article_id,
                failure.repetition,
                failure.outcome,
                failure.error_type.as_deref().unwrap_or("unknown")
            ));
        }
    }
    text.push_str("\nThe review file deterministically uses the lowest repetition for each article; if that repetition failed, the article is excluded even when another repetition succeeded.\n\n");
    let o = &m.operational;
    text.push_str(&format!("Cost basis: {} articles with a successful Jev record out of {} attempted articles. OpenAI cost: {} microdollars total, {:.2}/article, {:.2}/1,000. Sync: {} total. Batch: {} total. Cost is observed only and is never a gate.\n\n", o.successful_article_count, o.attempted_article_count, o.openai_cost.total_microdollars, o.openai_cost.per_article_microdollars, o.openai_cost.per_1000_articles_microdollars, o.openai_sync_cost.total_microdollars, o.openai_batch_cost.total_microdollars));
    text.push_str(&format!(
        "Jev cost: {} microdollars total, {:.2}/article, {:.2}/1,000 (pricing `{}`).\n\n",
        o.jev_cost.total_microdollars,
        o.jev_cost.per_article_microdollars,
        o.jev_cost.per_1000_articles_microdollars,
        o.jev_pricing_version
    ));
    text.push_str(&format!("Historical OpenAI latency uses sync records only; {} batch rows and {} zero-latency sync rows excluded.\n\n", o.openai_batch_latency_excluded, o.openai_sync_zero_latency_excluded));
    if let Some(latency) = &o.openai_sync_latency_historical {
        if latency.count < DEFAULT_FLOOR {
            text.push_str(&format!("OpenAI historical sync latency: n = {}, percentiles not reported below the small-sample floor of {}.\n\n", latency.count, DEFAULT_FLOOR));
        } else {
            text.push_str(&format!(
                "OpenAI historical sync latency: median {} ms, p95 {} ms (n = {}).\n\n",
                latency.median_ms, latency.p95_ms, latency.count
            ));
        }
    }
    if let Some(latency) = &o.jev_latency_model {
        text.push_str(&format!(
            "Jev model-only latency: median {} ms, p95 {} ms (n = {}).\n\n",
            latency.median_ms, latency.p95_ms, latency.count
        ));
    }
    if let Some(latency) = &o.jev_latency_total {
        text.push_str(&format!(
            "Jev total elapsed latency: median {} ms, p95 {} ms (n = {}).\n\n",
            latency.median_ms, latency.p95_ms, latency.count
        ));
    }
    text.push_str(&format!(
        "Stability over repetitions: {} repeated articles, {} changed, change rate {}.\n",
        o.stability.repeated_articles,
        o.stability.changed_articles,
        o.stability.choice_change_rate.label(DEFAULT_FLOOR)
    ));
    text
}
