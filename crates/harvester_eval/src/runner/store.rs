//! Durable append-only result records and crash-boundary recovery.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::config::Transport;

pub const RESULT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResultRecord {
    pub schema_version: u32,
    pub run_id: String,
    pub config_hash: String,
    pub manifest_hash: String,
    pub prompt_identity_hash: String,
    pub rubric_sha256: String,
    pub transport_kind: Transport,
    pub article_id: String,
    pub evidence_hash: String,
    pub split: String,
    pub repetition: u32,
    pub attempt_sequence: u32,
    pub provider: String,
    pub requested_model: String,
    pub returned_model: Option<String>,
    pub timestamp_utc: String,
    pub outcome: String,
    pub priority: Option<u8>,
    pub priority_probabilities: Option<BTreeMap<String, f64>>,
    pub priority_confidence: Option<f64>,
    pub p_high: Option<f64>,
    pub relevance_score: Option<f64>,
    pub relevance_legend: Option<BTreeMap<String, String>>,
    pub relevance_probabilities: Option<BTreeMap<String, f64>>,
    pub category_probabilities: Option<BTreeMap<String, f64>>,
    pub categories_selected: Option<Vec<String>>,
    pub category_threshold: f64,
    pub tag_probabilities: Option<BTreeMap<String, f64>>,
    pub tags_selected: Option<Vec<String>>,
    pub tag_threshold: f64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub first_attempt_latency_ms: Option<u64>,
    pub model_latency_ms: Option<u64>,
    pub total_elapsed_ms: u64,
    pub attempt_count: u32,
    #[serde(default)]
    pub retry_reasons: Vec<String>,
    pub http_status: Option<u16>,
    pub error_type: Option<String>,
    pub error_detail: Option<String>,
    pub estimated_cost_microdollars: Option<u64>,
    pub pricing_version: String,
    pub request_bytes: u64,
    pub response_bytes: u64,
    #[serde(default)]
    pub raw_response_paths: Vec<String>,
}

impl ResultRecord {
    pub fn pair(&self) -> (String, u32) {
        (self.article_id.clone(), self.repetition)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CompletionState {
    pub completed: BTreeSet<(String, u32)>,
    pub newest_failures: BTreeMap<(String, u32), u32>,
    pub superseded_records: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecoveryEvent {
    pub quarantined_path: PathBuf,
    pub bytes: usize,
}

pub struct ResultStore {
    path: PathBuf,
    file: File,
    pub recovery: Option<RecoveryEvent>,
}

impl ResultStore {
    pub fn open(run_dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(run_dir)?;
        let path = run_dir.join("results.jsonl");
        let recovery = recover_tail(&path)?;
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            file,
            recovery,
        })
    }

    pub fn append(&mut self, record: &ResultRecord) -> anyhow::Result<()> {
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        // One write request plus flush and sync gives an unambiguous record
        // boundary to recovery after a process or machine interruption.
        self.file.write_all(&line)?;
        self.file.flush()?;
        self.file.sync_data()?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn load_completed(path: &Path) -> anyhow::Result<CompletionState> {
    if !path.exists() {
        return Ok(CompletionState::default());
    }
    let records = load_records(path)?;
    let mut state = CompletionState::default();
    let mut newest: BTreeMap<(String, u32), &ResultRecord> = BTreeMap::new();
    for record in &records {
        let pair = record.pair();
        if record.outcome == "ok" {
            state.completed.insert(pair.clone());
        }
        newest.insert(pair, record);
    }
    for (pair, record) in newest {
        if !state.completed.contains(&pair) {
            state.newest_failures.insert(pair, record.attempt_sequence);
        }
    }
    state.superseded_records = superseded_record_count(&records);
    Ok(state)
}

pub fn load_records(path: &Path) -> anyhow::Result<Vec<ResultRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(path)?;
    bytes
        .split_inclusive(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| Ok(serde_json::from_slice(line)?))
        .collect()
}

/// Selects one record per article/repetition using append order. The newest
/// `ok` record is the comparison record when one exists; when a pair has no
/// successful record, its newest record represents a failure. Older records,
/// including failures appended after a successful retry, remain available for
/// audit and are counted as superseded.
pub fn select_records(records: &[ResultRecord]) -> (Vec<ResultRecord>, u64) {
    type Candidate = (Option<usize>, usize);
    let mut candidates: BTreeMap<(String, u32), Candidate> = BTreeMap::new();
    for (index, record) in records.iter().enumerate() {
        let entry = candidates.entry(record.pair()).or_insert((None, index));
        entry.1 = index;
        if record.outcome == "ok" {
            entry.0 = Some(index);
        }
    }
    let mut selected: Vec<(usize, ResultRecord)> = candidates
        .into_values()
        .map(|(ok_index, newest_index)| {
            let index = ok_index.unwrap_or(newest_index);
            (index, records[index].clone())
        })
        .collect();
    selected.sort_by_key(|(index, _)| *index);
    let superseded = superseded_record_count(records);
    (
        selected.into_iter().map(|(_, record)| record).collect(),
        superseded,
    )
}

fn superseded_record_count(records: &[ResultRecord]) -> u64 {
    let pair_count = records
        .iter()
        .map(ResultRecord::pair)
        .collect::<BTreeSet<_>>()
        .len();
    records.len().saturating_sub(pair_count) as u64
}

/// Writes raw provider bytes under a restart-safe, attempt-specific name.
pub fn write_raw_response(
    run_dir: &Path,
    article_id: &str,
    repetition: u32,
    attempt: u32,
    bytes: &[u8],
) -> anyhow::Result<PathBuf> {
    let directory = run_dir.join("raw").join(article_id);
    fs::create_dir_all(&directory)?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.6fZ");
    let stem = format!("r{repetition}-{stamp}-a{attempt}");
    write_unique(&directory, &stem, "json", bytes)
}

fn write_unique(
    directory: &Path,
    stem: &str,
    extension: &str,
    bytes: &[u8],
) -> anyhow::Result<PathBuf> {
    for suffix in 0_u32.. {
        let base = if suffix == 0 {
            stem.to_string()
        } else {
            format!("{stem}-{suffix}")
        };
        let name = if extension.is_empty() {
            base
        } else {
            format!("{base}.{extension}")
        };
        let path = directory.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.flush()?;
                file.sync_data()?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    unreachable!("u32 suffix space cannot be exhausted in practice")
}

fn recover_tail(path: &Path) -> anyhow::Result<Option<RecoveryEvent>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let mut boundary = 0_usize;
    let mut cursor = 0_usize;
    while cursor < bytes.len() {
        let Some(relative_end) = bytes[cursor..].iter().position(|b| *b == b'\n') else {
            break;
        };
        let end = cursor + relative_end + 1;
        if serde_json::from_slice::<ResultRecord>(&bytes[cursor..end]).is_err() {
            // A complete but malformed record is not a crash tail: retaining it
            // avoids silently rewriting history and makes corruption explicit.
            // The user must move the file aside or repair the malformed line
            // before the runner can resume.
            anyhow::bail!("malformed complete result record in {}", path.display());
        }
        boundary = end;
        cursor = end;
    }
    if boundary == bytes.len() {
        return Ok(None);
    }
    let fragment = &bytes[boundary..];
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.6fZ");
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let quarantine = write_unique(
        directory,
        &format!("results.jsonl.broken-{stamp}"),
        "",
        fragment,
    )?;
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(boundary as u64)?;
    Ok(Some(RecoveryEvent {
        quarantined_path: quarantine,
        bytes: fragment.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(outcome: &str, sequence: u32) -> ResultRecord {
        ResultRecord {
            schema_version: RESULT_SCHEMA_VERSION,
            run_id: "run".into(),
            config_hash: "config".into(),
            manifest_hash: "manifest".into(),
            prompt_identity_hash: "prompt".into(),
            rubric_sha256: "rubric".into(),
            transport_kind: Transport::Fake,
            article_id: "article".into(),
            evidence_hash: "evidence".into(),
            split: "dev".into(),
            repetition: 1,
            attempt_sequence: sequence,
            provider: "jev".into(),
            requested_model: "jev".into(),
            returned_model: (outcome == "ok").then_some("jev".into()),
            timestamp_utc: "2026-01-01T00:00:00Z".into(),
            outcome: outcome.into(),
            priority: None,
            priority_probabilities: None,
            priority_confidence: None,
            p_high: None,
            relevance_score: None,
            relevance_legend: None,
            relevance_probabilities: None,
            category_probabilities: None,
            categories_selected: None,
            category_threshold: 0.5,
            tag_probabilities: None,
            tags_selected: None,
            tag_threshold: 0.5,
            input_tokens: None,
            output_tokens: None,
            first_attempt_latency_ms: Some(1),
            model_latency_ms: Some(1),
            total_elapsed_ms: 1,
            attempt_count: 1,
            retry_reasons: vec![],
            http_status: None,
            error_type: None,
            error_detail: None,
            estimated_cost_microdollars: None,
            pricing_version: "test".into(),
            request_bytes: 1,
            response_bytes: 1,
            raw_response_paths: vec![],
        }
    }

    #[test]
    fn recovers_a_partial_tail_and_keeps_evidence() {
        let temporary = tempfile::TempDir::new().unwrap();
        let path = temporary.path().join("results.jsonl");
        let good = serde_json::to_vec(&record("ok", 1)).unwrap();
        let fragment = b"{\"half\":";
        fs::write(&path, [good.as_slice(), b"\n", fragment].concat()).unwrap();
        let store = ResultStore::open(temporary.path()).unwrap();
        let event = store.recovery.unwrap();
        assert_eq!(fs::read(event.quarantined_path).unwrap(), fragment);
        assert_eq!(load_records(&path).unwrap(), vec![record("ok", 1)]);
    }

    #[test]
    fn newest_ok_completes_and_older_records_are_superseded() {
        let temporary = tempfile::TempDir::new().unwrap();
        let mut store = ResultStore::open(temporary.path()).unwrap();
        store.append(&record("timeout", 1)).unwrap();
        store.append(&record("ok", 2)).unwrap();
        let state = load_completed(store.path()).unwrap();
        assert!(state.completed.contains(&(String::from("article"), 1)));
        assert_eq!(state.superseded_records, 1);
    }

    #[test]
    fn any_ok_completes_even_when_a_later_record_failed() {
        let temporary = tempfile::TempDir::new().unwrap();
        let mut store = ResultStore::open(temporary.path()).unwrap();
        store.append(&record("ok", 1)).unwrap();
        store.append(&record("timeout", 2)).unwrap();
        let state = load_completed(store.path()).unwrap();
        assert!(state.completed.contains(&(String::from("article"), 1)));
        assert!(state.newest_failures.is_empty());
        assert_eq!(state.superseded_records, 1);
    }

    #[test]
    fn recovery_can_run_more_than_once() {
        let temporary = tempfile::TempDir::new().unwrap();
        let path = temporary.path().join("results.jsonl");
        let good = serde_json::to_vec(&record("ok", 1)).unwrap();
        fs::write(&path, [good.as_slice(), b"\n", b"first"].concat()).unwrap();
        drop(ResultStore::open(temporary.path()).unwrap());
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(b"second");
        fs::write(&path, bytes).unwrap();
        drop(ResultStore::open(temporary.path()).unwrap());
        assert_eq!(load_records(&path).unwrap(), vec![record("ok", 1)]);
        let quarantines = fs::read_dir(temporary.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".broken-"))
            .count();
        assert_eq!(quarantines, 2);
    }

    #[test]
    fn records_round_trip_and_missing_optional_fields_default() {
        let temporary = tempfile::TempDir::new().unwrap();
        let path = temporary.path().join("results.jsonl");
        let original = record("ok", 1);
        fs::write(
            &path,
            [serde_json::to_vec(&original).unwrap(), b"\n".to_vec()].concat(),
        )
        .unwrap();
        assert_eq!(load_records(&path).unwrap(), vec![original.clone()]);

        let mut value = serde_json::to_value(&original).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("relevance_probabilities");
        fs::write(
            &path,
            [serde_json::to_vec(&value).unwrap(), b"\n".to_vec()].concat(),
        )
        .unwrap();
        let loaded = load_records(&path).unwrap();
        assert_eq!(loaded[0].relevance_probabilities, None);
    }
}
