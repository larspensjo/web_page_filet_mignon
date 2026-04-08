use crate::prompt_lab::{PromptLabCompareBatchStatus, PromptLabRunStatus, PromptLabStage};
use crate::state::PromptLabPendingRunRegistration;
use crate::tabs::LeftTab;
use crate::{AppState, Effect};
use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::llm::types::ModelId;

pub(super) fn handle_open_requested(state: &mut AppState) -> Vec<Effect> {
    // Bridge: selecting the PromptLab tab is now the canonical open action.
    state.open_prompt_lab();
    Vec::new()
}

pub(super) fn handle_close_requested(state: &mut AppState) -> Vec<Effect> {
    state.close_prompt_lab_internals();
    state.set_left_tab(LeftTab::Jobs);
    Vec::new()
}

pub(super) fn handle_stage_selected(state: &mut AppState, stage: PromptLabStage) -> Vec<Effect> {
    state.select_prompt_lab_stage(stage);
    Vec::new()
}

pub(super) fn handle_input_source_selected(
    state: &mut AppState,
    source: crate::prompt_lab::PromptLabInputSource,
) -> Vec<Effect> {
    state.prompt_lab_mut().select_input_source(source);
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_input_changed(state: &mut AppState, text: String) -> Vec<Effect> {
    state.set_prompt_lab_input(text);
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_advanced_mode_set(state: &mut AppState, enabled: bool) -> Vec<Effect> {
    state.prompt_lab_mut().set_advanced_mode(enabled);
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_model_catalog_loaded(
    state: &mut AppState,
    models: Vec<ModelId>,
    source: crate::prompt_lab::ModelCatalogSource,
) -> Vec<Effect> {
    let sample = models
        .iter()
        .take(5)
        .map(|m| m.model_name().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    engine_info!(
        "[prompt-lab-model] reducer received catalog source={:?} count={} sample=[{}]",
        source,
        models.len(),
        sample
    );
    state.prompt_lab_mut().set_model_catalog(models, source);
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_model_override_set(
    state: &mut AppState,
    model: Option<ModelId>,
) -> Vec<Effect> {
    state.prompt_lab_mut().set_model_override_checked(model);
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_compare_section_toggled(state: &mut AppState) -> Vec<Effect> {
    state.prompt_lab_mut().toggle_compare_section();
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_context_section_toggled(state: &mut AppState) -> Vec<Effect> {
    state.prompt_lab_mut().toggle_context_section();
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_template_section_toggled(state: &mut AppState) -> Vec<Effect> {
    state.prompt_lab_mut().toggle_template_section();
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_run_details_section_toggled(state: &mut AppState) -> Vec<Effect> {
    state.prompt_lab_mut().toggle_run_details_section();
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_url_input_changed(state: &mut AppState, url: String) -> Vec<Effect> {
    state.prompt_lab_mut().set_url_input(url);
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_resolve_requested(state: &mut AppState) -> Vec<Effect> {
    let url = state.prompt_lab().url_input().to_owned();
    let has_pending = state.prompt_lab().pending_resolve_id().is_some();
    if url.is_empty() || has_pending {
        return Vec::new();
    }
    let resolve_id = state.allocate_next_prompt_lab_resolve_id();
    state.prompt_lab_mut().begin_url_resolution(resolve_id);
    state.mark_dirty();
    vec![Effect::ResolvePromptLabInputFromUrl { resolve_id, url }]
}

pub(super) fn handle_input_resolved(
    state: &mut AppState,
    resolve_id: u64,
    result: Result<String, String>,
) -> Vec<Effect> {
    if state
        .prompt_lab_mut()
        .finish_url_resolution(resolve_id, result)
    {
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_context_editor_opened(state: &mut AppState) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    let base_snapshot = state.context_for(prompt_id).to_vec();
    state
        .prompt_lab_mut()
        .initialize_context_draft(prompt_id, &base_snapshot);
    engine_info!(
        "[prompt-lab-context] PromptLabContextEditorOpened prompt_id={:?}",
        prompt_id
    );
    Vec::new()
}

pub(super) fn handle_context_draft_changed(state: &mut AppState, text: String) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    let base_snapshot = state.context_for(prompt_id).to_vec();
    state
        .prompt_lab_mut()
        .initialize_context_draft(prompt_id, &base_snapshot);
    state
        .prompt_lab_mut()
        .update_context_draft_text(prompt_id, text);
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_context_apply_requested(state: &mut AppState) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    let base_snapshot = state.context_for(prompt_id).to_vec();
    state
        .prompt_lab_mut()
        .initialize_context_draft(prompt_id, &base_snapshot);
    if state.prompt_lab_mut().apply_context_draft(prompt_id) {
        let count = state
            .prompt_lab()
            .applied_context_pairs(prompt_id)
            .map(|pairs: &[(String, String)]| pairs.len())
            .unwrap_or(0);
        engine_info!(
            "[prompt-lab-context] PromptLabContextApplied prompt_id={:?} pair_count={}",
            prompt_id,
            count
        );
        state.mark_dirty();
    } else {
        engine_warn!(
            "[prompt-lab-context] PromptLabContextApplyRequested rejected for {:?}",
            prompt_id
        );
    }
    Vec::new()
}

pub(super) fn handle_context_apply_and_rerun_requested(state: &mut AppState) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    let base_snapshot = state.context_for(prompt_id).to_vec();
    state
        .prompt_lab_mut()
        .initialize_context_draft(prompt_id, &base_snapshot);
    if !state.prompt_lab_mut().apply_context_draft(prompt_id) {
        return Vec::new();
    }
    if state.prompt_lab().has_in_flight_run() {
        return Vec::new();
    }
    let snapshot = state
        .prompt_lab()
        .resolved_url_snapshot()
        .map(str::to_owned);
    let input = match snapshot {
        Some(text) => text,
        None => return Vec::new(),
    };
    let prompt_version = state
        .prompt_lab()
        .selected_prompt_version()
        .or_else(|| state.active_version_for(prompt_id));
    let model_override = state.prompt_lab().selected_model_override().cloned();
    state.mark_dirty();
    dispatch_prompt_lab_run(
        state,
        PromptLabDispatchRequest {
            stage,
            prompt_id,
            input_snapshot: input,
            prompt_version,
            model_override,
            compare_batch_id: None,
            compare_candidate_id: None,
        },
    )
}

pub(super) fn handle_context_revert_requested(state: &mut AppState) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    let base_snapshot = state.context_for(prompt_id).to_vec();
    state
        .prompt_lab_mut()
        .initialize_context_draft(prompt_id, &base_snapshot);
    if state.prompt_lab_mut().revert_context_draft(prompt_id) {
        engine_info!(
            "[prompt-lab-context] PromptLabContextReverted prompt_id={:?}",
            prompt_id
        );
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_context_save_requested(state: &mut AppState) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    let base_snapshot = state.context_for(prompt_id).to_vec();
    state
        .prompt_lab_mut()
        .initialize_context_draft(prompt_id, &base_snapshot);
    if !state.prompt_lab().can_save_context(prompt_id) {
        engine_warn!(
            "[prompt-lab-context] PromptLabContextSaveRequested without applied changes for {:?}",
            prompt_id
        );
        return Vec::new();
    }
    let context_pairs = match state.prompt_lab().applied_context_pairs(prompt_id) {
        Some(pairs) => pairs.to_vec(),
        None => {
            engine_warn!(
                "[prompt-lab-context] Save requested but no applied context for {:?}",
                prompt_id
            );
            return Vec::new();
        }
    };
    engine_info!(
        "[prompt-lab-context] PromptLabContextSaveRequested prompt_id={:?} pair_count={}",
        prompt_id,
        context_pairs.len()
    );
    vec![Effect::SavePromptContextFile {
        prompt_id,
        context_pairs,
    }]
}

pub(super) fn handle_context_reload_requested(state: &mut AppState) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    state.prompt_lab_mut().drop_context_draft(prompt_id);
    engine_info!(
        "[prompt-lab-context] PromptLabContextReloadRequested prompt_id={:?}",
        prompt_id
    );
    vec![Effect::LoadPromptContexts]
}

pub(super) fn handle_context_saved(
    state: &mut AppState,
    prompt_id: PromptId,
    path: String,
    version: u64,
) -> Vec<Effect> {
    let message = Some(format!("Saved prompt context v{} to {}", version, path));
    state
        .prompt_lab_mut()
        .mark_context_saved(prompt_id, message.clone());
    engine_info!(
        "[prompt-lab-context] PromptLabContextSaved prompt_id={:?} path={} version={}",
        prompt_id,
        path,
        version
    );
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_context_save_failed(
    state: &mut AppState,
    prompt_id: PromptId,
    reason: String,
) -> Vec<Effect> {
    let message = Some(format!("Save failed: {}", reason));
    state
        .prompt_lab_mut()
        .set_context_status_message(prompt_id, message.clone());
    engine_error!(
        "[prompt-lab-context] PromptLabContextSaveFailed prompt_id={:?} reason={}",
        prompt_id,
        reason
    );
    Vec::new()
}

pub(super) fn handle_template_editor_toggled(state: &mut AppState) -> Vec<Effect> {
    let currently_open = state.prompt_lab().template_editor_open();
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    state
        .prompt_lab_mut()
        .set_template_editor_open(!currently_open);
    if !currently_open {
        ensure_prompt_lab_template_draft(state, prompt_id);
    }
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_template_system_draft_changed(
    state: &mut AppState,
    text: String,
) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    ensure_prompt_lab_template_draft(state, prompt_id);
    if state
        .prompt_lab_mut()
        .update_template_system(prompt_id, text)
    {
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_template_user_draft_changed(
    state: &mut AppState,
    text: String,
) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    ensure_prompt_lab_template_draft(state, prompt_id);
    if state.prompt_lab_mut().update_template_user(prompt_id, text) {
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_template_apply_requested(state: &mut AppState) -> Vec<Effect> {
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(state.prompt_lab().selected_stage());
    if apply_prompt_lab_template_draft(state, prompt_id) {
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_template_apply_and_rerun_requested(state: &mut AppState) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    if !apply_prompt_lab_template_draft(state, prompt_id) {
        return Vec::new();
    }
    if state.prompt_lab().has_in_flight_run() {
        return Vec::new();
    }
    let snapshot = state
        .prompt_lab()
        .resolved_url_snapshot()
        .map(str::to_owned);
    let input = match snapshot {
        Some(text) => text,
        None => return Vec::new(),
    };
    let prompt_version = state
        .prompt_lab()
        .selected_prompt_version()
        .or_else(|| state.active_version_for(prompt_id));
    let model_override = state.prompt_lab().selected_model_override().cloned();
    state.mark_dirty();
    dispatch_prompt_lab_run(
        state,
        PromptLabDispatchRequest {
            stage,
            prompt_id,
            input_snapshot: input,
            prompt_version,
            model_override,
            compare_batch_id: None,
            compare_candidate_id: None,
        },
    )
}

pub(super) fn handle_template_revert_requested(state: &mut AppState) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    ensure_prompt_lab_template_draft(state, prompt_id);
    if state.prompt_lab_mut().revert_template(prompt_id) {
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_template_save_requested(state: &mut AppState) -> Vec<Effect> {
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    let effect = if let Some(draft) = state.prompt_lab().template_draft(prompt_id) {
        if draft.is_applied() && draft.validation_errors().is_empty() {
            Some(Effect::SavePromptTemplateFile {
                prompt_id,
                system_template: draft.system_draft().to_string(),
                user_template: draft.user_draft().to_string(),
                description: draft.description().to_string(),
                expected_format: draft.expected_format().to_string(),
            })
        } else {
            engine_warn!(
                "[prompt-lab-template] Save requested without applied draft prompt_id={:?}",
                prompt_id
            );
            None
        }
    } else {
        engine_warn!(
            "[prompt-lab-template] Save requested but no draft open prompt_id={:?}",
            prompt_id
        );
        None
    };
    if let Some(effect) = effect {
        return vec![effect];
    }
    Vec::new()
}

pub(super) fn handle_template_saved(
    state: &mut AppState,
    prompt_id: PromptId,
    version: PromptVersion,
    path: String,
) -> Vec<Effect> {
    let path_buf = std::path::PathBuf::from(path.clone());
    state
        .prompt_lab_mut()
        .mark_template_saved(prompt_id, version, path_buf);
    engine_info!(
        "[prompt-lab-template] PromptLabTemplateSaved prompt_id={:?} path={} version={}",
        prompt_id,
        path,
        version
    );
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_template_save_failed(
    _state: &mut AppState,
    prompt_id: PromptId,
    reason: String,
) -> Vec<Effect> {
    engine_error!(
        "[prompt-lab-template] PromptLabTemplateSaveFailed prompt_id={:?} reason={}",
        prompt_id,
        reason
    );
    Vec::new()
}

pub(super) fn handle_run_requested(state: &mut AppState) -> Vec<Effect> {
    if state.prompt_lab().has_in_flight_run() {
        return Vec::new();
    }
    let snapshot = state
        .prompt_lab()
        .resolved_url_snapshot()
        .map(str::to_owned);
    let input = match snapshot {
        Some(text) => text,
        None => return Vec::new(),
    };
    let stage = state.prompt_lab().selected_stage();
    let prompt_id = crate::prompt_lab::prompt_id_for_stage(stage);
    let prompt_version = state
        .prompt_lab()
        .selected_prompt_version()
        .or_else(|| state.active_version_for(prompt_id));
    let model_override = state.prompt_lab().selected_model_override().cloned();
    dispatch_prompt_lab_run(
        state,
        PromptLabDispatchRequest {
            stage,
            prompt_id,
            input_snapshot: input,
            prompt_version,
            model_override,
            compare_batch_id: None,
            compare_candidate_id: None,
        },
    )
}

pub(super) fn handle_rerun_requested(state: &mut AppState) -> Vec<Effect> {
    if state.prompt_lab().has_in_flight_run() {
        return Vec::new();
    }
    let latest = match state.prompt_lab().latest_run() {
        Some(run) => run,
        None => return Vec::new(),
    };
    if matches!(latest.status, PromptLabRunStatus::Pending { .. }) {
        return Vec::new();
    }
    let stage = latest.stage;
    let prompt_id = latest.prompt_id;
    let input_snapshot = latest.input_snapshot.clone();
    let prompt_version = latest.prompt_version_used;
    let model_override = latest.model_override.clone();
    dispatch_prompt_lab_run(
        state,
        PromptLabDispatchRequest {
            stage,
            prompt_id,
            input_snapshot,
            prompt_version,
            model_override,
            compare_batch_id: None,
            compare_candidate_id: None,
        },
    )
}

pub(super) fn handle_history_cleared(state: &mut AppState) -> Vec<Effect> {
    state.clear_prompt_lab_history();
    Vec::new()
}

pub(super) fn handle_compare_draft_reset(state: &mut AppState) -> Vec<Effect> {
    state.prompt_lab_mut().clear_draft_candidates();
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_compare_current_settings_captured(state: &mut AppState) -> Vec<Effect> {
    if state
        .prompt_lab_mut()
        .add_draft_candidate_from_current(None)
        .is_ok()
    {
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_compare_baseline_captured(state: &mut AppState) -> Vec<Effect> {
    if state.prompt_lab_mut().add_baseline_candidate(None).is_ok() {
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_compare_candidate_removed(
    state: &mut AppState,
    candidate_id: u64,
) -> Vec<Effect> {
    if state.prompt_lab_mut().remove_draft_candidate(candidate_id) {
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_compare_candidate_label_changed(
    state: &mut AppState,
    candidate_id: u64,
    label: String,
) -> Vec<Effect> {
    if state
        .prompt_lab_mut()
        .rename_draft_candidate(candidate_id, label)
    {
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_compare_batch_start_requested(state: &mut AppState) -> Vec<Effect> {
    if state.prompt_lab().has_in_flight_run() {
        return Vec::new();
    }
    let snapshot = state
        .prompt_lab()
        .resolved_url_snapshot()
        .map(str::to_owned);
    let input = match snapshot {
        Some(text) => text,
        None => return Vec::new(),
    };
    let batch_id = match state.prompt_lab_mut().freeze_batch(input.clone()) {
        Ok(batch_id) => batch_id,
        Err(_) => return Vec::new(),
    };
    let effects = dispatch_next_compare_candidate(state, batch_id);
    state.mark_dirty();
    effects
}

pub(super) fn handle_compare_batch_cancel_requested(state: &mut AppState) -> Vec<Effect> {
    if let Some(batch) = state.prompt_lab_mut().active_batch_mut() {
        batch.status = PromptLabCompareBatchStatus::Cancelled;
        let batch_id = batch.batch_id;
        state.prompt_lab_mut().clear_active_batch_if(batch_id);
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_compare_winner_selected(
    state: &mut AppState,
    run_id: crate::prompt_lab::PromptLabRunId,
) -> Vec<Effect> {
    if let Some(batch) = state.prompt_lab_mut().active_batch_mut() {
        batch.selected_run_id = Some(run_id);
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_compare_winner_cleared(state: &mut AppState) -> Vec<Effect> {
    if let Some(batch) = state.prompt_lab_mut().active_batch_mut() {
        batch.selected_run_id = None;
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_compare_run_rated(
    state: &mut AppState,
    run_id: crate::prompt_lab::PromptLabRunId,
    rating: u8,
) -> Vec<Effect> {
    if !(1..=5).contains(&rating) {
        return Vec::new();
    }
    if let Some(run) = state.prompt_lab_mut().run_by_id_mut(run_id) {
        run.operator_rating = Some(rating);
        if let Some(batch_id) = run.compare_batch_id {
            state
                .prompt_lab_mut()
                .recompute_auto_select_for_batch(batch_id);
        }
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_compare_policy_updated(
    state: &mut AppState,
    require_parse_ok: Option<bool>,
    max_cost_microdollars: Option<Option<u64>>,
    max_wall_ms: Option<Option<u64>>,
    rating_beats_cost: Option<bool>,
) -> Vec<Effect> {
    let lab = state.prompt_lab_mut();
    if let Some(value) = require_parse_ok {
        lab.compare_policy.require_parse_ok = value;
    }
    if let Some(value) = max_cost_microdollars {
        lab.compare_policy.max_cost_microdollars = value;
    }
    if let Some(value) = max_wall_ms {
        lab.compare_policy.max_wall_ms = value;
    }
    if let Some(value) = rating_beats_cost {
        lab.compare_policy.rating_beats_cost = value;
    }
    if let Some(batch_id) = lab.active_batch().map(|batch| batch.batch_id) {
        lab.recompute_auto_select_for_batch(batch_id);
    }
    state.mark_dirty();
    Vec::new()
}

pub(super) fn handle_compare_auto_select_requested(state: &mut AppState) -> Vec<Effect> {
    if let Some(batch_id) = state
        .prompt_lab()
        .active_batch()
        .map(|batch| batch.batch_id)
    {
        state
            .prompt_lab_mut()
            .recompute_auto_select_for_batch(batch_id);
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn handle_compare_batch_set_warning(
    state: &mut AppState,
    batch_id: crate::prompt_lab::PromptLabCompareBatchId,
    warning: Option<String>,
) -> Vec<Effect> {
    if let Some(batch) = state
        .prompt_lab_mut()
        .batches
        .iter_mut()
        .find(|batch| batch.batch_id == batch_id)
    {
        batch.warning = warning;
        state.mark_dirty();
    }
    Vec::new()
}

pub(super) fn dispatch_prompt_lab_run(
    state: &mut AppState,
    dispatch: PromptLabDispatchRequest,
) -> Vec<Effect> {
    let PromptLabDispatchRequest {
        stage,
        prompt_id,
        input_snapshot,
        prompt_version,
        model_override,
        compare_batch_id,
        compare_candidate_id,
    } = dispatch;
    let request_id = state.allocate_next_llm_request_id();
    let run_id = state.allocate_next_prompt_lab_run_id();
    let context = state
        .prompt_lab()
        .applied_context_pairs(prompt_id)
        .map(|pairs| pairs.to_vec())
        .unwrap_or_else(|| state.context_for(prompt_id).to_vec());
    let pending_prompt_version = prompt_version;
    let pending_model_override = model_override.clone();
    state.record_pending_llm_request(request_id, prompt_id);
    state.add_prompt_lab_pending_run(PromptLabPendingRunRegistration {
        run_id,
        stage,
        prompt_id,
        input_snapshot: input_snapshot.clone(),
        request_id,
        overrides: crate::prompt_lab::PromptLabRunOverrides {
            prompt_version_used: pending_prompt_version,
            model_override: pending_model_override,
        },
        compare_batch_id,
        compare_candidate_id,
    });
    state.mark_dirty();
    engine_info!(
        "[prompt-lab] run requested run_id={} request_id={} stage={:?}",
        run_id.0,
        request_id,
        stage
    );
    let extra_template_vars = if prompt_id == PromptId::AggregateBriefing {
        vec![
            (
                "previous_briefings".to_string(),
                crate::briefing::format_previous_briefings_block(state.briefing_history()),
            ),
            (
                "briefing_time_window".to_string(),
                crate::briefing::format_briefing_time_window_label(state.briefing_since_utc()),
            ),
        ]
    } else {
        vec![]
    };
    vec![Effect::RequestLlmCompletion {
        request_id,
        prompt_id,
        prompt_version,
        model_override,
        input_content: input_snapshot,
        context,
        template_override: state.prompt_lab().applied_template_override(prompt_id),
        extra_template_vars,
    }]
}

pub(super) struct PromptLabDispatchRequest {
    pub(super) stage: PromptLabStage,
    pub(super) prompt_id: PromptId,
    pub(super) input_snapshot: String,
    pub(super) prompt_version: Option<PromptVersion>,
    pub(super) model_override: Option<ModelId>,
    pub(super) compare_batch_id: Option<crate::prompt_lab::PromptLabCompareBatchId>,
    pub(super) compare_candidate_id: Option<u64>,
}

pub(super) fn dispatch_next_compare_candidate(
    state: &mut AppState,
    batch_id: crate::prompt_lab::PromptLabCompareBatchId,
) -> Vec<Effect> {
    let Some(batch) = state
        .prompt_lab()
        .batches()
        .iter()
        .find(|batch| batch.batch_id == batch_id)
        .cloned()
    else {
        return Vec::new();
    };
    let Some(candidate) = batch.next_undispatched_candidate().cloned() else {
        return Vec::new();
    };
    let input_snapshot = batch.input_snapshot.clone();
    let effects = dispatch_prompt_lab_run(
        state,
        PromptLabDispatchRequest {
            stage: candidate.stage,
            prompt_id: candidate.prompt_id,
            input_snapshot,
            prompt_version: candidate.prompt_version,
            model_override: candidate.model_override.clone(),
            compare_batch_id: Some(batch_id),
            compare_candidate_id: Some(candidate.candidate_id),
        },
    );
    if let Some(run_id) = state.prompt_lab().latest_run().map(|run| run.run_id) {
        state
            .prompt_lab_mut()
            .record_compare_dispatch(batch_id, candidate.candidate_id, run_id);
    }
    effects
}

fn ensure_prompt_lab_template_draft(state: &mut AppState, prompt_id: PromptId) {
    if state.prompt_lab().template_draft(prompt_id).is_some() {
        return;
    }
    if let Some(snapshot) = state.prompt_lab_template_snapshot(prompt_id).cloned() {
        let template = snapshot.template;
        state.prompt_lab_mut().open_template_draft(
            prompt_id,
            &template.system_template,
            &template.user_template,
            &template.description,
            &template.expected_format,
        );
    }
}

fn template_draft_texts(state: &mut AppState, prompt_id: PromptId) -> Option<(String, String)> {
    ensure_prompt_lab_template_draft(state, prompt_id);
    state.prompt_lab().template_draft(prompt_id).map(|draft| {
        (
            draft.system_draft().to_string(),
            draft.user_draft().to_string(),
        )
    })
}

fn apply_prompt_lab_template_draft(state: &mut AppState, prompt_id: PromptId) -> bool {
    let (system, user) = match template_draft_texts(state, prompt_id) {
        Some(pair) => pair,
        None => return false,
    };
    let errors = harvester_engine::llm::validate_template(prompt_id, &system, &user);
    if !errors.is_empty() {
        engine_warn!(
            "[prompt-lab-template] validation failed prompt_id={:?} error_count={}",
            prompt_id,
            errors.len()
        );
    }
    let applied = state.prompt_lab_mut().apply_template(prompt_id, errors);
    if applied {
        engine_info!(
            "[prompt-lab-template] PromptLabTemplateApplied prompt_id={:?}",
            prompt_id
        );
    }
    applied
}
