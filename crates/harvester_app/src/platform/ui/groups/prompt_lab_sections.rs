use commanductui::types::ControlId;
use commanductui::{PlatformCommand, StyleId, WindowId};
use harvester_core::{AppViewModel, Msg};

use crate::platform::ui::constants::*;
use crate::platform::ui::render::emit_if_changed;

#[derive(Debug, Clone, Copy)]
struct PromptLabSectionCheckBoxDescriptor {
    control_id: ControlId,
    parent_control_id: ControlId,
    label: &'static str,
    initial_style: StyleId,
    msg: fn() -> Msg,
    view_field: fn(&AppViewModel) -> bool,
    checked_slot: fn(&mut PromptLabSectionsRenderState) -> &mut Option<bool>,
}

const CHECKBOXES: &[PromptLabSectionCheckBoxDescriptor] = &[
    PromptLabSectionCheckBoxDescriptor {
        control_id: CHK_PROMPT_LAB_SECTION_COMPARE,
        parent_control_id: PANEL_PROMPT_LAB_COMPARE_HEADER_ROW,
        label: "Compare",
        initial_style: StyleId::CheckBox,
        msg: || Msg::PromptLabCompareSectionToggled,
        view_field: |view| view.left_pane.prompt_lab.compare_section_open,
        checked_slot: |state| &mut state.prev_compare_checked,
    },
    PromptLabSectionCheckBoxDescriptor {
        control_id: CHK_PROMPT_LAB_SECTION_CONTEXT,
        parent_control_id: PANEL_PROMPT_LAB_CONTEXT_HEADER_ROW,
        label: "Context",
        initial_style: StyleId::CheckBox,
        msg: || Msg::PromptLabContextSectionToggled,
        view_field: |view| view.left_pane.prompt_lab.context_section_open,
        checked_slot: |state| &mut state.prev_context_checked,
    },
    PromptLabSectionCheckBoxDescriptor {
        control_id: CHK_PROMPT_LAB_SECTION_TEMPLATE,
        parent_control_id: PANEL_PROMPT_LAB_TEMPLATE_HEADER_ROW,
        label: "Templates",
        initial_style: StyleId::CheckBox,
        msg: || Msg::PromptLabTemplateSectionToggled,
        view_field: |view| view.left_pane.prompt_lab.template_section_open,
        checked_slot: |state| &mut state.prev_template_checked,
    },
    PromptLabSectionCheckBoxDescriptor {
        control_id: CHK_PROMPT_LAB_SECTION_RUN_DETAILS,
        parent_control_id: PANEL_PROMPT_LAB_RUN_DETAILS_HEADER_ROW,
        label: "Run details",
        initial_style: StyleId::CheckBox,
        msg: || Msg::PromptLabRunDetailsSectionToggled,
        view_field: |view| view.left_pane.prompt_lab.run_details_section_open,
        checked_slot: |state| &mut state.prev_run_details_checked,
    },
];

#[derive(Debug, Default)]
pub(in crate::platform) struct PromptLabSectionsRenderState {
    prev_compare_checked: Option<bool>,
    prev_context_checked: Option<bool>,
    prev_template_checked: Option<bool>,
    prev_run_details_checked: Option<bool>,
}

pub(in crate::platform) fn create_controls(
    window_id: WindowId,
    commands: &mut Vec<PlatformCommand>,
) {
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
    for checkbox in CHECKBOXES {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id: checkbox.control_id,
            style_id: checkbox.initial_style,
        });
    }
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
    state: &mut PromptLabSectionsRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    for checkbox in CHECKBOXES {
        emit_if_changed(
            (checkbox.checked_slot)(state),
            (checkbox.view_field)(view),
            cmds,
            |checked| PlatformCommand::SetCheckBoxChecked {
                window_id,
                control_id: checkbox.control_id,
                checked,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn prompt_lab_section_checkbox_descriptors_are_unique() {
        let ids = CHECKBOXES
            .iter()
            .map(|checkbox| checkbox.control_id)
            .collect::<HashSet<_>>();

        assert_eq!(ids.len(), CHECKBOXES.len());
    }

    #[test]
    fn prompt_lab_section_checkbox_descriptor_count_matches_initial_scope() {
        assert_eq!(CHECKBOXES.len(), 4);
    }

    #[test]
    fn prompt_lab_section_msg_mapping_matches_current_actions() {
        assert_eq!(
            msg_for_checkbox(CHK_PROMPT_LAB_SECTION_COMPARE),
            Some(Msg::PromptLabCompareSectionToggled)
        );
        assert_eq!(
            msg_for_checkbox(CHK_PROMPT_LAB_SECTION_CONTEXT),
            Some(Msg::PromptLabContextSectionToggled)
        );
        assert_eq!(
            msg_for_checkbox(CHK_PROMPT_LAB_SECTION_TEMPLATE),
            Some(Msg::PromptLabTemplateSectionToggled)
        );
        assert_eq!(
            msg_for_checkbox(CHK_PROMPT_LAB_SECTION_RUN_DETAILS),
            Some(Msg::PromptLabRunDetailsSectionToggled)
        );
        assert_eq!(msg_for_checkbox(BTN_PROMPT_LAB_RUN), None);
    }

    #[test]
    fn prompt_lab_section_checkbox_metadata_matches_initial_layout() {
        let metadata = CHECKBOXES
            .iter()
            .map(|checkbox| {
                (
                    checkbox.control_id,
                    checkbox.parent_control_id,
                    checkbox.label,
                    checkbox.initial_style,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            metadata,
            vec![
                (
                    CHK_PROMPT_LAB_SECTION_COMPARE,
                    PANEL_PROMPT_LAB_COMPARE_HEADER_ROW,
                    "Compare",
                    StyleId::CheckBox,
                ),
                (
                    CHK_PROMPT_LAB_SECTION_CONTEXT,
                    PANEL_PROMPT_LAB_CONTEXT_HEADER_ROW,
                    "Context",
                    StyleId::CheckBox,
                ),
                (
                    CHK_PROMPT_LAB_SECTION_TEMPLATE,
                    PANEL_PROMPT_LAB_TEMPLATE_HEADER_ROW,
                    "Templates",
                    StyleId::CheckBox,
                ),
                (
                    CHK_PROMPT_LAB_SECTION_RUN_DETAILS,
                    PANEL_PROMPT_LAB_RUN_DETAILS_HEADER_ROW,
                    "Run details",
                    StyleId::CheckBox,
                ),
            ]
        );
    }
}
