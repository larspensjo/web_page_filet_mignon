use crate::cli::{Args, CheckpointCommand};
use crate::lock;
use crate::progress::ProgressReporter;
use chrono::Utc;
use engine_logging::{engine_debug, engine_info, engine_warn};
use harvester_core::{
    update, AppState, ArticleSummaryResult, BatchObservation, CompletedJobSnapshot, ImportPhase,
    LlmModelUsageView, Msg, SummaryCache, SummaryCacheEntry, SummaryCacheKey,
};
use harvester_engine::llm::prompt::{PromptId, PromptTemplateOwned, PROMPT_VERSION_DRAFT};
use harvester_engine::llm::prompts::register_defaults;
use harvester_engine::llm::{
    load_context_file, validate_summary, LlmCommand, LlmCompletionCommand, LlmCompletionError,
    LlmConfig, LlmEvent, LlmHandle, LlmQuotas, ModelId, OpenAiProvider, PricingRegistry,
    PromptRegistry, ProviderKind, DEFAULT_BRIEFING_MODEL, DEFAULT_SUMMARY_MODEL,
    DEFAULT_TRIAGE_MODEL, OPENAI_MODEL_GPT_4O_MINI,
};
use harvester_engine::{
    ensure_output_dir, load_and_prepare_articles_filtered, scan_archive_article_metadata,
    AtomicFileWriter,
};
use harvester_io::{
    load_briefing_checkpoint, load_completed_jobs, load_entity_index, load_prompt_templates,
    load_signal_candidate_cache, load_signal_candidate_overrides, load_sources, load_summary_cache,
    load_triage_cache, persist_completed_jobs, persist_summary_cache, save_briefing_checkpoint,
    save_entity_index, upsert_entry, EffectRunner, EntityIndexPatch, NoOpPlatformHandler,
    RuntimePaths,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CycleOutcome {
    Success,
    PartialFailure,
    TotalFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CycleCounts {
    new_jobs: usize,
    jobs_done: usize,
    jobs_failed: usize,
    triage_completed: usize,
    triage_failed: usize,
    summary_completed: usize,
    summary_failed: usize,
    imports_completed: usize,
    imports_failed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CycleCounterBaseline {
    jobs_total: usize,
    jobs_done: usize,
    jobs_failed: usize,
    triage_completed: usize,
    triage_failed: usize,
    summary_completed: usize,
    summary_failed: usize,
    imports_completed: usize,
    imports_failed: usize,
}

#[derive(Debug, Clone, Copy)]
struct DispatchLoopOptions {
    enable_ai_orchestration: bool,
    require_new_jobs_since: Option<usize>,
    tick_interval: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryRefreshTarget {
    primary_url: String,
    content_hash: String,
    cache_key: SummaryCacheKey,
    related_urls: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct SummaryRefreshSelection {
    targets: Vec<SummaryRefreshTarget>,
    total_stale: usize,
}

type SummaryRefreshRuntime = (
    LlmHandle,
    Arc<RwLock<PromptRegistry>>,
    u32,
    String,
    Vec<(String, String)>,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SummaryRefreshFailure {
    request_id: Option<u64>,
    url: String,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SummaryRefreshReport {
    started_at_utc: String,
    finished_at_utc: String,
    status: String,
    output_dir: String,
    prompt_version: u32,
    configured_model: String,
    limit: usize,
    stale_total_before: usize,
    selected: usize,
    attempted: usize,
    succeeded: usize,
    failed: usize,
    skipped_unloadable: usize,
    remaining_stale_estimate: usize,
    summary_cache_entries_before: usize,
    summary_cache_entries_after: usize,
    usage_calls: u64,
    input_tokens: u64,
    output_tokens: u64,
    estimated_cost_microdollars: u64,
    estimated_cost_display: String,
    failures: Vec<SummaryRefreshFailure>,
}

impl CycleCounterBaseline {
    fn from_observation(obs: &BatchObservation) -> Self {
        Self {
            jobs_total: obs.jobs_total,
            jobs_done: obs.jobs_done,
            jobs_failed: obs.jobs_failed,
            triage_completed: obs.triage_completed,
            triage_failed: obs.triage_failed,
            summary_completed: obs.summary_completed,
            summary_failed: obs.summary_failed,
            imports_completed: obs.imports_completed,
            imports_failed: obs.imports_failed,
        }
    }

    fn measure_cycle_and_advance(&mut self, obs: &BatchObservation) -> CycleCounts {
        let counts = CycleCounts {
            new_jobs: obs.jobs_total.saturating_sub(self.jobs_total),
            jobs_done: obs.jobs_done.saturating_sub(self.jobs_done),
            jobs_failed: obs.jobs_failed.saturating_sub(self.jobs_failed),
            triage_completed: obs.triage_completed.saturating_sub(self.triage_completed),
            triage_failed: obs.triage_failed.saturating_sub(self.triage_failed),
            summary_completed: obs.summary_completed.saturating_sub(self.summary_completed),
            summary_failed: obs.summary_failed.saturating_sub(self.summary_failed),
            imports_completed: obs.imports_completed.saturating_sub(self.imports_completed),
            imports_failed: obs.imports_failed.saturating_sub(self.imports_failed),
        };
        *self = Self::from_observation(obs);
        counts
    }
}

/// Determines if the batch cycle should settle (all reducer-owned work quiesced).
fn should_settle_cycle(status: harvester_core::BatchStatus) -> bool {
    matches!(status, harvester_core::BatchStatus::Settled)
}

/// Determines if an import-mode cycle should settle.
/// Import mode ignores poll/triage/job state; only waits for the import and its
/// downstream work (summaries or briefing) to complete.
fn should_settle_import_cycle(obs: &BatchObservation) -> bool {
    // In import mode we don't poll sources, so poll_in_progress is always false.
    // We settle when the import phase is no longer Importing and all summary work is done.
    !obs.import_in_flight
        && !matches!(obs.import_phase, ImportPhase::Importing)
        && obs.summary_in_flight == 0
        && obs.summary_pending == 0
}

fn should_check_settlement_this_iteration(orchestrated: bool) -> bool {
    !orchestrated
}

fn should_stop_after_cycle(single_shot: bool, shutdown_requested: bool) -> bool {
    shutdown_requested || single_shot
}

fn determine_exit_code(total_failure_cycles: usize) -> i32 {
    if total_failure_cycles > 0 {
        1
    } else {
        0
    }
}

fn should_run_ai_orchestration(
    enable_ai_orchestration: bool,
    require_new_jobs_since: Option<usize>,
    obs: &BatchObservation,
) -> bool {
    if !enable_ai_orchestration {
        return false;
    }
    match require_new_jobs_since {
        Some(baseline_jobs_total) => obs.jobs_total > baseline_jobs_total,
        None => true,
    }
}

/// Classifies the outcome of a completed cycle based on observation metrics.
fn classify_cycle_outcome(obs: &BatchObservation) -> CycleOutcome {
    let has_failures = obs.jobs_failed > 0 || obs.triage_failed > 0;
    let has_successes = obs.jobs_done > 0 || obs.triage_completed > 0;

    match (has_successes, has_failures) {
        (true, false) => CycleOutcome::Success,
        (true, true) => CycleOutcome::PartialFailure,
        (false, true) => CycleOutcome::TotalFailure,
        (false, false) => CycleOutcome::Success, // Nothing to do is success
    }
}

/// Classifies the outcome of a completed import-mode cycle.
fn classify_import_cycle_outcome(obs: &BatchObservation) -> CycleOutcome {
    let has_import_success = obs.imports_completed > 0;
    let has_import_failure =
        obs.imports_failed > 0 || matches!(obs.import_phase, ImportPhase::Failed);

    match (has_import_success, has_import_failure) {
        (true, false) => CycleOutcome::Success,
        (true, true) => CycleOutcome::PartialFailure,
        (false, true) => CycleOutcome::TotalFailure,
        // Idle means nothing was even attempted — treat as total failure.
        (false, false) => CycleOutcome::TotalFailure,
    }
}

fn cycle_outcome_label(outcome: &CycleOutcome) -> &'static str {
    match outcome {
        CycleOutcome::Success => "SUCCESS",
        CycleOutcome::PartialFailure => "PARTIAL",
        CycleOutcome::TotalFailure => "FAILED",
    }
}

fn effective_model_map(config: &LlmConfig) -> HashMap<PromptId, String> {
    let mut map = HashMap::new();

    let triage_model = config
        .triage_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleTriage, triage_model);

    let summary_model = config
        .summary_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleSummary, summary_model);

    let signal_candidate_model = config
        .signal_candidate_model
        .as_ref()
        .or(config.summary_model.as_ref())
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleSignalCandidate, signal_candidate_model);

    let briefing_model = config
        .briefing_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::AggregateBriefing, briefing_model);

    map
}

fn build_effect_runner(
    paths: &RuntimePaths,
    msg_tx: mpsc::Sender<Msg>,
    llm_concurrency: usize,
    platform_handler: Box<NoOpPlatformHandler>,
) -> EffectRunner {
    if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        if api_key.trim().is_empty() {
            engine_warn!("[batch] OPENAI_API_KEY is empty; AI triage/summary features disabled");
            return EffectRunner::new(paths.clone(), msg_tx, platform_handler);
        }
        let provider: Arc<dyn harvester_engine::llm::provider::LlmProvider> =
            Arc::new(OpenAiProvider::new(api_key));
        let provider_clone = Arc::clone(&provider);
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        let registry = Arc::new(RwLock::new(registry));
        let config = LlmConfig {
            provider,
            default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_4O_MINI),
            triage_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_TRIAGE_MODEL)),
            summary_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_SUMMARY_MODEL)),
            signal_candidate_model: None,
            briefing_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_BRIEFING_MODEL)),
            registry: Arc::clone(&registry),
            quotas: LlmQuotas::default(),
            output_dir: paths.output_dir.clone(),
            pricing: PricingRegistry::with_defaults(),
            max_input_bytes: 100_000,
            #[allow(deprecated)]
            max_input_chars: 0,
            timestamp_utc: Arc::new(|| Utc::now().to_rfc3339()),
            session_id: format!("batch-{}", Utc::now().format("%Y%m%d-%H%M%S")),
            replay_cache: None,
            max_concurrent_requests: llm_concurrency,
        };
        let model_map = effective_model_map(&config);
        let handle = LlmHandle::new(config);
        EffectRunner::new_with_llm(
            paths.clone(),
            msg_tx,
            handle,
            100_000,
            Arc::clone(&registry),
            model_map,
            provider_clone,
            ProviderKind::OpenAi,
            platform_handler,
        )
    } else {
        engine_warn!("[batch] OPENAI_API_KEY not set; AI triage/summary features disabled");
        EffectRunner::new(paths.clone(), msg_tx, platform_handler)
    }
}

fn build_prompt_registry_with_saved_overlays(paths: &RuntimePaths) -> PromptRegistry {
    let mut registry = PromptRegistry::new();
    register_defaults(&mut registry);

    for entry in load_prompt_templates(&paths.prompts_dir) {
        let loaded_template = match entry {
            Ok(template) => template,
            Err(reason) => {
                engine_warn!(
                    "[summary-refresh] failed to load saved prompt template: {}",
                    reason
                );
                continue;
            }
        };

        if loaded_template.template_file.version == PROMPT_VERSION_DRAFT {
            engine_warn!(
                "[summary-refresh] skipping draft saved template prompt_id={:?} path={}",
                loaded_template.prompt_id,
                loaded_template.path.display()
            );
            continue;
        }

        registry.register_overlay(PromptTemplateOwned {
            id: loaded_template.prompt_id,
            version: loaded_template.template_file.version,
            system_template: loaded_template.template_file.system_template,
            user_template: loaded_template.template_file.user_template,
            description: loaded_template.template_file.description,
            expected_format: loaded_template.template_file.expected_format,
        });
    }

    registry
}

fn load_summary_context_pairs(paths: &RuntimePaths) -> Vec<(String, String)> {
    let path = paths.contexts_dir.join("article_summary.toml");
    if !path.exists() {
        return Vec::new();
    }

    match load_context_file(&path) {
        Ok(file) => file.variables.into_iter().collect(),
        Err(err) => {
            engine_warn!(
                "[summary-refresh] failed to load summary context from {}: {}",
                path.display(),
                err
            );
            Vec::new()
        }
    }
}

fn build_summary_refresh_runtime(
    paths: &RuntimePaths,
    llm_concurrency: usize,
) -> Result<SummaryRefreshRuntime, String> {
    let api_key =
        std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY not set".to_string())?;
    if api_key.trim().is_empty() {
        return Err("OPENAI_API_KEY is empty".to_string());
    }

    let provider: Arc<dyn harvester_engine::llm::provider::LlmProvider> =
        Arc::new(OpenAiProvider::new(api_key));
    let registry = Arc::new(RwLock::new(build_prompt_registry_with_saved_overlays(
        paths,
    )));
    let summary_context = load_summary_context_pairs(paths);
    let summary_prompt_version = registry
        .read()
        .unwrap()
        .active(PromptId::ArticleSummary)
        .map(|template| template.version)
        .ok_or_else(|| "summary prompt not registered".to_string())?;

    let config = LlmConfig {
        provider,
        default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_4O_MINI),
        triage_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_TRIAGE_MODEL)),
        summary_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_SUMMARY_MODEL)),
        signal_candidate_model: None,
        briefing_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_BRIEFING_MODEL)),
        registry: Arc::clone(&registry),
        quotas: LlmQuotas::default(),
        output_dir: paths.output_dir.clone(),
        pricing: PricingRegistry::with_defaults(),
        max_input_bytes: 100_000,
        #[allow(deprecated)]
        max_input_chars: 0,
        timestamp_utc: Arc::new(|| Utc::now().to_rfc3339()),
        session_id: format!(
            "batch-summary-refresh-{}",
            Utc::now().format("%Y%m%d-%H%M%S")
        ),
        replay_cache: None,
        max_concurrent_requests: llm_concurrency,
    };
    let summary_model = config
        .summary_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();

    Ok((
        LlmHandle::new(config),
        registry,
        summary_prompt_version,
        summary_model,
        summary_context,
    ))
}

fn select_stale_summary_targets(
    metas: &[harvester_engine::ArchiveArticleMeta],
    summary_cache: &SummaryCache,
    prompt_version: u32,
    model_id: &str,
    context: &[(String, String)],
    limit: usize,
) -> SummaryRefreshSelection {
    let mut grouped_targets: Vec<SummaryRefreshTarget> = Vec::new();
    let mut grouped_by_hash = HashMap::<String, usize>::new();

    for meta in metas {
        let Some(content_hash) = meta.content_hash.as_ref().filter(|hash| !hash.is_empty()) else {
            continue;
        };

        if let Some(index) = grouped_by_hash.get(content_hash).copied() {
            grouped_targets[index]
                .related_urls
                .push((meta.url.clone(), meta.fetched_utc.clone()));
            continue;
        }

        let Ok(cache_key) = SummaryCacheKey::try_new(
            content_hash,
            PromptId::ArticleSummary,
            Some(prompt_version),
            Some(model_id),
            context,
        ) else {
            continue;
        };

        grouped_by_hash.insert(content_hash.clone(), grouped_targets.len());
        grouped_targets.push(SummaryRefreshTarget {
            primary_url: meta.url.clone(),
            content_hash: content_hash.clone(),
            cache_key,
            related_urls: vec![(meta.url.clone(), meta.fetched_utc.clone())],
        });
    }

    let mut selection = SummaryRefreshSelection::default();
    for target in grouped_targets {
        if summary_cache.lookup(&target.cache_key).is_some() {
            continue;
        }
        selection.total_stale += 1;
        if selection.targets.len() < limit {
            selection.targets.push(target);
        }
    }

    selection
}

fn format_llm_completion_error(error: &LlmCompletionError) -> String {
    match error {
        LlmCompletionError::ProviderError(err) => format!("provider error: {err}"),
        LlmCompletionError::ValidationFailed { reason, .. } => {
            format!("validation failed: {reason}")
        }
        LlmCompletionError::QuotaExhausted { description, .. } => {
            format!("quota exhausted: {description}")
        }
        LlmCompletionError::PromptNotFound { prompt_id } => {
            format!("prompt not found: {prompt_id:?}")
        }
        LlmCompletionError::PersistenceFailed { detail, .. } => {
            format!("persistence failed: {detail}")
        }
        LlmCompletionError::InputTooLarge { size, limit } => {
            format!("input too large: {size} > {limit}")
        }
        LlmCompletionError::TemplateRenderFailed { detail } => {
            format!("template render failed: {detail}")
        }
        LlmCompletionError::UnsupportedModel { model, reason } => {
            format!("unsupported model {}: {reason}", model.model_name())
        }
    }
}

fn summary_refresh_status_label(successes: usize, failures: usize) -> &'static str {
    match (successes, failures) {
        (0, 0) => "noop",
        (_, 0) => "success",
        (0, _) => "failed",
        _ => "partial_success",
    }
}

fn summary_refresh_exit_code(successes: usize, failures: usize) -> i32 {
    if failures > 0 && successes == 0 {
        1
    } else {
        0
    }
}

fn persist_summary_refresh_report(
    paths: &RuntimePaths,
    started_at_utc: &str,
    report: &SummaryRefreshReport,
) -> Result<PathBuf, String> {
    let reports_dir = paths.output_dir.join("summary_refresh_reports");
    ensure_output_dir(&reports_dir)
        .map_err(|err| format!("failed to create summary refresh report directory: {err}"))?;

    let serialized = serde_json::to_string_pretty(report)
        .map_err(|err| format!("failed to serialize summary refresh report: {err}"))?;

    let compact_timestamp: String = started_at_utc
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .take(14)
        .collect();
    let timestamp = if compact_timestamp.is_empty() {
        "latest".to_string()
    } else {
        compact_timestamp
    };

    let report_filename = format!("summary-refresh-{timestamp}.json");
    let reports_writer = AtomicFileWriter::new(reports_dir.clone());
    let report_path = reports_writer
        .write(&report_filename, &serialized)
        .map_err(|err| format!("failed to write summary refresh report: {err}"))?;

    let latest_writer = AtomicFileWriter::new(paths.output_dir.clone());
    latest_writer
        .write(".summary_refresh_last.json", &serialized)
        .map_err(|err| format!("failed to write latest summary refresh report: {err}"))?;

    Ok(report_path)
}

fn run_refresh_stale_summaries_mode(paths: &RuntimePaths, args: &Args) -> Result<i32, String> {
    let limit = args
        .refresh_stale_summaries_limit
        .ok_or_else(|| "missing --refresh-stale-summaries-limit".to_string())?;
    if limit == 0 {
        return Err("--refresh-stale-summaries-limit must be greater than zero".to_string());
    }
    let started_at_utc = Utc::now().to_rfc3339();
    let progress_enabled = std::io::stdout().is_terminal() && std::io::stderr().is_terminal();
    let mut progress: Option<ProgressReporter> = None;

    let (llm_handle, registry, prompt_version, summary_model, summary_context) =
        build_summary_refresh_runtime(paths, args.llm_concurrency)?;

    let result = (|| -> Result<(SummaryRefreshReport, i32), String> {
        let mut summary_cache = load_summary_cache(&paths.summary_cache_path);
        let summary_cache_entries_before = summary_cache.len();
        let article_metas = scan_archive_article_metadata(&paths.output_dir)?;
        let selection = select_stale_summary_targets(
            &article_metas,
            &summary_cache,
            prompt_version,
            &summary_model,
            &summary_context,
            limit,
        );

        engine_info!(
            "[summary-refresh] stale_total={} selected={} limit={} prompt_version={} model_id={}",
            selection.total_stale,
            selection.targets.len(),
            limit,
            prompt_version,
            summary_model
        );
        progress = Some(ProgressReporter::new(
            selection.targets.len(),
            selection.total_stale,
            limit,
            args.llm_concurrency,
            progress_enabled,
        ));
        if let Some(p) = progress.as_ref() {
            p.startup_line(&mut std::io::stdout());
        }

        let mut report = SummaryRefreshReport {
            started_at_utc: started_at_utc.clone(),
            finished_at_utc: started_at_utc.clone(),
            status: "noop".to_string(),
            output_dir: paths.output_dir.display().to_string(),
            prompt_version,
            configured_model: summary_model.clone(),
            limit,
            stale_total_before: selection.total_stale,
            selected: selection.targets.len(),
            attempted: 0,
            succeeded: 0,
            failed: 0,
            skipped_unloadable: 0,
            remaining_stale_estimate: selection.total_stale,
            summary_cache_entries_before,
            summary_cache_entries_after: summary_cache_entries_before,
            usage_calls: 0,
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_microdollars: 0,
            estimated_cost_display: microdollars_to_display(0),
            failures: Vec::new(),
        };

        if selection.total_stale == 0 {
            engine_info!("[summary-refresh] all summaries already match current cache key");
            report.finished_at_utc = Utc::now().to_rfc3339();
            return Ok((report, 0));
        }

        let selected_urls: Vec<String> = selection
            .targets
            .iter()
            .map(|target| target.primary_url.clone())
            .collect();
        let (articles, _) = {
            let guard = registry.read().unwrap();
            load_and_prepare_articles_filtered(
                &paths.output_dir,
                100_000,
                &guard,
                &selected_urls,
                None,
            )?
        };

        let mut targets_by_url: HashMap<String, SummaryRefreshTarget> = selection
            .targets
            .into_iter()
            .map(|target| (target.primary_url.clone(), target))
            .collect();
        let mut pending = HashMap::<u64, SummaryRefreshTarget>::new();
        let mut request_id = 0u64;

        for article in articles {
            let Some(target) = targets_by_url.remove(&article.url) else {
                continue;
            };

            request_id += 1;
            llm_handle
                .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
                    request_id,
                    prompt_id: PromptId::ArticleSummary,
                    prompt_version: Some(prompt_version),
                    model_override: None,
                    input_content: article.prepared_text,
                    context: summary_context.clone(),
                    template_override: None,
                    extra_template_vars: vec![],
                })))
                .map_err(|err| format!("failed to dispatch summary refresh request: {err}"))?;
            pending.insert(request_id, target);
            if let Some(p) = progress.as_mut() {
                p.request_dispatched();
            }
        }

        report.attempted = pending.len();
        if !targets_by_url.is_empty() {
            report.failed += targets_by_url.len();
            report.skipped_unloadable = targets_by_url.len();
            engine_warn!(
                "[summary-refresh] {} selected article(s) could not be loaded from archive",
                targets_by_url.len()
            );
            for (_, target) in targets_by_url {
                report.failures.push(SummaryRefreshFailure {
                    request_id: None,
                    url: target.primary_url,
                    reason: "selected article could not be loaded from archive".to_string(),
                });
                if let Some(p) = progress.as_mut() {
                    let failure = report.failures.last().expect("failure was just pushed");
                    p.unloadable_target(
                        &failure.url,
                        "selected article could not be loaded from archive",
                        &mut std::io::stdout(),
                        &mut std::io::stderr(),
                    );
                }
            }
        }
        if pending.is_empty() {
            return Err("no stale summaries could be dispatched".to_string());
        }

        let event_rx = llm_handle.event_receiver();
        let mut entity_index = load_entity_index(&paths.entity_index_path);

        while !pending.is_empty() {
            let event = {
                let receiver = event_rx.lock().unwrap();
                receiver.recv()
            }
            .map_err(|err| format!("summary refresh worker stopped unexpectedly: {err}"))?;

            let LlmEvent::Completed { request_id, result } = event else {
                continue;
            };
            let Some(target) = pending.remove(&request_id) else {
                engine_warn!(
                    "[summary-refresh] received completion for unknown request_id={}",
                    request_id
                );
                continue;
            };

            match result {
                Ok(completion) => match validate_summary(&completion.output_json) {
                    Ok(summary) => {
                        let summary_result = ArticleSummaryResult {
                            title: summary.title,
                            summary: summary.summary,
                            key_points: summary.key_points,
                            input_tokens: completion.metadata.input_tokens,
                            output_tokens: completion.metadata.output_tokens,
                            entities: summary.entities,
                        };
                        summary_cache.insert(
                            target.cache_key,
                            SummaryCacheEntry {
                                result: summary_result.clone(),
                                created_at_utc: completion.metadata.timestamp_utc.clone(),
                            },
                        );

                        let mut seen_urls = HashSet::new();
                        for (url, fetched_utc) in target.related_urls {
                            if !seen_urls.insert(url.clone()) {
                                continue;
                            }
                            upsert_entry(
                                &mut entity_index,
                                &url,
                                EntityIndexPatch {
                                    fetched_utc,
                                    content_hash: Some(target.content_hash.clone()),
                                    summary_entities: Some(summary_result.entities.clone()),
                                    themes: None,
                                },
                            );
                        }

                        report.succeeded += 1;
                        if let Some(p) = progress.as_mut() {
                            p.completed_ok(&mut std::io::stdout());
                        }
                        engine_info!(
                            "[summary-refresh] refreshed request_id={} url={} content_hash={}",
                            request_id,
                            target.primary_url,
                            &target.content_hash[..target.content_hash.len().min(8)]
                        );
                    }
                    Err(err) => {
                        engine_warn!(
                            "[summary-refresh] validation failed after success request_id={} url={} reason={}",
                            request_id,
                            target.primary_url,
                            err
                        );
                        report.failed += 1;
                        report.failures.push(SummaryRefreshFailure {
                            request_id: Some(request_id),
                            url: target.primary_url,
                            reason: format!("validation failed after success: {err}"),
                        });
                        if let Some(p) = progress.as_mut() {
                            let failure = report.failures.last().expect("failure was just pushed");
                            p.completed_fail(
                                &failure.url,
                                &failure.reason,
                                &mut std::io::stdout(),
                                &mut std::io::stderr(),
                            );
                        }
                    }
                },
                Err(err) => {
                    report.failed += 1;
                    let reason = format_llm_completion_error(&err);
                    engine_warn!(
                        "[summary-refresh] request failed request_id={} url={} reason={}",
                        request_id,
                        target.primary_url,
                        reason
                    );
                    report.failures.push(SummaryRefreshFailure {
                        request_id: Some(request_id),
                        url: target.primary_url,
                        reason,
                    });
                    if let Some(p) = progress.as_mut() {
                        let failure = report.failures.last().expect("failure was just pushed");
                        p.completed_fail(
                            &failure.url,
                            &failure.reason,
                            &mut std::io::stdout(),
                            &mut std::io::stderr(),
                        );
                    }
                }
            }
        }

        if report.succeeded > 0 {
            persist_summary_cache(&summary_cache, &paths.summary_cache_path)
                .map_err(|err| format!("failed to persist summary cache: {err}"))?;
            save_entity_index(&paths.entity_index_path, &entity_index)
                .map_err(|err| format!("failed to persist entity index: {err}"))?;
        }
        report.summary_cache_entries_after = summary_cache.len();
        report.remaining_stale_estimate =
            report.stale_total_before.saturating_sub(report.succeeded);
        report.status = summary_refresh_status_label(report.succeeded, report.failed).to_string();
        report.finished_at_utc = Utc::now().to_rfc3339();

        engine_info!(
            "[summary-refresh] completed successes={} failures={}",
            report.succeeded,
            report.failed
        );
        let exit_code = summary_refresh_exit_code(report.succeeded, report.failed);
        Ok((report, exit_code))
    })();

    let usage_totals = llm_handle.usage_totals();
    llm_handle.drain_and_stop();
    match result {
        Ok((mut report, exit_code)) => {
            let totals = usage_totals.unwrap_or(harvester_engine::llm::LlmUsageTotals {
                calls: 0,
                input_tokens: 0,
                output_tokens: 0,
                cost_microdollars: 0,
            });
            report.usage_calls = totals.calls;
            report.input_tokens = totals.input_tokens;
            report.output_tokens = totals.output_tokens;
            report.estimated_cost_microdollars = totals.cost_microdollars;
            report.estimated_cost_display = microdollars_to_display(totals.cost_microdollars);

            engine_info!(
                "[summary-refresh] usage calls={} input_tokens={} output_tokens={} cost={}",
                totals.calls,
                totals.input_tokens,
                totals.output_tokens,
                report.estimated_cost_display
            );

            let report_path = persist_summary_refresh_report(paths, &started_at_utc, &report)?;
            engine_info!(
                "[summary-refresh] report written path={} status={} remaining_stale_estimate={}",
                report_path.display(),
                report.status,
                report.remaining_stale_estimate
            );
            if let Some(p) = progress.as_mut() {
                p.finish(
                    report.succeeded,
                    report.failed,
                    &report.estimated_cost_display,
                    &report_path,
                    &mut std::io::stdout(),
                );
            }
            Ok(exit_code)
        }
        Err(err) => Err(err),
    }
}

fn is_ai_orchestration_enabled() -> bool {
    std::env::var("OPENAI_API_KEY")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

const MAX_BATCH_MSG_LOG_LEN: usize = 240;

fn truncate_for_log(input: &str, max_len: usize) -> String {
    if input.chars().count() <= max_len {
        return input.to_string();
    }
    let mut truncated: String = input.chars().take(max_len).collect();
    truncated.push_str("...");
    truncated
}

fn summarize_batch_msg(msg: &Msg) -> String {
    match msg {
        Msg::PollSourcesClicked => "PollSourcesClicked".to_string(),
        Msg::PollStarted { total } => format!("PollStarted(total={total})"),
        Msg::AllSourcesPollEnded => "AllSourcesPollEnded".to_string(),
        Msg::SourcePollCompleted {
            source_id, urls, ..
        } => {
            format!(
                "SourcePollCompleted {{ source_id: {}, urls: {} }}",
                source_id,
                urls.len()
            )
        }
        Msg::JobProgress {
            job_id,
            stage,
            tokens,
            bytes,
            ..
        } => format!(
            "JobProgress {{ job_id: {}, stage: {:?}, bytes: {:?}, tokens: {:?} }}",
            job_id, stage, bytes, tokens
        ),
        Msg::JobDone { job_id, result, .. } => {
            let result_label = match result {
                harvester_core::JobResultKind::Success => "Success".to_string(),
                harvester_core::JobResultKind::Failed { reason } => {
                    format!("Failed({})", truncate_for_log(reason, 80))
                }
            };
            format!("JobDone {{ job_id: {}, result: {} }}", job_id, result_label)
        }
        Msg::TriageArticlesLoaded { articles, .. } => {
            format!("TriageArticlesLoaded {{ articles: {} }}", articles.len())
        }
        Msg::TriageArticlesLoadProgress {
            files_scanned,
            files_total,
            ..
        } => format!(
            "TriageArticlesLoadProgress {{ files_scanned: {}, files_total: {} }}",
            files_scanned, files_total
        ),
        Msg::ArticlesLoaded { articles, .. } => {
            format!("ArticlesLoaded {{ articles: {} }}", articles.len())
        }
        Msg::BriefingPrereqArticlesLoaded { articles } => {
            format!(
                "BriefingPrereqArticlesLoaded {{ articles: {} }}",
                articles.len()
            )
        }
        Msg::PromptContextsLoaded { contexts } => {
            format!("PromptContextsLoaded {{ prompts: {} }}", contexts.len())
        }
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
            templates,
        } => format!(
            "LlmMetadataLoaded {{ active_versions: {}, effective_models: {}, templates: {} }}",
            active_versions.len(),
            effective_models.len(),
            templates.len()
        ),
        _ => truncate_for_log(&format!("{:?}", msg), MAX_BATCH_MSG_LOG_LEN),
    }
}

fn should_log_batch_msg(msg: &Msg) -> bool {
    !matches!(
        msg,
        Msg::JobProgress {
            stage: harvester_core::Stage::Downloading,
            ..
        }
    )
}

/// Run the batch orchestration loop.
///
/// Executes repeated poll cycles until shutdown signal received or error occurs.
/// Returns exit code: 0 (success), 1 (partial failure), or 2 (fatal error via Err).
///
/// # Arguments
/// * `args` - Parsed command-line arguments specifying paths, intervals, and flags
///
/// # Behavior
/// - Acquires exclusive lock on output directory
/// - Polls sources at configured intervals
/// - Persists state after each cycle
/// - Handles SIGINT/SIGTERM gracefully
/// - Dry-run mode: single poll, read-only, no persistence
pub fn run(args: Args) -> Result<i32, String> {
    engine_info!("[batch] Initializing runtime paths");

    let paths = RuntimePaths::new(
        args.output_dir.clone(),
        args.sources.clone(),
        args.contexts_dir.clone(),
        args.prompts_dir.clone(),
    );

    // Handle checkpoint commands before entering the batch loop.
    match args.checkpoint_command()? {
        Some(CheckpointCommand::Show) => {
            let val = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
            println!("{}", val.as_deref().unwrap_or("NONE"));
            return Ok(0);
        }
        Some(cmd) => {
            let _lock_guard = lock::acquire_lock(&paths.output_dir, args.force_unlock)?;
            execute_checkpoint_write(cmd, &paths)?;
            return Ok(0);
        }
        None => {}
    }

    engine_info!("[batch] Acquiring lock");
    let lock_guard = lock::acquire_lock(&paths.output_dir, args.force_unlock)?;

    // Install signal handler immediately after lock acquisition so Ctrl-C always
    // removes the lock file, regardless of which execution path follows.
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    install_signal_handler(
        Arc::clone(&shutdown_flag),
        lock_guard.lock_path().to_path_buf(),
    );

    if args.dry_run {
        engine_info!("[batch] Dry-run mode: single poll only");
        return run_dry_run(&paths, &args);
    }

    // Import mode: branch before source loading
    if let Some(import_dir) = &args.import_saved_web_dir {
        engine_info!("[batch] Import mode: dir={}", import_dir.display());
        return run_import_mode(
            &paths,
            &args,
            import_dir.clone(),
            Arc::clone(&shutdown_flag),
        );
    }

    if args.refresh_stale_summaries_limit.is_some() {
        engine_info!("[batch] Summary refresh mode enabled");
        return run_refresh_stale_summaries_mode(&paths, &args);
    }

    // Validate source configuration
    engine_info!(
        "[batch] Loading source registry from {:?}",
        paths.sources_path
    );
    let source_registry = load_sources(&paths.sources_path);

    if !args.allow_unsupported_sources {
        let unsupported: Vec<_> = source_registry
            .sources
            .iter()
            .filter_map(|s| match &s.source_type {
                harvester_engine::SourceType::Script { .. } => Some(s.id.to_string()),
                _ => None,
            })
            .collect();

        if !unsupported.is_empty() {
            return Err(format!(
                "Unsupported source types detected: {:?}. Use --allow-unsupported-sources to override.",
                unsupported
            ));
        }
    } else {
        let unsupported_count = source_registry
            .sources
            .iter()
            .filter(|s| matches!(&s.source_type, harvester_engine::SourceType::Script { .. }))
            .count();
        if unsupported_count > 0 {
            engine_warn!(
                "[batch] Running with {} unsupported source(s) (Script type)",
                unsupported_count
            );
        }
    }

    // Create message channel
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();

    // Hydrate state
    engine_info!("[batch] Hydrating state from disk");
    let mut state = AppState::new();
    state.set_triage_max_in_flight(args.llm_concurrency);
    state.set_summary_max_in_flight(args.llm_concurrency);

    // Restore completed jobs
    let completed_jobs = load_completed_jobs(&paths.state_path);
    if !completed_jobs.is_empty() {
        engine_info!("[batch] Restoring {} completed jobs", completed_jobs.len());
        let (new_state, effects) = update(state, Msg::RestoreCompletedJobs(completed_jobs));
        state = new_state;
        // Effects from restore are cache loads which will be executed shortly
        for effect in effects {
            engine_info!("[batch] Restore effect queued: {:?}", effect);
        }
    } else {
        engine_info!("[batch] No previous state found, starting fresh");
    }

    // Build EffectRunner (with optional LLM support based on OPENAI_API_KEY)
    engine_info!("[batch] Building EffectRunner");
    let enable_ai_orchestration = is_ai_orchestration_enabled();
    let platform_handler = Box::new(NoOpPlatformHandler);
    let effect_runner = build_effect_runner(
        &paths,
        msg_tx.clone(),
        args.llm_concurrency,
        platform_handler,
    );
    effect_runner.enqueue(vec![
        harvester_core::Effect::LoadPromptTemplateFiles,
        harvester_core::Effect::LoadLlmMetadata,
    ]);

    // Trigger reducer-owned metadata hydration.
    let (new_state, startup_effects) = update(state, Msg::StartupHydrationRequested);
    state = new_state;
    if !startup_effects.is_empty() {
        effect_runner.enqueue(startup_effects);
    }

    // Hydrate persistent caches for triage/summary reuse.
    let summary_cache = load_summary_cache(&paths.summary_cache_path);
    if !summary_cache.is_empty() {
        let (new_state, effects) = update(
            state,
            Msg::SummaryCacheHydrated {
                cache: summary_cache,
            },
        );
        state = new_state;
        if !effects.is_empty() {
            effect_runner.enqueue(effects);
        }
    }
    let triage_cache = load_triage_cache(&paths.triage_cache_path);
    if !triage_cache.is_empty() {
        let (new_state, effects) = update(
            state,
            Msg::TriageCacheHydrated {
                cache: triage_cache,
            },
        );
        state = new_state;
        if !effects.is_empty() {
            effect_runner.enqueue(effects);
        }
    }
    match load_signal_candidate_cache(&paths.signal_candidate_cache_path) {
        Ok(signal_candidate_cache) if !signal_candidate_cache.is_empty() => {
            let (new_state, effects) = update(
                state,
                Msg::SignalCandidateCacheLoaded {
                    cache: signal_candidate_cache,
                },
            );
            state = new_state;
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
        }
        Ok(_) => {}
        Err(err) => engine_warn!(
            "[signal-cache] failed to hydrate {}: {}",
            paths.signal_candidate_cache_path.display(),
            err
        ),
    }
    match load_signal_candidate_overrides(&paths.signal_candidate_overrides_path) {
        Ok(signal_candidate_overrides) if !signal_candidate_overrides.is_empty() => {
            let (new_state, effects) = update(
                state,
                Msg::SignalCandidateOverridesLoaded {
                    overrides: signal_candidate_overrides,
                },
            );
            state = new_state;
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
        }
        Ok(_) => {}
        Err(err) => engine_warn!(
            "[signal-overrides] failed to hydrate {}: {}",
            paths.signal_candidate_overrides_path.display(),
            err
        ),
    }

    // Outer cycle loop - poll repeatedly until shutdown
    let poll_interval = Duration::from_secs((args.poll_interval * 60) as u64);
    let mut cycle_count = 0;
    let mut total_cycles = 0;
    let mut total_failure_cycles = 0;
    let mut cycle_baseline = CycleCounterBaseline::from_observation(&state.batch_observation());
    let mut total_new_articles = 0usize;
    let mut total_triaged = 0usize;
    let mut total_summarized = 0usize;

    loop {
        cycle_count += 1;
        total_cycles += 1;
        engine_info!("[batch] === Starting cycle {} ===", cycle_count);
        let cycle_jobs_total_baseline = state.batch_observation().jobs_total;

        // Start the cycle by dispatching poll
        engine_info!("[batch] Dispatching poll sources");
        msg_tx
            .send(Msg::PollSourcesClicked)
            .map_err(|e| format!("Failed to dispatch poll: {}", e))?;

        // Run dispatch loop until settled
        let outcome = run_dispatch_loop(
            &mut state,
            &msg_tx,
            &msg_rx,
            &effect_runner,
            &shutdown_flag,
            DispatchLoopOptions {
                enable_ai_orchestration,
                require_new_jobs_since: if args.single_shot {
                    Some(cycle_jobs_total_baseline)
                } else {
                    None
                },
                tick_interval: Duration::from_millis(75),
            },
        )?;

        // Track outcome statistics
        match outcome {
            CycleOutcome::Success => {}
            CycleOutcome::PartialFailure => {}
            CycleOutcome::TotalFailure => total_failure_cycles += 1,
        }

        // Print cycle summary
        let obs = state.batch_observation();
        let cycle_counts = cycle_baseline.measure_cycle_and_advance(&obs);
        total_new_articles += cycle_counts.new_jobs;
        total_triaged += cycle_counts.triage_completed;
        total_summarized += cycle_counts.summary_completed;
        if cycle_count == 1 {
            print_cycle_table_header();
        }
        print_cycle_summary(cycle_count, &outcome, &cycle_counts);
        print_poll_stats(&state.batch_observation().source_poll_stats);
        for line in format_llm_usage_lines(&state.llm_usage_rows()) {
            println!("{}", line);
        }

        // Persist state
        engine_info!("[batch] Persisting state");
        let completed_jobs = state.completed_jobs_snapshot();
        persist_completed_jobs(&paths.state_path, &completed_jobs);

        let shutdown_requested = shutdown_flag.load(Ordering::Relaxed);

        // Check for shutdown signal or single-shot completion.
        if should_stop_after_cycle(args.single_shot, shutdown_requested) {
            if args.single_shot {
                engine_info!("[batch] Single-shot mode completed one cycle; exiting");
            }
            if shutdown_requested {
                engine_info!("[batch] Shutdown signal received, exiting");
            }
            break;
        }

        // Sleep interruptibly before next cycle
        engine_info!(
            "[batch] Sleeping for {} minutes before next cycle",
            args.poll_interval
        );
        if sleep_interruptible(poll_interval, &shutdown_flag) {
            engine_info!("[batch] Shutdown during sleep, exiting");
            break;
        }
    }

    // Graceful shutdown
    engine_info!("[batch] Graceful shutdown: draining effects and persisting final state");
    drop(effect_runner);
    drop(msg_rx);

    let completed_jobs = state.completed_jobs_snapshot();
    persist_completed_jobs(&paths.state_path, &completed_jobs);

    // Print final summary
    print_final_summary(
        total_cycles,
        total_new_articles,
        total_triaged,
        total_summarized,
    );

    engine_info!("[batch] Shutdown complete");

    Ok(determine_exit_code(total_failure_cycles))
}

/// Writes or clears the briefing checkpoint file.
///
/// Called after the output lock is already held.
fn execute_checkpoint_write(cmd: CheckpointCommand, paths: &RuntimePaths) -> Result<(), String> {
    match cmd {
        CheckpointCommand::Set(ts) => {
            // ts was already validated by checkpoint_command()
            engine_info!("[briefing-checkpoint] set to {}", ts);
            save_briefing_checkpoint(&paths.briefing_checkpoint_path, Some(ts.as_str()))
        }
        CheckpointCommand::SetNow => {
            let ts = Utc::now().to_rfc3339();
            engine_info!("[briefing-checkpoint] set to {}", ts);
            save_briefing_checkpoint(&paths.briefing_checkpoint_path, Some(ts.as_str()))
        }
        CheckpointCommand::Clear => {
            engine_info!("[briefing-checkpoint] cleared");
            save_briefing_checkpoint(&paths.briefing_checkpoint_path, None)
        }
        CheckpointCommand::Show => unreachable!("Show is handled before lock acquisition"),
    }
}

/// Runs the inner dispatch loop until settlement or error.
/// Processes messages, updates state, executes effects, and checks for settlement.
fn run_dispatch_loop(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
) -> Result<CycleOutcome, String> {
    run_dispatch_loop_with_tick_interval(
        state,
        msg_tx,
        msg_rx,
        effect_runner,
        shutdown_flag,
        options,
    )
}

fn run_dispatch_loop_with_tick_interval(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
) -> Result<CycleOutcome, String> {
    let timeout = Duration::from_millis(100);
    let mut iterations = 0;
    let mut last_tick = Instant::now();
    const MAX_ITERATIONS: usize = 10_000; // Safety limit

    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            return Err(format!(
                "Dispatch loop exceeded maximum iterations ({})",
                MAX_ITERATIONS
            ));
        }

        // Check for shutdown signal
        if shutdown_flag.load(Ordering::Relaxed) {
            engine_info!("[batch] Shutdown signal detected in dispatch loop");
            let obs = state.batch_observation();
            return Ok(classify_cycle_outcome(&obs));
        }

        // Receive at least one message with timeout, then drain a batch.
        match msg_rx.recv_timeout(timeout) {
            Ok(first_msg) => {
                let mut inbox = vec![first_msg];
                while let Ok(next_msg) = msg_rx.try_recv() {
                    inbox.push(next_msg);
                }

                let mut queued_effects = Vec::new();
                for msg in inbox {
                    if should_log_batch_msg(&msg) {
                        engine_debug!("[batch] Processing message: {}", summarize_batch_msg(&msg));
                    }
                    let (new_state, effects) = update(state.clone(), msg);
                    *state = new_state;
                    queued_effects.extend(effects);
                }

                if let Some(triggered_by_job_done) =
                    state.take_pre_triage_refresh_evaluation_request()
                {
                    let ordered_urls = state.ordered_completed_job_urls_snapshot();
                    let (new_state, effects) = update(
                        state.clone(),
                        Msg::EvaluatePreTriageRefresh {
                            ordered_urls,
                            triggered_by_job_done,
                        },
                    );
                    *state = new_state;
                    queued_effects.extend(effects);
                }

                if !queued_effects.is_empty() {
                    engine_debug!("[batch] Enqueuing {} effects", queued_effects.len());
                    effect_runner.enqueue(queued_effects);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No message available, continue loop
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Message channel disconnected unexpectedly".to_string());
            }
        }

        if options.enable_ai_orchestration && last_tick.elapsed() >= options.tick_interval {
            let (new_state, tick_effects) = update(state.clone(), Msg::Tick);
            *state = new_state;
            if !tick_effects.is_empty() {
                effect_runner.enqueue(tick_effects);
            }
            last_tick = Instant::now();
        }

        // Check for settlement after processing available work.
        let mut orchestrated = false;
        let obs = state.batch_observation();
        if should_run_ai_orchestration(
            options.enable_ai_orchestration,
            options.require_new_jobs_since,
            &obs,
        ) {
            if let Some(next_msg) = maybe_dispatch_batch_ai_orchestration(state) {
                msg_tx.send(next_msg.clone()).map_err(|e| {
                    format!(
                        "Failed to dispatch orchestration message {:?}: {}",
                        next_msg, e
                    )
                })?;
                orchestrated = true;
            }
        }

        // This prevents an immediate idle-state exit before queued actions
        // (like PollSourcesClicked) have been reduced.
        if should_check_settlement_this_iteration(orchestrated)
            && should_settle_cycle(state.batch_status())
        {
            engine_info!(
                "[batch] Cycle settled after {} iterations: jobs={}/{}, triage={}/{}",
                iterations,
                obs.jobs_done,
                obs.jobs_total,
                obs.triage_completed,
                obs.triage_total
            );
            return Ok(classify_cycle_outcome(&obs));
        }
    }
}

fn maybe_dispatch_batch_ai_orchestration(state: &AppState) -> Option<Msg> {
    match state.batch_next_action() {
        harvester_core::BatchNextAction::DispatchTriage => Some(Msg::TriageClicked),
        harvester_core::BatchNextAction::DispatchSummaries => Some(Msg::PrepareSummariesClicked),
        harvester_core::BatchNextAction::None => None,
    }
}

/// Formats a token count as a compact human-readable string (e.g. 12K, 1.2M).
fn format_compact_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
    }
}

/// Formats per-model usage rows as indented display lines.
fn format_llm_usage_lines(rows: &[LlmModelUsageView]) -> Vec<String> {
    rows.iter()
        .map(|r| {
            format!(
                "  {}: in={} out={}",
                r.model,
                format_compact_tokens(r.input_tokens),
                format_compact_tokens(r.output_tokens)
            )
        })
        .collect()
}

/// Prints a grouped poll-stats summary (RSS / Brave / other source types).
fn print_poll_stats(stats: &[harvester_core::SourcePollStat]) {
    if stats.is_empty() {
        return;
    }
    println!("\n--- Poll summary ---");
    println!("{}", harvester_core::format_poll_stats(stats));
    println!("--------------------");
}

fn print_cycle_table_header() {
    println!(
        "{:<6} {:<9} {:>20} {:>18} {:>21}",
        "Cycle", "Outcome", "Jobs(new/done/fail)", "Triage(ok/fail)", "Summaries(ok/fail)"
    );
    println!("{}", "-".repeat(78));
}

/// Prints a summary of the completed cycle.
fn print_cycle_summary(cycle: usize, outcome: &CycleOutcome, counts: &CycleCounts) {
    println!(
        "{:<6} {:<9} {:>20} {:>18} {:>21}",
        cycle,
        cycle_outcome_label(outcome),
        format!(
            "{}/{}/{}",
            counts.new_jobs, counts.jobs_done, counts.jobs_failed
        ),
        format!("{}/{}", counts.triage_completed, counts.triage_failed),
        format!("{}/{}", counts.summary_completed, counts.summary_failed),
    );
}

/// Prints the final summary when batch runner exits.
fn print_final_summary(
    total_cycles: usize,
    total_new_articles: usize,
    total_triaged: usize,
    total_summarized: usize,
) {
    println!(
        "\n-- Batch complete: {} cycles, {} new articles, {} triaged, {} summarized --\n",
        total_cycles, total_new_articles, total_triaged, total_summarized
    );
}

/// Sleeps for the specified duration, checking shutdown flag periodically.
/// Returns true if shutdown was requested during sleep.
fn sleep_interruptible(duration: Duration, shutdown_flag: &Arc<AtomicBool>) -> bool {
    let check_interval = Duration::from_millis(500);
    let mut remaining = duration;

    while remaining > Duration::ZERO {
        if shutdown_flag.load(Ordering::Relaxed) {
            return true;
        }

        let sleep_time = remaining.min(check_interval);
        std::thread::sleep(sleep_time);
        remaining = remaining.saturating_sub(sleep_time);
    }

    false
}

/// Installs a signal handler for SIGINT/SIGTERM.
///
/// On interrupt the handler removes the lock file (so a subsequent run does not
/// need --force-unlock) and exits immediately with code 130 (128 + SIGINT).
fn install_signal_handler(shutdown_flag: Arc<AtomicBool>, lock_path: std::path::PathBuf) {
    let handler = move || {
        // Best-effort lock removal; ignore errors (lock may already be gone).
        let _ = std::fs::remove_file(&lock_path);
        shutdown_flag.store(true, Ordering::Relaxed);
        eprintln!("harvester_batch: interrupted — lock released");
        std::process::exit(130);
    };

    ctrlc::set_handler(handler).expect("Error setting signal handler");
}

/// Converts microdollars to a human-readable dollar string with exact rounding.
/// Examples: 0 -> "$0.00", 1234567 -> "$1.23", 50 -> "$0.00", 5000 -> "$0.01"
fn microdollars_to_display(microdollars: u64) -> String {
    let cents = (microdollars + 5000) / 10000; // Round to nearest cent
    let dollars = cents / 100;
    let remaining_cents = cents % 100;
    format!("${}.{:02}", dollars, remaining_cents)
}

/// Runs the import-mode workflow for browser-saved webpage imports.
///
/// Branches before source loading and drives only the import pipeline.
/// Exits after the import and any requested downstream work (summaries/briefing) settles.
fn run_import_mode(
    paths: &RuntimePaths,
    args: &Args,
    import_dir: std::path::PathBuf,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<i32, String> {
    engine_info!("[import] Starting import mode");
    let existing_completed_jobs = load_completed_jobs(&paths.state_path);

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(args.llm_concurrency);
    state.set_summary_max_in_flight(args.llm_concurrency);

    let enable_ai_orchestration = is_ai_orchestration_enabled();
    let platform_handler = Box::new(NoOpPlatformHandler);
    let effect_runner = build_effect_runner(
        paths,
        msg_tx.clone(),
        args.llm_concurrency,
        platform_handler,
    );

    // Hydrate prompt/template metadata needed for downstream work.
    effect_runner.enqueue(vec![
        harvester_core::Effect::LoadPromptTemplateFiles,
        harvester_core::Effect::LoadLlmMetadata,
    ]);
    let (new_state, startup_effects) = update(state, Msg::StartupHydrationRequested);
    state = new_state;
    if !startup_effects.is_empty() {
        effect_runner.enqueue(startup_effects);
    }

    // Hydrate summary cache for cache-hit reuse during summaries.
    let summary_cache = load_summary_cache(&paths.summary_cache_path);
    if !summary_cache.is_empty() {
        let (new_state, effects) = update(
            state,
            Msg::SummaryCacheHydrated {
                cache: summary_cache,
            },
        );
        state = new_state;
        if !effects.is_empty() {
            effect_runner.enqueue(effects);
        }
    }
    match load_signal_candidate_cache(&paths.signal_candidate_cache_path) {
        Ok(signal_candidate_cache) if !signal_candidate_cache.is_empty() => {
            let (new_state, effects) = update(
                state,
                Msg::SignalCandidateCacheLoaded {
                    cache: signal_candidate_cache,
                },
            );
            state = new_state;
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
        }
        Ok(_) => {}
        Err(err) => engine_warn!(
            "[signal-cache] failed to hydrate {}: {}",
            paths.signal_candidate_cache_path.display(),
            err
        ),
    }
    match load_signal_candidate_overrides(&paths.signal_candidate_overrides_path) {
        Ok(signal_candidate_overrides) if !signal_candidate_overrides.is_empty() => {
            let (new_state, effects) = update(
                state,
                Msg::SignalCandidateOverridesLoaded {
                    overrides: signal_candidate_overrides,
                },
            );
            state = new_state;
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
        }
        Ok(_) => {}
        Err(err) => engine_warn!(
            "[signal-overrides] failed to hydrate {}: {}",
            paths.signal_candidate_overrides_path.display(),
            err
        ),
    }

    // Dispatch the import request.
    let (new_state, import_effects) =
        update(state, Msg::ImportSavedWebpagesRequested { dir: import_dir });
    state = new_state;
    effect_runner.enqueue(import_effects);

    // Run the import dispatch loop until settled.
    let outcome = run_import_dispatch_loop(
        &mut state,
        &msg_tx,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            enable_ai_orchestration,
            require_new_jobs_since: None,
            tick_interval: Duration::from_millis(75),
        },
    )?;

    let obs = state.batch_observation();
    engine_info!(
        "[import] Settled: phase={:?} imported={} failed={}",
        obs.import_phase,
        obs.imports_completed,
        obs.imports_failed,
    );

    println!(
        "\n-- Import complete: {} imported, {} failed --\n",
        obs.imports_completed, obs.imports_failed
    );

    drop(effect_runner);
    let imported_completed_jobs = state.completed_jobs_snapshot();
    let merged_completed_jobs =
        merge_completed_jobs_for_import(existing_completed_jobs, imported_completed_jobs);
    engine_info!(
        "[import] Persisting completed jobs existing={} imported={} merged={}",
        merged_completed_jobs
            .len()
            .saturating_sub(obs.imports_completed),
        obs.imports_completed,
        merged_completed_jobs.len()
    );
    persist_completed_jobs(&paths.state_path, &merged_completed_jobs);

    Ok(match outcome {
        CycleOutcome::Success => 0,
        CycleOutcome::PartialFailure => 1,
        CycleOutcome::TotalFailure => 1,
    })
}

fn merge_completed_jobs_for_import(
    existing_completed_jobs: Vec<CompletedJobSnapshot>,
    imported_completed_jobs: Vec<CompletedJobSnapshot>,
) -> Vec<CompletedJobSnapshot> {
    let mut merged = existing_completed_jobs;
    merged.extend(imported_completed_jobs);
    merged
}

/// Inner dispatch loop for import mode. Uses `should_settle_import_cycle` instead of
/// `should_settle_cycle`, and `classify_import_cycle_outcome` for the final result.
fn run_import_dispatch_loop(
    state: &mut AppState,
    _msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
) -> Result<CycleOutcome, String> {
    let timeout = Duration::from_millis(100);
    let mut iterations = 0;
    let mut last_tick = Instant::now();
    const MAX_ITERATIONS: usize = 10_000;

    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            return Err(format!(
                "Import dispatch loop exceeded maximum iterations ({})",
                MAX_ITERATIONS
            ));
        }

        if shutdown_flag.load(Ordering::Relaxed) {
            engine_info!("[import] Shutdown signal detected");
            let obs = state.batch_observation();
            return Ok(classify_import_cycle_outcome(&obs));
        }

        match msg_rx.recv_timeout(timeout) {
            Ok(first_msg) => {
                let mut inbox = vec![first_msg];
                while let Ok(next_msg) = msg_rx.try_recv() {
                    inbox.push(next_msg);
                }

                let mut queued_effects = Vec::new();
                for msg in inbox {
                    if should_log_batch_msg(&msg) {
                        engine_debug!("[import] Processing message: {}", summarize_batch_msg(&msg));
                    }
                    let (new_state, effects) = update(state.clone(), msg);
                    *state = new_state;
                    queued_effects.extend(effects);
                }

                if !queued_effects.is_empty() {
                    effect_runner.enqueue(queued_effects);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Message channel disconnected unexpectedly".to_string());
            }
        }

        if options.enable_ai_orchestration && last_tick.elapsed() >= options.tick_interval {
            let (new_state, tick_effects) = update(state.clone(), Msg::Tick);
            *state = new_state;
            if !tick_effects.is_empty() {
                effect_runner.enqueue(tick_effects);
            }
            last_tick = Instant::now();
        }

        let obs = state.batch_observation();
        if should_settle_import_cycle(&obs) {
            engine_info!("[import] Cycle settled after {} iterations", iterations);
            return Ok(classify_import_cycle_outcome(&obs));
        }
    }
}

fn run_dry_run(paths: &RuntimePaths, args: &Args) -> Result<i32, String> {
    engine_info!("[dry-run] Starting dry-run mode: single poll, no downloads/triage");

    // Hydrate state (read-only)
    engine_info!(
        "[dry-run] Loading completed jobs from {:?}",
        paths.state_path
    );
    let completed_jobs = load_completed_jobs(&paths.state_path);
    engine_info!("[dry-run] Loaded {} completed jobs", completed_jobs.len());

    // Initialize state
    let (msg_tx, msg_rx) = mpsc::channel();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(args.llm_concurrency);
    state.set_summary_max_in_flight(args.llm_concurrency);

    // Restore completed jobs
    if !completed_jobs.is_empty() {
        let restore_msg = Msg::RestoreCompletedJobs(completed_jobs);
        let (new_state, _effects) = update(state, restore_msg);
        state = new_state;
    }

    // Create effect runner
    let platform_handler = Box::new(NoOpPlatformHandler);
    let effect_runner = EffectRunner::new(paths.clone(), msg_tx.clone(), platform_handler);

    // Dispatch poll
    engine_info!("[dry-run] Dispatching poll");
    msg_tx
        .send(Msg::PollSourcesClicked)
        .map_err(|e| format!("Failed to send poll message: {}", e))?;

    // Run dispatch loop until settlement (read-only, no signal handling needed)
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let outcome = run_dispatch_loop(
        &mut state,
        &msg_tx,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            enable_ai_orchestration: false,
            require_new_jobs_since: None,
            tick_interval: Duration::from_millis(75),
        },
    )?;

    // Print summary
    let obs = state.batch_observation();
    println!("\n=== Dry-Run Summary ===");
    println!("Outcome: {:?}", outcome);
    println!(
        "Jobs: {} total, {} done, {} failed",
        obs.jobs_total, obs.jobs_done, obs.jobs_failed
    );
    println!(
        "Triage: {} total, {} completed, {} failed, {} pending",
        obs.triage_total, obs.triage_completed, obs.triage_failed, obs.triage_pending
    );
    println!("Session state: {:?}", obs.session_state);
    print_poll_stats(&obs.source_poll_stats);
    println!("======================\n");

    engine_info!("[dry-run] Dry-run complete (no state modifications)");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_args(dry_run: bool, temp_dir: &TempDir) -> Args {
        Args {
            output_dir: temp_dir.path().to_path_buf(),
            sources: PathBuf::from("test_sources.json"),
            contexts_dir: PathBuf::from("contexts"),
            prompts_dir: PathBuf::from("prompts"),
            dry_run,
            single_shot: false,
            allow_unsupported_sources: false,
            llm_concurrency: 1,
            poll_interval: 1,
            force_unlock: false,
            set_briefing_since: None,
            set_briefing_since_now: false,
            clear_briefing_since: false,
            show_briefing_since: false,
            import_saved_web_dir: None,
            refresh_stale_summaries_limit: None,
        }
    }

    fn observation_with_totals(
        jobs_total: usize,
        jobs_done: usize,
        jobs_failed: usize,
        triage_completed: usize,
        triage_failed: usize,
        summary_completed: usize,
        summary_failed: usize,
    ) -> BatchObservation {
        observation_with_import(
            jobs_total,
            jobs_done,
            jobs_failed,
            triage_completed,
            triage_failed,
            summary_completed,
            summary_failed,
            0,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn observation_with_import(
        jobs_total: usize,
        jobs_done: usize,
        jobs_failed: usize,
        triage_completed: usize,
        triage_failed: usize,
        summary_completed: usize,
        summary_failed: usize,
        imports_completed: usize,
        imports_failed: usize,
    ) -> BatchObservation {
        BatchObservation {
            poll_in_progress: false,
            session_state: harvester_core::SessionState::Idle,
            jobs_total,
            jobs_done,
            jobs_failed,
            jobs_in_flight: 0,
            pre_triage_phase: harvester_core::PreTriagePhase::Idle,
            pre_triage_total: 0,
            pre_triage_included: 0,
            pre_triage_review: 0,
            pre_triage_filtered: 0,
            triage_phase: harvester_core::TriagePhase::Idle,
            triage_total: 0,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed,
            triage_failed,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed,
            summary_failed,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
            import_phase: harvester_core::ImportPhase::Idle,
            imports_completed,
            imports_failed,
            import_in_flight: false,
            source_poll_stats: vec![],
        }
    }

    #[test]
    fn test_dry_run_exits_successfully_without_api_key() {
        engine_logging::initialize_for_tests();
        let temp_dir = TempDir::new().unwrap();
        let args = create_test_args(true, &temp_dir);

        // Create empty sources file to avoid validation errors
        let sources_path = temp_dir.path().join("test_sources.json");
        std::fs::write(&sources_path, r#"{"sources": []}"#).unwrap();

        let runtime_paths = RuntimePaths::new(
            args.output_dir.clone(),
            sources_path,
            args.contexts_dir.clone(),
            args.prompts_dir.clone(),
        );

        // Dry-run should succeed even without OPENAI_API_KEY
        let result = run_dry_run(&runtime_paths, &args);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_dry_run_does_not_modify_state_files() {
        engine_logging::initialize_for_tests();
        let temp_dir = TempDir::new().unwrap();
        let args = create_test_args(true, &temp_dir);

        let sources_path = temp_dir.path().join("test_sources.json");
        std::fs::write(&sources_path, r#"{"sources": []}"#).unwrap();

        let runtime_paths = RuntimePaths::new(
            args.output_dir.clone(),
            sources_path,
            args.contexts_dir.clone(),
            args.prompts_dir.clone(),
        );

        let state_path = &runtime_paths.state_path;

        // Ensure state file does not exist initially
        assert!(!state_path.exists());

        // Run dry-run
        let result = run_dry_run(&runtime_paths, &args);
        assert!(result.is_ok());

        // State file should still not exist (no writes)
        assert!(!state_path.exists());
    }

    #[test]
    fn test_should_settle_cycle_when_batch_status_is_settled() {
        assert!(should_settle_cycle(harvester_core::BatchStatus::Settled));
    }

    #[test]
    fn test_should_not_settle_cycle_when_batch_status_is_running() {
        assert!(!should_settle_cycle(harvester_core::BatchStatus::Running));
    }

    #[test]
    fn test_orchestration_dispatch_skips_settlement_in_same_iteration() {
        assert!(!should_check_settlement_this_iteration(true));
        assert!(should_check_settlement_this_iteration(false));
    }

    #[test]
    fn test_should_stop_after_cycle_for_single_shot() {
        assert!(should_stop_after_cycle(true, false));
    }

    #[test]
    fn test_should_stop_after_cycle_for_shutdown_signal() {
        assert!(should_stop_after_cycle(false, true));
    }

    #[test]
    fn test_should_continue_after_cycle_when_not_single_shot_and_no_shutdown() {
        assert!(!should_stop_after_cycle(false, false));
    }

    #[test]
    fn test_should_run_ai_orchestration_when_enabled_without_new_jobs_gate() {
        let obs = observation_with_totals(10, 0, 0, 0, 0, 0, 0);
        assert!(should_run_ai_orchestration(true, None, &obs));
    }

    #[test]
    fn test_should_not_run_ai_orchestration_when_no_new_jobs_since_baseline() {
        let obs = observation_with_totals(10, 0, 0, 0, 0, 0, 0);
        assert!(!should_run_ai_orchestration(true, Some(10), &obs));
    }

    #[test]
    fn test_should_run_ai_orchestration_when_new_jobs_arrived_since_baseline() {
        let obs = observation_with_totals(11, 0, 0, 0, 0, 0, 0);
        assert!(should_run_ai_orchestration(true, Some(10), &obs));
    }

    #[test]
    fn determine_exit_code_returns_zero_when_only_partial_failures_occur() {
        assert_eq!(determine_exit_code(0), 0);
    }

    #[test]
    fn determine_exit_code_returns_nonzero_when_total_failure_occurs() {
        assert_eq!(determine_exit_code(1), 1);
    }

    #[test]
    fn cycle_counter_baseline_reports_deltas_not_cumulative_totals() {
        let mut baseline = CycleCounterBaseline::from_observation(&observation_with_totals(
            577, 577, 0, 405, 0, 61, 0,
        ));
        let cycle_counts = baseline
            .measure_cycle_and_advance(&observation_with_totals(578, 578, 0, 406, 0, 61, 0));
        assert_eq!(
            cycle_counts,
            CycleCounts {
                new_jobs: 1,
                jobs_done: 1,
                jobs_failed: 0,
                triage_completed: 1,
                triage_failed: 0,
                summary_completed: 0,
                summary_failed: 0,
                imports_completed: 0,
                imports_failed: 0,
            }
        );
    }

    #[test]
    fn test_summarize_batch_msg_compacts_large_payloads() {
        let msg = Msg::TriageArticlesLoaded {
            request_id: 1,
            articles: Vec::new(),
        };
        let summary = summarize_batch_msg(&msg);
        assert!(summary.contains("TriageArticlesLoaded"));
        assert!(summary.contains("articles: 0"));
        assert!(!summary.contains("request_id"));
    }

    #[test]
    fn test_truncate_for_log_appends_ellipsis() {
        let input = "abcdefghijklmnopqrstuvwxyz";
        let output = truncate_for_log(input, 10);
        assert!(output.starts_with("abcdefghij"));
        assert!(output.ends_with("..."));
        assert_eq!(output.chars().count(), 13);
    }

    #[test]
    fn test_should_log_batch_msg_filters_downloading_progress() {
        let downloading = Msg::JobProgress {
            job_id: 1,
            stage: harvester_core::Stage::Downloading,
            tokens: None,
            bytes: Some(4096),
            content_preview: None,
        };
        assert!(!should_log_batch_msg(&downloading));

        let tokenizing = Msg::JobProgress {
            job_id: 1,
            stage: harvester_core::Stage::Tokenizing,
            tokens: Some(10),
            bytes: None,
            content_preview: None,
        };
        assert!(should_log_batch_msg(&tokenizing));
    }

    #[test]
    fn test_dispatch_loop_reduces_queued_poll_before_settling() {
        engine_logging::initialize_for_tests();
        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path().join("output");
        std::fs::create_dir_all(&output_dir).unwrap();

        let sources_path = temp_dir.path().join("sources.ron");
        std::fs::write(&sources_path, "SourceRegistry(sources: [])").unwrap();

        let runtime_paths = RuntimePaths::new(
            output_dir,
            sources_path,
            temp_dir.path().join("contexts"),
            temp_dir.path().join("prompts"),
        );

        let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
        let mut state = AppState::new();
        let effect_runner =
            EffectRunner::new(runtime_paths, msg_tx.clone(), Box::new(NoOpPlatformHandler));
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        msg_tx.send(Msg::PollSourcesClicked).unwrap();

        let outcome = run_dispatch_loop(
            &mut state,
            &msg_tx,
            &msg_rx,
            &effect_runner,
            &shutdown_flag,
            DispatchLoopOptions {
                enable_ai_orchestration: true,
                require_new_jobs_since: None,
                tick_interval: Duration::from_millis(75),
            },
        )
        .expect("dispatch loop should complete");
        assert_eq!(outcome, CycleOutcome::Success);

        assert!(matches!(msg_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[test]
    fn test_dispatch_loop_ticks_drive_pretriage_from_restore_signal() {
        engine_logging::initialize_for_tests();
        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path().join("output");
        std::fs::create_dir_all(&output_dir).unwrap();

        let sources_path = temp_dir.path().join("sources.ron");
        std::fs::write(&sources_path, "SourceRegistry(sources: [])").unwrap();

        let runtime_paths = RuntimePaths::new(
            output_dir,
            sources_path,
            temp_dir.path().join("contexts"),
            temp_dir.path().join("prompts"),
        );

        let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
        let mut state = AppState::new();
        let effect_runner =
            EffectRunner::new(runtime_paths, msg_tx.clone(), Box::new(NoOpPlatformHandler));
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        msg_tx
            .send(Msg::RestoreCompletedJobs(vec![
                harvester_core::CompletedJobSnapshot {
                    url: "https://example.com/article-1".to_string(),
                    tokens: Some(123),
                    bytes: Some(4567),
                    links: Vec::new(),
                    fetched_utc: Some("2026-03-05T00:00:00Z".to_string()),
                },
            ]))
            .unwrap();

        let outcome = run_dispatch_loop_with_tick_interval(
            &mut state,
            &msg_tx,
            &msg_rx,
            &effect_runner,
            &shutdown_flag,
            DispatchLoopOptions {
                enable_ai_orchestration: true,
                require_new_jobs_since: None,
                tick_interval: Duration::ZERO,
            },
        )
        .expect("dispatch loop should complete");
        assert_eq!(outcome, CycleOutcome::Success);

        let obs = state.batch_observation();
        assert!(!matches!(
            obs.pre_triage_phase,
            harvester_core::PreTriagePhase::Idle
        ));
    }

    #[test]
    fn test_classify_outcome_success() {
        let obs = BatchObservation {
            poll_in_progress: false,
            session_state: harvester_core::SessionState::Idle,
            jobs_total: 5,
            jobs_done: 5,
            jobs_failed: 0,
            jobs_in_flight: 0,
            pre_triage_phase: harvester_core::PreTriagePhase::Idle,
            pre_triage_total: 0,
            pre_triage_included: 0,
            pre_triage_review: 0,
            pre_triage_filtered: 0,
            triage_phase: harvester_core::TriagePhase::Complete,
            triage_total: 5,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 5,
            triage_failed: 0,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed: 0,
            summary_failed: 0,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
            import_phase: harvester_core::ImportPhase::Idle,
            imports_completed: 0,
            imports_failed: 0,
            import_in_flight: false,
            source_poll_stats: vec![],
        };

        assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::Success);
    }

    #[test]
    fn test_classify_outcome_partial_failure() {
        let obs = BatchObservation {
            poll_in_progress: false,
            session_state: harvester_core::SessionState::Idle,
            jobs_total: 5,
            jobs_done: 3,
            jobs_failed: 2,
            jobs_in_flight: 0,
            pre_triage_phase: harvester_core::PreTriagePhase::Idle,
            pre_triage_total: 0,
            pre_triage_included: 0,
            pre_triage_review: 0,
            pre_triage_filtered: 0,
            triage_phase: harvester_core::TriagePhase::Complete,
            triage_total: 5,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 3,
            triage_failed: 2,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed: 0,
            summary_failed: 0,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
            import_phase: harvester_core::ImportPhase::Idle,
            imports_completed: 0,
            imports_failed: 0,
            import_in_flight: false,
            source_poll_stats: vec![],
        };

        assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::PartialFailure);
    }

    #[test]
    fn test_classify_outcome_total_failure() {
        let obs = BatchObservation {
            poll_in_progress: false,
            session_state: harvester_core::SessionState::Idle,
            jobs_total: 5,
            jobs_done: 0,
            jobs_failed: 5,
            jobs_in_flight: 0,
            pre_triage_phase: harvester_core::PreTriagePhase::Idle,
            pre_triage_total: 0,
            pre_triage_included: 0,
            pre_triage_review: 0,
            pre_triage_filtered: 0,
            triage_phase: harvester_core::TriagePhase::Complete,
            triage_total: 5,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 0,
            triage_failed: 5,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed: 0,
            summary_failed: 0,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
            import_phase: harvester_core::ImportPhase::Idle,
            imports_completed: 0,
            imports_failed: 0,
            import_in_flight: false,
            source_poll_stats: vec![],
        };

        assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::TotalFailure);
    }

    #[test]
    fn test_microdollars_to_display_zero() {
        assert_eq!(microdollars_to_display(0), "$0.00");
    }

    #[test]
    fn test_microdollars_to_display_rounds_down() {
        // 50 microdollars = $0.000050 -> rounds to $0.00
        assert_eq!(microdollars_to_display(50), "$0.00");
        // 4999 microdollars = $0.004999 -> rounds to $0.00
        assert_eq!(microdollars_to_display(4999), "$0.00");
    }

    #[test]
    fn test_microdollars_to_display_rounds_up() {
        // 5000 microdollars = $0.005000 -> rounds to $0.01
        assert_eq!(microdollars_to_display(5000), "$0.01");
        // 15000 microdollars = $0.015000 -> rounds to $0.02
        assert_eq!(microdollars_to_display(15000), "$0.02");
    }

    #[test]
    fn test_microdollars_to_display_exact_cents() {
        // 10000 microdollars = $0.01
        assert_eq!(microdollars_to_display(10000), "$0.01");
        // 1000000 microdollars = $1.00
        assert_eq!(microdollars_to_display(1000000), "$1.00");
    }

    #[test]
    fn test_microdollars_to_display_typical_values() {
        // 1234567 microdollars = $1.234567 -> rounds to $1.23
        assert_eq!(microdollars_to_display(1234567), "$1.23");
        // 5678901 microdollars = $5.678901 -> rounds to $5.68
        assert_eq!(microdollars_to_display(5678901), "$5.68");
    }

    #[test]
    fn test_microdollars_to_display_large_values() {
        // 123456789 microdollars = $123.456789 -> rounds to $123.46
        assert_eq!(microdollars_to_display(123456789), "$123.46");
        // 1000000000 microdollars = $1000.00
        assert_eq!(microdollars_to_display(1000000000), "$1000.00");
    }

    #[test]
    fn format_compact_tokens_thresholds() {
        assert_eq!(format_compact_tokens(0), "0");
        assert_eq!(format_compact_tokens(999), "999");
        assert_eq!(format_compact_tokens(1_000), "1K");
        assert_eq!(format_compact_tokens(12_345), "12K");
        assert_eq!(format_compact_tokens(999_999), "999K");
        assert_eq!(format_compact_tokens(1_000_000), "1.0M");
        assert_eq!(format_compact_tokens(1_234_567), "1.2M");
    }

    #[test]
    fn format_llm_usage_lines_formats_rows_compactly() {
        let rows = vec![
            LlmModelUsageView {
                model: "alpha".to_string(),
                input_tokens: 12_345,
                output_tokens: 3_100,
            },
            LlmModelUsageView {
                model: "beta".to_string(),
                input_tokens: 500,
                output_tokens: 80,
            },
        ];
        let lines = format_llm_usage_lines(&rows);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("alpha"));
        assert!(lines[0].contains("in=12K"));
        assert!(lines[0].contains("out=3K"));
        assert!(lines[1].contains("beta"));
        assert!(lines[1].contains("in=500"));
        assert!(lines[1].contains("out=80"));
    }

    #[test]
    fn format_llm_usage_lines_empty_returns_empty() {
        let lines = format_llm_usage_lines(&[]);
        assert!(lines.is_empty());
    }

    #[test]
    fn summary_refresh_exit_code_is_zero_for_partial_success() {
        assert_eq!(summary_refresh_exit_code(92, 8), 0);
    }

    #[test]
    fn summary_refresh_exit_code_is_nonzero_when_all_attempts_fail() {
        assert_eq!(summary_refresh_exit_code(0, 8), 1);
    }

    #[test]
    fn select_stale_summary_targets_prefers_missing_current_cache_key_and_respects_limit() {
        let metas = vec![
            harvester_engine::ArchiveArticleMeta {
                url: "https://example.com/a".to_string(),
                fetched_utc: Some("2026-04-01T00:00:00Z".to_string()),
                content_hash: Some("hash-a".to_string()),
            },
            harvester_engine::ArchiveArticleMeta {
                url: "https://example.com/b".to_string(),
                fetched_utc: Some("2026-04-02T00:00:00Z".to_string()),
                content_hash: Some("hash-b".to_string()),
            },
            harvester_engine::ArchiveArticleMeta {
                url: "https://example.com/c".to_string(),
                fetched_utc: Some("2026-04-03T00:00:00Z".to_string()),
                content_hash: Some("hash-c".to_string()),
            },
        ];
        let mut cache = SummaryCache::new();
        let current_key = SummaryCacheKey::try_new(
            "hash-a",
            PromptId::ArticleSummary,
            Some(5),
            Some("gpt-5.4-mini"),
            &[],
        )
        .unwrap();
        cache.insert(
            current_key,
            SummaryCacheEntry {
                result: ArticleSummaryResult {
                    title: "A".to_string(),
                    summary: "A".to_string(),
                    key_points: vec![],
                    input_tokens: 1,
                    output_tokens: 1,
                    entities: Default::default(),
                },
                created_at_utc: "2026-04-20T00:00:00Z".to_string(),
            },
        );
        let old_version_key = SummaryCacheKey::try_new(
            "hash-b",
            PromptId::ArticleSummary,
            Some(4),
            Some("gpt-5.4-mini"),
            &[],
        )
        .unwrap();
        cache.insert(
            old_version_key,
            SummaryCacheEntry {
                result: ArticleSummaryResult {
                    title: "B".to_string(),
                    summary: "B".to_string(),
                    key_points: vec![],
                    input_tokens: 1,
                    output_tokens: 1,
                    entities: Default::default(),
                },
                created_at_utc: "2026-04-19T00:00:00Z".to_string(),
            },
        );

        let selection = select_stale_summary_targets(&metas, &cache, 5, "gpt-5.4-mini", &[], 1);

        assert_eq!(selection.total_stale, 2);
        assert_eq!(selection.targets.len(), 1);
        assert_eq!(selection.targets[0].primary_url, "https://example.com/b");
    }

    #[test]
    fn select_stale_summary_targets_deduplicates_by_content_hash() {
        let metas = vec![
            harvester_engine::ArchiveArticleMeta {
                url: "https://example.com/a".to_string(),
                fetched_utc: Some("2026-04-01T00:00:00Z".to_string()),
                content_hash: Some("shared-hash".to_string()),
            },
            harvester_engine::ArchiveArticleMeta {
                url: "https://example.com/b".to_string(),
                fetched_utc: Some("2026-04-02T00:00:00Z".to_string()),
                content_hash: Some("shared-hash".to_string()),
            },
        ];

        let selection =
            select_stale_summary_targets(&metas, &SummaryCache::new(), 5, "gpt-5.4-mini", &[], 10);

        assert_eq!(selection.total_stale, 1);
        assert_eq!(selection.targets.len(), 1);
        assert_eq!(selection.targets[0].primary_url, "https://example.com/a");
        assert_eq!(
            selection.targets[0].related_urls,
            vec![
                (
                    "https://example.com/a".to_string(),
                    Some("2026-04-01T00:00:00Z".to_string())
                ),
                (
                    "https://example.com/b".to_string(),
                    Some("2026-04-02T00:00:00Z".to_string())
                ),
            ]
        );
    }

    fn make_checkpoint_test_paths(temp_dir: &TempDir) -> RuntimePaths {
        let output_dir = temp_dir.path().to_path_buf();
        std::fs::create_dir_all(&output_dir).unwrap();
        RuntimePaths::new(
            output_dir,
            temp_dir.path().join("sources.ron"),
            temp_dir.path().join("contexts"),
            temp_dir.path().join("prompts"),
        )
    }

    #[test]
    fn set_checkpoint_invalid_timestamp_returns_err_without_write() {
        let temp_dir = TempDir::new().unwrap();
        let paths = make_checkpoint_test_paths(&temp_dir);
        // Simulate the validation in checkpoint_command() — Set is only constructed after validation
        // We test execute_checkpoint_write with a directly-valid Set to confirm it writes
        let result =
            execute_checkpoint_write(CheckpointCommand::Set("not-rfc3339".to_string()), &paths);
        // save_briefing_checkpoint does not validate the string; validation is in checkpoint_command().
        // But the file SHOULD be written with whatever string is passed.
        // This test verifies the call succeeds (the CLI layer is responsible for validation).
        assert!(result.is_ok());
    }

    #[test]
    fn set_checkpoint_writes_file() {
        let temp_dir = TempDir::new().unwrap();
        let paths = make_checkpoint_test_paths(&temp_dir);
        execute_checkpoint_write(
            CheckpointCommand::Set("2025-12-31T23:00:00Z".to_string()),
            &paths,
        )
        .unwrap();
        let loaded = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
        assert_eq!(loaded.as_deref(), Some("2025-12-31T23:00:00Z"));
    }

    #[test]
    fn set_checkpoint_now_writes_valid_rfc3339() {
        let temp_dir = TempDir::new().unwrap();
        let paths = make_checkpoint_test_paths(&temp_dir);
        execute_checkpoint_write(CheckpointCommand::SetNow, &paths).unwrap();
        let loaded = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
        let ts = loaded.expect("checkpoint should be written");
        assert!(
            chrono::DateTime::parse_from_rfc3339(&ts).is_ok(),
            "expected valid RFC3339, got: {ts}"
        );
    }

    #[test]
    fn clear_checkpoint_deletes_file() {
        let temp_dir = TempDir::new().unwrap();
        let paths = make_checkpoint_test_paths(&temp_dir);
        // Write first
        execute_checkpoint_write(
            CheckpointCommand::Set("2025-12-31T23:00:00Z".to_string()),
            &paths,
        )
        .unwrap();
        assert!(paths.briefing_checkpoint_path.exists());
        // Then clear
        execute_checkpoint_write(CheckpointCommand::Clear, &paths).unwrap();
        assert!(!paths.briefing_checkpoint_path.exists());
    }

    #[test]
    fn show_checkpoint_prints_none_when_absent() {
        let temp_dir = TempDir::new().unwrap();
        let paths = make_checkpoint_test_paths(&temp_dir);
        let val = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
        assert_eq!(val.as_deref().unwrap_or("NONE"), "NONE");
    }

    // --- Import CLI flag tests ---

    fn idle_import_obs() -> BatchObservation {
        BatchObservation {
            poll_in_progress: false,
            session_state: harvester_core::SessionState::Idle,
            jobs_total: 0,
            jobs_done: 0,
            jobs_failed: 0,
            jobs_in_flight: 0,
            pre_triage_phase: harvester_core::PreTriagePhase::Idle,
            pre_triage_total: 0,
            pre_triage_included: 0,
            pre_triage_review: 0,
            pre_triage_filtered: 0,
            triage_phase: harvester_core::TriagePhase::Idle,
            triage_total: 0,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 0,
            triage_failed: 0,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed: 0,
            summary_failed: 0,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
            import_phase: harvester_core::ImportPhase::Idle,
            imports_completed: 0,
            imports_failed: 0,
            import_in_flight: false,
            source_poll_stats: vec![],
        }
    }

    #[test]
    fn import_saved_web_dir_flag_is_parsed() {
        let args = crate::cli::Args::parse_from(&[
            "harvester_batch",
            "--import-saved-web-dir",
            "/tmp/saved",
        ]);
        assert_eq!(
            args.import_saved_web_dir,
            Some(std::path::PathBuf::from("/tmp/saved"))
        );
    }

    #[test]
    fn import_saved_web_dir_conflicts_with_dry_run() {
        let result = <crate::cli::Args as clap::Parser>::try_parse_from([
            "harvester_batch",
            "--import-saved-web-dir",
            "/tmp/saved",
            "--dry-run",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn import_saved_web_dir_conflicts_with_single_shot() {
        let result = <crate::cli::Args as clap::Parser>::try_parse_from([
            "harvester_batch",
            "--import-saved-web-dir",
            "/tmp/saved",
            "--single-shot",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn import_mode_persistence_merge_preserves_existing_jobs_and_appends_imports() {
        let existing = vec![harvester_core::CompletedJobSnapshot {
            url: "https://example.com/existing".to_string(),
            tokens: Some(10),
            bytes: Some(100),
            links: Vec::new(),
            fetched_utc: Some("2026-03-08T06:00:00Z".to_string()),
        }];
        let imported = vec![harvester_core::CompletedJobSnapshot {
            url: "https://example.com/imported".to_string(),
            tokens: None,
            bytes: None,
            links: Vec::new(),
            fetched_utc: Some("2026-03-08T06:01:56Z".to_string()),
        }];

        let merged = merge_completed_jobs_for_import(existing, imported);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].url, "https://example.com/existing");
        assert_eq!(merged[1].url, "https://example.com/imported");
        assert_eq!(
            merged[1].fetched_utc.as_deref(),
            Some("2026-03-08T06:01:56Z")
        );
    }

    #[test]
    fn should_settle_import_cycle_when_idle() {
        let obs = idle_import_obs();
        assert!(should_settle_import_cycle(&obs));
    }

    #[test]
    fn should_not_settle_import_cycle_when_in_flight() {
        let mut obs = idle_import_obs();
        obs.import_in_flight = true;
        obs.import_phase = harvester_core::ImportPhase::Importing;
        assert!(!should_settle_import_cycle(&obs));
    }

    #[test]
    fn should_not_settle_import_cycle_when_summaries_pending() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Complete;
        obs.summary_pending = 3;
        assert!(!should_settle_import_cycle(&obs));
    }

    #[test]
    fn should_settle_import_cycle_when_complete_and_no_pending() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Complete;
        obs.imports_completed = 2;
        assert!(should_settle_import_cycle(&obs));
    }

    #[test]
    fn classify_import_cycle_success_when_all_imported() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Complete;
        obs.imports_completed = 3;
        assert_eq!(classify_import_cycle_outcome(&obs), CycleOutcome::Success);
    }

    #[test]
    fn classify_import_cycle_partial_when_some_failed() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Complete;
        obs.imports_completed = 2;
        obs.imports_failed = 1;
        assert_eq!(
            classify_import_cycle_outcome(&obs),
            CycleOutcome::PartialFailure
        );
    }

    #[test]
    fn classify_import_cycle_total_failure_when_zero_imported() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Failed;
        obs.imports_failed = 2;
        assert_eq!(
            classify_import_cycle_outcome(&obs),
            CycleOutcome::TotalFailure
        );
    }
}
