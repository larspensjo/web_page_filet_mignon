use commanductui::types::{DockStyle, LabelClass, LayoutRule};
use commanductui::{Color, ControlStyle, PlatformCommand, StyleId, WindowId};
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
        control_id: PANEL_BUTTONS,
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: None,
        control_id: PANEL_INPUT,
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: None,
        control_id: PANEL_JOBS,
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: None,
        control_id: PANEL_PREVIEW,
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW),
        control_id: LABEL_PREVIEW_HEADER,
        initial_text: String::new(),
        class: LabelClass::Default,
    });

    commands.push(PlatformCommand::CreateInput {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW),
        control_id: VIEWER_PREVIEW,
        initial_text: String::new(),
        read_only: true,
        multiline: true,
        vertical_scroll: true,
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_JOBS),
        control_id: LABEL_JOBS_HEADER,
        initial_text: "Job List".to_string(),
        class: LabelClass::Default,
    });

    commands.push(PlatformCommand::CreateTreeView {
        window_id,
        parent_control_id: Some(PANEL_JOBS),
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
        parent_control_id: Some(PANEL_BUTTONS),
        control_id: BUTTON_STOP,
        text: "Stop / Finish".to_string(),
    });

    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_BUTTONS),
        control_id: BUTTON_ARCHIVE,
        text: "Archive".to_string(),
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_BOTTOM),
        control_id: LABEL_STATUS,
        initial_text: "Ready".to_string(),
        class: LabelClass::StatusBar,
    });

    // Define main window background style
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::MainWindowBackground,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x2E,
                g: 0x32,
                b: 0x39,
            }),
            ..Default::default()
        },
    });

    // Define panel background style
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::PanelBackground,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x26,
                g: 0x2A,
                b: 0x2E,
            }),
            text_color: Some(Color {
                r: 0xE0,
                g: 0xE5,
                b: 0xEC,
            }),
            ..Default::default()
        },
    });

    // Apply panel background style to panels
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PANEL_PROGRESS,
        style_id: StyleId::PanelBackground,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PANEL_INPUT,
        style_id: StyleId::PanelBackground,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PANEL_JOBS,
        style_id: StyleId::PanelBackground,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PANEL_PREVIEW,
        style_id: StyleId::PanelBackground,
    });

    // Define default label style (for status bar, headers, etc.)
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::DefaultText,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x2E,
                g: 0x32,
                b: 0x39,
            }),
            text_color: Some(Color {
                r: 0xE0,
                g: 0xE5,
                b: 0xEC,
            }),
            ..Default::default()
        },
    });

    // Define default input style (for the URL text area)
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::DefaultInput,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1A,
                g: 0x1D,
                b: 0x22,
            }),
            text_color: Some(Color {
                r: 0xE0,
                g: 0xE5,
                b: 0xEC,
            }),
            ..Default::default()
        },
    });

    // Define TreeView style
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TreeView,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x26,
                g: 0x2A,
                b: 0x2E,
            }),
            text_color: Some(Color {
                r: 0xE0,
                g: 0xE5,
                b: 0xEC,
            }),
            ..Default::default()
        },
    });

    // Define button style
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::DefaultButton,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x2E,
                g: 0x32,
                b: 0x39,
            }),
            text_color: Some(Color {
                r: 0xE0,
                g: 0xE5,
                b: 0xEC,
            }),
            ..Default::default()
        },
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_STATUS,
        style_id: StyleId::DefaultText,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_URLS,
        style_id: StyleId::DefaultInput,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: TREE_JOBS,
        style_id: StyleId::TreeView,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: BUTTON_ARCHIVE,
        style_id: StyleId::DefaultButton,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: BUTTON_STOP,
        style_id: StyleId::DefaultButton,
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
            // Buttons panel above the status bar
            LayoutRule {
                control_id: PANEL_BUTTONS,
                parent_control_id: None,
                dock_style: DockStyle::Bottom,
                order: 110,
                fixed_size: Some(44),
                margin: (0, 0, 0, 0),
            },
            // URL drop box on the left (fixed width)
            LayoutRule {
                control_id: PANEL_INPUT,
                parent_control_id: None,
                dock_style: DockStyle::Left,
                order: 200,
                fixed_size: Some(320),
                margin: (6, 6, 6, 6),
            },
            // Jobs panel fills the new left column
            LayoutRule {
                control_id: PANEL_JOBS,
                parent_control_id: None,
                dock_style: DockStyle::Left,
                order: 300,
                fixed_size: Some(280),
                margin: (6, 6, 6, 6),
            },
            // Jobs header label
            LayoutRule {
                control_id: LABEL_JOBS_HEADER,
                parent_control_id: Some(PANEL_JOBS),
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(28),
                margin: (0, 0, 4, 0),
            },
            // Jobs tree fills remaining space in panel
            LayoutRule {
                control_id: TREE_JOBS,
                parent_control_id: Some(PANEL_JOBS),
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: PANEL_PREVIEW,
                parent_control_id: None,
                dock_style: DockStyle::Fill,
                order: 310,
                fixed_size: None,
                margin: (6, 6, 6, 6),
            },
            LayoutRule {
                control_id: LABEL_PREVIEW_HEADER,
                parent_control_id: Some(PANEL_PREVIEW),
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(28),
                margin: (6, 6, 4, 0),
            },
            LayoutRule {
                control_id: VIEWER_PREVIEW,
                parent_control_id: Some(PANEL_PREVIEW),
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 0, 0, 0),
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
            // Buttons placed horizontally with fixed width
            LayoutRule {
                control_id: BUTTON_ARCHIVE,
                parent_control_id: Some(PANEL_BUTTONS),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(160),
                margin: (6, 6, 6, 6),
            },
            LayoutRule {
                control_id: BUTTON_STOP,
                parent_control_id: Some(PANEL_BUTTONS),
                dock_style: DockStyle::Left,
                order: 1,
                fixed_size: Some(160),
                margin: (6, 6, 6, 0),
            },
        ],
    });

    commands.push(PlatformCommand::SignalMainWindowUISetupComplete { window_id });
    commands.push(PlatformCommand::ShowWindow { window_id });

    commands
}
