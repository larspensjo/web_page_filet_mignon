use commanductui::types::{ControlId, DockStyle, LayoutRule};
use commanductui::{PlatformCommand, StyleId, WindowId};
use harvester_core::{AppViewModel, Msg};

use crate::platform::ui::constants::{
    BUTTON_ARCHIVE, BUTTON_BRIEFING, BUTTON_POLL_SOURCES, BUTTON_STOP, BUTTON_SUMMARIZE,
    BUTTON_TRIAGE, PANEL_BUTTONS,
};
use crate::platform::ui::render::emit_if_changed;

pub(in crate::platform) const FOOTER_BUTTON_VERTICAL_MARGIN: (i32, i32) = (0, 6);

#[derive(Debug, Clone, Copy)]
struct BottomButtonDescriptor {
    control_id: ControlId,
    label: &'static str,
    order: u32,
    width: i32,
    margin: (i32, i32, i32, i32),
    initial_style: StyleId,
    msg: fn() -> Msg,
}

const fn footer_button_margin(left: i32, right: i32) -> (i32, i32, i32, i32) {
    (
        FOOTER_BUTTON_VERTICAL_MARGIN.0,
        right,
        FOOTER_BUTTON_VERTICAL_MARGIN.1,
        left,
    )
}

const BUTTONS: &[BottomButtonDescriptor] = &[
    BottomButtonDescriptor {
        control_id: BUTTON_STOP,
        label: "Stop / Finish",
        order: 0,
        width: 144,
        margin: footer_button_margin(14, 22),
        initial_style: StyleId::SecondaryButton,
        msg: || Msg::StopFinishClicked,
    },
    BottomButtonDescriptor {
        control_id: BUTTON_POLL_SOURCES,
        label: "Poll sources",
        order: 1,
        width: 144,
        margin: footer_button_margin(6, 6),
        initial_style: StyleId::PrimaryButton,
        msg: || Msg::PollSourcesClicked,
    },
    BottomButtonDescriptor {
        control_id: BUTTON_TRIAGE,
        label: "Run Triage",
        order: 2,
        width: 144,
        margin: footer_button_margin(6, 6),
        initial_style: StyleId::SecondaryButton,
        msg: || Msg::TriageClicked,
    },
    BottomButtonDescriptor {
        control_id: BUTTON_SUMMARIZE,
        label: "Summarize Articles",
        order: 3,
        width: 144,
        margin: footer_button_margin(6, 6),
        initial_style: StyleId::SecondaryButton,
        msg: || Msg::PrepareSummariesClicked,
    },
    BottomButtonDescriptor {
        control_id: BUTTON_BRIEFING,
        label: "Generate Briefing",
        order: 4,
        width: 168,
        margin: footer_button_margin(6, 6),
        initial_style: StyleId::SecondaryButton,
        msg: || Msg::GenerateBriefingClicked,
    },
    BottomButtonDescriptor {
        control_id: BUTTON_ARCHIVE,
        label: "Archive",
        order: 5,
        width: 112,
        margin: footer_button_margin(6, 6),
        initial_style: StyleId::SecondaryButton,
        msg: || Msg::ArchiveClicked,
    },
];

#[derive(Debug, Default)]
pub(in crate::platform) struct BottomButtonsRenderState {
    prev_stop_enabled: Option<bool>,
    prev_stop_style: Option<StyleId>,
    prev_briefing_enabled: Option<bool>,
    prev_summarize_enabled: Option<bool>,
    prev_triage_enabled: Option<bool>,
    prev_poll_enabled: Option<bool>,
}

pub(in crate::platform) fn create_controls(
    window_id: WindowId,
    commands: &mut Vec<PlatformCommand>,
) {
    for button in BUTTONS {
        commands.push(PlatformCommand::CreateButton {
            window_id,
            parent_control_id: Some(PANEL_BUTTONS),
            control_id: button.control_id,
            text: button.label.to_string(),
        });
    }
}

pub(in crate::platform) fn layout_rules() -> Vec<LayoutRule> {
    BUTTONS
        .iter()
        .map(|button| LayoutRule {
            control_id: button.control_id,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: button.order,
            fixed_size: Some(button.width),
            margin: button.margin,
        })
        .collect()
}

pub(in crate::platform) fn apply_theme(window_id: WindowId, commands: &mut Vec<PlatformCommand>) {
    for button in BUTTONS {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id: button.control_id,
            style_id: button.initial_style,
        });
    }
}

pub(in crate::platform) fn render(
    window_id: WindowId,
    view: &AppViewModel,
    state: &mut BottomButtonsRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    emit_if_changed(
        &mut state.prev_stop_style,
        if view.stop_finish_button.is_enabled() {
            StyleId::DestructiveButton
        } else {
            StyleId::SecondaryButton
        },
        cmds,
        |style_id| PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id: BUTTON_STOP,
            style_id,
        },
    );
    emit_if_changed(
        &mut state.prev_stop_enabled,
        view.stop_finish_button.is_enabled(),
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_STOP,
            enabled,
        },
    );
    emit_if_changed(
        &mut state.prev_briefing_enabled,
        view.briefing_can_start,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_BRIEFING,
            enabled,
        },
    );
    emit_if_changed(
        &mut state.prev_summarize_enabled,
        view.briefing_can_start,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_SUMMARIZE,
            enabled,
        },
    );
    emit_if_changed(
        &mut state.prev_triage_enabled,
        view.triage_can_start,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_TRIAGE,
            enabled,
        },
    );
    emit_if_changed(
        &mut state.prev_poll_enabled,
        view.poll_sources_enabled,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_POLL_SOURCES,
            enabled,
        },
    );
    // BUTTON_ARCHIVE intentionally has no dynamic enablement today.
}

pub(in crate::platform) fn msg_for_control(control_id: ControlId) -> Option<Msg> {
    BUTTONS
        .iter()
        .find(|button| button.control_id == control_id)
        .map(|button| (button.msg)())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use commanductui::StyleId;

    use super::*;

    #[test]
    fn bottom_button_descriptors_are_unique() {
        let ids = BUTTONS
            .iter()
            .map(|button| button.control_id)
            .collect::<HashSet<_>>();
        let orders = BUTTONS
            .iter()
            .map(|button| button.order)
            .collect::<HashSet<_>>();

        assert_eq!(ids.len(), BUTTONS.len());
        assert_eq!(orders.len(), BUTTONS.len());
    }

    #[test]
    fn bottom_button_descriptors_capture_current_order_sizes_and_styles() {
        let actual: Vec<_> = BUTTONS
            .iter()
            .map(|button| {
                (
                    button.control_id,
                    button.order,
                    button.width,
                    button.margin,
                    button.initial_style,
                )
            })
            .collect();

        assert_eq!(
            actual,
            vec![
                (
                    BUTTON_STOP,
                    0,
                    144,
                    (0, 22, 6, 14),
                    StyleId::SecondaryButton
                ),
                (
                    BUTTON_POLL_SOURCES,
                    1,
                    144,
                    (0, 6, 6, 6),
                    StyleId::PrimaryButton
                ),
                (
                    BUTTON_TRIAGE,
                    2,
                    144,
                    (0, 6, 6, 6),
                    StyleId::SecondaryButton
                ),
                (
                    BUTTON_SUMMARIZE,
                    3,
                    144,
                    (0, 6, 6, 6),
                    StyleId::SecondaryButton
                ),
                (
                    BUTTON_BRIEFING,
                    4,
                    168,
                    (0, 6, 6, 6),
                    StyleId::SecondaryButton
                ),
                (
                    BUTTON_ARCHIVE,
                    5,
                    112,
                    (0, 6, 6, 6),
                    StyleId::SecondaryButton
                ),
            ]
        );
    }

    #[test]
    fn bottom_button_msg_mapping_matches_current_actions() {
        assert_eq!(msg_for_control(BUTTON_STOP), Some(Msg::StopFinishClicked));
        assert_eq!(
            msg_for_control(BUTTON_POLL_SOURCES),
            Some(Msg::PollSourcesClicked)
        );
        assert_eq!(msg_for_control(BUTTON_TRIAGE), Some(Msg::TriageClicked));
        assert_eq!(
            msg_for_control(BUTTON_SUMMARIZE),
            Some(Msg::PrepareSummariesClicked)
        );
        assert_eq!(
            msg_for_control(BUTTON_BRIEFING),
            Some(Msg::GenerateBriefingClicked)
        );
        assert_eq!(msg_for_control(BUTTON_ARCHIVE), Some(Msg::ArchiveClicked));
    }
}
