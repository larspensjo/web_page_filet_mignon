use commanductui::types::{DockStyle, LabelClass, LayoutRule};
use commanductui::{PlatformCommand, WindowId};

use super::constants::*;

#[allow(clippy::vec_init_then_push)]
pub fn initial_commands(window_id: WindowId) -> Vec<PlatformCommand> {
    let mut commands = Vec::new();

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: None,
        control_id: PANEL_BOTTOM,
    });

    commands.push(PlatformCommand::CreateInput {
        window_id,
        parent_control_id: None,
        control_id: INPUT_URLS,
        initial_text: String::new(),
        read_only: false,
        multiline: true,
        vertical_scroll: true,
    });

    commands.push(PlatformCommand::CreateButton {
        window_id,
        control_id: BUTTON_START,
        text: "Start".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        control_id: BUTTON_STOP,
        text: "Stop / Finish".to_string(),
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_panel_id: PANEL_BOTTOM,
        control_id: LABEL_STATUS,
        initial_text: "Ready".to_string(),
        class: LabelClass::StatusBar,
    });

    commands.push(PlatformCommand::DefineLayout {
        window_id,
        rules: vec![
            LayoutRule {
                control_id: PANEL_BOTTOM,
                parent_control_id: None,
                dock_style: DockStyle::Bottom,
                order: 0,
                fixed_size: Some(64),
                margin: (6, 6, 6, 6),
            },
            LayoutRule {
                control_id: INPUT_URLS,
                parent_control_id: None,
                dock_style: DockStyle::Fill,
                order: 10,
                fixed_size: None,
                margin: (6, 6, 70, 6),
            },
            LayoutRule {
                control_id: BUTTON_START,
                parent_control_id: Some(PANEL_BOTTOM),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(120),
                margin: (6, 6, 6, 6),
            },
            LayoutRule {
                control_id: BUTTON_STOP,
                parent_control_id: Some(PANEL_BOTTOM),
                dock_style: DockStyle::Left,
                order: 1,
                fixed_size: Some(140),
                margin: (6, 6, 6, 0),
            },
            LayoutRule {
                control_id: LABEL_STATUS,
                parent_control_id: Some(PANEL_BOTTOM),
                dock_style: DockStyle::Fill,
                order: 10,
                fixed_size: None,
                margin: (8, 8, 8, 8),
            },
        ],
    });

    commands.push(PlatformCommand::SignalMainWindowUISetupComplete { window_id });
    commands.push(PlatformCommand::ShowWindow { window_id });

    commands
}
