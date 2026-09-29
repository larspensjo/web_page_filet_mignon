use super::summary_cache_support::log_summary_cache_run_summary;
use crate::briefing::BriefingPhase;
use crate::state::BriefingGenerateReadiness;
use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::llm::prompt::PromptId;

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

    state.clear_provider_alert();
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
        return vec![Effect::FlushResults];
    }
    state.briefing_mut().set_articles(articles, collection_text);
    state.briefing_mut().transition_to_summarizing();
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_articles_load_failed(state: &mut AppState, reason: String) -> Vec<Effect> {
    state.briefing_mut().fail(reason);
    log_summary_cache_run_summary(state);
    state.mark_dirty();
    vec![Effect::FlushResults]
}

pub(super) fn settle_summaries(state: &mut AppState, effects: &mut Vec<Effect>) {
    // Gate aggregate briefing on ALL articles settled (no pending, no in-progress).
    if state.briefing().pending_count() > 0 || state.briefing().in_progress_count() > 0 {
        return;
    }

    if state.briefing().deferred_count() > 0 {
        state.briefing_mut().set_phase(BriefingPhase::AwaitingBatch);
        state.mark_dirty();
        return;
    }

    if state.briefing().completed_summary_count() == 0 {
        state
            .briefing_mut()
            .fail("all article summaries failed".to_string());
        state.mark_dirty();
        log_summary_cache_run_summary(state);
        effects.push(Effect::FlushResults);
        return;
    }

    if state.briefing_orchestration_skip_aggregate() || state.model_dispatch_halt_reason().is_some()
    {
        state.briefing_mut().complete_without_briefing();
        state.clear_briefing_orchestration();
        state.mark_dirty();
        log_summary_cache_run_summary(state);
        effects.push(Effect::FlushResults);
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
            effects.push(Effect::FlushResults);
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
