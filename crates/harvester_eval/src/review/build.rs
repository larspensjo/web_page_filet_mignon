//! Construction of the deliberately provider-free disagreement file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::DatasetSelection;
use crate::manifest::Manifest;
use crate::runner::load_run_data;
use crate::runner::store::{select_records, ResultRecord};

pub const REVIEW_HEADER: [&str; 9] = [
    "review_row",
    "article_id",
    "title",
    "text_path",
    "excerpt",
    "priority_a",
    "priority_b",
    "your_priority",
    "notes",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewCsvRow {
    pub review_row: u64,
    pub article_id: String,
    pub title: String,
    pub text_path: String,
    pub excerpt: String,
    pub priority_a: u8,
    pub priority_b: u8,
    pub your_priority: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewKeyRow {
    pub review_row: u64,
    pub article_id: String,
    pub a_provider: String,
    pub b_provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewKey {
    pub blinding_seed: u64,
    pub run_id: String,
    pub rows: Vec<ReviewKeyRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewFailure {
    pub article_id: String,
    pub repetition: u32,
    pub outcome: String,
    pub error_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewBuild {
    pub rows: Vec<ReviewCsvRow>,
    pub key: ReviewKey,
    pub failures: Vec<ReviewFailure>,
}

/// Builds review rows from explicitly supplied article text. The explicit map
/// is intentional: no provider output or rationale can accidentally leak into
/// this file through a broad serialisation of a manifest/result object.
pub fn build_review_rows(
    manifest: &Manifest,
    records: &[ResultRecord],
    texts: &BTreeMap<String, String>,
    run_id: &str,
    blinding_seed: u64,
    excerpt_bytes: usize,
) -> ReviewBuild {
    let articles: BTreeMap<_, _> = manifest
        .articles
        .iter()
        .map(|a| (a.article_id.as_str(), a))
        .collect();
    let (selected, _) = select_records(records);
    let mut primary_by_article: BTreeMap<String, ResultRecord> = BTreeMap::new();
    for record in selected {
        primary_by_article
            .entry(record.article_id.clone())
            .and_modify(|current| {
                if record.repetition < current.repetition {
                    *current = record.clone();
                }
            })
            .or_insert(record);
    }
    let mut candidates = Vec::new();
    let mut failures = Vec::new();
    for record in primary_by_article.into_values() {
        let Some(article) = articles.get(record.article_id.as_str()) else {
            continue;
        };
        if record.outcome != "ok" || record.priority.is_none() {
            failures.push(ReviewFailure {
                article_id: record.article_id,
                repetition: record.repetition,
                outcome: record.outcome,
                error_type: record.error_type,
            });
            continue;
        }
        let priority = record.priority.expect("checked above");
        if priority == article.baseline.priority {
            continue;
        }
        let boundary = (article.baseline.priority <= 2 && priority >= 3)
            || (priority <= 2 && article.baseline.priority >= 3);
        candidates.push((article, priority, boundary));
    }
    candidates.sort_by(|left, right| {
        let left_gap = left.0.baseline.priority.abs_diff(left.1);
        let right_gap = right.0.baseline.priority.abs_diff(right.1);
        right_gap
            .cmp(&left_gap)
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| left.0.article_id.cmp(&right.0.article_id))
    });
    let mut rows = Vec::new();
    let mut key_rows = Vec::new();
    for (index, (article, jev_priority, _)) in candidates.into_iter().enumerate() {
        let review_row = index as u64 + 1;
        let a_is_jev = assignment_bit(blinding_seed, &article.article_id, review_row);
        let (priority_a, priority_b, a_provider, b_provider) = if a_is_jev {
            (jev_priority, article.baseline.priority, "jev", "openai")
        } else {
            (article.baseline.priority, jev_priority, "openai", "jev")
        };
        let text = texts
            .get(&article.article_id)
            .map(String::as_str)
            .unwrap_or("");
        rows.push(ReviewCsvRow {
            review_row,
            article_id: article.article_id.clone(),
            title: article
                .article_file
                .as_ref()
                .and_then(|f| f.title.clone())
                .unwrap_or_default(),
            text_path: article.text_path.clone(),
            excerpt: excerpt(text, excerpt_bytes),
            priority_a,
            priority_b,
            your_priority: String::new(),
            notes: String::new(),
        });
        key_rows.push(ReviewKeyRow {
            review_row,
            article_id: article.article_id.clone(),
            a_provider: a_provider.into(),
            b_provider: b_provider.into(),
        });
    }
    ReviewBuild {
        rows,
        key: ReviewKey {
            blinding_seed,
            run_id: run_id.into(),
            rows: key_rows,
        },
        failures,
    }
}

fn assignment_bit(seed: u64, article_id: &str, row: u64) -> bool {
    let digest = Sha256::digest(format!("{seed}:{row}:{article_id}").as_bytes());
    digest[0] & 1 == 1
}

pub fn excerpt(text: &str, max_bytes: usize) -> String {
    let end = text
        .char_indices()
        .take_while(|(index, _)| *index < max_bytes)
        .map(|(index, character)| index + character.len_utf8())
        .last()
        .unwrap_or(0);
    text[..end.min(text.len())].to_string()
}

pub fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.into()
    }
}

pub fn render_csv(rows: &[ReviewCsvRow]) -> String {
    let mut out = REVIEW_HEADER.join(",");
    out.push('\n');
    for row in rows {
        let fields = [
            row.review_row.to_string(),
            row.article_id.clone(),
            row.title.clone(),
            row.text_path.clone(),
            row.excerpt.clone(),
            row.priority_a.to_string(),
            row.priority_b.to_string(),
            row.your_priority.clone(),
            row.notes.clone(),
        ];
        out.push_str(
            &fields
                .iter()
                .map(|field| csv_escape(field))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out
}

pub fn write_review_files(
    experiment_dir: &Path,
    run_id: &str,
    seed: u64,
    excerpt_bytes: usize,
    out: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    write_review_files_with_split(experiment_dir, run_id, seed, excerpt_bytes, None, out)
}

pub fn write_review_files_with_split(
    experiment_dir: &Path,
    run_id: &str,
    seed: u64,
    excerpt_bytes: usize,
    split: Option<DatasetSelection>,
    out: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    let data = load_run_data(experiment_dir, run_id)?;
    let records = data
        .records
        .iter()
        .filter(|record| match split {
            None | Some(DatasetSelection::All) => true,
            Some(DatasetSelection::Dev) => record.split == "dev",
            Some(DatasetSelection::Heldout) => record.split == "heldout",
        })
        .cloned()
        .collect::<Vec<_>>();
    let manifest_root = data
        .manifest_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("run manifest has no parent directory"))?;
    let texts = data
        .manifest
        .articles
        .iter()
        .filter_map(|article| {
            fs::read_to_string(manifest_root.join(&article.text_path))
                .ok()
                .map(|text| (article.article_id.clone(), text))
        })
        .collect();
    let build = build_review_rows(
        &data.manifest,
        &records,
        &texts,
        run_id,
        seed,
        excerpt_bytes,
    );
    let directory = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| experiment_dir.join("reports").join(run_id));
    fs::create_dir_all(&directory)?;
    let csv_path = directory.join("review.csv");
    fs::write(&csv_path, render_csv(&build.rows))?;
    engine_logging::engine_info!(
        "run_id={} operation=review-file artifact={}",
        run_id,
        csv_path.display()
    );
    let key_path = directory.join("review.key.json");
    fs::write(&key_path, serde_json::to_vec_pretty(&build.key)?)?;
    engine_logging::engine_info!(
        "run_id={} operation=review-file artifact={}",
        run_id,
        key_path.display()
    );
    Ok(directory)
}
