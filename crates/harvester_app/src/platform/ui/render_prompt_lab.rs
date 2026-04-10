use commanductui::{PlatformCommand, WindowId};
use engine_logging::engine_info;
use harvester_core::{AppViewModel, PromptLabStage};
use harvester_engine::llm::ModelId;

use super::constants::*;
use super::render::{emit_if_changed, TreeRenderState};

/// Helper: Convert optional ModelId to combo box selection index.
/// Returns 0 for None (default), or index+1 for a specific model in the catalog.
pub(crate) fn model_to_combo_index(
    selected: Option<&ModelId>,
    catalog: &[ModelId],
) -> Option<usize> {
    match selected {
        None => Some(0),
        Some(model) => catalog.iter().position(|m| m == model).map(|idx| idx + 1),
    }
}

/// Helper: Convert combo box selection index to optional ModelId.
/// Returns None for index 0 (default), or the model at (index-1) in the catalog.
pub(crate) fn combo_index_to_model(index: usize, catalog: &[ModelId]) -> Option<ModelId> {
    if index == 0 {
        None
    } else {
        catalog.get(index - 1).cloned()
    }
}

pub(super) fn render_prompt_lab_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_mode_basic_checked,
        !view.left_pane.prompt_lab.advanced_mode,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_PROMPT_LAB_MODE_BASIC,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_mode_advanced_checked,
        view.left_pane.prompt_lab.advanced_mode,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_PROMPT_LAB_MODE_ADVANCED,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_stage_triage_checked,
        view.left_pane.prompt_lab.selected_stage == PromptLabStage::Triage,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_STAGE_TRIAGE,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_stage_summary_checked,
        view.left_pane.prompt_lab.selected_stage == PromptLabStage::Summary,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_STAGE_SUMMARY,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_stage_briefing_checked,
        view.left_pane.prompt_lab.selected_stage == PromptLabStage::Briefing,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_STAGE_BRIEFING,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_section_compare_checked,
        view.left_pane.prompt_lab.compare_section_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_SECTION_COMPARE,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_section_context_checked,
        view.left_pane.prompt_lab.context_section_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_SECTION_CONTEXT,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_section_template_checked,
        view.left_pane.prompt_lab.template_section_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_SECTION_TEMPLATE,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_section_run_details_checked,
        view.left_pane.prompt_lab.run_details_section_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_SECTION_RUN_DETAILS,
            checked,
        },
    );

    let model_catalog = &view.left_pane.prompt_lab.model_catalog;
    let selected_model = view.left_pane.prompt_lab.selected_model_override.as_ref();
    if tree_state
        .prompt_lab
        .prev_prompt_lab_model_catalog
        .as_deref()
        != Some(model_catalog.as_slice())
    {
        engine_info!(
            "[prompt-lab-model] render updating combo items source={:?} count={}",
            view.left_pane.prompt_lab.model_catalog_source,
            model_catalog.len()
        );
        let mut items = vec!["Default".to_string()];
        items.extend(model_catalog.iter().map(|m| m.model_name().to_string()));
        cmds.push(PlatformCommand::SetComboBoxItems {
            window_id,
            control_id: COMBO_PROMPT_LAB_MODEL_SELECTOR,
            items,
        });
        tree_state.prompt_lab.prev_prompt_lab_model_catalog = Some(model_catalog.clone());
    }
    let selected_model_key = selected_model
        .map(|m| m.model_name().to_string())
        .unwrap_or_else(|| "__DEFAULT__".to_string());
    if tree_state
        .prompt_lab
        .prev_prompt_lab_selected_model
        .as_deref()
        != Some(selected_model_key.as_str())
    {
        let index = model_to_combo_index(selected_model, model_catalog);
        engine_info!(
            "[prompt-lab-model] render updating combo selection key={} index={:?} catalog_count={}",
            selected_model_key,
            index,
            model_catalog.len()
        );
        cmds.push(PlatformCommand::SetComboBoxSelection {
            window_id,
            control_id: COMBO_PROMPT_LAB_MODEL_SELECTOR,
            selected_index: index,
        });
        tree_state.prompt_lab.prev_prompt_lab_selected_model = Some(selected_model_key);
    }

    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_run_enabled,
        view.left_pane.prompt_lab.can_run,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_RUN,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_resolve_enabled,
        !view.left_pane.prompt_lab.resolve_pending,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_RESOLVE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_url_enabled,
        true,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: INPUT_PROMPT_LAB_URL,
            enabled,
        },
    );

    let compare_add_enabled = view.left_pane.prompt_lab.can_add_candidate;
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_compare_add_current_enabled,
        compare_add_enabled,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_ADD_CURRENT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_compare_add_baseline_enabled,
        compare_add_enabled,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_ADD_BASELINE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_compare_reset_draft_enabled,
        view.left_pane.prompt_lab.can_reset_draft,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_RESET_DRAFT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_compare_start_enabled,
        view.left_pane.prompt_lab.active_batch.is_none()
            && view.left_pane.prompt_lab.draft_candidates.len() >= 2,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_START,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_compare_cancel_enabled,
        view.left_pane
            .prompt_lab
            .active_batch
            .as_ref()
            .map(|batch| batch.can_cancel)
            .unwrap_or(false),
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_CANCEL,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_compare_auto_select_enabled,
        view.left_pane
            .prompt_lab
            .active_batch
            .as_ref()
            .map(|batch| batch.can_auto_select)
            .unwrap_or(false),
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_AUTO_SELECT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_compare_winner_clear_enabled,
        view.left_pane
            .prompt_lab
            .active_batch
            .as_ref()
            .map(|batch| {
                batch
                    .rows
                    .iter()
                    .any(|row| row.is_manual_winner || row.is_auto_winner)
            })
            .unwrap_or(false),
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_WINNER_CLEAR,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_context_apply_enabled,
        view.left_pane.prompt_lab.can_apply_context,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_APPLY,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_context_apply_rerun_enabled,
        view.left_pane.prompt_lab.can_apply_and_rerun,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_context_revert_enabled,
        view.left_pane.prompt_lab.can_revert_context,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_REVERT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_context_save_enabled,
        view.left_pane.prompt_lab.can_save_context,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_SAVE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_open_checked,
        view.left_pane.prompt_lab.template_editor_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_TEMPLATE_OPEN,
            checked,
        },
    );

    let can_apply_template = view.left_pane.prompt_lab.template_dirty
        && view
            .left_pane
            .prompt_lab
            .template_validation_errors
            .is_empty();
    let can_apply_template_and_rerun = can_apply_template && view.left_pane.prompt_lab.can_run;
    let can_revert_template =
        view.left_pane.prompt_lab.template_dirty || view.left_pane.prompt_lab.template_applied;
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_apply_enabled,
        can_apply_template,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_template_apply_rerun_enabled,
        can_apply_template_and_rerun,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_template_revert_enabled,
        can_revert_template,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_REVERT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_save_enabled,
        view.left_pane.prompt_lab.template_applied,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_SAVE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_system_text,
        view.left_pane.prompt_lab.template_system_draft.clone(),
        cmds,
        |text| PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_user_text,
        view.left_pane.prompt_lab.template_user_draft.clone(),
        cmds,
        |text| PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_template_system_enabled,
        view.left_pane.prompt_lab.template_editor_open,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_user_enabled,
        view.left_pane.prompt_lab.template_editor_open,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_url_input,
        view.left_pane.prompt_lab.url_input.clone(),
        cmds,
        |text| PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_URL,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_status_text,
        prompt_lab_status_text(&view.left_pane.prompt_lab),
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_STATUS,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_metadata_text,
        prompt_lab_metadata_text(&view.left_pane.prompt_lab),
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_METADATA,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_context_text,
        view.left_pane.prompt_lab.context_draft_text.clone(),
        cmds,
        |text| PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_CONTEXT,
            text,
        },
    );
    let context_status_text = if !view
        .left_pane
        .prompt_lab
        .context_validation_errors
        .is_empty()
    {
        view.left_pane
            .prompt_lab
            .context_validation_errors
            .join(" • ")
    } else {
        view.left_pane
            .prompt_lab
            .context_status_message
            .clone()
            .unwrap_or_default()
    };
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_context_status_text,
        context_status_text,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_CONTEXT_STATUS,
            text,
        },
    );
    let template_status_text = if !view
        .left_pane
        .prompt_lab
        .template_validation_errors
        .is_empty()
    {
        view.left_pane
            .prompt_lab
            .template_validation_errors
            .join(" • ")
    } else if let (Some(version), Some(path)) = (
        view.left_pane.prompt_lab.template_saved_version,
        view.left_pane.prompt_lab.template_saved_path.as_deref(),
    ) {
        format!("Saved template v{version} to {path}")
    } else if view.left_pane.prompt_lab.template_applied {
        "Template draft applied".to_string()
    } else if view.left_pane.prompt_lab.template_dirty {
        "Template draft has unapplied changes".to_string()
    } else {
        String::new()
    };
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_status_text,
        template_status_text,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_TEMPLATE_STATUS,
            text,
        },
    );
}

pub(super) fn prompt_lab_status_text(prompt_lab: &harvester_core::PromptLabView) -> String {
    if prompt_lab.is_in_flight {
        return "Prompt Lab: Running...".to_string();
    }
    if let Some(reason) = prompt_lab.run_disabled_reason {
        if !prompt_lab.can_run {
            return format!("Prompt Lab: {reason}");
        }
    }
    if let Some(err) = prompt_lab.latest_validation_error.as_deref() {
        return format!("Prompt Lab validation: {err}");
    }
    if prompt_lab.url_resolve_failed {
        return "Prompt Lab: URL resolve failed".to_string();
    }
    if let Some(run) = prompt_lab.latest_run.as_ref() {
        if let Some(reason) = run.failure_reason.as_deref() {
            return format!("Prompt Lab failed: {reason}");
        }
        return format!("Prompt Lab: latest run {}", run.status_label);
    }
    "Prompt Lab ready".to_string()
}

pub(super) fn prompt_lab_metadata_text(prompt_lab: &harvester_core::PromptLabView) -> String {
    let template_source = prompt_lab
        .template_snapshot_source
        .map(|source| format!("{source:?}").to_lowercase())
        .unwrap_or_else(|| "-".to_string());
    let template_version = prompt_lab
        .template_snapshot_version
        .map(|version| version.to_string())
        .unwrap_or_else(|| "-".to_string());
    let template_description = prompt_lab
        .template_snapshot_description
        .as_deref()
        .unwrap_or("-");
    if let Some(run) = prompt_lab.latest_run.as_ref() {
        return format!(
            "model={} in={} out={} cost={} wall={}ms parse_ok={} cache={} template_source={} template_version={} template_desc={}",
            run.resolved_model.as_deref().unwrap_or("-"),
            run.input_tokens
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run.output_tokens
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run.cost_microdollars
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run.wall_ms
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run.parse_ok
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run.cache_status.as_deref().unwrap_or("-"),
            template_source,
            template_version,
            template_description
        );
    }
    format!(
        "model=- in=- out=- cost=- wall=- parse_ok=- cache=- template_source={} template_version={} template_desc={}",
        template_source,
        template_version,
        template_description
    )
}
