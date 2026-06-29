use super::summary_cache_support::{
    build_summary_cache_key, context_hash_for_log, log_summary_cache_run_summary,
    log_summary_cache_warmup_if_needed, short_hash, summary_cache_key_error_reason,
};
use crate::briefing::{BriefingPhase, BriefingSession};
use crate::state::BriefingGenerateReadiness;
use crate::tabs::AppTab;
use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::llm::prompt::PromptId;

fn briefing_ready_to_start(state: &AppState) -> bool {
    state.briefing_ai_available() && state.briefing().can_start()
}

fn briefing_ready_to_generate(state: &AppState) -> bool {
    state.briefing_ai_available() && state.briefing().can_generate()
}

fn briefing_stream_hydrated(state: &AppState) -> bool {
    state.prompt_contexts_loaded() && state.prompt_templates_loaded() && state.llm_metadata_loaded()
}

fn briefing_stream_hydration_effects(state: &AppState) -> Vec<Effect> {
    let mut effects = Vec::new();
    if !state.prompt_contexts_loaded() {
        effects.push(Effect::LoadPromptContexts);
    }
    if !state.prompt_templates_loaded() {
        effects.push(Effect::LoadPromptTemplateFiles);
    }
    if state.prompt_templates_loaded() && !state.llm_metadata_loaded() {
        effects.push(Effect::LoadLlmMetadata);
    }
    effects
}

fn begin_briefing_article_load(
    state: &mut AppState,
    ordered_urls: Vec<String>,
    skip_aggregate: bool,
) -> Vec<Effect> {
    if skip_aggregate {
        state.request_summary_preparation();
    } else {
        state.request_briefing_orchestration();
    }
    state.start_summary_cache_run();
    state.set_briefing(BriefingSession::new_loading(None));
    snapshot_briefing_coverage_window(state);
    state.revert_preview_to_briefing();
    let since_utc = state.briefing_since_utc();
    vec![
        Effect::LoadPromptContexts,
        Effect::LoadPromptTemplateFiles,
        Effect::LoadLlmMetadata,
        Effect::LoadArticlesForBriefing {
            ordered_urls,
            since_utc,
        },
    ]
}

fn fail_generate(state: &mut AppState, reason: &str) -> Vec<Effect> {
    engine_warn!("[briefing-triage] generate blocked: {}", reason);
    state.briefing_mut().fail(reason.to_string());
    state.clear_briefing_orchestration();
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_generate_clicked(state: &mut AppState) -> Vec<Effect> {
    if !briefing_ready_to_generate(state) {
        return Vec::new();
    }
    state.select_tab(AppTab::Briefing);
    match state.briefing_generate_readiness() {
        BriefingGenerateReadiness::Ready { .. } => {}
        BriefingGenerateReadiness::TriageOrCorpusNotReady => {
            return fail_generate(
                state,
                "No completed triage. Run triage before generating a briefing.",
            );
        }
        BriefingGenerateReadiness::SummariesNotSettled => {
            return fail_generate(state, "Summarize articles before generating a briefing.");
        }
        BriefingGenerateReadiness::SignalScoringInProgress => {
            return fail_generate(
                state,
                "Signal scoring still in progress. Wait for it to finish.",
            );
        }
    };

    let snapshot = state.build_briefing_snapshot_now();
    if snapshot.included_count == 0 {
        return fail_generate(state, "No article summaries available for the briefing.");
    }

    state.briefing_mut().start_stream(
        snapshot.text,
        snapshot.coverage_window_label,
        snapshot.included_count,
        snapshot.skipped_count,
        snapshot.dropped_count,
        snapshot.truncated,
    );
    state
        .briefing_mut()
        .set_phase(BriefingPhase::GeneratingBriefing);
    state.revert_preview_to_briefing();
    if snapshot.truncated {
        engine_warn!(
            "[briefing-stream] snapshot truncated: dropped={} budget_bytes={}",
            snapshot.dropped_count,
            crate::BRIEFING_SNAPSHOT_BUDGET_BYTES
        );
    }
    engine_info!(
        "[briefing-stream] generate frozen snapshot epoch={} included={} skipped={} dropped={} truncated={}",
        state.briefing().stream_epoch(),
        snapshot.included_count,
        snapshot.skipped_count,
        snapshot.dropped_count,
        snapshot.truncated
    );
    state.mark_dirty();

    if !briefing_stream_hydrated(state) {
        state.briefing_mut().defer_exec_dispatch();
        return briefing_stream_hydration_effects(state);
    }

    vec![dispatch_executive_summary_call(state)]
}

fn dispatch_executive_summary_call(state: &mut AppState) -> Effect {
    let snapshot = state
        .briefing()
        .summaries_snapshot()
        .map(str::to_owned)
        .unwrap_or_default();
    let coverage = state
        .briefing()
        .coverage_window_label()
        .map(str::to_owned)
        .unwrap_or_default();

    let request_id = state.allocate_next_llm_request_id();
    state.record_pending_llm_request(request_id, PromptId::BriefingExecutiveSummary);
    state.briefing_mut().set_briefing_request_id(request_id);

    let context = state
        .context_for(PromptId::BriefingExecutiveSummary)
        .to_vec();
    state.mark_dirty();
    Effect::RequestLlmCompletion {
        request_id,
        prompt_id: PromptId::BriefingExecutiveSummary,
        prompt_version: None,
        model_override: None,
        input_content: snapshot,
        context,
        template_override: None,
        extra_template_vars: vec![("briefing_time_window".to_string(), coverage)],
    }
}

pub(super) fn resume_deferred_exec_dispatch(state: &mut AppState) -> Vec<Effect> {
    if !state.briefing().exec_dispatch_deferred() || !briefing_stream_hydrated(state) {
        return Vec::new();
    }
    state.briefing_mut().take_exec_dispatch_deferred();
    vec![dispatch_executive_summary_call(state)]
}

pub(super) fn handle_next_item_clicked(state: &mut AppState) -> Vec<Effect> {
    if !state.briefing().next_item_enabled() {
        return Vec::new();
    }
    let Some(snapshot) = state.briefing().summaries_snapshot().map(str::to_owned) else {
        return Vec::new();
    };
    let already_shown = state.briefing().already_shown_headlines();
    let coverage = state
        .briefing()
        .coverage_window_label()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            crate::briefing::format_briefing_time_window_label(state.briefing_since_utc())
        });

    let request_id = state.allocate_next_llm_request_id();
    state.record_pending_llm_request(request_id, PromptId::BriefingNextItem);
    state.briefing_mut().set_next_item_request_id(request_id);

    let context = state.context_for(PromptId::BriefingNextItem).to_vec();
    state.mark_dirty();
    vec![Effect::RequestLlmCompletion {
        request_id,
        prompt_id: PromptId::BriefingNextItem,
        prompt_version: None,
        model_override: None,
        input_content: snapshot,
        context,
        template_override: None,
        extra_template_vars: vec![
            ("already_shown".to_string(), already_shown),
            ("briefing_time_window".to_string(), coverage),
        ],
    }]
}

pub(super) fn handle_prepare_summaries_clicked(state: &mut AppState) -> Vec<Effect> {
    if !briefing_ready_to_start(state) {
        return Vec::new();
    }
    if !state.summaries_can_start() {
        engine_info!("[briefing-triage] summary-prep blocked: base corpus not ready");
        return Vec::new();
    }
    let ordered_urls = state.archive_corpus().ordered_urls().to_vec();
    engine_info!(
        "[briefing-triage] summary-prep base-corpus count={}",
        ordered_urls.len()
    );
    begin_briefing_article_load(state, ordered_urls, true)
}

pub(super) fn handle_history_loaded(
    state: &mut AppState,
    entries: Vec<crate::briefing::BriefingHistoryEntry>,
) -> Vec<Effect> {
    state.set_briefing_history(entries);
    Vec::new()
}

pub(super) fn handle_checkpoint_loaded(
    state: &mut AppState,
    since_utc: Option<String>,
) -> Vec<Effect> {
    let parsed = since_utc
        .as_deref()
        .and_then(|s| match chrono::DateTime::parse_from_rfc3339(s) {
            Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
            Err(e) => {
                engine_warn!(
                    "[briefing-checkpoint] file contained invalid RFC3339: {}",
                    e
                );
                None
            }
        });
    state.set_briefing_since_utc(parsed);
    state.clear_briefing_checkpoint_save_tracking();
    vec![]
}

pub(super) fn handle_checkpoint_save_succeeded(state: &mut AppState, save_id: u64) -> Vec<Effect> {
    if !state.finish_briefing_checkpoint_save_success(save_id) {
        return vec![];
    }
    engine_info!("[briefing-checkpoint] save succeeded save_id={}", save_id);
    state.mark_dirty();
    vec![]
}

pub(super) fn handle_checkpoint_save_failed(
    state: &mut AppState,
    save_id: u64,
    reason: String,
) -> Vec<Effect> {
    if !state.finish_briefing_checkpoint_save_failure(save_id, &reason) {
        return vec![];
    }
    engine_warn!(
        "[briefing-checkpoint] save failed save_id={} reason={}; reverted in-memory checkpoint",
        save_id,
        reason
    );
    state.mark_dirty();
    vec![]
}

pub(super) fn handle_checkpoint_set(state: &mut AppState, since: Option<String>) -> Vec<Effect> {
    let parsed = since
        .as_deref()
        .and_then(|s| match chrono::DateTime::parse_from_rfc3339(s) {
            Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
            Err(_) => {
                engine_warn!("[briefing-checkpoint] ignoring invalid timestamp: {}", s);
                None
            }
        });
    // If caller passed Some(bad string), treat as no-op
    if since.is_some() && parsed.is_none() {
        return vec![];
    }
    let save_id = state.begin_briefing_checkpoint_save(parsed);
    state.mark_dirty();
    vec![Effect::SaveBriefingCheckpoint {
        save_id,
        since_utc: parsed,
    }]
}

pub(super) fn handle_articles_loaded(
    state: &mut AppState,
    articles: Vec<crate::briefing::LoadedArticle>,
    collection_text: String,
) -> Vec<Effect> {
    if articles.is_empty() {
        state
            .briefing_mut()
            .fail("no completed articles found".to_string());
        log_summary_cache_run_summary(state);
        state.mark_dirty();
        let cache = state.summary_cache().clone();
        return vec![Effect::PersistSummaryCache { cache }];
    }
    state.briefing_mut().set_articles(articles, collection_text);
    state.briefing_mut().transition_to_summarizing();
    state.mark_dirty();
    let mut effects = Vec::new();
    try_start_briefing_with_metadata(state, &mut effects);
    effects
}

pub(super) fn handle_articles_load_failed(state: &mut AppState, reason: String) -> Vec<Effect> {
    state.briefing_mut().fail(reason);
    log_summary_cache_run_summary(state);
    state.mark_dirty();
    let cache = state.summary_cache().clone();
    vec![Effect::PersistSummaryCache { cache }]
}

pub(super) fn dispatch_next_briefing_step(state: &mut AppState, effects: &mut Vec<Effect>) {
    log_summary_cache_warmup_if_needed(state);

    let limit = state.summary_max_in_flight();

    loop {
        // Stop filling if we've reached the concurrency limit.
        if state.briefing().in_progress_count() >= limit {
            break;
        }

        let Some(next_idx) = state.briefing().next_pending_index() else {
            break;
        };

        let article = &state.briefing().articles()[next_idx];
        let prepared_text = article.prepared_text.clone();
        let content_hash = article.content_hash.clone();
        let content_hash_short = short_hash(&content_hash);
        let context = state.context_for(PromptId::ArticleSummary).to_vec();
        let context_hash_value = context_hash_for_log(&context);
        let metadata = state.summary_cache_metadata();
        let version_display = metadata
            .map(|(version, _)| version.to_string())
            .unwrap_or_else(|| "<none>".to_string());
        let model_display = metadata
            .map(|(_, model)| model.to_string())
            .unwrap_or_else(|| "<none>".to_string());

        match build_summary_cache_key(
            &content_hash,
            PromptId::ArticleSummary,
            metadata.map(|(version, _)| version),
            metadata.map(|(_, model)| model),
            &context,
        ) {
            Ok(key) => {
                state
                    .briefing_mut()
                    .set_article_cache_key(next_idx, Some(key.clone()));
                if let Some(cached_result) = state.try_reuse_summary(&key) {
                    let article_entities = cached_result.entities.clone();
                    let url = state.briefing().articles()[next_idx].url.clone();
                    let fetched_utc = state.briefing().articles()[next_idx].fetched_utc.clone();
                    let result = cached_result.clone();
                    state.record_summary_cache_hit();
                    engine_info!(
                        "[summary-cache] article={} decision=hit reason=cache-hit prompt_version={} model_id={} context_hash={} content_hash_short={}",
                        next_idx,
                        version_display,
                        model_display,
                        &context_hash_value,
                        content_hash_short
                    );
                    state.briefing_mut().complete_article(next_idx, result);
                    state.briefing_mut().set_article_cache_key(next_idx, None);
                    state.refresh_selected_preview();
                    state.mark_dirty();
                    effects.push(Effect::UpsertEntityIndexEntry {
                        url,
                        fetched_utc,
                        content_hash: Some(content_hash.clone()),
                        summary_entities: Some(article_entities),
                        themes: None,
                    });
                    let article_url = state.briefing().articles()[next_idx].url.clone();
                    let _ =
                        crate::update::signal_candidate::try_enqueue(state, &article_url, effects);
                    // Cache hit: slot not consumed, continue filling.
                    continue;
                }

                state.record_summary_cache_miss();
                engine_info!(
                    "[summary-cache] article={} decision=miss reason=cache-miss prompt_version={} model_id={} context_hash={} content_hash_short={}",
                    next_idx,
                    version_display,
                    model_display,
                    &context_hash_value,
                    content_hash_short
                );
                let request_id = state.allocate_next_llm_request_id();
                state.record_pending_llm_request(request_id, PromptId::ArticleSummary);
                state.briefing_mut().start_article(next_idx, request_id);
                engine_info!(
                    "[llm-concurrency] summary dispatch request_id={} article={} in_flight={} limit={}",
                    request_id,
                    next_idx,
                    state.briefing().in_progress_count(),
                    limit
                );
                effects.push(Effect::RequestLlmCompletion {
                    request_id,
                    prompt_id: PromptId::ArticleSummary,
                    prompt_version: None,
                    model_override: None,
                    input_content: prepared_text,
                    context,
                    template_override: None,
                    extra_template_vars: vec![],
                });
                state.mark_dirty();
                // Live request: continue loop to fill remaining slots.
                continue;
            }
            Err(err) => {
                state.briefing_mut().set_article_cache_key(next_idx, None);
                state.record_summary_cache_key_unavailable();
                let reason = summary_cache_key_error_reason(&err);
                engine_info!(
                    "[summary-cache] article={} decision=key_unavailable reason={} prompt_version={} model_id={} context_hash={} content_hash_short={}",
                    next_idx,
                    reason,
                    version_display,
                    model_display,
                    &context_hash_value,
                    content_hash_short
                );
                let request_id = state.allocate_next_llm_request_id();
                state.record_pending_llm_request(request_id, PromptId::ArticleSummary);
                state.briefing_mut().start_article(next_idx, request_id);
                engine_info!(
                    "[llm-concurrency] summary dispatch (no-cache-key) request_id={} article={} in_flight={} limit={}",
                    request_id,
                    next_idx,
                    state.briefing().in_progress_count(),
                    limit
                );
                effects.push(Effect::RequestLlmCompletion {
                    request_id,
                    prompt_id: PromptId::ArticleSummary,
                    prompt_version: None,
                    model_override: None,
                    input_content: prepared_text,
                    context,
                    template_override: None,
                    extra_template_vars: vec![],
                });
                state.mark_dirty();
                // Live request: continue loop to fill remaining slots.
                continue;
            }
        }
    }

    // Gate aggregate briefing on ALL articles settled (no pending, no in-progress).
    if state.briefing().pending_count() > 0 || state.briefing().in_progress_count() > 0 {
        return;
    }

    if state.briefing().completed_summary_count() == 0 {
        state
            .briefing_mut()
            .fail("all article summaries failed".to_string());
        state.mark_dirty();
        log_summary_cache_run_summary(state);
        effects.push(Effect::PersistSummaryCache {
            cache: state.summary_cache().clone(),
        });
        return;
    }

    if state.briefing_orchestration_skip_aggregate() {
        state.briefing_mut().complete_without_briefing();
        state.revert_preview_to_briefing();
        state.clear_briefing_orchestration();
        state.mark_dirty();
        log_summary_cache_run_summary(state);
        effects.push(Effect::PersistSummaryCache {
            cache: state.summary_cache().clone(),
        });
        return;
    }

    let collection_text = match state.briefing().collection_text() {
        Some(text) => text.to_string(),
        None => {
            state
                .briefing_mut()
                .fail("missing briefing collection".to_string());
            state.mark_dirty();
            log_summary_cache_run_summary(state);
            effects.push(Effect::PersistSummaryCache {
                cache: state.summary_cache().clone(),
            });
            return;
        }
    };
    let request_id = state.allocate_next_llm_request_id();
    state.record_pending_llm_request(request_id, PromptId::AggregateBriefing);
    state.briefing_mut().set_briefing_request_id(request_id);

    let context = state.context_for(PromptId::AggregateBriefing).to_vec();

    let previous_briefings =
        crate::briefing::format_previous_briefings_block(state.briefing_history());
    let briefing_time_window = state
        .briefing()
        .coverage_window_label()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            crate::briefing::format_briefing_time_window_label(state.briefing_since_utc())
        });

    effects.push(Effect::RequestLlmCompletion {
        request_id,
        prompt_id: PromptId::AggregateBriefing,
        prompt_version: None,
        model_override: None,
        input_content: collection_text,
        context,
        template_override: None,
        extra_template_vars: vec![
            ("previous_briefings".to_string(), previous_briefings),
            ("briefing_time_window".to_string(), briefing_time_window),
        ],
    });
    state.mark_dirty();
}

fn snapshot_briefing_coverage_window(state: &mut AppState) {
    let label = crate::briefing::format_briefing_time_window_label(state.briefing_since_utc());
    state.briefing_mut().set_coverage_window_label(label);
}

pub(super) fn try_start_briefing_with_metadata(state: &mut AppState, effects: &mut Vec<Effect>) {
    if !state.is_briefing_metadata_ready() {
        return;
    }
    if matches!(state.briefing().phase(), BriefingPhase::Summarizing) {
        dispatch_next_briefing_step(state, effects);
    }
}
