use commanductui::types::{DockStyle, LabelClass, LayoutRule};
use commanductui::{PlatformCommand, WindowId};
use harvester_core::TOKEN_LIMIT;

use super::constants::*;

#[allow(clippy::vec_init_then_push)]
pub fn initial_commands(window_id: WindowId) -> Vec<PlatformCommand> {
    let mut commands = Vec::new();

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: None,
        control_id: PANEL_PROGRESS,
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: None,
        control_id: PANEL_BOTTOM,
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: None,
        control_id: PANEL_INPUT,
    });

    commands.push(PlatformCommand::CreateTreeView {
        window_id,
        parent_control_id: None,
        control_id: TREE_JOBS,
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PROGRESS),
        control_id: LABEL_TOKEN_PROGRESS,
        initial_text: format!("Tokens: 0 / {} (0%)", TOKEN_LIMIT),
        class: LabelClass::Default,
    });

    commands.push(PlatformCommand::CreateProgressBar {
        window_id,
        parent_control_id: Some(PANEL_PROGRESS),
        control_id: PROGRESS_TOKENS,
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_INPUT),
        control_id: LABEL_INPUT_HINT,
        initial_text: "Paste URL(s) here. Jobs are created immediately.".to_string(),
        class: LabelClass::Default,
    });

    commands.push(PlatformCommand::CreateInput {
        window_id,
        parent_control_id: Some(PANEL_INPUT),
        control_id: INPUT_URLS,
        initial_text: String::new(),
        read_only: false,
        multiline: true,
        vertical_scroll: true,
    });

    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: None,
        control_id: BUTTON_START,
        text: "Start".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: None,
        control_id: BUTTON_STOP,
        text: "Stop / Finish".to_string(),
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_BOTTOM),
        control_id: LABEL_STATUS,
        initial_text: "Ready".to_string(),
        class: LabelClass::StatusBar,
    });

    commands.push(PlatformCommand::DefineLayout {
        window_id,
        rules: vec![
            // Progress panel at the top
            LayoutRule {
                control_id: PANEL_PROGRESS,
                parent_control_id: None,
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(64),
                margin: (0, 0, 0, 0),
            },
            // Progress label and bar inside the panel
            LayoutRule {
                control_id: LABEL_TOKEN_PROGRESS,
                parent_control_id: Some(PANEL_PROGRESS),
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(22),
                margin: (8, 8, 4, 8),
            },
            LayoutRule {
                control_id: PROGRESS_TOKENS,
                parent_control_id: Some(PANEL_PROGRESS),
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 8, 8, 8),
            },
            // Status bar panel at the very bottom
            LayoutRule {
                control_id: PANEL_BOTTOM,
                parent_control_id: None,
                dock_style: DockStyle::Bottom,
                order: 100,
                fixed_size: Some(32),
                margin: (0, 0, 0, 0),
            },
            // Buttons above the status bar
            LayoutRule {
                control_id: BUTTON_START,
                parent_control_id: None,
                dock_style: DockStyle::Bottom,
                order: 110,
                fixed_size: Some(40),
                margin: (6, 6, 6, 6),
            },
            LayoutRule {
                control_id: BUTTON_STOP,
                parent_control_id: None,
                dock_style: DockStyle::Bottom,
                order: 120,
                fixed_size: Some(40),
                margin: (6, 6, 6, 6),
            },
            // Jobs tree on the right
            LayoutRule {
                control_id: TREE_JOBS,
                parent_control_id: None,
                dock_style: DockStyle::Right,
                order: 200,
                fixed_size: Some(320),
                margin: (6, 6, 6, 100),
            },
            LayoutRule {
                control_id: PANEL_INPUT,
                parent_control_id: None,
                dock_style: DockStyle::Fill,
                order: 300,
                fixed_size: None,
                margin: (6, 6, 6, 100),
            },
            // Input hint label above the text box
            LayoutRule {
                control_id: LABEL_INPUT_HINT,
                parent_control_id: Some(PANEL_INPUT),
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(28),
                margin: (0, 0, 4, 0),
            },
            // URL input fills remaining space
            LayoutRule {
                control_id: INPUT_URLS,
                parent_control_id: Some(PANEL_INPUT),
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            // Status label fills the panel
            LayoutRule {
                control_id: LABEL_STATUS,
                parent_control_id: Some(PANEL_BOTTOM),
                dock_style: DockStyle::Fill,
                order: 0,
                fixed_size: None,
                margin: (6, 6, 6, 6),
            },
        ],
    });

    commands.push(PlatformCommand::SignalMainWindowUISetupComplete { window_id });
    commands.push(PlatformCommand::ShowWindow { window_id });

    commands
}
