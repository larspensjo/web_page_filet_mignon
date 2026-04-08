use super::summary_cache_support::{
    build_summary_cache_key, context_hash_for_log, log_summary_cache_run_summary,
    log_summary_cache_warmup_if_needed, short_hash, summary_cache_key_error_reason,
};
use crate::briefing::{BriefingPhase, BriefingSession, CorpusFingerprint};
use crate::pre_triage_filter::{PreTriagePolicy, PreTriageSession};
use crate::tabs::AppTab;
use crate::triage::TriagePhase;
use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::llm::prompt::PromptId;

pub(super) fn handle_generate_clicked(state: &mut AppState) -> Vec<Effect> {
    if !state.briefing_ai_available() {
        return Vec::new();
    }
    if !state.briefing().can_start() {
        return Vec::new();
    }
    if state.triage().is_active() {
        engine_info!("[briefing-triage] interleave blocked: triage in progress");
        return Vec::new();
    }
    state.select_tab(AppTab::Briefing);
    state.request_briefing_orchestration();
    state.set_briefing(BriefingSession::new_waiting_for_triage(None));
    snapshot_briefing_coverage_window(state);
    state.revert_preview_to_briefing();
    engine_info!("[briefing-triage] generate requested");
    // INTENTIONAL EXCEPTION: briefing article loading does NOT use the shared
    // working-corpus selector. Briefing starts from all completed jobs, then
    // runs a fresh triage pass with TriageSelectionPolicy cutoff semantics to
    // select which articles are worth summarizing. The shared selector's
    // pre-triage results (which use a different filter set) are not appropriate
    // here — briefing wants priority-ranked triage results, not just "ready to
    // triage" candidates.
    let ordered_urls = state.ordered_completed_job_urls_snapshot();
    let since_utc = state.briefing_since_utc();
    vec![
        Effect::LoadPromptContexts,
        Effect::LoadPromptTemplateFiles,
        Effect::LoadLlmMetadata,
        Effect::LoadArticlesForBriefingPrereq {
            ordered_urls,
            since_utc,
        },
    ]
}

pub(super) fn handle_prepare_summaries_clicked(state: &mut AppState) -> Vec<Effect> {
    if !state.briefing().can_start() {
        return Vec::new();
    }
    if state.triage().is_active() {
        engine_info!("[briefing-triage] summary-prep blocked: triage in progress");
        return Vec::new();
    }
    state.request_summary_preparation();
    state.set_briefing(BriefingSession::new_waiting_for_triage(None));
    snapshot_briefing_coverage_window(state);
    state.revert_preview_to_briefing();
    engine_info!("[briefing-triage] summary-prep requested");
    // INTENTIONAL EXCEPTION: same rationale as GenerateBriefingClicked —
    // briefing uses all completed jobs as its starting feed, then applies
    // TriageSelectionPolicy cutoff semantics, bypassing the shared selector.
    let ordered_urls = state.ordered_completed_job_urls_snapshot();
    let since_utc = state.briefing_since_utc();
    vec![
        Effect::LoadPromptContexts,
        Effect::LoadPromptTemplateFiles,
        Effect::LoadLlmMetadata,
        Effect::LoadArticlesForBriefingPrereq {
            ordered_urls,
            since_utc,
        },
    ]
}

pub(super) fn handle_prereq_articles_loaded(
    state: &mut AppState,
    articles: Vec<crate::briefing::LoadedArticle>,
) -> Vec<Effect> {
    engine_info!("[briefing-triage] prereq loaded count={}", articles.len());
    // INTENTIONAL EXCEPTION: briefing builds its own ephemeral pre-triage pass
    // from the freshly loaded prerequisite articles (all completed jobs). This
    // is not the shared working-corpus pre-triage session — it is a transient
    // filter step that feeds the briefing-owned triage run (with
    // TriageSelectionPolicy cutoff semantics applied afterwards). The shared
    // selector's ReadyToTriage session is irrelevant here because briefing
    // needs to score articles for priority, not just include ready ones.
    let policy = PreTriagePolicy::default();
    let pre_triage = PreTriageSession::load_articles(articles, &policy);
    let filtered_articles = pre_triage.resolved_included_articles();
    if filtered_articles.is_empty() {
        state
            .briefing_mut()
            .fail("No articles available after pre-triage filters".to_string());
        state.clear_briefing_orchestration();
        state.mark_dirty();
        return Vec::new();
    }
    let prereq_fingerprint = CorpusFingerprint::from_articles(&filtered_articles);
    state.store_briefing_prereq_articles(filtered_articles.clone());
    let triage_reusable = matches!(state.triage().phase(), TriagePhase::Complete)
        && CorpusFingerprint::from_triage_results(state.triage()) == prereq_fingerprint;
    let mut effects = Vec::new();
    if triage_reusable {
        engine_info!("[briefing-triage] triage reused");
        on_triage_settled_for_briefing(state, &mut effects);
    } else {
        engine_info!("[briefing-triage] triage rerun");
        state.triage_mut().reset_with_articles(filtered_articles);
        state.triage_mut().transition_to_triaging();
        state.start_triage_cache_run();
        state.mark_triage_metadata_ready();
        super::triage::dispatch_next_triage_step(state, &mut effects);
    }
    effects
}

pub(super) fn handle_prereq_load_failed(state: &mut AppState, reason: String) -> Vec<Effect> {
    engine_warn!("[briefing-triage] prereq load failed reason={}", reason);
    state.briefing_mut().fail(reason);
    state.clear_briefing_orchestration();
    state.mark_dirty();
    Vec::new()
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

pub(super) fn on_triage_settled_for_briefing(state: &mut AppState, effects: &mut Vec<Effect>) {
    if !state.briefing_orchestration_requested() {
        return;
    }
    // INTENTIONAL EXCEPTION: briefing article selection applies TriageSelectionPolicy
    // cutoff semantics on top of the briefing-owned triage results — only articles with
    // sufficient priority are included. This is distinct from the shared working-corpus
    // selector: the selector picks what is "ready to act on now", while this step picks
    // what is "worth summarizing in this briefing run". Using the shared selector here
    // would ignore the priority cutoff and could include low-signal articles.
    let policy = state.briefing_triage_policy();
    let ordered_urls = policy.eligible_urls(state.triage());
    engine_info!(
        "[briefing-triage] eligible count={} cutoff={}",
        ordered_urls.len(),
        policy.cutoff_exclusive
    );
    if ordered_urls.is_empty() {
        state
            .briefing_mut()
            .fail("No articles with sufficient priority".to_string());
        state.clear_briefing_orchestration();
        state.mark_dirty();
        return;
    }

    let _ = state.take_briefing_prereq_articles();
    state.start_summary_cache_run();
    state.mark_briefing_metadata_ready();
    state.set_briefing(BriefingSession::new_loading(None));
    snapshot_briefing_coverage_window(state);
    state.clear_briefing_orchestration_request();
    let since_utc = state.briefing_since_utc();
    effects.push(Effect::LoadArticlesForBriefing {
        ordered_urls,
        since_utc,
    });
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
