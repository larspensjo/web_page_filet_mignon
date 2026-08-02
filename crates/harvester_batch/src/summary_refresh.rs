use crate::cli::Args;
use crate::progress::ProgressReporter;
use crate::runner::{exit_code_with_shutdown, microdollars_to_display};
use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{ArticleSummaryResult, SummaryCache, SummaryCacheEntry, SummaryCacheKey};
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
    load_entity_index, load_prompt_templates, load_summary_cache, persist_summary_cache,
    save_entity_index, upsert_entry, EntityIndexPatch, RuntimePaths,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::Duration;

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

pub(crate) fn run_refresh_stale_summaries_mode(
    paths: &RuntimePaths,
    args: &Args,
    shutdown_flag: &Arc<AtomicBool>,
) -> Result<i32, String> {
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
            if shutdown_flag.load(Ordering::Relaxed) {
                engine_info!(
                    "[summary-refresh] Shutdown requested; stopping new LLM request dispatch"
                );
                break;
            }
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
        let interrupted_during_dispatch = shutdown_flag.load(Ordering::Relaxed);
        if !interrupted_during_dispatch && !targets_by_url.is_empty() {
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
        if pending.is_empty() && interrupted_during_dispatch {
            report.status = "interrupted".to_string();
            report.finished_at_utc = Utc::now().to_rfc3339();
            return Ok((report, 130));
        }
        if pending.is_empty() {
            return Err("no stale summaries could be dispatched".to_string());
        }

        let event_rx = llm_handle.event_receiver();
        let mut entity_index = load_entity_index(&paths.entity_index_path);

        while !pending.is_empty() && !shutdown_flag.load(Ordering::Relaxed) {
            let event = {
                let receiver = event_rx.lock().unwrap();
                receiver.recv_timeout(Duration::from_millis(100))
            };
            let event = match event {
                Ok(event) => event,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("summary refresh worker stopped unexpectedly".to_string())
                }
            };

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
        let interrupted = shutdown_flag.load(Ordering::Relaxed);
        report.status = if interrupted {
            "interrupted".to_string()
        } else {
            summary_refresh_status_label(report.succeeded, report.failed).to_string()
        };
        report.finished_at_utc = Utc::now().to_rfc3339();

        engine_info!(
            "[summary-refresh] completed successes={} failures={}",
            report.succeeded,
            report.failed
        );
        let exit_code = if interrupted {
            130
        } else {
            summary_refresh_exit_code(report.succeeded, report.failed)
        };
        Ok((report, exit_code))
    })();

    let usage_totals = if shutdown_flag.load(Ordering::Relaxed) {
        None
    } else {
        llm_handle.usage_totals()
    };
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
            Ok(exit_code_with_shutdown(
                exit_code,
                shutdown_flag.load(Ordering::Relaxed),
            ))
        }
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
