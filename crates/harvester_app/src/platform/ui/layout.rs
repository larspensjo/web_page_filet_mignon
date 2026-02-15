use commanductui::types::{
    DockStyle, LabelClass, LayoutRule, MenuActionId, MenuItemConfig, SplitterOrientation,
};
use commanductui::{
    Color, ControlStyle, FontDescription, FontWeight, PlatformCommand, StyleId, WindowId,
};
use harvester_core::{DEFAULT_JOBS_PANEL_WIDTH, INPUT_PANEL_FIXED_WIDTH, TOKEN_LIMIT};

use super::constants::*;

const MENU_ACTION_ADD_URL: MenuActionId = MenuActionId(1);
const MENU_ACTION_ARCHIVE: MenuActionId = MenuActionId(2);
const MENU_ACTION_PROMPT_LAB: MenuActionId = MenuActionId(3);

#[allow(clippy::vec_init_then_push)]
pub fn initial_commands(window_id: WindowId) -> Vec<PlatformCommand> {
    let mut commands = Vec::new();
    define_dark_theme_styles(&mut commands);
    let initial_left_width = DEFAULT_JOBS_PANEL_WIDTH;

    commands.push(PlatformCommand::CreateMainMenu {
        window_id,
        menu_items: vec![MenuItemConfig {
            action: None,
            text: "File".to_string(),
            children: vec![
                MenuItemConfig {
                    action: Some(MENU_ACTION_ADD_URL),
                    text: "Add URL\tCtrl+L".to_string(),
                    children: Vec::new(),
                },
                MenuItemConfig {
                    action: Some(MENU_ACTION_ARCHIVE),
                    text: "Archive".to_string(),
                    children: Vec::new(),
                },
                MenuItemConfig {
                    action: Some(MENU_ACTION_PROMPT_LAB),
                    text: "Prompt Lab".to_string(),
                    children: Vec::new(),
                },
            ],
        }],
    });

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

    // Create the vertical splitter between left panels and preview
    commands.push(PlatformCommand::CreateSplitter {
        window_id,
        parent_control_id: None,
        control_id: SPLITTER_MAIN,
        orientation: SplitterOrientation::Vertical,
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW),
        control_id: LABEL_PREVIEW_HEADER,
        initial_text: String::new(),
        class: LabelClass::Default,
    });

    commands.push(PlatformCommand::CreateRichEdit {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW),
        control_id: VIEWER_PREVIEW,
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

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_INPUT),
        control_id: PANEL_PROMPT_LAB,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_STAGE_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_SOURCE_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_INPUT_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_ACTION_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_COMPARE_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_CONTEXT_ROW,
    });
    commands.push(PlatformCommand::CreateInput {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ROW),
        control_id: INPUT_PROMPT_LAB_CONTEXT,
        initial_text: String::new(),
        read_only: false,
        multiline: true,
        vertical_scroll: true,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_TEMPLATE_USER_ROW,
    });
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: LABEL_PROMPT_LAB_STATUS,
        initial_text: String::new(),
        class: LabelClass::Default,
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
        control_id: BTN_STAGE_TRIAGE,
        text: "Triage".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
        control_id: BTN_STAGE_SUMMARY,
        text: "Summary".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
        control_id: BTN_STAGE_BRIEFING,
        text: "Briefing".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_SOURCE_ROW),
        control_id: BTN_SOURCE_FROM_TRIAGE,
        text: "From triage".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_SOURCE_ROW),
        control_id: BTN_SOURCE_TYPE_URL,
        text: "Type URL".to_string(),
    });
    commands.push(PlatformCommand::CreateInput {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_INPUT_ROW),
        control_id: INPUT_PROMPT_LAB_URL,
        initial_text: String::new(),
        read_only: false,
        multiline: false,
        vertical_scroll: false,
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_INPUT_ROW),
        control_id: BTN_PROMPT_LAB_RESOLVE,
        text: "Resolve".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_RUN,
        text: "Run".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_RERUN,
        text: "Rerun".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_CLEAR,
        text: "Clear".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
        control_id: BTN_COMPARE_ADD_CURRENT,
        text: "Add current settings".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
        control_id: BTN_COMPARE_ADD_BASELINE,
        text: "Add baseline".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
        control_id: BTN_COMPARE_RESET_DRAFT,
        text: "Reset draft".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
        control_id: BTN_COMPARE_START,
        text: "Start compare".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
        control_id: BTN_COMPARE_CANCEL,
        text: "Cancel compare".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
        control_id: BTN_COMPARE_AUTO_SELECT,
        text: "Auto-select".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
        control_id: BTN_COMPARE_WINNER_CLEAR,
        text: "Clear winner".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_CONTEXT_APPLY,
        text: "Apply".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN,
        text: "Apply+Run".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_CONTEXT_REVERT,
        text: "Revert".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_CONTEXT_SAVE,
        text: "Save".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_CONTEXT_RELOAD,
        text: "Reload".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_TEMPLATE_OPEN,
        text: "Edit".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY,
        text: "Apply".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN,
        text: "Apply+Run".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_TEMPLATE_REVERT,
        text: "Revert".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
        control_id: BTN_PROMPT_LAB_TEMPLATE_SAVE,
        text: "Save".to_string(),
    });
    commands.push(PlatformCommand::CreateInput {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW),
        control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
        initial_text: String::new(),
        read_only: false,
        multiline: true,
        vertical_scroll: true,
    });
    commands.push(PlatformCommand::CreateInput {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_USER_ROW),
        control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
        initial_text: String::new(),
        read_only: false,
        multiline: true,
        vertical_scroll: true,
    });
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: LABEL_PROMPT_LAB_METADATA,
        initial_text: String::new(),
        class: LabelClass::Default,
    });
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: LABEL_PROMPT_LAB_CONTEXT_STATUS,
        initial_text: String::new(),
        class: LabelClass::Default,
    });
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: LABEL_PROMPT_LAB_TEMPLATE_STATUS,
        initial_text: String::new(),
        class: LabelClass::Default,
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
        control_id: BUTTON_BRIEFING,
        text: "Generate Briefing".to_string(),
    });

    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_BUTTONS),
        control_id: BUTTON_TRIAGE,
        text: "Triage Articles".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_BUTTONS),
        control_id: BUTTON_POLL_SOURCES,
        text: "Poll Sources".to_string(),
    });

    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_BUTTONS),
        control_id: BUTTON_OPEN_BROWSER,
        text: "Open in Browser".to_string(),
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_BOTTOM),
        control_id: LABEL_STATUS,
        initial_text: "Ready".to_string(),
        class: LabelClass::StatusBar,
    });

    apply_dark_theme(window_id, &mut commands);

    commands.push(build_layout_command(
        window_id,
        initial_left_width,
        false,
        false,
        false,
    ));

    commands.push(PlatformCommand::SignalMainWindowUISetupComplete { window_id });
    commands.push(PlatformCommand::ShowWindow { window_id });

    commands
}

fn define_dark_theme_styles(commands: &mut Vec<PlatformCommand>) {
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

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::StatusBarBackground,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x2E,
                g: 0x32,
                b: 0x39,
            }),
            text_color: Some(Color {
                r: 0x80,
                g: 0x90,
                b: 0xA0,
            }),
            ..Default::default()
        },
    });

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

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::HeaderLabel,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x26,
                g: 0x2A,
                b: 0x2E,
            }),
            text_color: Some(Color {
                r: 0xFF,
                g: 0xB3,
                b: 0x47,
            }),
            ..Default::default()
        },
    });

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

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ViewerMonospace,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1A,
                g: 0x1D,
                b: 0x22,
            }),
            text_color: Some(Color {
                r: 0x00,
                g: 0xC9,
                b: 0xFF,
            }),
            font: Some(FontDescription {
                name: Some("Cascadia Code".to_string()),
                size: Some(10),
                weight: Some(FontWeight::Normal),
            }),
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ViewerReadable,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1A,
                g: 0x1D,
                b: 0x22,
            }),
            text_color: Some(Color {
                r: 0xD8,
                g: 0xDE,
                b: 0xE9,
            }),
            font: Some(FontDescription {
                name: Some("Segoe UI".to_string()),
                size: Some(10),
                weight: Some(FontWeight::Normal),
            }),
        },
    });

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ProgressBar,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1A,
                g: 0x1D,
                b: 0x22,
            }),
            text_color: Some(Color {
                r: 0x00,
                g: 0xC9,
                b: 0xFF,
            }),
            ..Default::default()
        },
    });

    // Splitter control style: neutral gray matching the theme
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::Splitter,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x40,
                g: 0x44,
                b: 0x4B,
            }),
            ..Default::default()
        },
    });

    // Muted gray for tree items without summaries
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TreeItemDisabled,
        style: ControlStyle {
            text_color: Some(Color {
                r: 0x60,
                g: 0x65,
                b: 0x6B,
            }),
            ..Default::default()
        },
    });
}

pub(crate) fn build_layout_command(
    window_id: WindowId,
    left_panel_width: i32,
    input_panel_visible: bool,
    prompt_lab_visible: bool,
    template_editor_open: bool,
) -> PlatformCommand {
    PlatformCommand::DefineLayout {
        window_id,
        rules: build_layout_rules(
            left_panel_width,
            input_panel_visible,
            prompt_lab_visible,
            template_editor_open,
        ),
    }
}

fn build_layout_rules(
    left_panel_width: i32,
    input_panel_visible: bool,
    prompt_lab_visible: bool,
    template_editor_open: bool,
) -> Vec<LayoutRule> {
    let input_width = if input_panel_visible {
        INPUT_PANEL_FIXED_WIDTH
    } else {
        0
    };
    let jobs_width = (left_panel_width - input_width).max(0);
    let prompt_lab_height = if prompt_lab_visible {
        if template_editor_open {
            750
        } else {
            420
        }
    } else {
        56
    };
    let mut rules = vec![
        LayoutRule {
            control_id: PANEL_PROGRESS,
            parent_control_id: None,
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(64),
            margin: (0, 0, 0, 0),
        },
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
        LayoutRule {
            control_id: PANEL_BOTTOM,
            parent_control_id: None,
            dock_style: DockStyle::Bottom,
            order: 100,
            fixed_size: Some(32),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: PANEL_BUTTONS,
            parent_control_id: None,
            dock_style: DockStyle::Bottom,
            order: 110,
            fixed_size: Some(44),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: PANEL_INPUT,
            parent_control_id: None,
            dock_style: DockStyle::Left,
            order: 200,
            fixed_size: Some(input_width),
            margin: (6, 6, 6, 6),
        },
        LayoutRule {
            control_id: PANEL_JOBS,
            parent_control_id: None,
            dock_style: DockStyle::Left,
            order: 300,
            fixed_size: Some(jobs_width),
            margin: (6, 6, 6, 6),
        },
        LayoutRule {
            control_id: LABEL_JOBS_HEADER,
            parent_control_id: Some(PANEL_JOBS),
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(28),
            margin: (0, 0, 4, 0),
        },
        LayoutRule {
            control_id: TREE_JOBS,
            parent_control_id: Some(PANEL_JOBS),
            dock_style: DockStyle::Fill,
            order: 1,
            fixed_size: None,
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: SPLITTER_MAIN,
            parent_control_id: None,
            dock_style: DockStyle::Left,
            order: 305,
            fixed_size: Some(4),
            margin: (6, 0, 6, 0),
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
        LayoutRule {
            control_id: LABEL_INPUT_HINT,
            parent_control_id: Some(PANEL_INPUT),
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(28),
            margin: (0, 0, 4, 0),
        },
        LayoutRule {
            control_id: INPUT_URLS,
            parent_control_id: Some(PANEL_INPUT),
            dock_style: DockStyle::Fill,
            order: 1,
            fixed_size: None,
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: PANEL_PROMPT_LAB,
            parent_control_id: Some(PANEL_INPUT),
            dock_style: DockStyle::Bottom,
            order: 2,
            fixed_size: Some(prompt_lab_height),
            margin: (0, 6, 0, 6),
        },
        LayoutRule {
            control_id: LABEL_PROMPT_LAB_STATUS,
            parent_control_id: Some(PANEL_PROMPT_LAB),
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(24),
            margin: (0, 0, 2, 0),
        },
        LayoutRule {
            control_id: LABEL_STATUS,
            parent_control_id: Some(PANEL_BOTTOM),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (6, 6, 6, 6),
        },
        LayoutRule {
            control_id: BUTTON_STOP,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 0,
            fixed_size: Some(160),
            margin: (6, 6, 6, 0),
        },
        LayoutRule {
            control_id: BUTTON_BRIEFING,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 1,
            fixed_size: Some(160),
            margin: (6, 6, 6, 0),
        },
        LayoutRule {
            control_id: BUTTON_TRIAGE,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 2,
            fixed_size: Some(160),
            margin: (6, 6, 6, 0),
        },
        LayoutRule {
            control_id: BUTTON_POLL_SOURCES,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 3,
            fixed_size: Some(160),
            margin: (6, 6, 6, 0),
        },
        LayoutRule {
            control_id: BUTTON_OPEN_BROWSER,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 4,
            fixed_size: Some(160),
            margin: (6, 6, 6, 0),
        },
    ];

    if prompt_lab_visible {
        rules.extend([
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_STAGE_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 2,
                fixed_size: Some(30),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: BTN_STAGE_TRIAGE,
                parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(66),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_STAGE_SUMMARY,
                parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
                dock_style: DockStyle::Left,
                order: 1,
                fixed_size: Some(66),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_STAGE_BRIEFING,
                parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
                dock_style: DockStyle::Fill,
                order: 2,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_SOURCE_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 3,
                fixed_size: Some(30),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: BTN_SOURCE_FROM_TRIAGE,
                parent_control_id: Some(PANEL_PROMPT_LAB_SOURCE_ROW),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(100),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_SOURCE_TYPE_URL,
                parent_control_id: Some(PANEL_PROMPT_LAB_SOURCE_ROW),
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_INPUT_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 4,
                fixed_size: Some(30),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_RESOLVE,
                parent_control_id: Some(PANEL_PROMPT_LAB_INPUT_ROW),
                dock_style: DockStyle::Right,
                order: 1,
                fixed_size: Some(60),
                margin: (0, 0, 0, 4),
            },
            LayoutRule {
                control_id: INPUT_PROMPT_LAB_URL,
                parent_control_id: Some(PANEL_PROMPT_LAB_INPUT_ROW),
                dock_style: DockStyle::Fill,
                order: 0,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_ACTION_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 5,
                fixed_size: Some(30),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_RUN,
                parent_control_id: Some(PANEL_PROMPT_LAB_ACTION_ROW),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(44),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_RERUN,
                parent_control_id: Some(PANEL_PROMPT_LAB_ACTION_ROW),
                dock_style: DockStyle::Left,
                order: 1,
                fixed_size: Some(54),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_CLEAR,
                parent_control_id: Some(PANEL_PROMPT_LAB_ACTION_ROW),
                dock_style: DockStyle::Fill,
                order: 2,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_COMPARE_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 6,
                fixed_size: Some(30),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: BTN_COMPARE_ADD_CURRENT,
                parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(114),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_COMPARE_ADD_BASELINE,
                parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                dock_style: DockStyle::Left,
                order: 1,
                fixed_size: Some(88),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_COMPARE_RESET_DRAFT,
                parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                dock_style: DockStyle::Left,
                order: 2,
                fixed_size: Some(82),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_COMPARE_START,
                parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                dock_style: DockStyle::Left,
                order: 3,
                fixed_size: Some(86),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_COMPARE_CANCEL,
                parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                dock_style: DockStyle::Left,
                order: 4,
                fixed_size: Some(94),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_COMPARE_AUTO_SELECT,
                parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                dock_style: DockStyle::Left,
                order: 5,
                fixed_size: Some(78),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_COMPARE_WINNER_CLEAR,
                parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                dock_style: DockStyle::Fill,
                order: 6,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_CONTEXT_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 7,
                fixed_size: Some(150),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: INPUT_PROMPT_LAB_CONTEXT,
                parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ROW),
                dock_style: DockStyle::Fill,
                order: 0,
                fixed_size: None,
                margin: (4, 4, 4, 4),
            },
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 8,
                fixed_size: Some(32),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_CONTEXT_APPLY,
                parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(48),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN,
                parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW),
                dock_style: DockStyle::Left,
                order: 1,
                fixed_size: Some(74),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_CONTEXT_REVERT,
                parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW),
                dock_style: DockStyle::Left,
                order: 2,
                fixed_size: Some(58),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_CONTEXT_SAVE,
                parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW),
                dock_style: DockStyle::Left,
                order: 3,
                fixed_size: Some(48),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_CONTEXT_RELOAD,
                parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW),
                dock_style: DockStyle::Fill,
                order: 4,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: LABEL_PROMPT_LAB_CONTEXT_STATUS,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 9,
                fixed_size: Some(24),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 10,
                fixed_size: Some(32),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_TEMPLATE_OPEN,
                parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(50),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY,
                parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
                dock_style: DockStyle::Left,
                order: 1,
                fixed_size: Some(50),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN,
                parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
                dock_style: DockStyle::Left,
                order: 2,
                fixed_size: Some(74),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_TEMPLATE_REVERT,
                parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
                dock_style: DockStyle::Left,
                order: 3,
                fixed_size: Some(58),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_TEMPLATE_SAVE,
                parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
                dock_style: DockStyle::Fill,
                order: 4,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            LayoutRule {
                control_id: LABEL_PROMPT_LAB_TEMPLATE_STATUS,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 11,
                fixed_size: Some(24),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: LABEL_PROMPT_LAB_METADATA,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Fill,
                order: 14,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
        ]);

        if template_editor_open {
            rules.extend([
                LayoutRule {
                    control_id: PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW,
                    parent_control_id: Some(PANEL_PROMPT_LAB),
                    dock_style: DockStyle::Top,
                    order: 12,
                    fixed_size: Some(120),
                    margin: (0, 0, 2, 0),
                },
                LayoutRule {
                    control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
                    parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW),
                    dock_style: DockStyle::Fill,
                    order: 0,
                    fixed_size: None,
                    margin: (4, 4, 4, 4),
                },
                LayoutRule {
                    control_id: PANEL_PROMPT_LAB_TEMPLATE_USER_ROW,
                    parent_control_id: Some(PANEL_PROMPT_LAB),
                    dock_style: DockStyle::Top,
                    order: 13,
                    fixed_size: Some(120),
                    margin: (0, 0, 2, 0),
                },
                LayoutRule {
                    control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
                    parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_USER_ROW),
                    dock_style: DockStyle::Fill,
                    order: 0,
                    fixed_size: None,
                    margin: (4, 4, 4, 4),
                },
            ]);
        }
    }

    rules
}

fn apply_dark_theme(window_id: WindowId, commands: &mut Vec<PlatformCommand>) {
    for control_id in [
        PANEL_PROGRESS,
        PANEL_BUTTONS,
        PANEL_INPUT,
        PANEL_JOBS,
        PANEL_PREVIEW,
        PANEL_PROMPT_LAB,
        PANEL_PROMPT_LAB_STAGE_ROW,
        PANEL_PROMPT_LAB_SOURCE_ROW,
        PANEL_PROMPT_LAB_INPUT_ROW,
        PANEL_PROMPT_LAB_ACTION_ROW,
        PANEL_PROMPT_LAB_COMPARE_ROW,
        PANEL_PROMPT_LAB_CONTEXT_ROW,
        PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW,
        PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW,
        PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW,
        PANEL_PROMPT_LAB_TEMPLATE_USER_ROW,
    ] {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id,
            style_id: StyleId::PanelBackground,
        });
    }

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PANEL_BOTTOM,
        style_id: StyleId::StatusBarBackground,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_PREVIEW_HEADER,
        style_id: StyleId::HeaderLabel,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_JOBS_HEADER,
        style_id: StyleId::HeaderLabel,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_TOKEN_PROGRESS,
        style_id: StyleId::HeaderLabel,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_INPUT_HINT,
        style_id: StyleId::HeaderLabel,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_STATUS,
        style_id: StyleId::StatusBarBackground,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_PROMPT_LAB_STATUS,
        style_id: StyleId::HeaderLabel,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_PROMPT_LAB_METADATA,
        style_id: StyleId::DefaultText,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_URLS,
        style_id: StyleId::DefaultInput,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_PROMPT_LAB_URL,
        style_id: StyleId::DefaultInput,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: VIEWER_PREVIEW,
        style_id: StyleId::ViewerReadable,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: BUTTON_STOP,
        style_id: StyleId::DefaultButton,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: BUTTON_BRIEFING,
        style_id: StyleId::DefaultButton,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: BUTTON_TRIAGE,
        style_id: StyleId::DefaultButton,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: BUTTON_POLL_SOURCES,
        style_id: StyleId::DefaultButton,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: BUTTON_OPEN_BROWSER,
        style_id: StyleId::DefaultButton,
    });
    for control_id in [
        BTN_STAGE_TRIAGE,
        BTN_STAGE_SUMMARY,
        BTN_STAGE_BRIEFING,
        BTN_SOURCE_FROM_TRIAGE,
        BTN_SOURCE_TYPE_URL,
        BTN_PROMPT_LAB_RESOLVE,
        BTN_PROMPT_LAB_RUN,
        BTN_PROMPT_LAB_RERUN,
        BTN_PROMPT_LAB_CLEAR,
        BTN_PROMPT_LAB_CONTEXT_APPLY,
        BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN,
        BTN_PROMPT_LAB_CONTEXT_REVERT,
        BTN_PROMPT_LAB_CONTEXT_SAVE,
        BTN_PROMPT_LAB_CONTEXT_RELOAD,
        BTN_PROMPT_LAB_TEMPLATE_OPEN,
        BTN_PROMPT_LAB_TEMPLATE_APPLY,
        BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN,
        BTN_PROMPT_LAB_TEMPLATE_REVERT,
        BTN_PROMPT_LAB_TEMPLATE_SAVE,
        BTN_COMPARE_ADD_CURRENT,
        BTN_COMPARE_ADD_BASELINE,
        BTN_COMPARE_RESET_DRAFT,
        BTN_COMPARE_START,
        BTN_COMPARE_CANCEL,
        BTN_COMPARE_AUTO_SELECT,
        BTN_COMPARE_WINNER_CLEAR,
    ] {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id,
            style_id: StyleId::DefaultButton,
        });
    }

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_PROMPT_LAB_CONTEXT,
        style_id: StyleId::DefaultInput,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
        style_id: StyleId::DefaultInput,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
        style_id: StyleId::DefaultInput,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: TREE_JOBS,
        style_id: StyleId::TreeView,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: PROGRESS_TOKENS,
        style_id: StyleId::ProgressBar,
    });

    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: SPLITTER_MAIN,
        style_id: StyleId::Splitter,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_control_uses_create_rich_edit() {
        let commands = initial_commands(WindowId::new(1));
        assert!(commands.iter().any(|cmd| matches!(
            cmd,
            PlatformCommand::CreateRichEdit { control_id, .. } if *control_id == VIEWER_PREVIEW
        )));
        assert!(!commands.iter().any(|cmd| matches!(
            cmd,
            PlatformCommand::CreateInput { control_id, .. } if *control_id == VIEWER_PREVIEW
        )));
    }

    #[test]
    fn new_controls_created_in_initial_commands() {
        let commands = initial_commands(WindowId::new(2));
        assert!(commands.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::CreatePanel { control_id, .. } if *control_id == PANEL_PROMPT_LAB
            )
        }));
        assert!(commands.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::CreateButton { control_id, .. } if *control_id == BTN_PROMPT_LAB_RUN
            )
        }));
        assert!(commands.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::CreateInput { control_id, .. } if *control_id == INPUT_PROMPT_LAB_URL
            )
        }));
        assert!(commands.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::CreateButton { control_id, .. } if *control_id == BTN_COMPARE_ADD_CURRENT
            )
        }));
        assert!(commands.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::CreateButton { control_id, .. } if *control_id == BTN_COMPARE_START
            )
        }));
    }

    #[test]
    fn collapsed_layout_height_is_minimal() {
        let cmd = build_layout_command(WindowId::new(3), 600, true, false, false);
        let rules = match cmd {
            PlatformCommand::DefineLayout { rules, .. } => rules,
            _ => panic!("expected DefineLayout"),
        };
        let panel = rules
            .iter()
            .find(|r| r.control_id == PANEL_PROMPT_LAB)
            .expect("prompt lab panel rule");
        assert_eq!(panel.fixed_size, Some(56));
        assert!(!rules
            .iter()
            .any(|r| r.control_id == PANEL_PROMPT_LAB_STAGE_ROW));
    }

    #[test]
    fn expanded_layout_includes_all_controls() {
        let cmd = build_layout_command(WindowId::new(4), 600, true, true, true);
        let rules = match cmd {
            PlatformCommand::DefineLayout { rules, .. } => rules,
            _ => panic!("expected DefineLayout"),
        };
        assert!(rules
            .iter()
            .any(|r| r.control_id == PANEL_PROMPT_LAB_STAGE_ROW));
        assert!(rules
            .iter()
            .any(|r| r.control_id == PANEL_PROMPT_LAB_SOURCE_ROW));
        assert!(rules
            .iter()
            .any(|r| r.control_id == PANEL_PROMPT_LAB_INPUT_ROW));
        assert!(rules
            .iter()
            .any(|r| r.control_id == PANEL_PROMPT_LAB_ACTION_ROW));
        assert!(rules
            .iter()
            .any(|r| r.control_id == PANEL_PROMPT_LAB_CONTEXT_ROW));
        assert!(rules
            .iter()
            .any(|r| r.control_id == PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW));
        assert!(rules
            .iter()
            .any(|r| r.control_id == LABEL_PROMPT_LAB_CONTEXT_STATUS));
    }
}
