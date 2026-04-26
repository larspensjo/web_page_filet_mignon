use commanductui::types::ControlId;
use commanductui::{PlatformCommand, StyleId, WindowId};
use harvester_core::{AppViewModel, Msg};

use crate::platform::ui::constants::*;
use crate::platform::ui::render::emit_if_changed;

#[derive(Debug, Clone, Copy)]
struct PromptLabActionButtonDescriptor {
    control_id: ControlId,
    parent_control_id: ControlId,
    label: &'static str,
    initial_style: StyleId,
    msg: Option<fn() -> Msg>,
}

#[derive(Debug, Clone, Copy)]
struct PromptLabActionCheckBoxDescriptor {
    control_id: ControlId,
    parent_control_id: ControlId,
    label: &'static str,
    initial_style: StyleId,
    msg: fn() -> Msg,
}

const BUTTONS: &[PromptLabActionButtonDescriptor] = &[
    // These source-row controls are intentionally unrouted and currently hidden
    // by the collapsed source row.
    PromptLabActionButtonDescriptor {
        control_id: BTN_SOURCE_FROM_TRIAGE,
        parent_control_id: PANEL_PROMPT_LAB_SOURCE_ROW,
        label: "Selected article",
        initial_style: StyleId::DefaultButton,
        msg: None,
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_SOURCE_TYPE_URL,
        parent_control_id: PANEL_PROMPT_LAB_SOURCE_ROW,
        label: "Type URL",
        initial_style: StyleId::DefaultButton,
        msg: None,
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_PROMPT_LAB_RESOLVE,
        parent_control_id: PANEL_PROMPT_LAB_INPUT_ROW,
        label: "Resolve",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabResolveRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_PROMPT_LAB_RUN,
        parent_control_id: PANEL_PROMPT_LAB_ACTION_ROW,
        label: "Run",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabRunRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_COMPARE_ADD_CURRENT,
        parent_control_id: PANEL_PROMPT_LAB_COMPARE_ROW,
        label: "Add current settings",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabCompareCurrentSettingsCaptured),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_COMPARE_ADD_BASELINE,
        parent_control_id: PANEL_PROMPT_LAB_COMPARE_ROW,
        label: "Add baseline",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabCompareBaselineCaptured),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_COMPARE_RESET_DRAFT,
        parent_control_id: PANEL_PROMPT_LAB_COMPARE_ROW,
        label: "Reset draft",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabCompareDraftReset),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_COMPARE_START,
        parent_control_id: PANEL_PROMPT_LAB_COMPARE_ROW,
        label: "Start compare",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabCompareBatchStartRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_COMPARE_CANCEL,
        parent_control_id: PANEL_PROMPT_LAB_COMPARE_ROW,
        label: "Cancel compare",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabCompareBatchCancelRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_COMPARE_AUTO_SELECT,
        parent_control_id: PANEL_PROMPT_LAB_COMPARE_ROW,
        label: "Auto-select",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabCompareAutoSelectRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_COMPARE_WINNER_CLEAR,
        parent_control_id: PANEL_PROMPT_LAB_COMPARE_ROW,
        label: "Clear winner",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabCompareWinnerCleared),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_PROMPT_LAB_CONTEXT_APPLY,
        parent_control_id: PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW,
        label: "Apply",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabContextApplyRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN,
        parent_control_id: PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW,
        label: "Apply+Run",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabContextApplyAndRerunRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_PROMPT_LAB_CONTEXT_REVERT,
        parent_control_id: PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW,
        label: "Revert",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabContextRevertRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_PROMPT_LAB_CONTEXT_SAVE,
        parent_control_id: PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW,
        label: "Save",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabContextSaveRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_PROMPT_LAB_CONTEXT_RELOAD,
        parent_control_id: PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW,
        label: "Reload",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabContextReloadRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY,
        parent_control_id: PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW,
        label: "Apply",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabTemplateApplyRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN,
        parent_control_id: PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW,
        label: "Apply+Run",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabTemplateApplyAndRerunRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_PROMPT_LAB_TEMPLATE_REVERT,
        parent_control_id: PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW,
        label: "Revert",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabTemplateRevertRequested),
    },
    PromptLabActionButtonDescriptor {
        control_id: BTN_PROMPT_LAB_TEMPLATE_SAVE,
        parent_control_id: PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW,
        label: "Save",
        initial_style: StyleId::DefaultButton,
        msg: Some(|| Msg::PromptLabTemplateSaveRequested),
    },
];

const CHECKBOXES: &[PromptLabActionCheckBoxDescriptor] = &[PromptLabActionCheckBoxDescriptor {
    control_id: CHK_PROMPT_LAB_TEMPLATE_OPEN,
    parent_control_id: PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW,
    label: "Edit Templates",
    initial_style: StyleId::DefaultButton,
    msg: || Msg::PromptLabTemplateEditorToggled,
}];

#[derive(Debug, Default)]
pub(in crate::platform) struct PromptLabActionsRenderState {
    prev_run_enabled: Option<bool>,
    prev_resolve_enabled: Option<bool>,
    prev_compare_add_current_enabled: Option<bool>,
    prev_compare_add_baseline_enabled: Option<bool>,
    prev_compare_reset_draft_enabled: Option<bool>,
    prev_compare_start_enabled: Option<bool>,
    prev_compare_cancel_enabled: Option<bool>,
    prev_compare_auto_select_enabled: Option<bool>,
    prev_compare_winner_clear_enabled: Option<bool>,
    prev_context_apply_enabled: Option<bool>,
    prev_context_apply_rerun_enabled: Option<bool>,
    prev_context_revert_enabled: Option<bool>,
    prev_context_save_enabled: Option<bool>,
    prev_template_open_checked: Option<bool>,
    prev_template_apply_enabled: Option<bool>,
    prev_template_apply_rerun_enabled: Option<bool>,
    prev_template_revert_enabled: Option<bool>,
    prev_template_save_enabled: Option<bool>,
}

pub(in crate::platform) fn create_controls(
    window_id: WindowId,
    commands: &mut Vec<PlatformCommand>,
) {
    for button in BUTTONS {
        commands.push(PlatformCommand::CreateButton {
            window_id,
            parent_control_id: Some(button.parent_control_id),
            control_id: button.control_id,
            text: button.label.to_string(),
        });
    }
    for checkbox in CHECKBOXES {
        commands.push(PlatformCommand::CreateCheckBox {
            window_id,
            parent_control_id: Some(checkbox.parent_control_id),
            control_id: checkbox.control_id,
            text: checkbox.label.to_string(),
        });
    }
}

pub(in crate::platform) fn apply_theme(window_id: WindowId, commands: &mut Vec<PlatformCommand>) {
    for button in BUTTONS {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id: button.control_id,
            style_id: button.initial_style,
        });
    }
    for checkbox in CHECKBOXES {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id: checkbox.control_id,
            style_id: checkbox.initial_style,
        });
    }
}

pub(in crate::platform) fn msg_for_button(control_id: ControlId) -> Option<Msg> {
    BUTTONS
        .iter()
        .find(|button| button.control_id == control_id)
        .and_then(|button| button.msg.map(|msg| msg()))
}

pub(in crate::platform) fn msg_for_checkbox(control_id: ControlId) -> Option<Msg> {
    CHECKBOXES
        .iter()
        .find(|checkbox| checkbox.control_id == control_id)
        .map(|checkbox| (checkbox.msg)())
}

pub(in crate::platform) fn render(
    window_id: WindowId,
    view: &AppViewModel,
    state: &mut PromptLabActionsRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let prompt_lab = &view.left_pane.prompt_lab;
    emit_enabled(
        window_id,
        BTN_PROMPT_LAB_RUN,
        &mut state.prev_run_enabled,
        prompt_lab.can_run,
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_PROMPT_LAB_RESOLVE,
        &mut state.prev_resolve_enabled,
        !prompt_lab.resolve_pending,
        cmds,
    );

    let compare_add_enabled = prompt_lab.can_add_candidate;
    emit_enabled(
        window_id,
        BTN_COMPARE_ADD_CURRENT,
        &mut state.prev_compare_add_current_enabled,
        compare_add_enabled,
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_COMPARE_ADD_BASELINE,
        &mut state.prev_compare_add_baseline_enabled,
        compare_add_enabled,
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_COMPARE_RESET_DRAFT,
        &mut state.prev_compare_reset_draft_enabled,
        prompt_lab.can_reset_draft,
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_COMPARE_START,
        &mut state.prev_compare_start_enabled,
        prompt_lab.active_batch.is_none() && prompt_lab.draft_candidates.len() >= 2,
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_COMPARE_CANCEL,
        &mut state.prev_compare_cancel_enabled,
        prompt_lab
            .active_batch
            .as_ref()
            .map(|batch| batch.can_cancel)
            .unwrap_or(false),
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_COMPARE_AUTO_SELECT,
        &mut state.prev_compare_auto_select_enabled,
        prompt_lab
            .active_batch
            .as_ref()
            .map(|batch| batch.can_auto_select)
            .unwrap_or(false),
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_COMPARE_WINNER_CLEAR,
        &mut state.prev_compare_winner_clear_enabled,
        prompt_lab
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
    );
    emit_enabled(
        window_id,
        BTN_PROMPT_LAB_CONTEXT_APPLY,
        &mut state.prev_context_apply_enabled,
        prompt_lab.can_apply_context,
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN,
        &mut state.prev_context_apply_rerun_enabled,
        prompt_lab.can_apply_and_rerun,
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_PROMPT_LAB_CONTEXT_REVERT,
        &mut state.prev_context_revert_enabled,
        prompt_lab.can_revert_context,
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_PROMPT_LAB_CONTEXT_SAVE,
        &mut state.prev_context_save_enabled,
        prompt_lab.can_save_context,
        cmds,
    );
    // BTN_PROMPT_LAB_CONTEXT_RELOAD has no dynamic enabled state today.
    emit_if_changed(
        &mut state.prev_template_open_checked,
        prompt_lab.template_editor_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_TEMPLATE_OPEN,
            checked,
        },
    );

    let can_apply_template =
        prompt_lab.template_dirty && prompt_lab.template_validation_errors.is_empty();
    let can_apply_template_and_rerun = can_apply_template && prompt_lab.can_run;
    let can_revert_template = prompt_lab.template_dirty || prompt_lab.template_applied;
    emit_enabled(
        window_id,
        BTN_PROMPT_LAB_TEMPLATE_APPLY,
        &mut state.prev_template_apply_enabled,
        can_apply_template,
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN,
        &mut state.prev_template_apply_rerun_enabled,
        can_apply_template_and_rerun,
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_PROMPT_LAB_TEMPLATE_REVERT,
        &mut state.prev_template_revert_enabled,
        can_revert_template,
        cmds,
    );
    emit_enabled(
        window_id,
        BTN_PROMPT_LAB_TEMPLATE_SAVE,
        &mut state.prev_template_save_enabled,
        prompt_lab.template_applied,
        cmds,
    );
}

fn emit_enabled(
    window_id: WindowId,
    control_id: ControlId,
    prev: &mut Option<bool>,
    enabled: bool,
    cmds: &mut Vec<PlatformCommand>,
) {
    emit_if_changed(prev, enabled, cmds, |enabled| {
        PlatformCommand::SetControlEnabled {
            window_id,
            control_id,
            enabled,
        }
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn prompt_lab_action_button_descriptors_are_unique() {
        let ids = BUTTONS
            .iter()
            .map(|button| button.control_id)
            .collect::<HashSet<_>>();

        assert_eq!(ids.len(), BUTTONS.len());
    }

    #[test]
    fn prompt_lab_action_checkbox_descriptors_are_unique() {
        let ids = CHECKBOXES
            .iter()
            .map(|checkbox| checkbox.control_id)
            .collect::<HashSet<_>>();

        assert_eq!(ids.len(), CHECKBOXES.len());
    }

    #[test]
    fn prompt_lab_action_descriptor_counts_match_initial_scope() {
        assert_eq!(BUTTONS.len(), 20);
        assert_eq!(CHECKBOXES.len(), 1);
    }

    #[test]
    fn prompt_lab_action_button_msg_mapping_matches_current_actions() {
        assert_eq!(
            msg_for_button(BTN_PROMPT_LAB_RESOLVE),
            Some(Msg::PromptLabResolveRequested)
        );
        assert_eq!(
            msg_for_button(BTN_PROMPT_LAB_RUN),
            Some(Msg::PromptLabRunRequested)
        );
        assert_eq!(
            msg_for_button(BTN_COMPARE_ADD_CURRENT),
            Some(Msg::PromptLabCompareCurrentSettingsCaptured)
        );
        assert_eq!(
            msg_for_button(BTN_COMPARE_ADD_BASELINE),
            Some(Msg::PromptLabCompareBaselineCaptured)
        );
        assert_eq!(
            msg_for_button(BTN_COMPARE_RESET_DRAFT),
            Some(Msg::PromptLabCompareDraftReset)
        );
        assert_eq!(
            msg_for_button(BTN_COMPARE_START),
            Some(Msg::PromptLabCompareBatchStartRequested)
        );
        assert_eq!(
            msg_for_button(BTN_COMPARE_CANCEL),
            Some(Msg::PromptLabCompareBatchCancelRequested)
        );
        assert_eq!(
            msg_for_button(BTN_COMPARE_AUTO_SELECT),
            Some(Msg::PromptLabCompareAutoSelectRequested)
        );
        assert_eq!(
            msg_for_button(BTN_COMPARE_WINNER_CLEAR),
            Some(Msg::PromptLabCompareWinnerCleared)
        );
        assert_eq!(
            msg_for_button(BTN_PROMPT_LAB_CONTEXT_APPLY),
            Some(Msg::PromptLabContextApplyRequested)
        );
        assert_eq!(
            msg_for_button(BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN),
            Some(Msg::PromptLabContextApplyAndRerunRequested)
        );
        assert_eq!(
            msg_for_button(BTN_PROMPT_LAB_CONTEXT_REVERT),
            Some(Msg::PromptLabContextRevertRequested)
        );
        assert_eq!(
            msg_for_button(BTN_PROMPT_LAB_CONTEXT_SAVE),
            Some(Msg::PromptLabContextSaveRequested)
        );
        assert_eq!(
            msg_for_button(BTN_PROMPT_LAB_CONTEXT_RELOAD),
            Some(Msg::PromptLabContextReloadRequested)
        );
        assert_eq!(
            msg_for_button(BTN_PROMPT_LAB_TEMPLATE_APPLY),
            Some(Msg::PromptLabTemplateApplyRequested)
        );
        assert_eq!(
            msg_for_button(BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN),
            Some(Msg::PromptLabTemplateApplyAndRerunRequested)
        );
        assert_eq!(
            msg_for_button(BTN_PROMPT_LAB_TEMPLATE_REVERT),
            Some(Msg::PromptLabTemplateRevertRequested)
        );
        assert_eq!(
            msg_for_button(BTN_PROMPT_LAB_TEMPLATE_SAVE),
            Some(Msg::PromptLabTemplateSaveRequested)
        );
    }

    #[test]
    fn prompt_lab_source_buttons_are_present_but_unrouted() {
        assert!(BUTTONS
            .iter()
            .any(|button| button.control_id == BTN_SOURCE_FROM_TRIAGE));
        assert!(BUTTONS
            .iter()
            .any(|button| button.control_id == BTN_SOURCE_TYPE_URL));
        assert_eq!(msg_for_button(BTN_SOURCE_FROM_TRIAGE), None);
        assert_eq!(msg_for_button(BTN_SOURCE_TYPE_URL), None);
    }

    #[test]
    fn prompt_lab_template_open_checkbox_maps_to_toggle_msg() {
        assert_eq!(
            msg_for_checkbox(CHK_PROMPT_LAB_TEMPLATE_OPEN),
            Some(Msg::PromptLabTemplateEditorToggled)
        );
    }
}
