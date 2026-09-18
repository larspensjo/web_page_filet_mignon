//! Unblinded diagnosis after scoring.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::manifest::Manifest;
use crate::review::build::{ReviewKey, REVIEW_HEADER};
use crate::review::score::parse_csv;
use crate::runner::load_run_data;
use crate::runner::store::{select_records, ResultRecord};

pub fn render_diagnosis(
    csv: &str,
    key: &ReviewKey,
    manifest: &Manifest,
    records: &[ResultRecord],
) -> anyhow::Result<String> {
    let fields = parse_csv(csv)?;
    if fields
        .first()
        .is_none_or(|header| header.iter().map(String::as_str).collect::<Vec<_>>() != REVIEW_HEADER)
    {
        anyhow::bail!("review CSV columns do not match the required blinded schema");
    }
    if fields.len().saturating_sub(1) != key.rows.len() {
        anyhow::bail!("review/key mismatch for diagnosis");
    }
    let articles: BTreeMap<_, _> = manifest
        .articles
        .iter()
        .map(|a| (a.article_id.as_str(), a))
        .collect();
    let (selected, _) = select_records(records);
    let mut jev: BTreeMap<&str, &ResultRecord> = BTreeMap::new();
    for record in selected.iter().filter(|r| r.outcome == "ok") {
        jev.entry(record.article_id.as_str())
            .and_modify(|current| {
                if record.repetition < current.repetition {
                    *current = record;
                }
            })
            .or_insert(record);
    }
    let mut out = String::from("# Unblinded diagnosis\n\nThis file is unblinded after review. It covers the review rows and may contain provider names, rationales, tags, categories and probabilities.\n\n");
    for (index, fields) in fields.iter().skip(1).enumerate() {
        let key_row = &key.rows[index];
        if fields.len() != REVIEW_HEADER.len() {
            anyhow::bail!(
                "diagnosis review row {} has {} columns",
                index + 1,
                fields.len()
            );
        }
        let row = fields[0].parse::<u64>().map_err(|_| {
            anyhow::anyhow!(
                "diagnosis review row {} has invalid review_row {:?}",
                index + 1,
                fields[0]
            )
        })?;
        let csv_article_id = fields.get(1).map(String::as_str).unwrap_or_default();
        if row != key_row.review_row || csv_article_id != key_row.article_id {
            anyhow::bail!(
                "review/key mismatch on row {row}: expected review row {} article {}, found article {}",
                key_row.review_row,
                key_row.article_id,
                csv_article_id
            );
        }
        let article = articles
            .get(key_row.article_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("article {} is not in manifest", key_row.article_id))?;
        let record = jev.get(key_row.article_id.as_str()).copied();
        let (openai_priority, jev_priority) = if key_row.a_provider == "openai" {
            (fields[5].clone(), fields[6].clone())
        } else {
            (fields[6].clone(), fields[5].clone())
        };
        out.push_str(&format!("## Review row {} — {}\n\n- Article: `{}`\n- Frozen text: [{}](../../{})\n- OpenAI priority: {}\n- Jev priority: {}\n- User priority: {}\n- Notes: {}\n- OpenAI rationale: {}\n- OpenAI category: {}\n- OpenAI tags: {}\n", key_row.review_row, key_row.article_id, key_row.article_id, article.text_path, article.text_path, openai_priority, jev_priority, fields.get(7).cloned().unwrap_or_default(), fields.get(8).cloned().unwrap_or_default(), article.baseline.rationale, article.baseline.category, article.baseline.tags.join(", ")));
        if let Some(record) = record {
            out.push_str(&format!("- Jev priority probabilities: {}\n- Jev confidence: {:?}\n- Jev relevance: {:?}\n- Jev relevance probabilities: {}\n- Jev category probabilities: {}\n- Jev selected categories: {}\n- Jev tag probabilities: {}\n- Jev selected tags: {}\n", json(record.priority_probabilities.as_ref()), record.priority_confidence, record.relevance_score, json(record.relevance_probabilities.as_ref()), json(record.category_probabilities.as_ref()), join(record.categories_selected.as_ref()), json(record.tag_probabilities.as_ref()), join(record.tags_selected.as_ref())));
        } else {
            out.push_str("- Jev record: unavailable or failed\n");
        }
        out.push('\n');
    }
    Ok(out)
}

fn json<T: serde::Serialize>(value: T) -> String {
    serde_json::to_string(&value).unwrap_or_else(|_| "null".into())
}
fn join(values: Option<&Vec<String>>) -> String {
    values.map_or_else(|| "[]".into(), |values| values.join(", "))
}

pub fn write_diagnosis(
    experiment_dir: &Path,
    run_id: &str,
    review: Option<&Path>,
    out: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    let directory = experiment_dir.join("reports").join(run_id);
    let review_path = review
        .map(Path::to_path_buf)
        .unwrap_or_else(|| directory.join("review.csv"));
    let csv = fs::read_to_string(&review_path)?;
    let key: ReviewKey = serde_json::from_slice(&fs::read(directory.join("review.key.json"))?)?;
    if key.run_id != run_id {
        anyhow::bail!(
            "review key run mismatch: expected {run_id}, found {}",
            key.run_id
        );
    }
    let data = load_run_data(experiment_dir, run_id)?;
    let diagnosis = render_diagnosis(&csv, &key, &data.manifest, &data.records)?;
    let path = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| directory.join("diagnosis.md"));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, diagnosis)?;
    engine_logging::engine_info!(
        "run_id={} operation=diagnose artifact={}",
        run_id,
        path.display()
    );
    Ok(path)
}
