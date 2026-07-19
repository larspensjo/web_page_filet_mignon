use std::collections::HashSet;
use std::sync::mpsc::Sender;

use chrono::Utc;
use harvester_core::{CollectedEntry, LlmResultKind, Msg};
use harvester_engine::llm::LlmQuotas;
use openai_provider_kit::{
    parse_batch_output_jsonl, BatchInputLine, BatchLifecycle, BatchRequestCounts, BatchTransport,
    LlmError,
};
use serde_json::Value;

use crate::batch_manifest::{
    BatchManifestStore, BatchState, CollectedEntryRecord, CollectedOutcomeRecord, PendingBatch,
    PendingEntry,
};

pub const MAX_BATCH_LINES: usize = 50_000;
pub const MAX_BATCH_BYTES: usize = 200 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct SubmissionBudget {
    pub max_lines_per_file: usize,
    pub max_bytes_per_file: usize,
    pub max_requests_per_run: Option<u64>,
    pub max_input_tokens_per_run: Option<u64>,
    pub max_cost_microdollars_per_run: Option<u64>,
}
impl Default for SubmissionBudget {
    fn default() -> Self {
        Self {
            max_lines_per_file: MAX_BATCH_LINES,
            max_bytes_per_file: MAX_BATCH_BYTES,
            max_requests_per_run: None,
            max_input_tokens_per_run: None,
            max_cost_microdollars_per_run: None,
        }
    }
}

impl SubmissionBudget {
    pub fn from_quotas(quotas: &LlmQuotas) -> Self {
        Self {
            max_requests_per_run: quotas.max_calls_per_session.map(u64::from),
            max_input_tokens_per_run: quotas.max_input_tokens_per_session,
            max_cost_microdollars_per_run: quotas.max_cost_microdollars_per_session,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct BufferedRequest {
    pub request_id: u64,
    pub stage: String,
    pub line: BatchInputLine,
    pub entry: PendingEntry,
    pub estimated_input_tokens: u64,
    pub estimated_cost_microdollars: u64,
}

/// Status-only snapshot of a manifest batch obtained during drain-mode waits.
/// A missing status means the provider lookup failed and will be retried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchPeek {
    pub batch_id: String,
    pub stage: String,
    pub status: Option<BatchLifecycle>,
    pub request_counts: Option<BatchRequestCounts>,
}

pub struct BatchCoordinator<T> {
    transport: T,
    manifest: BatchManifestStore,
    budget: SubmissionBudget,
    buffered: Vec<BufferedRequest>,
    submitted_requests: u64,
    submitted_input_tokens: u64,
    submitted_cost_microdollars: u64,
    submission_stopped: bool,
}

impl<T: BatchTransport> BatchCoordinator<T> {
    pub fn new(transport: T, manifest: BatchManifestStore, budget: SubmissionBudget) -> Self {
        Self {
            transport,
            manifest,
            budget,
            buffered: Vec::new(),
            submitted_requests: 0,
            submitted_input_tokens: 0,
            submitted_cost_microdollars: 0,
            submission_stopped: false,
        }
    }
    #[cfg(test)]
    pub fn pending_custom_ids(&self) -> HashSet<String> {
        self.manifest.pending_custom_ids()
    }
    #[cfg(test)]
    pub fn manifest(&self) -> &BatchManifestStore {
        &self.manifest
    }

    pub fn remove_collected_if(
        &mut self,
        mut confirmed: impl FnMut(&PendingEntry) -> bool,
    ) -> Result<(), String> {
        let removable: Vec<_> = self
            .manifest
            .manifest()
            .batches
            .iter()
            .filter(|batch| {
                batch.status == BatchState::Collected
                    && batch.entries.iter().all(|entry| {
                        matches!(
                            entry.collected.as_ref().map(|record| &record.outcome),
                            Some(CollectedOutcomeRecord::LineError { .. })
                        ) || confirmed(entry)
                    })
            })
            .map(|batch| batch.input_file_id.clone())
            .collect();
        for input_file_id in removable {
            self.manifest
                .remove_batch(&input_file_id)
                .map_err(|err| err.to_string())?;
        }
        Ok(())
    }

    pub fn release_invalid_collected(
        &mut self,
        custom_ids: &HashSet<String>,
    ) -> Result<(), String> {
        self.manifest
            .mark_entries_line_error(custom_ids, "batch output failed local validation")
            .map_err(|err| err.to_string())
    }

    /// Resolve the reserve-before-create crash window without cancelling any
    /// remotely-created paid work.  There is deliberately no shutdown cancel.
    pub async fn reconcile_once(&mut self) -> Result<(), String> {
        let mut remote = Vec::new();
        let mut after = None;
        loop {
            let page = self
                .transport
                .list_batches(after.as_deref())
                .await
                .map_err(|err| format!("after={:?} list batches: {err}", after))?;
            if page.is_empty() {
                break;
            }
            let next = page.last().map(|handle| handle.id.clone());
            if next == after {
                return Err(format!(
                    "after={:?} list batches returned a non-advancing cursor",
                    after
                ));
            }
            remote.extend(page);
            after = next;
        }
        let reservations: Vec<_> = self
            .manifest
            .manifest()
            .batches
            .iter()
            .filter(|batch| batch.batch_id.is_none())
            .map(|batch| batch.input_file_id.clone())
            .collect();
        for input_file_id in reservations {
            if let Some(handle) = remote
                .iter()
                .find(|handle| handle.input_file_id == input_file_id)
            {
                self.manifest
                    .adopt_batch_id(&input_file_id, handle.id.clone())
                    .map_err(|err| err.to_string())?;
            } else {
                engine_logging::engine_warn!(
                    "[batch-reconcile] input_file_id={} no remote batch found; releasing reservation",
                    input_file_id
                );
                self.manifest
                    .remove_batch(&input_file_id)
                    .map_err(|err| err.to_string())?;
            }
        }
        Ok(())
    }

    /// Snapshot completed output to the manifest before it crosses into the
    /// reducer. A later process can replay `Collected` snapshots idempotently.
    pub async fn collect_completed(&mut self) -> Result<Vec<CollectedEntry>, String> {
        let candidates: Vec<_> = self
            .manifest
            .manifest()
            .batches
            .iter()
            .filter_map(|batch| {
                (batch.status != BatchState::Collected)
                    .then(|| batch.batch_id.clone())
                    .flatten()
                    .map(|id| (batch.input_file_id.clone(), id))
            })
            .collect();
        let mut replay = Vec::new();
        for (input_file_id, batch_id) in candidates {
            let handle = match self.transport.retrieve_batch(&batch_id).await {
                Ok(handle) => handle,
                Err(err) => {
                    engine_logging::engine_warn!(
                        "[batch-collect] batch_id={} input_file_id={} retrieve failed; retrying next cycle: {}",
                        batch_id,
                        input_file_id,
                        err
                    );
                    continue;
                }
            };
            match handle.status {
                BatchLifecycle::Completed => {
                    let records = match self.download_records(&handle).await {
                        Ok(records) => records,
                        Err(err) => {
                            engine_logging::engine_warn!(
                                "[batch-collect] batch_id={} input_file_id={} download/parse failed; retrying next cycle: {}",
                                batch_id,
                                input_file_id,
                                err
                            );
                            continue;
                        }
                    };
                    self.manifest
                        .mark_collected(&input_file_id, records)
                        .map_err(|err| err.to_string())?;
                    engine_logging::engine_info!(
                        "[batch-collect] batch_id={} input_file_id={} completed; requests completed={}/{} failed={}; results collected",
                        batch_id,
                        input_file_id,
                        handle.request_counts.completed,
                        handle.request_counts.total,
                        handle.request_counts.failed
                    );
                }
                BatchLifecycle::Failed | BatchLifecycle::Expired | BatchLifecycle::Cancelled => {
                    engine_logging::engine_warn!(
                        "[batch-collect] batch_id={} input_file_id={} lifecycle={:?}; releasing entries",
                        batch_id, input_file_id, handle.status
                    );
                    self.manifest
                        .mark_failed(&input_file_id)
                        .map_err(|err| err.to_string())?;
                }
                BatchLifecycle::Validating
                | BatchLifecycle::InProgress
                | BatchLifecycle::Finalizing
                | BatchLifecycle::Cancelling => {
                    engine_logging::engine_info!(
                        "[batch-collect] batch_id={} input_file_id={} lifecycle={:?}; progress completed={}/{} failed={}; awaiting completion",
                        batch_id,
                        input_file_id,
                        handle.status,
                        handle.request_counts.completed,
                        handle.request_counts.total,
                        handle.request_counts.failed
                    );
                }
            }
        }
        for batch in &self.manifest.manifest().batches {
            if batch.status == BatchState::Collected {
                let batch_id = batch.batch_id.as_deref().unwrap_or_default();
                replay.extend(
                    batch
                        .entries
                        .iter()
                        .filter_map(|entry| entry.collected_entry(batch_id)),
                );
            }
        }
        Ok(replay)
    }

    async fn download_records(
        &self,
        handle: &openai_provider_kit::BatchHandle,
    ) -> Result<Vec<CollectedEntryRecord>, String> {
        let mut lines = Vec::new();
        if let Some(file_id) = &handle.output_file_id {
            lines.extend(
                parse_batch_output_jsonl(
                    &self
                        .transport
                        .download_file(file_id)
                        .await
                        .map_err(|err| err.to_string())?,
                )
                .map_err(|err| err.to_string())?,
            );
        }
        if let Some(file_id) = &handle.error_file_id {
            lines.extend(
                parse_batch_output_jsonl(
                    &self
                        .transport
                        .download_file(file_id)
                        .await
                        .map_err(|err| err.to_string())?,
                )
                .map_err(|err| err.to_string())?,
            );
        }
        Ok(lines
            .into_iter()
            .map(|line| collected_record(line, Utc::now().to_rfc3339()))
            .collect())
    }
    pub fn buffer(&mut self, request: BufferedRequest) {
        self.buffered.push(request);
    }
    pub fn buffered_request_ids(&self) -> HashSet<u64> {
        self.buffered
            .iter()
            .map(|request| request.request_id)
            .collect()
    }
    pub fn failed_attempts_for(&self, custom_id: &str) -> u32 {
        self.manifest.failed_attempts_for(custom_id)
    }

    pub fn pending_batch_count(&self) -> usize {
        self.manifest
            .manifest()
            .batches
            .iter()
            .filter(|batch| batch.status != BatchState::Collected && batch.batch_id.is_some())
            .count()
    }

    pub fn pending_manifest_batches(&self) -> Vec<(String, Option<String>)> {
        let mut batches: Vec<_> = self
            .manifest
            .manifest()
            .batches
            .iter()
            .filter(|batch| batch.status != BatchState::Collected)
            .map(|batch| (batch.input_file_id.clone(), batch.batch_id.clone()))
            .collect();
        batches.sort();
        batches
    }

    /// Retrieves the status for every pending remote batch without downloading
    /// output. Lookup failures are intentionally non-fatal during drain waits.
    pub async fn peek_pending_batches(&self) -> Vec<BatchPeek> {
        let candidates: Vec<_> = self
            .manifest
            .manifest()
            .batches
            .iter()
            .filter_map(|batch| {
                (batch.status != BatchState::Collected)
                    .then(|| {
                        batch
                            .batch_id
                            .as_ref()
                            .map(|id| (id.clone(), batch.stage.clone()))
                    })
                    .flatten()
            })
            .collect();

        let mut peeks = Vec::with_capacity(candidates.len());
        for (batch_id, stage) in candidates {
            match self.transport.retrieve_batch(&batch_id).await {
                Ok(handle) => peeks.push(BatchPeek {
                    batch_id,
                    stage,
                    status: Some(handle.status),
                    request_counts: Some(handle.request_counts),
                }),
                Err(err) => {
                    engine_logging::engine_warn!(
                        "[batch-wait] batch_id={} stage={} status retrieve failed; retrying next check: {}",
                        batch_id,
                        stage,
                        err
                    );
                    peeks.push(BatchPeek {
                        batch_id,
                        stage,
                        status: None,
                        request_counts: None,
                    });
                }
            }
        }
        peeks
    }

    pub async fn flush(
        &mut self,
        msg_tx: &Sender<Msg>,
        submitted_at_utc: String,
    ) -> Result<(), String> {
        let pending = self.manifest.pending_custom_ids();
        let mut work = std::mem::take(&mut self.buffered);
        let mut deduped = Vec::new();
        let mut seen = HashSet::new();
        for request in work.drain(..) {
            if pending.contains(&request.line.custom_id)
                || !seen.insert(request.line.custom_id.clone())
            {
                reply(msg_tx, request.request_id, LlmResultKind::DeferredToBatch)?;
            } else {
                deduped.push(request);
            }
        }
        for stage in ["triage", "summary", "signal_candidate"] {
            let group: Vec<_> = deduped
                .iter()
                .filter(|request| request.stage == stage)
                .cloned()
                .collect();
            if group.is_empty() {
                continue;
            }
            let chunks = split_group(
                group,
                self.budget.max_lines_per_file,
                self.budget.max_bytes_per_file,
                msg_tx,
            )?;
            for mut group in chunks {
                if self.submission_stopped || !self.within_submission_budget(&group) {
                    self.submission_stopped = true;
                    engine_logging::engine_warn!(
                    "[batch-submit] outcome=budget-exhausted stage={} requests_submitted={} estimated_input_tokens={} estimated_cost_microdollars={}; leaving {} requests deferred",
                    stage,
                    self.submitted_requests,
                    self.submitted_input_tokens,
                    self.submitted_cost_microdollars,
                    group.len()
                );
                    for request in group {
                        reply(msg_tx, request.request_id, LlmResultKind::DeferredToBatch)?;
                    }
                    continue;
                }
                let lines: Vec<_> = group.iter().map(|request| request.line.clone()).collect();
                let submitted_requests = group.len() as u64;
                let submitted_input_tokens = group
                    .iter()
                    .map(|request| request.estimated_input_tokens)
                    .sum::<u64>();
                let submitted_cost_microdollars = group
                    .iter()
                    .map(|request| request.estimated_cost_microdollars)
                    .sum::<u64>();
                let jsonl = BatchInputLine::to_jsonl(&lines).map_err(|err| err.to_string())?;
                let input_file_id = match self.transport.upload_input(&jsonl).await {
                    Ok(id) => id,
                    Err(err) => {
                        fail_group(msg_tx, group, err)?;
                        continue;
                    }
                };
                for request in &mut group {
                    request.entry.attempts =
                        self.manifest.failed_attempts_for(&request.line.custom_id);
                }
                let entries = group.iter().map(|request| request.entry.clone()).collect();
                let batch = PendingBatch {
                    input_file_id: input_file_id.clone(),
                    batch_id: None,
                    stage: stage.into(),
                    completion_window: "24h".into(),
                    submitted_at_utc: submitted_at_utc.clone(),
                    status: BatchState::Created,
                    entries,
                };
                if let Err(err) = self.manifest.reserve(batch) {
                    fail_group(
                        msg_tx,
                        group,
                        LlmError::Configuration {
                            detail: err.to_string(),
                        },
                    )?;
                    continue;
                }
                match self
                    .transport
                    .create_batch(&input_file_id, "/v1/chat/completions", "24h")
                    .await
                {
                    Ok(handle) => {
                        engine_logging::engine_info!(
                            "[batch-submit] stage={} batch_id={} input_file_id={} requests={} estimated_input_tokens={} estimated_cost_microdollars={}",
                            stage,
                            handle.id,
                            input_file_id,
                            submitted_requests,
                            submitted_input_tokens,
                            submitted_cost_microdollars
                        );
                        if let Err(err) = self
                            .manifest
                            .attach_batch_id(&input_file_id, handle.id.clone())
                        {
                            engine_logging::engine_warn!(
                            "[batch-submit] batch_id={} input_file_id={} attach persistence failed; reservation will reconcile next cycle: {}",
                            handle.id,
                            input_file_id,
                            err
                        );
                        }
                        for request in group {
                            reply(msg_tx, request.request_id, LlmResultKind::DeferredToBatch)?;
                        }
                        self.submitted_requests =
                            self.submitted_requests.saturating_add(submitted_requests);
                        self.submitted_input_tokens = self
                            .submitted_input_tokens
                            .saturating_add(submitted_input_tokens);
                        self.submitted_cost_microdollars = self
                            .submitted_cost_microdollars
                            .saturating_add(submitted_cost_microdollars);
                    }
                    Err(err) => {
                        let _ = self.manifest.remove_batch(&input_file_id);
                        fail_group(msg_tx, group, err)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn within_submission_budget(&self, group: &[BufferedRequest]) -> bool {
        let requests = self.submitted_requests.saturating_add(group.len() as u64);
        let input_tokens = self.submitted_input_tokens.saturating_add(
            group
                .iter()
                .map(|request| request.estimated_input_tokens)
                .sum(),
        );
        let cost = self.submitted_cost_microdollars.saturating_add(
            group
                .iter()
                .map(|request| request.estimated_cost_microdollars)
                .sum(),
        );
        self.budget
            .max_requests_per_run
            .is_none_or(|limit| requests <= limit)
            && self
                .budget
                .max_input_tokens_per_run
                .is_none_or(|limit| input_tokens <= limit)
            && self
                .budget
                .max_cost_microdollars_per_run
                .is_none_or(|limit| cost <= limit)
    }
}

fn split_group(
    group: Vec<BufferedRequest>,
    max_lines: usize,
    max_bytes: usize,
    msg_tx: &Sender<Msg>,
) -> Result<Vec<Vec<BufferedRequest>>, String> {
    let mut chunks = Vec::new();
    let mut chunk = Vec::new();
    let mut chunk_bytes = 0usize;
    for request in group {
        let line_bytes = BatchInputLine::to_jsonl(std::slice::from_ref(&request.line))
            .map_err(|err| err.to_string())?
            .len();
        if line_bytes > max_bytes || max_lines == 0 {
            engine_logging::engine_warn!(
                "[batch-submit] custom_id={} cache_key={} request_id={} JSONL line exceeds cap bytes={} cap={}",
                request.line.custom_id,
                request.entry.key.content_hash,
                request.request_id,
                line_bytes,
                max_bytes
            );
            reply(
                msg_tx,
                request.request_id,
                LlmResultKind::Failed {
                    reason: format!(
                        "batch JSONL line exceeds configured file cap: bytes={line_bytes} cap={max_bytes}"
                    ),
                },
            )?;
            continue;
        }
        if !chunk.is_empty()
            && (chunk.len() >= max_lines || chunk_bytes.saturating_add(line_bytes) > max_bytes)
        {
            chunks.push(std::mem::take(&mut chunk));
            chunk_bytes = 0;
        }
        chunk_bytes = chunk_bytes.saturating_add(line_bytes);
        chunk.push(request);
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    Ok(chunks)
}

fn collected_record(
    line: openai_provider_kit::BatchOutputLine,
    created_at_utc: String,
) -> CollectedEntryRecord {
    let failure = |detail| CollectedEntryRecord {
        custom_id: line.custom_id.clone(),
        created_at_utc: created_at_utc.clone(),
        outcome: CollectedOutcomeRecord::LineError { detail },
    };
    if let Some(error) = line.error {
        return failure(error.to_string());
    }
    let Some(response) = line.response else {
        return failure("batch line had neither response nor error".to_string());
    };
    if !(200..300).contains(&response.status_code) {
        return failure(format!(
            "non-success response status={}",
            response.status_code
        ));
    }
    let body = response.body;
    let raw_output_json = body
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let Some(raw_output_json) = raw_output_json else {
        return failure("successful response missing choices[0].message.content".to_string());
    };
    let usage = body.get("usage").cloned().unwrap_or(Value::Null);
    let input_tokens = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let output_tokens = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let cached_input_tokens = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    CollectedEntryRecord {
        custom_id: line.custom_id,
        created_at_utc,
        outcome: CollectedOutcomeRecord::Success {
            raw_output_json,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            resolved_model: body
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
    }
}
fn reply(tx: &Sender<Msg>, request_id: u64, result: LlmResultKind) -> Result<(), String> {
    tx.send(Msg::LlmCompleted {
        request_id,
        result,
        metadata: None,
    })
    .map_err(|err| err.to_string())
}
fn fail_group(
    tx: &Sender<Msg>,
    group: Vec<BufferedRequest>,
    error: LlmError,
) -> Result<(), String> {
    for request in group {
        engine_logging::engine_warn!(
            "[batch-submit] custom_id={} cache_key={} request_id={} submission failed: {}",
            request.line.custom_id,
            request.entry.key.content_hash,
            request.request_id,
            error
        );
        reply(
            tx,
            request.request_id,
            LlmResultKind::Failed {
                reason: format!("batch submission failed: {error}"),
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use harvester_core::{FrozenBatchKey, StageKind};
    use harvester_engine::llm::PromptId;
    use openai_provider_kit::{BatchHandle, BatchLifecycle, BatchRequestCounts};
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    #[derive(Clone, Default)]
    struct FakeTransport {
        uploaded: Arc<Mutex<Vec<Vec<u8>>>>,
        created: Arc<Mutex<Vec<String>>>,
        fail_upload: bool,
        fail_create: bool,
        list_pages: Arc<Mutex<VecDeque<Vec<BatchHandle>>>>,
        list_after: Arc<Mutex<Vec<Option<String>>>>,
        retrieved: Arc<Mutex<HashMap<String, BatchHandle>>>,
        downloads: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    }

    #[async_trait]
    impl BatchTransport for FakeTransport {
        async fn upload_input(&self, jsonl: &[u8]) -> Result<String, LlmError> {
            if self.fail_upload {
                return Err(LlmError::Network {
                    detail: "upload failed".to_string(),
                });
            }
            self.uploaded.lock().unwrap().push(jsonl.to_vec());
            Ok(format!("file_{}", self.uploaded.lock().unwrap().len()))
        }
        async fn create_batch(
            &self,
            input_file_id: &str,
            _: &str,
            _: &str,
        ) -> Result<openai_provider_kit::BatchHandle, LlmError> {
            if self.fail_create {
                return Err(LlmError::Network {
                    detail: "create failed".to_string(),
                });
            }
            let mut created = self.created.lock().unwrap();
            let id = format!("batch_{}", created.len() + 1);
            created.push(input_file_id.to_string());
            Ok(handle(&id, input_file_id, BatchLifecycle::InProgress))
        }
        async fn retrieve_batch(
            &self,
            batch_id: &str,
        ) -> Result<openai_provider_kit::BatchHandle, LlmError> {
            self.retrieved
                .lock()
                .unwrap()
                .get(batch_id)
                .cloned()
                .ok_or_else(|| LlmError::InvalidResponse {
                    detail: format!("missing fake batch {batch_id}"),
                })
        }
        async fn list_batches(
            &self,
            after: Option<&str>,
        ) -> Result<Vec<openai_provider_kit::BatchHandle>, LlmError> {
            self.list_after
                .lock()
                .unwrap()
                .push(after.map(ToString::to_string));
            Ok(self
                .list_pages
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default())
        }
        async fn download_file(&self, file_id: &str) -> Result<Vec<u8>, LlmError> {
            self.downloads
                .lock()
                .unwrap()
                .get(file_id)
                .cloned()
                .ok_or_else(|| LlmError::InvalidResponse {
                    detail: format!("missing fake file {file_id}"),
                })
        }
        async fn cancel_batch(
            &self,
            _: &str,
        ) -> Result<openai_provider_kit::BatchHandle, LlmError> {
            unreachable!()
        }
    }

    fn handle(id: &str, input_file_id: &str, status: BatchLifecycle) -> BatchHandle {
        BatchHandle {
            id: id.into(),
            status,
            input_file_id: input_file_id.into(),
            output_file_id: None,
            error_file_id: None,
            request_counts: BatchRequestCounts::default(),
        }
    }

    fn buffered_request() -> BufferedRequest {
        buffered_request_with(7, "triage-content", "triage")
    }

    fn buffered_request_with(request_id: u64, custom_id: &str, stage: &str) -> BufferedRequest {
        let stage_kind = match stage {
            "triage" => StageKind::Triage,
            "summary" => StageKind::Summary,
            "signal_candidate" => StageKind::SignalCandidate,
            other => panic!("unknown stage {other}"),
        };
        let key = FrozenBatchKey {
            content_hash: format!("content-{request_id}"),
            prompt_id: PromptId::ArticleTriage,
            prompt_version: 1,
            model_id: "model".into(),
            context_hash: "context".into(),
            stage: stage_kind,
            url: "https://example.test".into(),
            rendered_system: "system".into(),
            rendered_user: "user".into(),
        };
        BufferedRequest {
            request_id,
            stage: stage.into(),
            line: BatchInputLine {
                custom_id: custom_id.into(),
                method: "POST".into(),
                url: "/v1/chat/completions".into(),
                body: serde_json::json!({"model":"model"}),
            },
            entry: PendingEntry {
                custom_id: custom_id.into(),
                key,
                stage: stage_kind,
                attempts: 0,
                collected: None,
            },
            estimated_input_tokens: 10,
            estimated_cost_microdollars: 1,
        }
    }

    #[test]
    fn submit_reserves_attaches_and_defers_the_request() {
        let dir = TempDir::new().unwrap();
        let transport = FakeTransport::default();
        let uploaded = Arc::clone(&transport.uploaded);
        let mut coordinator = BatchCoordinator::new(
            transport,
            BatchManifestStore::load(dir.path().to_path_buf()).unwrap(),
            SubmissionBudget::default(),
        );
        coordinator.buffer(buffered_request());
        let (tx, rx) = std::sync::mpsc::channel();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(coordinator.flush(&tx, "2026-07-19T00:00:00Z".into()))
            .unwrap();
        assert_eq!(uploaded.lock().unwrap().len(), 1);
        assert!(matches!(
            rx.recv().unwrap(),
            Msg::LlmCompleted {
                request_id: 7,
                result: LlmResultKind::DeferredToBatch,
                ..
            }
        ));
        let persisted = BatchManifestStore::load(dir.path().to_path_buf()).unwrap();
        assert_eq!(
            persisted.manifest().batches[0].batch_id.as_deref(),
            Some("batch_1")
        );
        assert!(coordinator.pending_custom_ids().contains("triage-content"));
        assert_eq!(coordinator.manifest().manifest().batches.len(), 1);
    }

    #[test]
    fn oversized_stage_group_is_chunked_by_line_cap() {
        let dir = TempDir::new().unwrap();
        let transport = FakeTransport::default();
        let uploaded = Arc::clone(&transport.uploaded);
        let created = Arc::clone(&transport.created);
        let mut coordinator = BatchCoordinator::new(
            transport,
            BatchManifestStore::load(dir.path().to_path_buf()).unwrap(),
            SubmissionBudget {
                max_lines_per_file: 2,
                ..SubmissionBudget::default()
            },
        );
        for request_id in 1..=5 {
            coordinator.buffer(buffered_request_with(
                request_id,
                &format!("triage-{request_id}"),
                "triage",
            ));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(coordinator.flush(&tx, "now".into()))
            .unwrap();

        assert_eq!(uploaded.lock().unwrap().len(), 3);
        assert_eq!(created.lock().unwrap().len(), 3);
        assert_eq!(rx.try_iter().count(), 5);
        let manifest = BatchManifestStore::load(dir.path().to_path_buf()).unwrap();
        assert_eq!(manifest.manifest().batches.len(), 3);
        assert_eq!(
            manifest
                .manifest()
                .batches
                .iter()
                .map(|batch| batch.entries.len())
                .sum::<usize>(),
            5
        );
    }

    #[test]
    fn submission_budget_stops_new_batches_and_leaves_remainder_deferred() {
        let dir = TempDir::new().unwrap();
        let transport = FakeTransport::default();
        let created = Arc::clone(&transport.created);
        let mut coordinator = BatchCoordinator::new(
            transport,
            BatchManifestStore::load(dir.path().to_path_buf()).unwrap(),
            SubmissionBudget {
                max_lines_per_file: 2,
                max_requests_per_run: Some(2),
                ..SubmissionBudget::default()
            },
        );
        for request_id in 1..=3 {
            coordinator.buffer(buffered_request_with(
                request_id,
                &format!("triage-{request_id}"),
                "triage",
            ));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(coordinator.flush(&tx, "now".into()))
            .unwrap();

        assert_eq!(created.lock().unwrap().len(), 1);
        let replies: Vec<_> = rx.try_iter().collect();
        assert_eq!(replies.len(), 3);
        assert!(replies.iter().all(|reply| matches!(
            reply,
            Msg::LlmCompleted {
                result: LlmResultKind::DeferredToBatch,
                ..
            }
        )));
        assert_eq!(
            coordinator.manifest().manifest().batches[0].entries.len(),
            2
        );
    }

    #[test]
    fn duplicate_custom_id_uploads_once_and_replies_to_every_request() {
        let dir = TempDir::new().unwrap();
        let transport = FakeTransport::default();
        let uploaded = Arc::clone(&transport.uploaded);
        let mut coordinator = BatchCoordinator::new(
            transport,
            BatchManifestStore::load(dir.path().to_path_buf()).unwrap(),
            SubmissionBudget::default(),
        );
        coordinator.buffer(buffered_request_with(1, "same", "triage"));
        coordinator.buffer(buffered_request_with(2, "same", "triage"));
        let (tx, rx) = std::sync::mpsc::channel();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(coordinator.flush(&tx, "now".into()))
            .unwrap();

        assert_eq!(uploaded.lock().unwrap().len(), 1);
        assert_eq!(rx.try_iter().count(), 2);
        assert_eq!(
            coordinator.manifest().manifest().batches[0].entries.len(),
            1
        );
    }

    #[test]
    fn upload_failure_fails_group_without_manifest_reservation() {
        let dir = TempDir::new().unwrap();
        let transport = FakeTransport {
            fail_upload: true,
            ..FakeTransport::default()
        };
        let mut coordinator = BatchCoordinator::new(
            transport,
            BatchManifestStore::load(dir.path().to_path_buf()).unwrap(),
            SubmissionBudget::default(),
        );
        coordinator.buffer(buffered_request_with(1, "one", "triage"));
        coordinator.buffer(buffered_request_with(2, "two", "triage"));
        let (tx, rx) = std::sync::mpsc::channel();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(coordinator.flush(&tx, "now".into()))
            .unwrap();

        let replies: Vec<_> = rx.try_iter().collect();
        assert_eq!(replies.len(), 2);
        assert!(replies.iter().all(|reply| matches!(
            reply,
            Msg::LlmCompleted {
                result: LlmResultKind::Failed { .. },
                ..
            }
        )));
        assert!(coordinator.manifest().manifest().batches.is_empty());
    }

    #[test]
    fn create_failure_releases_reservation_and_fails_group() {
        let dir = TempDir::new().unwrap();
        let transport = FakeTransport {
            fail_create: true,
            ..FakeTransport::default()
        };
        let mut coordinator = BatchCoordinator::new(
            transport,
            BatchManifestStore::load(dir.path().to_path_buf()).unwrap(),
            SubmissionBudget::default(),
        );
        coordinator.buffer(buffered_request_with(1, "one", "triage"));
        coordinator.buffer(buffered_request_with(2, "two", "triage"));
        let (tx, rx) = std::sync::mpsc::channel();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(coordinator.flush(&tx, "now".into()))
            .unwrap();

        let replies: Vec<_> = rx.try_iter().collect();
        assert_eq!(replies.len(), 2);
        assert!(replies.iter().all(|reply| matches!(
            reply,
            Msg::LlmCompleted {
                result: LlmResultKind::Failed { .. },
                ..
            }
        )));
        assert!(coordinator.manifest().manifest().batches.is_empty());
        assert!(BatchManifestStore::load(dir.path().to_path_buf())
            .unwrap()
            .manifest()
            .batches
            .is_empty());
    }

    #[test]
    fn stage_partition_submits_one_batch_per_stage() {
        let dir = TempDir::new().unwrap();
        let transport = FakeTransport::default();
        let created = Arc::clone(&transport.created);
        let mut coordinator = BatchCoordinator::new(
            transport,
            BatchManifestStore::load(dir.path().to_path_buf()).unwrap(),
            SubmissionBudget::default(),
        );
        coordinator.buffer(buffered_request_with(1, "triage", "triage"));
        coordinator.buffer(buffered_request_with(2, "summary", "summary"));
        coordinator.buffer(buffered_request_with(3, "signal", "signal_candidate"));
        let (tx, rx) = std::sync::mpsc::channel();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(coordinator.flush(&tx, "now".into()))
            .unwrap();

        assert_eq!(created.lock().unwrap().len(), 3);
        assert_eq!(rx.try_iter().count(), 3);
        let stages: HashSet<_> = coordinator
            .manifest()
            .manifest()
            .batches
            .iter()
            .map(|batch| batch.stage.as_str())
            .collect();
        assert_eq!(
            stages,
            HashSet::from(["triage", "summary", "signal_candidate"])
        );
    }

    #[test]
    fn reconciliation_paginates_until_reserved_file_is_found() {
        let dir = TempDir::new().unwrap();
        let mut store = BatchManifestStore::load(dir.path().to_path_buf()).unwrap();
        let request = buffered_request();
        store
            .reserve(PendingBatch {
                input_file_id: "target-file".to_string(),
                batch_id: None,
                stage: "triage".to_string(),
                completion_window: "24h".to_string(),
                submitted_at_utc: "now".to_string(),
                status: BatchState::Created,
                entries: vec![request.entry],
            })
            .unwrap();
        let transport = FakeTransport::default();
        transport.list_pages.lock().unwrap().extend([
            vec![handle(
                "page-1-last",
                "other-file",
                BatchLifecycle::InProgress,
            )],
            vec![handle(
                "target-batch",
                "target-file",
                BatchLifecycle::InProgress,
            )],
            vec![],
        ]);
        let after_calls = Arc::clone(&transport.list_after);
        let mut coordinator = BatchCoordinator::new(transport, store, SubmissionBudget::default());
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(coordinator.reconcile_once())
            .unwrap();

        assert_eq!(
            after_calls.lock().unwrap().as_slice(),
            &[
                None,
                Some("page-1-last".to_string()),
                Some("target-batch".to_string())
            ]
        );
        assert_eq!(
            coordinator.manifest().manifest().batches[0]
                .batch_id
                .as_deref(),
            Some("target-batch")
        );
    }

    #[test]
    fn collected_snapshot_replays_after_restart_until_cache_confirmation() {
        let dir = TempDir::new().unwrap();
        let mut store = BatchManifestStore::load(dir.path().to_path_buf()).unwrap();
        store
            .reserve(PendingBatch {
                input_file_id: "input-file".to_string(),
                batch_id: Some("completed-batch".to_string()),
                stage: "triage".to_string(),
                completion_window: "24h".to_string(),
                submitted_at_utc: "now".to_string(),
                status: BatchState::Submitted,
                entries: vec![buffered_request().entry],
            })
            .unwrap();
        let transport = FakeTransport::default();
        let mut completed = handle("completed-batch", "input-file", BatchLifecycle::Completed);
        completed.output_file_id = Some("output-file".to_string());
        transport
            .retrieved
            .lock()
            .unwrap()
            .insert("completed-batch".to_string(), completed);
        transport.downloads.lock().unwrap().insert(
            "output-file".to_string(),
            br#"{"custom_id":"triage-content","response":{"status_code":200,"body":{"choices":[{"message":{"content":"{\"category\":\"news\",\"priority\":3,\"tags\":[\"ai\"],\"rationale\":\"ok\"}"}}],"usage":{"prompt_tokens":20,"completion_tokens":8},"model":"model"}},"error":null}
"#
            .to_vec(),
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut coordinator =
            BatchCoordinator::new(transport.clone(), store, SubmissionBudget::default());
        let first = runtime.block_on(coordinator.collect_completed()).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(
            coordinator.manifest().manifest().batches[0].status,
            BatchState::Collected
        );

        // Simulate a crash before the reducer's cache persistence effect. A
        // new coordinator replays the durable Collected snapshot without
        // retrieving or downloading again.
        transport.retrieved.lock().unwrap().clear();
        transport.downloads.lock().unwrap().clear();
        let mut restarted = BatchCoordinator::new(
            transport,
            BatchManifestStore::load(dir.path().to_path_buf()).unwrap(),
            SubmissionBudget::default(),
        );
        let replay = runtime.block_on(restarted.collect_completed()).unwrap();
        assert_eq!(replay, first);
        restarted.remove_collected_if(|_| false).unwrap();
        assert_eq!(restarted.manifest().manifest().batches.len(), 1);
        restarted.remove_collected_if(|_| true).unwrap();
        assert!(restarted.manifest().manifest().batches.is_empty());
    }
}
