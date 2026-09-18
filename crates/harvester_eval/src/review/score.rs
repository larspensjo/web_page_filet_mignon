//! Pure scoring of the filled, blinded review file.

use serde::{Deserialize, Serialize};

use super::build::{ReviewKey, REVIEW_HEADER};
use crate::config::{AcceptanceConfig, RunConfig};
use crate::manifest::Manifest;
use crate::metrics::operational::compute as compute_operational;
use crate::metrics::small_sample::Rate;
use crate::runner::store::ResultRecord;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArmScore {
    pub exact_agreement: Rate,
    pub within_one: Rate,
    pub mean_absolute_error: Option<f64>,
    pub high_priority_recall: Rate,
    pub high_priority_precision: Rate,
    pub severe_miss_rate: Rate,
    pub severe_false_positive_rate: Rate,
    pub admission_boundary_agreement: Rate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Evaluation {
    Pass,
    Fail,
    Inconclusive,
}

impl Evaluation {
    fn label(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Inconclusive => "inconclusive",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThresholdResult {
    pub name: String,
    pub status: Evaluation,
    pub observed: Option<f64>,
    pub threshold: Option<f64>,
    pub numerator: u64,
    pub denominator: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoppingRule {
    pub condition_a_large_gaps_reviewed: bool,
    pub condition_b_minimum_rows: bool,
    pub condition_c_high_priority_support: bool,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoreReport {
    pub biased_subset_statement: String,
    pub reviewed_rows: u64,
    pub blank_rows: u64,
    pub total_review_rows: u64,
    pub openai: ArmScore,
    pub jev: ArmScore,
    pub stopping_rule: StoppingRule,
    pub thresholds: Vec<ThresholdResult>,
    pub overall: Evaluation,
    pub small_sample_floor: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewedRow {
    row: u64,
    article_id: String,
    openai: u8,
    jev: u8,
    user: u8,
}

pub fn score_review(
    csv: &str,
    key: &ReviewKey,
    manifest: &Manifest,
    records: &[ResultRecord],
    config: &RunConfig,
) -> anyhow::Result<ScoreReport> {
    if let Some(record) = records.iter().find(|record| record.run_id != key.run_id) {
        anyhow::bail!(
            "review key run mismatch: key is for {}, result record is for {}",
            key.run_id,
            record.run_id
        );
    }
    let fields = parse_csv(csv)?;
    validate_header(fields.first())?;
    let rows = &fields[1..];
    if rows.len() != key.rows.len() {
        anyhow::bail!(
            "review/key mismatch: row count {} vs {}; leave unreviewed rows in place with a blank your_priority",
            rows.len(),
            key.rows.len()
        );
    }
    let mut reviewed = Vec::new();
    let mut blank_rows = 0;
    for (index, fields) in rows.iter().enumerate() {
        if fields.len() != 9 {
            anyhow::bail!("review row {} has {} columns", index + 1, fields.len());
        }
        let row = parse_u64(&fields[0], "review_row")?;
        let key_row = &key.rows[index];
        if row != key_row.review_row || fields[1] != key_row.article_id {
            anyhow::bail!("review/key mismatch on row {} or article id", row);
        }
        let priority_a = parse_priority(&fields[5], row, "priority_a")?;
        let priority_b = parse_priority(&fields[6], row, "priority_b")?;
        if fields[7].trim().is_empty() {
            blank_rows += 1;
            continue;
        }
        let user = parse_priority(&fields[7], row, "your_priority")?;
        let (openai, jev) = match (key_row.a_provider.as_str(), key_row.b_provider.as_str()) {
            ("openai", "jev") => (priority_a, priority_b),
            ("jev", "openai") => (priority_b, priority_a),
            _ => anyhow::bail!("review key row {} has invalid providers", row),
        };
        reviewed.push(ReviewedRow {
            row,
            article_id: fields[1].clone(),
            openai,
            jev,
            user,
        });
    }
    let openai = arm_score(&reviewed, |row| row.openai);
    let jev = arm_score(&reviewed, |row| row.jev);
    let stopping_rule = stopping_rule(&reviewed, rows, key, config);
    let operational = compute_operational(manifest, records, config);
    let thresholds = threshold_results(
        &jev,
        &stopping_rule,
        config.acceptance.as_ref(),
        operational.jev_latency_model.as_ref().map(|v| v.p95_ms),
    );
    let overall = if thresholds.iter().any(|t| t.status == Evaluation::Fail) {
        Evaluation::Fail
    } else if thresholds
        .iter()
        .any(|t| t.status == Evaluation::Inconclusive)
    {
        Evaluation::Inconclusive
    } else {
        Evaluation::Pass
    };
    Ok(ScoreReport { biased_subset_statement: "Measured on the biased reviewed subset: only disagreements were reviewed; agreements were never verified.".into(), reviewed_rows: reviewed.len() as u64, blank_rows, total_review_rows: rows.len() as u64, openai, jev, stopping_rule, thresholds, overall, small_sample_floor: config.review.min_high_priority_support })
}

fn arm_score(rows: &[ReviewedRow], get: impl Fn(&ReviewedRow) -> u8) -> ArmScore {
    let mut exact = 0;
    let mut within = 0;
    let mut error = 0_u64;
    let mut high_true = 0;
    let mut predicted_high = 0;
    let mut actual_high_count = 0;
    let mut severe_miss = 0;
    let mut severe_fp = 0;
    let mut boundary = 0;
    for row in rows {
        let actual = row.user;
        let value = get(row);
        exact += u64::from(value == actual);
        within += u64::from(value.abs_diff(actual) <= 1);
        error += u64::from(value.abs_diff(actual));
        let value_high = value >= 4;
        let actual_high = actual >= 4;
        predicted_high += u64::from(value_high);
        actual_high_count += u64::from(actual_high);
        high_true += u64::from(value_high && actual_high);
        severe_miss += u64::from(value <= 2 && actual_high);
        severe_fp += u64::from(value == 5 && actual <= 2);
        boundary += u64::from((value >= 3) == (actual >= 3));
    }
    ArmScore {
        exact_agreement: Rate::new(exact, rows.len() as u64),
        within_one: Rate::new(within, rows.len() as u64),
        mean_absolute_error: (!rows.is_empty()).then(|| error as f64 / rows.len() as f64),
        high_priority_recall: Rate::new(high_true, actual_high_count),
        high_priority_precision: Rate::new(high_true, predicted_high),
        severe_miss_rate: Rate::new(severe_miss, actual_high_count),
        severe_false_positive_rate: Rate::new(
            severe_fp,
            rows.iter().filter(|r| r.user <= 2).count() as u64,
        ),
        admission_boundary_agreement: Rate::new(boundary, rows.len() as u64),
    }
}

fn stopping_rule(
    reviewed: &[ReviewedRow],
    raw_rows: &[Vec<String>],
    key: &ReviewKey,
    config: &RunConfig,
) -> StoppingRule {
    let reviewed_ids: std::collections::BTreeSet<u64> = reviewed.iter().map(|r| r.row).collect();
    let mut large_missing = 0;
    for (index, row) in raw_rows.iter().enumerate() {
        let gap = row[5]
            .trim()
            .parse::<u8>()
            .unwrap_or_default()
            .abs_diff(row[6].trim().parse::<u8>().unwrap_or_default());
        if config.review.require_large_gaps_reviewed
            && gap >= config.review.large_gap
            && !reviewed_ids.contains(&key.rows[index].review_row)
        {
            large_missing += 1;
        }
    }
    let condition_a = !config.review.require_large_gaps_reviewed || large_missing == 0;
    let condition_b = reviewed.len() as u64 >= config.review.min_reviewed_rows
        || reviewed.len() == raw_rows.len();
    let high_support = reviewed.iter().filter(|r| r.user >= 4).count() as u64;
    let condition_c = high_support >= config.review.min_high_priority_support;
    let mut missing = Vec::new();
    if !condition_a {
        missing.push(format!(
            "(a): {large_missing} large-gap rows remain unreviewed"
        ));
    }
    if !condition_b {
        missing.push(format!(
            "(b): {} more reviewed rows needed",
            config
                .review
                .min_reviewed_rows
                .saturating_sub(reviewed.len() as u64)
        ));
    }
    if !condition_c {
        missing.push(format!(
            "(c): {} more U >= 4 labels needed",
            config
                .review
                .min_high_priority_support
                .saturating_sub(high_support)
        ));
    }
    StoppingRule {
        condition_a_large_gaps_reviewed: condition_a,
        condition_b_minimum_rows: condition_b,
        condition_c_high_priority_support: condition_c,
        missing,
    }
}

fn threshold_results(
    jev: &ArmScore,
    stopping: &StoppingRule,
    acceptance: Option<&AcceptanceConfig>,
    p95: Option<u64>,
) -> Vec<ThresholdResult> {
    let support_ok = stopping.condition_c_high_priority_support
        && stopping.condition_a_large_gaps_reviewed
        && stopping.condition_b_minimum_rows;
    let mut results = Vec::new();
    let (recall_status, recall_observed, recall_threshold) = if let Some(acceptance) = acceptance {
        if support_ok {
            (
                if jev
                    .high_priority_recall
                    .value()
                    .is_some_and(|v| v >= acceptance.high_priority_recall_min)
                {
                    Evaluation::Pass
                } else {
                    Evaluation::Fail
                },
                jev.high_priority_recall.value(),
                Some(acceptance.high_priority_recall_min),
            )
        } else {
            (
                Evaluation::Inconclusive,
                jev.high_priority_recall.value(),
                Some(acceptance.high_priority_recall_min),
            )
        }
    } else {
        (
            Evaluation::Inconclusive,
            jev.high_priority_recall.value(),
            None,
        )
    };
    results.push(ThresholdResult {
        name: "high_priority_recall".into(),
        status: recall_status,
        observed: recall_observed,
        threshold: recall_threshold,
        numerator: jev.high_priority_recall.numerator,
        denominator: jev.high_priority_recall.denominator,
    });
    let (miss_status, miss_observed, miss_threshold) = if let Some(acceptance) = acceptance {
        if support_ok {
            (
                if jev
                    .severe_miss_rate
                    .value()
                    .is_some_and(|v| v <= acceptance.severe_miss_rate_max)
                {
                    Evaluation::Pass
                } else {
                    Evaluation::Fail
                },
                jev.severe_miss_rate.value(),
                Some(acceptance.severe_miss_rate_max),
            )
        } else {
            (
                Evaluation::Inconclusive,
                jev.severe_miss_rate.value(),
                Some(acceptance.severe_miss_rate_max),
            )
        }
    } else {
        (Evaluation::Inconclusive, jev.severe_miss_rate.value(), None)
    };
    results.push(ThresholdResult {
        name: "severe_miss_rate".into(),
        status: miss_status,
        observed: miss_observed,
        threshold: miss_threshold,
        numerator: jev.severe_miss_rate.numerator,
        denominator: jev.severe_miss_rate.denominator,
    });
    let (latency_status, latency_observed, latency_threshold) = if let Some(acceptance) = acceptance
    {
        if support_ok {
            (
                match p95 {
                    Some(value) if value <= acceptance.max_p95_latency_ms => Evaluation::Pass,
                    Some(_) => Evaluation::Fail,
                    None => Evaluation::Inconclusive,
                },
                p95.map(|v| v as f64),
                Some(acceptance.max_p95_latency_ms as f64),
            )
        } else {
            (
                Evaluation::Inconclusive,
                p95.map(|v| v as f64),
                Some(acceptance.max_p95_latency_ms as f64),
            )
        }
    } else {
        (Evaluation::Inconclusive, p95.map(|v| v as f64), None)
    };
    results.push(ThresholdResult {
        name: "max_p95_latency_ms".into(),
        status: latency_status,
        observed: latency_observed,
        threshold: latency_threshold,
        numerator: p95.unwrap_or_default(),
        denominator: p95.map_or(0, |_| 1),
    });
    results
}

fn parse_priority(value: &str, row: u64, field: &str) -> anyhow::Result<u8> {
    let parsed = value
        .trim()
        .parse::<u8>()
        .map_err(|_| anyhow::anyhow!("row {row} {field} must be an integer in 1..=5"))?;
    if !(1..=5).contains(&parsed) {
        anyhow::bail!("row {row} {field} must be in 1..=5, got {parsed}");
    }
    Ok(parsed)
}
fn parse_u64(value: &str, field: &str) -> anyhow::Result<u64> {
    value
        .parse()
        .map_err(|_| anyhow::anyhow!("{field} must be an integer"))
}

pub fn parse_csv(text: &str) -> anyhow::Result<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .chars()
        .peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
        } else {
            match ch {
                '"' if field.is_empty() => quoted = true,
                ',' => {
                    row.push(std::mem::take(&mut field));
                }
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    if row.len() == 1 && row[0].is_empty() {
                        row.clear();
                    } else {
                        rows.push(std::mem::take(&mut row));
                    }
                }
                '\r' => {}
                _ => field.push(ch),
            }
        }
    }
    if quoted {
        anyhow::bail!("unterminated quoted CSV field");
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn validate_header(header: Option<&Vec<String>>) -> anyhow::Result<()> {
    let Some(header) = header else {
        anyhow::bail!("review CSV is empty; expected blinded review header");
    };
    let cells = header.len().max(REVIEW_HEADER.len());
    for index in 0..cells {
        let actual = header.get(index).map(String::as_str).unwrap_or("<missing>");
        let expected = REVIEW_HEADER.get(index).copied().unwrap_or("<no column>");
        if actual != expected {
            anyhow::bail!(
                "review CSV header cell {} is {:?}; expected {:?}",
                index + 1,
                actual,
                expected
            );
        }
    }
    Ok(())
}

pub fn render_markdown(report: &ScoreReport) -> String {
    let mut text = format!(
        "# Review scores\n\n{}\n\nReviewed: {} / {} ({} blank).\n\n",
        report.biased_subset_statement,
        report.reviewed_rows,
        report.total_review_rows,
        report.blank_rows
    );
    for (name, arm) in [("OpenAI reference", &report.openai), ("Jev", &report.jev)] {
        text.push_str(&format!("## {name}\n\n- exact agreement: {}\n- within one: {}\n- mean absolute error: {}\n- high-priority recall: {}\n- high-priority precision: {}\n- severe-miss rate: {}\n- severe-false-positive rate: {}\n- admission-boundary agreement: {}\n\n", arm.exact_agreement.label(report.small_sample_floor), arm.within_one.label(report.small_sample_floor), arm.mean_absolute_error.map_or_else(|| "not available".into(), |v| format!("{v:.3}")), arm.high_priority_recall.label(report.small_sample_floor), arm.high_priority_precision.label(report.small_sample_floor), arm.severe_miss_rate.label(report.small_sample_floor), arm.severe_false_positive_rate.label(report.small_sample_floor), arm.admission_boundary_agreement.label(report.small_sample_floor)));
    }
    text.push_str(&format!("## Stopping rule\n\n(a) large gaps reviewed: {}\n\n(b) minimum rows: {}\n\n(c) high-priority support: {}\n\n", report.stopping_rule.condition_a_large_gaps_reviewed, report.stopping_rule.condition_b_minimum_rows, report.stopping_rule.condition_c_high_priority_support));
    for missing in &report.stopping_rule.missing {
        text.push_str(&format!("- {missing}\n"));
    }
    text.push_str(&format!(
        "\n## Threshold evaluation\n\nOverall: **{}**{}\n\n",
        report.overall.label(),
        if report.overall == Evaluation::Inconclusive {
            " — stopping rule not met or support is insufficient"
        } else {
            ""
        }
    ));
    for threshold in &report.thresholds {
        let (observed, expected, support) = if threshold.name == "max_p95_latency_ms" {
            (
                threshold
                    .observed
                    .map_or_else(|| "not available".into(), |value| format!("{value:.0} ms")),
                threshold
                    .threshold
                    .map_or_else(|| "not configured".into(), |value| format!("{value:.0} ms")),
                String::new(),
            )
        } else {
            (
                Rate::new(threshold.numerator, threshold.denominator)
                    .label(report.small_sample_floor),
                threshold
                    .threshold
                    .map_or_else(|| "not configured".into(), |value| format!("{value:.3}")),
                format!(", {}/{}", threshold.numerator, threshold.denominator),
            )
        };
        text.push_str(&format!(
            "- {}: **{}**, observed {}, threshold {}{}\n",
            threshold.name,
            threshold.status.label(),
            observed,
            expected,
            support
        ));
    }
    text
}
