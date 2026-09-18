//! Runner orchestration; policy decisions live in sibling pure modules.

pub mod identity;
pub mod retry;
pub mod store;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use chrono::Utc;
use futures_util::{pin_mut, stream, StreamExt};
use serde::{Deserialize, Serialize};

use crate::config::{DatasetSelection, RunConfig, Transport};
use crate::jev::request::{build_request, FrozenRubric, FrozenText};
use crate::jev::response::{parse_jev_response, validate_answers};
use crate::jev::transport::{
    AttemptContext, FakeTransport, HttpTransport, JevTransport, TransportOutcome,
};
use crate::manifest::{load_manifest, DatasetSplit, Manifest};
use crate::pricing::jev_cost_microdollars;

use self::identity::{check_resume, ResumeDecision, RunIdentity};
use self::retry::{backoff_delay, classify, Disposition};
use self::store::{
    load_completed, load_records, write_raw_response, ResultRecord, ResultStore,
    RESULT_SCHEMA_VERSION,
};

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub experiment_dir: PathBuf,
    pub config: RunConfig,
    pub dry_run: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunFile {
    identity: RunIdentity,
    resolved_config: RunConfig,
    acceptance: Option<crate::config::AcceptanceConfig>,
    transport_kind: Transport,
    started_utc: String,
    ended_utc: Option<String>,
    records_written: u64,
    superseded_records: u64,
    recovery_bytes: Option<usize>,
    #[serde(default)]
    recovery_paths: Vec<String>,
}

/// Resolves frozen inputs and executes the selected work.  It is deliberately
/// generic over the transport so tests never need a network connection.
pub async fn run(options: RunOptions) -> anyhow::Result<()> {
    let config = options.config;
    config.validate()?;
    let manifest_path = PathBuf::from(&config.dataset.manifest);
    let manifest = load_manifest(&manifest_path)?;
    let root = manifest_path.parent().context("manifest has no parent")?;
    let selected = select_articles(&manifest, config.dataset.split, config.dataset.limit);
    ensure_heldout_acceptance(&selected, &config)?;
    let run_id = config
        .run
        .run_id
        .clone()
        .unwrap_or_else(|| generated_run_id(&config));
    let identity = make_identity(&run_id, &manifest, &config);
    let run_dir = options.experiment_dir.join("runs").join(&run_id);
    if options.dry_run {
        let estimated_tokens = selected
            .iter()
            .map(|article| article.text_bytes / 4)
            .sum::<usize>()
            .saturating_mul(config.dataset.repeat as usize);
        let cost = jev_cost_microdollars(
            estimated_tokens as u64,
            config.pricing.input_microdollars_per_million,
        );
        println!("run {run_id}: {} articles × {} repetitions; estimated input tokens {estimated_tokens}, estimated cost {cost} microdollars", selected.len(), config.dataset.repeat);
        return Ok(());
    }
    fs::create_dir_all(&run_dir)?;
    let run_file_path = run_dir.join("run.json");
    let existing_run = if run_file_path.exists() {
        let existing: RunFile = serde_json::from_slice(&fs::read(&run_file_path)?)?;
        if let ResumeDecision::Incompatible {
            field,
            existing,
            resolved,
        } = check_resume(&existing.identity, &identity)
        {
            bail!("incompatible resume: {field} changed from {existing} to {resolved}; choose a new run_id");
        }
        Some(existing)
    } else {
        None
    };
    let mut store = ResultStore::open(&run_dir)?;
    let recovery_bytes = existing_run
        .as_ref()
        .and_then(|file| file.recovery_bytes)
        .unwrap_or_default()
        .saturating_add(store.recovery.as_ref().map_or(0, |event| event.bytes));
    let mut recovery_paths = existing_run
        .as_ref()
        .map(|file| file.recovery_paths.clone())
        .unwrap_or_default();
    if let Some(event) = &store.recovery {
        recovery_paths.push(
            event
                .quarantined_path
                .strip_prefix(&run_dir)
                .unwrap_or(&event.quarantined_path)
                .to_string_lossy()
                .replace('\\', "/"),
        );
        log::warn!(
            "run_id={} article_id=- operation=store_recovery bytes={} quarantine={}",
            run_id,
            event.bytes,
            event.quarantined_path.display()
        );
    }
    let mut run_file = RunFile {
        identity: identity.clone(),
        resolved_config: config.clone(),
        acceptance: config.acceptance.clone(),
        transport_kind: config.run.transport,
        started_utc: existing_run
            .as_ref()
            .map(|file| file.started_utc.clone())
            .unwrap_or_else(|| Utc::now().to_rfc3339()),
        ended_utc: None,
        records_written: load_records(store.path())?.len() as u64,
        superseded_records: load_completed(store.path())?.superseded_records,
        recovery_bytes: (recovery_bytes > 0).then_some(recovery_bytes),
        recovery_paths,
    };
    write_run_file(&run_file_path, &run_file)?;
    let result = match config.run.transport {
        Transport::Fake => {
            let transport = FakeTransport::new(
                config
                    .run
                    .fake_dir
                    .clone()
                    .expect("validated fake directory"),
            )?;
            run_with_transport(
                &transport,
                &mut store,
                &run_dir,
                root,
                &manifest,
                &selected,
                &config,
                &identity,
                &mut run_file,
            )
            .await
        }
        Transport::Live => {
            let transport = HttpTransport::from_environment(
                config.jev.endpoint.clone(),
                Duration::from_millis(config.jev.request_timeout_ms),
            )?;
            run_with_transport(
                &transport,
                &mut store,
                &run_dir,
                root,
                &manifest,
                &selected,
                &config,
                &identity,
                &mut run_file,
            )
            .await
        }
    };
    run_file.ended_utc = Some(Utc::now().to_rfc3339());
    run_file.superseded_records = load_completed(store.path())?.superseded_records;
    write_run_file(&run_file_path, &run_file)?;
    result?;
    if load_completed(store.path())?.completed.is_empty() {
        bail!("run has zero successful result records");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_with_transport<T: JevTransport>(
    transport: &T,
    store: &mut ResultStore,
    run_dir: &Path,
    root: &Path,
    manifest: &Manifest,
    selected: &[&crate::manifest::ArticleEntry],
    config: &RunConfig,
    identity: &RunIdentity,
    run_file: &mut RunFile,
) -> anyhow::Result<()> {
    let completion = load_completed(store.path())?;
    let rubric_text = fs::read_to_string(root.join(&manifest.frozen_inputs.rubric_path))?;
    let rubric = FrozenRubric::from_manifest(rubric_text, manifest)?;
    let mut work = Vec::new();
    for article in selected {
        for repetition in 1..=config.dataset.repeat {
            let pair = (article.article_id.clone(), repetition);
            if completion.completed.contains(&pair)
                || (!config.run.retry_failed && completion.newest_failures.contains_key(&pair))
            {
                continue;
            }
            let sequence = completion.newest_failures.get(&pair).copied().unwrap_or(0) + 1;
            let text = fs::read_to_string(root.join(&article.text_path))?;
            let request = build_request(
                article,
                &FrozenText::for_article(article, text)?,
                &rubric,
                config,
            )?;
            work.push(((*article).clone(), repetition, sequence, request));
        }
    }
    let executions = stream::iter(work.into_iter().map(
        |(article, repetition, sequence, request)| async move {
            execute_pair(
                transport, run_dir, &article, repetition, sequence, request, config, identity,
            )
            .await
        },
    ))
    .buffer_unordered(config.run.concurrency as usize);
    pin_mut!(executions);
    while let Some(record) = executions.next().await {
        let record = record?;
        let terminal_access = record.http_status == Some(401) || record.http_status == Some(422);
        // The coordinator is the single serialization point for durable appends,
        // regardless of how many requests execute concurrently.
        store.append(&record)?;
        run_file.records_written += 1;
        log::info!(
            "run_id={} article_id={} outcome={} attempts={} total_elapsed_ms={}",
            identity.run_id,
            record.article_id,
            record.outcome,
            record.attempt_count,
            record.total_elapsed_ms
        );
        if terminal_access {
            log::error!(
                "run_id={} article_id={} terminal={}",
                identity.run_id,
                record.article_id,
                record.error_type.as_deref().unwrap_or("terminal")
            );
            bail!(
                "terminal provider failure for article {}: {}",
                record.article_id,
                record.error_type.as_deref().unwrap_or("terminal")
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn execute_pair<T: JevTransport>(
    transport: &T,
    run_dir: &Path,
    article: &crate::manifest::ArticleEntry,
    repetition: u32,
    attempt_sequence: u32,
    request: crate::jev::request::JevRequest,
    config: &RunConfig,
    identity: &RunIdentity,
) -> anyhow::Result<ResultRecord> {
    let started = Instant::now();
    let mut raw_paths = Vec::new();
    let mut retries = Vec::new();
    let mut first_latency = None;
    let mut status = None;
    let mut response_bytes = 0_u64;
    for attempt in 1..=config.jev.max_attempts {
        let outcome = transport
            .send(
                &request,
                AttemptContext {
                    repetition,
                    attempt_sequence,
                    attempt,
                },
            )
            .await;
        let latency = match &outcome {
            TransportOutcome::Response { model_latency, .. }
            | TransportOutcome::Timeout { model_latency }
            | TransportOutcome::ConnectionError { model_latency, .. } => *model_latency,
        };
        let millis = latency.as_millis() as u64;
        first_latency.get_or_insert(millis);
        if let TransportOutcome::Response {
            status: http_status,
            body,
            ..
        } = &outcome
        {
            status = Some(*http_status);
            response_bytes = response_bytes.saturating_add(body.len() as u64);
            if !body.is_empty() {
                let path =
                    write_raw_response(run_dir, &article.article_id, repetition, attempt, body)?;
                raw_paths.push(
                    path.strip_prefix(run_dir)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
        match classify(&outcome) {
            Disposition::Success => {
                let TransportOutcome::Response { body, .. } = outcome else {
                    unreachable!()
                };
                return Ok(match parse_jev_response(&body) {
                    Ok(answers) => match validate_answers(&answers, config) {
                        Ok(validated) => success_record(
                            identity,
                            article,
                            repetition,
                            attempt_sequence,
                            config,
                            &request,
                            validated,
                            first_latency,
                            Some(millis),
                            started,
                            attempt,
                            retries,
                            status,
                            response_bytes,
                            raw_paths,
                        ),
                        Err(error) => failure_record(
                            identity,
                            article,
                            repetition,
                            attempt_sequence,
                            config,
                            &request,
                            "invalid",
                            Some("validation".into()),
                            Some(error.to_string()),
                            first_latency,
                            Some(millis),
                            started,
                            attempt,
                            retries,
                            status,
                            response_bytes,
                            raw_paths,
                        ),
                    },
                    Err(error) => failure_record(
                        identity,
                        article,
                        repetition,
                        attempt_sequence,
                        config,
                        &request,
                        "invalid",
                        Some("parse".into()),
                        Some(error.to_string()),
                        first_latency,
                        Some(millis),
                        started,
                        attempt,
                        retries,
                        status,
                        response_bytes,
                        raw_paths,
                    ),
                });
            }
            Disposition::Terminal { reason } => {
                return Ok(failure_record(
                    identity,
                    article,
                    repetition,
                    attempt_sequence,
                    config,
                    &request,
                    "http_error",
                    Some(reason.into()),
                    None,
                    first_latency,
                    Some(millis),
                    started,
                    attempt,
                    retries,
                    status,
                    response_bytes,
                    raw_paths,
                ))
            }
            Disposition::Retryable { reason } if attempt < config.jev.max_attempts => {
                retries.push(reason.into());
                log::warn!(
                    "run_id={} article_id={} retry={} reason={}",
                    identity.run_id,
                    article.article_id,
                    attempt,
                    reason
                );
                tokio::time::sleep(backoff_delay(
                    attempt,
                    &config.jev,
                    if config.jev.jitter {
                        fastrand::f64()
                    } else {
                        0.0
                    },
                ))
                .await;
            }
            Disposition::Retryable { reason } => {
                let detail = match &outcome {
                    TransportOutcome::Timeout { .. } => Some("request timed out".into()),
                    TransportOutcome::ConnectionError { detail, .. } => Some(detail.clone()),
                    TransportOutcome::Response { .. } => None,
                };
                let kind = match outcome {
                    TransportOutcome::Timeout { .. } => "timeout",
                    TransportOutcome::ConnectionError { .. } => "transport_error",
                    _ => "http_error",
                };
                return Ok(failure_record(
                    identity,
                    article,
                    repetition,
                    attempt_sequence,
                    config,
                    &request,
                    kind,
                    Some(reason.into()),
                    detail,
                    first_latency,
                    Some(millis),
                    started,
                    attempt,
                    retries,
                    status,
                    response_bytes,
                    raw_paths,
                ));
            }
        }
    }
    unreachable!("max_attempts is validated non-zero")
}

#[allow(clippy::too_many_arguments)]
fn success_record(
    identity: &RunIdentity,
    article: &crate::manifest::ArticleEntry,
    repetition: u32,
    sequence: u32,
    config: &RunConfig,
    request: &crate::jev::request::JevRequest,
    answers: crate::jev::response::ValidatedAnswers,
    first: Option<u64>,
    last: Option<u64>,
    started: Instant,
    attempts: u32,
    retries: Vec<String>,
    status: Option<u16>,
    response_bytes: u64,
    paths: Vec<String>,
) -> ResultRecord {
    ResultRecord {
        schema_version: RESULT_SCHEMA_VERSION,
        run_id: identity.run_id.clone(),
        config_hash: identity.config_hash.clone(),
        manifest_hash: identity.manifest_hash.clone(),
        prompt_identity_hash: identity.prompt_identity_hash.clone(),
        rubric_sha256: identity.rubric_sha256.clone(),
        transport_kind: identity.transport_kind,
        article_id: article.article_id.clone(),
        evidence_hash: article.evidence_hash.clone(),
        split: split_name(article.split).into(),
        repetition,
        attempt_sequence: sequence,
        provider: "jev".into(),
        requested_model: request.model.clone(),
        returned_model: Some(answers.model),
        timestamp_utc: Utc::now().to_rfc3339(),
        outcome: "ok".into(),
        priority: Some(answers.priority),
        priority_probabilities: Some(answers.priority_distribution.probabilities().clone()),
        priority_confidence: answers.confidence,
        p_high: Some(answers.p_high),
        relevance_score: answers.relevance,
        relevance_legend: answers.relevance_legend,
        relevance_probabilities: answers.relevance_probabilities,
        category_probabilities: Some(answers.category_probabilities),
        categories_selected: Some(answers.selected_categories),
        category_threshold: config.questions.category_probability_threshold,
        tag_probabilities: Some(answers.tag_probabilities),
        tags_selected: Some(answers.selected_tags),
        tag_threshold: config.questions.tag_probability_threshold,
        input_tokens: Some(answers.usage.input_tokens),
        output_tokens: Some(answers.usage.output_tokens),
        first_attempt_latency_ms: first,
        model_latency_ms: last,
        total_elapsed_ms: started.elapsed().as_millis() as u64,
        attempt_count: attempts,
        retry_reasons: retries,
        http_status: status,
        error_type: None,
        error_detail: None,
        estimated_cost_microdollars: Some(jev_cost_microdollars(
            answers.usage.input_tokens,
            config.pricing.input_microdollars_per_million,
        )),
        pricing_version: config.pricing.pricing_version.clone(),
        request_bytes: request.request_bytes().unwrap_or_default().len() as u64,
        response_bytes,
        raw_response_paths: paths,
    }
}

#[allow(clippy::too_many_arguments)]
fn failure_record(
    identity: &RunIdentity,
    article: &crate::manifest::ArticleEntry,
    repetition: u32,
    sequence: u32,
    config: &RunConfig,
    request: &crate::jev::request::JevRequest,
    outcome: &str,
    error_type: Option<String>,
    error_detail: Option<String>,
    first: Option<u64>,
    last: Option<u64>,
    started: Instant,
    attempts: u32,
    retries: Vec<String>,
    status: Option<u16>,
    response_bytes: u64,
    paths: Vec<String>,
) -> ResultRecord {
    ResultRecord {
        schema_version: RESULT_SCHEMA_VERSION,
        run_id: identity.run_id.clone(),
        config_hash: identity.config_hash.clone(),
        manifest_hash: identity.manifest_hash.clone(),
        prompt_identity_hash: identity.prompt_identity_hash.clone(),
        rubric_sha256: identity.rubric_sha256.clone(),
        transport_kind: identity.transport_kind,
        article_id: article.article_id.clone(),
        evidence_hash: article.evidence_hash.clone(),
        split: split_name(article.split).into(),
        repetition,
        attempt_sequence: sequence,
        provider: "jev".into(),
        requested_model: request.model.clone(),
        returned_model: None,
        timestamp_utc: Utc::now().to_rfc3339(),
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
        category_threshold: config.questions.category_probability_threshold,
        tag_probabilities: None,
        tags_selected: None,
        tag_threshold: config.questions.tag_probability_threshold,
        input_tokens: None,
        output_tokens: None,
        first_attempt_latency_ms: first,
        model_latency_ms: last,
        total_elapsed_ms: started.elapsed().as_millis() as u64,
        attempt_count: attempts,
        retry_reasons: retries,
        http_status: status,
        error_type,
        error_detail,
        estimated_cost_microdollars: None,
        pricing_version: config.pricing.pricing_version.clone(),
        request_bytes: request.request_bytes().unwrap_or_default().len() as u64,
        response_bytes,
        raw_response_paths: paths,
    }
}

fn make_identity(run_id: &str, manifest: &Manifest, config: &RunConfig) -> RunIdentity {
    let mut stable_config = config.clone();
    // Retrying failed pairs is a resume operation, not a new experiment input.
    // Deliberately excluding it keeps identity.config_hash stable even though
    // run.json's resolved_config reflects the flag used for this invocation.
    stable_config.run.retry_failed = false;
    RunIdentity {
        run_id: run_id.into(),
        tool_version: env!("CARGO_PKG_VERSION").into(),
        manifest_hash: manifest.manifest_hash.clone(),
        config_hash: stable_config.config_hash(),
        prompt_identity_hash: manifest.frozen_inputs.system_sha256.clone(),
        rubric_sha256: manifest.frozen_inputs.rubric_sha256.clone(),
        transport_kind: config.run.transport,
        split: config.dataset.split,
        repeat: config.dataset.repeat,
    }
}
fn select_articles(
    manifest: &Manifest,
    split: DatasetSelection,
    limit: usize,
) -> Vec<&crate::manifest::ArticleEntry> {
    manifest
        .articles
        .iter()
        .filter(|article| {
            matches!(split, DatasetSelection::All)
                || (matches!(split, DatasetSelection::Dev) && article.split == DatasetSplit::Dev)
                || (matches!(split, DatasetSelection::Heldout)
                    && article.split == DatasetSplit::Heldout)
        })
        .take(if limit == 0 { usize::MAX } else { limit })
        .collect()
}
fn ensure_heldout_acceptance(
    selected: &[&crate::manifest::ArticleEntry],
    config: &RunConfig,
) -> anyhow::Result<()> {
    if selected
        .iter()
        .any(|article| article.split == DatasetSplit::Heldout)
        && config.acceptance.is_none()
    {
        bail!("held-out selection requires complete [acceptance]: high_priority_recall_min, severe_miss_rate_max, max_p95_latency_ms, agreed_utc, agreed_note");
    }
    Ok(())
}
fn generated_run_id(config: &RunConfig) -> String {
    format!(
        "{}-{:?}-{}",
        config.meta.name,
        config.dataset.split,
        Utc::now().format("%Y%m%dT%H%M%SZ")
    )
    .to_ascii_lowercase()
}
fn split_name(split: DatasetSplit) -> &'static str {
    match split {
        DatasetSplit::Dev => "dev",
        DatasetSplit::Heldout => "heldout",
    }
}
fn write_run_file(path: &Path, file: &RunFile) -> anyhow::Result<()> {
    fs::write(path, serde_json::to_vec_pretty(file)?)?;
    Ok(())
}
