use commanductui::types::{
    ControlId, DockStyle, LabelClass, LayoutRule, MenuActionId, MenuItemConfig, SplitterOrientation,
};
use commanductui::{
    Color, ControlStyle, FontDescription, FontWeight, PlatformCommand, StyleId, WindowId,
};
use harvester_core::{
    AppTab, LeftTab, DEFAULT_JOBS_PANEL_WIDTH, INPUT_PANEL_FIXED_WIDTH, TOKEN_LIMIT,
};

use super::constants::*;

const MENU_ACTION_ADD_URL: MenuActionId = MenuActionId(1);
const MENU_ACTION_ARCHIVE: MenuActionId = MenuActionId(2);
const MENU_ACTION_PROMPT_LAB: MenuActionId = MenuActionId(3);

const PROMPT_LAB_ROW_HEIGHT_STANDARD: i32 = 26;
const PROMPT_LAB_ROW_HEIGHT_ACTION: i32 = 28;
const PROMPT_LAB_ROW_HEIGHT_CONTEXT_INPUT: i32 = 150;
const PROMPT_LAB_ROW_HEIGHT_STATUS: i32 = 24;
const PROMPT_LAB_ROW_HEIGHT_TEMPLATE_EDITOR_INPUT: i32 = 120;
const PROMPT_LAB_ROW_HEIGHT_RUN_DETAILS_BODY: i32 = 42;
const PROMPT_LAB_TEMPLATE_TOGGLE_BUTTON_WIDTH: i32 = 120;

#[derive(Debug, Clone)]
pub(crate) struct PromptLabLayoutConfig {
    pub visible: bool,
    pub advanced_mode: bool,
    pub compare_section_open: bool,
    pub context_section_open: bool,
    pub template_section_open: bool,
    pub run_details_section_open: bool,
    pub template_editor_open: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct LayoutConfig {
    pub left_panel_width: i32,
    pub input_panel_visible: bool,
    pub prompt_lab: PromptLabLayoutConfig,
    pub active_tab: AppTab,
    pub left_tab: LeftTab,
}

#[derive(Debug, Clone, Copy)]
struct PromptLabVisibility {
    show_advanced: bool,
    show_compare_row: bool,
    show_context_row: bool,
    show_template_row: bool,
    show_template_editor_rows: bool,
    show_run_details_row: bool,
}

fn compute_prompt_lab_visibility(prompt_lab: &PromptLabLayoutConfig) -> PromptLabVisibility {
    let visible = prompt_lab.visible;
    let show_advanced = visible && prompt_lab.advanced_mode;
    let show_compare_row = show_advanced && prompt_lab.compare_section_open;
    let show_context_row = show_advanced && prompt_lab.context_section_open;
    let show_template_row = show_advanced && prompt_lab.template_section_open;
    let show_template_editor_rows = show_template_row && prompt_lab.template_editor_open;
    let show_run_details_row = show_advanced && prompt_lab.run_details_section_open;

    PromptLabVisibility {
        show_advanced,
        show_compare_row,
        show_context_row,
        show_template_row,
        show_template_editor_rows,
        show_run_details_row,
    }
}

fn collapsed_top_rule(
    control_id: ControlId,
    parent_control_id: ControlId,
    order: u32,
) -> LayoutRule {
    LayoutRule {
        control_id,
        parent_control_id: Some(parent_control_id),
        dock_style: DockStyle::Top,
        order,
        fixed_size: Some(0),
        margin: (0, 0, 0, 0),
    }
}

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

    // Left-pane container: tab bar at top + two swappable content panels.
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: None,
        control_id: PANEL_LEFT,
    });
    commands.push(PlatformCommand::CreateTabBar {
        window_id,
        control_id: TAB_BAR_LEFT,
        parent_control_id: Some(PANEL_LEFT),
        items: vec![
            "Jobs".to_string(),
            "Triage Review".to_string(),
            "Triage Results".to_string(),
            "Prompt Lab".to_string(),
        ],
    });
    commands.push(PlatformCommand::SetTabBarStyle {
        window_id,
        control_id: TAB_BAR_LEFT,
        background_color: Color {
            r: 0x2E,
            g: 0x32,
            b: 0x39,
        },
        text_color: Color {
            r: 0xE0,
            g: 0xE5,
            b: 0xEC,
        },
        accent_color: Color {
            r: 0x00,
            g: 0x80,
            b: 0xFF,
        },
        font: None,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_LEFT),
        control_id: PANEL_LEFT_JOBS,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_LEFT),
        control_id: PANEL_LEFT_PROMPT_LAB,
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_LEFT_JOBS),
        control_id: PANEL_INPUT,
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_LEFT_JOBS),
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

    // Right-pane tab bar (custom TabBar widget).
    commands.push(PlatformCommand::CreateTabBar {
        window_id,
        control_id: TAB_BAR_RIGHT,
        parent_control_id: Some(PANEL_PREVIEW),
        items: vec![
            "Triage".to_string(),
            "Summary".to_string(),
            "Briefing".to_string(),
            "Trends".to_string(),
        ],
    });
    commands.push(PlatformCommand::SetTabBarStyle {
        window_id,
        control_id: TAB_BAR_RIGHT,
        background_color: Color {
            r: 0x2E,
            g: 0x32,
            b: 0x39,
        },
        text_color: Color {
            r: 0xE0,
            g: 0xE5,
            b: 0xEC,
        },
        accent_color: Color {
            r: 0x00,
            g: 0x80,
            b: 0xFF,
        },
        font: None,
    });

    // Tab content panels — all created at startup; inactive ones are collapsed.
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW),
        control_id: PANEL_TAB_TRIAGE,
    });
    commands.push(PlatformCommand::CreateRichEdit {
        window_id,
        parent_control_id: Some(PANEL_TAB_TRIAGE),
        control_id: VIEWER_TRIAGE,
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW),
        control_id: PANEL_TAB_SUMMARY,
    });
    commands.push(PlatformCommand::CreateRichEdit {
        window_id,
        parent_control_id: Some(PANEL_TAB_SUMMARY),
        control_id: VIEWER_PREVIEW,
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW),
        control_id: PANEL_TAB_BRIEFING,
    });
    commands.push(PlatformCommand::CreateRichEdit {
        window_id,
        parent_control_id: Some(PANEL_TAB_BRIEFING),
        control_id: VIEWER_BRIEFING,
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW),
        control_id: PANEL_TAB_TRENDS,
    });
    // Trend-category selector bar (custom TabBar widget).
    commands.push(PlatformCommand::CreateTabBar {
        window_id,
        control_id: TAB_BAR_TRENDS,
        parent_control_id: Some(PANEL_TAB_TRENDS),
        items: vec![
            "Companies".to_string(),
            "Technologies".to_string(),
            "Products".to_string(),
            "Themes".to_string(),
        ],
    });
    commands.push(PlatformCommand::SetTabBarStyle {
        window_id,
        control_id: TAB_BAR_TRENDS,
        background_color: Color {
            r: 0x2E,
            g: 0x32,
            b: 0x39,
        },
        text_color: Color {
            r: 0xE0,
            g: 0xE5,
            b: 0xEC,
        },
        accent_color: Color {
            r: 0x00,
            g: 0x80,
            b: 0xFF,
        },
        font: None,
    });
    commands.push(PlatformCommand::CreateChart {
        window_id,
        parent_control_id: Some(PANEL_TAB_TRENDS),
        control_id: CHART_TRENDS,
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_JOBS),
        control_id: LABEL_JOBS_HEADER,
        initial_text: "Job List".to_string(),
        class: LabelClass::Default,
    });
    commands.push(PlatformCommand::CreateCheckBox {
        window_id,
        parent_control_id: Some(PANEL_JOBS),
        control_id: CHK_JOBS_SCOPE_SINCE_CHECKPOINT,
        text: "Since checkpoint only".to_string(),
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
        parent_control_id: Some(PANEL_LEFT_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_MODE_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_MODEL_ROW,
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
        control_id: PANEL_PROMPT_LAB_COMPARE_HEADER_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_COMPARE_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_CONTEXT_HEADER_ROW,
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
        control_id: PANEL_PROMPT_LAB_TEMPLATE_HEADER_ROW,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB),
        control_id: PANEL_PROMPT_LAB_RUN_DETAILS_HEADER_ROW,
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
    commands.push(PlatformCommand::CreateRadioButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_MODE_ROW),
        control_id: BTN_PROMPT_LAB_MODE_BASIC,
        text: "Basic".to_string(),
        group_start: true,
    });
    commands.push(PlatformCommand::CreateRadioButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_MODE_ROW),
        control_id: BTN_PROMPT_LAB_MODE_ADVANCED,
        text: "Advanced".to_string(),
        group_start: false,
    });
    // Model selector combo box
    commands.push(PlatformCommand::CreateComboBox {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_MODEL_ROW),
        control_id: COMBO_PROMPT_LAB_MODEL_SELECTOR,
    });
    commands.push(PlatformCommand::CreateRadioButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
        control_id: BTN_STAGE_TRIAGE,
        text: "Triage".to_string(),
        group_start: true,
    });
    commands.push(PlatformCommand::CreateRadioButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
        control_id: BTN_STAGE_SUMMARY,
        text: "Summary".to_string(),
        group_start: false,
    });
    commands.push(PlatformCommand::CreateRadioButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
        control_id: BTN_STAGE_BRIEFING,
        text: "Briefing".to_string(),
        group_start: false,
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_SOURCE_ROW),
        control_id: BTN_SOURCE_FROM_TRIAGE,
        text: "Selected article".to_string(),
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
    commands.push(PlatformCommand::CreateCheckBox {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_HEADER_ROW),
        control_id: CHK_PROMPT_LAB_SECTION_COMPARE,
        text: "Compare".to_string(),
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
    commands.push(PlatformCommand::CreateCheckBox {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_HEADER_ROW),
        control_id: CHK_PROMPT_LAB_SECTION_CONTEXT,
        text: "Context".to_string(),
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
    commands.push(PlatformCommand::CreateCheckBox {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_HEADER_ROW),
        control_id: CHK_PROMPT_LAB_SECTION_TEMPLATE,
        text: "Templates".to_string(),
    });
    commands.push(PlatformCommand::CreateCheckBox {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_RUN_DETAILS_HEADER_ROW),
        control_id: CHK_PROMPT_LAB_SECTION_RUN_DETAILS,
        text: "Run details".to_string(),
    });
    commands.push(PlatformCommand::CreateCheckBox {
        window_id,
        parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
        control_id: CHK_PROMPT_LAB_TEMPLATE_OPEN,
        text: "Edit Templates".to_string(),
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
        LayoutConfig {
            left_panel_width: initial_left_width,
            input_panel_visible: false,
            active_tab: AppTab::Summary,
            left_tab: LeftTab::Jobs,
            prompt_lab: PromptLabLayoutConfig {
                visible: false,
                advanced_mode: false,
                compare_section_open: false,
                context_section_open: false,
                template_section_open: false,
                run_details_section_open: false,
                template_editor_open: false,
            },
        },
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
        style_id: StyleId::RadioButton,
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
        style_id: StyleId::CheckBox,
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

    // TabBar background and text colors.
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TabBar,
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
    // TabBarAccent: the bottom accent line color (background_color carries the value).
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TabBarAccent,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x00,
                g: 0x80,
                b: 0xFF,
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

    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ComboBox,
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

pub(crate) fn build_layout_command(window_id: WindowId, config: LayoutConfig) -> PlatformCommand {
    PlatformCommand::DefineLayout {
        window_id,
        rules: build_layout_rules(
            config.left_panel_width,
            config.input_panel_visible,
            config.prompt_lab,
            config.active_tab,
            config.left_tab,
        ),
    }
}

fn build_layout_rules(
    left_panel_width: i32,
    input_panel_visible: bool,
    prompt_lab: PromptLabLayoutConfig,
    active_tab: AppTab,
    left_tab: LeftTab,
) -> Vec<LayoutRule> {
    let visibility = compute_prompt_lab_visibility(&prompt_lab);
    let input_width = if input_panel_visible {
        INPUT_PANEL_FIXED_WIDTH
    } else {
        0
    };
    let jobs_width = (left_panel_width - input_width).max(0);
    let _ = jobs_width; // jobs panel fills remaining space inside PANEL_LEFT_JOBS

    // Left-pane tab helpers: only one content panel fills; the other collapses to zero height.
    let left_tab_dock = |tab: LeftTab| -> DockStyle {
        if left_tab == tab {
            DockStyle::Fill
        } else {
            DockStyle::Top
        }
    };
    let left_tab_size = |tab: LeftTab| -> Option<i32> {
        if left_tab == tab {
            None
        } else {
            Some(0)
        }
    };
    // Right-pane tab helpers: only the active tab fills; the rest collapse to zero.
    let tab_dock = |tab: AppTab| -> DockStyle {
        if active_tab == tab {
            DockStyle::Fill
        } else {
            DockStyle::Top
        }
    };
    let tab_size = |tab: AppTab| -> Option<i32> {
        if active_tab == tab {
            None
        } else {
            Some(0)
        }
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
        // PANEL_LEFT replaces the old root-level PANEL_INPUT + PANEL_JOBS.
        LayoutRule {
            control_id: PANEL_LEFT,
            parent_control_id: None,
            dock_style: DockStyle::Left,
            order: 200,
            fixed_size: Some(left_panel_width),
            margin: (6, 6, 6, 6),
        },
        // Left-pane tab bar.
        LayoutRule {
            control_id: TAB_BAR_LEFT,
            parent_control_id: Some(PANEL_LEFT),
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(28),
            margin: (0, 0, 2, 0),
        },
        // Left content: Jobs (shown when left_tab is a job-oriented tab).
        LayoutRule {
            control_id: PANEL_LEFT_JOBS,
            parent_control_id: Some(PANEL_LEFT),
            dock_style: {
                let show = matches!(
                    left_tab,
                    LeftTab::Jobs | LeftTab::TriageReview | LeftTab::TriageResults
                );
                if show { DockStyle::Fill } else { DockStyle::Top }
            },
            order: 1,
            fixed_size: {
                let show = matches!(
                    left_tab,
                    LeftTab::Jobs | LeftTab::TriageReview | LeftTab::TriageResults
                );
                if show { None } else { Some(0) }
            },
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: PANEL_INPUT,
            parent_control_id: Some(PANEL_LEFT_JOBS),
            dock_style: DockStyle::Left,
            order: 0,
            fixed_size: Some(input_width),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: PANEL_JOBS,
            parent_control_id: Some(PANEL_LEFT_JOBS),
            dock_style: DockStyle::Fill,
            order: 1,
            fixed_size: None,
            margin: (0, 0, 0, 0),
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
            control_id: CHK_JOBS_SCOPE_SINCE_CHECKPOINT,
            parent_control_id: Some(PANEL_JOBS),
            dock_style: DockStyle::Top,
            order: 1,
            fixed_size: Some(24),
            margin: (4, 0, 4, 4),
        },
        LayoutRule {
            control_id: TREE_JOBS,
            parent_control_id: Some(PANEL_JOBS),
            dock_style: DockStyle::Fill,
            order: 2,
            fixed_size: None,
            margin: (0, 0, 0, 0),
        },
        // Left content: Prompt Lab (shown when left_tab == PromptLab).
        LayoutRule {
            control_id: PANEL_LEFT_PROMPT_LAB,
            parent_control_id: Some(PANEL_LEFT),
            dock_style: left_tab_dock(LeftTab::PromptLab),
            order: 2,
            fixed_size: left_tab_size(LeftTab::PromptLab),
            margin: (0, 0, 0, 0),
        },
        // PANEL_PROMPT_LAB fills PANEL_LEFT_PROMPT_LAB.
        LayoutRule {
            control_id: PANEL_PROMPT_LAB,
            parent_control_id: Some(PANEL_LEFT_PROMPT_LAB),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (0, 4, 0, 4),
        },
        LayoutRule {
            control_id: SPLITTER_MAIN,
            parent_control_id: None,
            dock_style: DockStyle::Left,
            order: 205,
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
        // Right-pane tab bar (custom TabBar widget).
        LayoutRule {
            control_id: TAB_BAR_RIGHT,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: DockStyle::Top,
            order: 1,
            fixed_size: Some(28),
            margin: (0, 0, 2, 0),
        },
        // Tab content panels — active tab fills remaining space; inactive ones collapse.
        LayoutRule {
            control_id: PANEL_TAB_TRIAGE,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: tab_dock(AppTab::Triage),
            order: 2,
            fixed_size: tab_size(AppTab::Triage),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: VIEWER_TRIAGE,
            parent_control_id: Some(PANEL_TAB_TRIAGE),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: PANEL_TAB_SUMMARY,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: tab_dock(AppTab::Summary),
            order: 3,
            fixed_size: tab_size(AppTab::Summary),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: VIEWER_PREVIEW,
            parent_control_id: Some(PANEL_TAB_SUMMARY),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: PANEL_TAB_BRIEFING,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: tab_dock(AppTab::Briefing),
            order: 4,
            fixed_size: tab_size(AppTab::Briefing),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: VIEWER_BRIEFING,
            parent_control_id: Some(PANEL_TAB_BRIEFING),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: PANEL_TAB_TRENDS,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: tab_dock(AppTab::Trends),
            order: 5,
            fixed_size: tab_size(AppTab::Trends),
            margin: (0, 0, 0, 0),
        },
        // Trend-category tab bar (custom TabBar widget).
        LayoutRule {
            control_id: TAB_BAR_TRENDS,
            parent_control_id: Some(PANEL_TAB_TRENDS),
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(28),
            margin: (0, 0, 2, 0),
        },
        LayoutRule {
            control_id: CHART_TRENDS,
            parent_control_id: Some(PANEL_TAB_TRENDS),
            dock_style: DockStyle::Fill,
            order: 1,
            fixed_size: None,
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: LABEL_PROMPT_LAB_STATUS,
            parent_control_id: Some(PANEL_PROMPT_LAB),
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STATUS),
            margin: (0, 0, 2, 0),
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

    // Always emit Prompt Lab sub-panel rules; tab collapse handles outer visibility.
    {
        rules.extend([
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_MODE_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 1,
                fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STANDARD),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_MODE_BASIC,
                parent_control_id: Some(PANEL_PROMPT_LAB_MODE_ROW),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(120),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_PROMPT_LAB_MODE_ADVANCED,
                parent_control_id: Some(PANEL_PROMPT_LAB_MODE_ROW),
                dock_style: DockStyle::Left,
                order: 1,
                fixed_size: Some(120),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_MODEL_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 2,
                fixed_size: if visibility.show_advanced {
                    Some(PROMPT_LAB_ROW_HEIGHT_STANDARD)
                } else {
                    Some(0)
                },
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: COMBO_PROMPT_LAB_MODEL_SELECTOR,
                parent_control_id: Some(PANEL_PROMPT_LAB_MODEL_ROW),
                dock_style: DockStyle::Fill,
                order: 0,
                fixed_size: None,
                margin: (0, 4, 0, 0),
            },
        ]);

        rules.extend([
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_STAGE_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 3,
                fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STANDARD),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: BTN_STAGE_TRIAGE,
                parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(110),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_STAGE_SUMMARY,
                parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
                dock_style: DockStyle::Left,
                order: 1,
                fixed_size: Some(110),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: BTN_STAGE_BRIEFING,
                parent_control_id: Some(PANEL_PROMPT_LAB_STAGE_ROW),
                dock_style: DockStyle::Left,
                order: 2,
                fixed_size: Some(110),
                margin: (0, 4, 0, 0),
            },
            LayoutRule {
                control_id: PANEL_PROMPT_LAB_SOURCE_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 4,
                fixed_size: Some(0),
                margin: (0, 0, 2, 0),
            },
            LayoutRule {
                control_id: BTN_SOURCE_FROM_TRIAGE,
                parent_control_id: Some(PANEL_PROMPT_LAB_SOURCE_ROW),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(180),
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
                order: 5,
                fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STANDARD),
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
                order: 6,
                fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STANDARD),
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
        ]);

        if visibility.show_advanced {
            rules.extend([
                LayoutRule {
                    control_id: PANEL_PROMPT_LAB_COMPARE_HEADER_ROW,
                    parent_control_id: Some(PANEL_PROMPT_LAB),
                    dock_style: DockStyle::Top,
                    order: 7,
                    fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STANDARD),
                    margin: (0, 0, 2, 0),
                },
                LayoutRule {
                    control_id: CHK_PROMPT_LAB_SECTION_COMPARE,
                    parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_HEADER_ROW),
                    dock_style: DockStyle::Fill,
                    order: 0,
                    fixed_size: None,
                    margin: (0, 0, 0, 0),
                },
            ]);
            if visibility.show_compare_row {
                rules.extend([
                    LayoutRule {
                        control_id: PANEL_PROMPT_LAB_COMPARE_ROW,
                        parent_control_id: Some(PANEL_PROMPT_LAB),
                        dock_style: DockStyle::Top,
                        order: 8,
                        fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STANDARD),
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
                ]);
            } else {
                rules.extend([
                    collapsed_top_rule(PANEL_PROMPT_LAB_COMPARE_ROW, PANEL_PROMPT_LAB, 8),
                    LayoutRule {
                        control_id: BTN_COMPARE_ADD_CURRENT,
                        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                        dock_style: DockStyle::Left,
                        order: 0,
                        fixed_size: Some(0),
                        margin: (0, 0, 0, 0),
                    },
                    LayoutRule {
                        control_id: BTN_COMPARE_ADD_BASELINE,
                        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                        dock_style: DockStyle::Left,
                        order: 1,
                        fixed_size: Some(0),
                        margin: (0, 0, 0, 0),
                    },
                    LayoutRule {
                        control_id: BTN_COMPARE_RESET_DRAFT,
                        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                        dock_style: DockStyle::Left,
                        order: 2,
                        fixed_size: Some(0),
                        margin: (0, 0, 0, 0),
                    },
                    LayoutRule {
                        control_id: BTN_COMPARE_START,
                        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                        dock_style: DockStyle::Left,
                        order: 3,
                        fixed_size: Some(0),
                        margin: (0, 0, 0, 0),
                    },
                    LayoutRule {
                        control_id: BTN_COMPARE_CANCEL,
                        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                        dock_style: DockStyle::Left,
                        order: 4,
                        fixed_size: Some(0),
                        margin: (0, 0, 0, 0),
                    },
                    LayoutRule {
                        control_id: BTN_COMPARE_AUTO_SELECT,
                        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                        dock_style: DockStyle::Left,
                        order: 5,
                        fixed_size: Some(0),
                        margin: (0, 0, 0, 0),
                    },
                    LayoutRule {
                        control_id: BTN_COMPARE_WINNER_CLEAR,
                        parent_control_id: Some(PANEL_PROMPT_LAB_COMPARE_ROW),
                        dock_style: DockStyle::Fill,
                        order: 6,
                        fixed_size: None,
                        margin: (0, 0, 0, 0),
                    },
                ]);
            }
            rules.extend([
                LayoutRule {
                    control_id: PANEL_PROMPT_LAB_CONTEXT_HEADER_ROW,
                    parent_control_id: Some(PANEL_PROMPT_LAB),
                    dock_style: DockStyle::Top,
                    order: 9,
                    fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STANDARD),
                    margin: (0, 0, 2, 0),
                },
                LayoutRule {
                    control_id: CHK_PROMPT_LAB_SECTION_CONTEXT,
                    parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_HEADER_ROW),
                    dock_style: DockStyle::Fill,
                    order: 0,
                    fixed_size: None,
                    margin: (0, 0, 0, 0),
                },
            ]);
            if visibility.show_context_row {
                rules.extend([
                    LayoutRule {
                        control_id: PANEL_PROMPT_LAB_CONTEXT_ROW,
                        parent_control_id: Some(PANEL_PROMPT_LAB),
                        dock_style: DockStyle::Top,
                        order: 10,
                        fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_CONTEXT_INPUT),
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
                        order: 11,
                        fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_ACTION),
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
                        order: 12,
                        fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STATUS),
                        margin: (0, 0, 2, 0),
                    },
                ]);
            } else {
                rules.extend([
                    collapsed_top_rule(PANEL_PROMPT_LAB_CONTEXT_ROW, PANEL_PROMPT_LAB, 10),
                    LayoutRule {
                        control_id: INPUT_PROMPT_LAB_CONTEXT,
                        parent_control_id: Some(PANEL_PROMPT_LAB_CONTEXT_ROW),
                        dock_style: DockStyle::Fill,
                        order: 0,
                        fixed_size: None,
                        margin: (0, 0, 0, 0),
                    },
                    collapsed_top_rule(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW, PANEL_PROMPT_LAB, 11),
                    collapsed_top_rule(LABEL_PROMPT_LAB_CONTEXT_STATUS, PANEL_PROMPT_LAB, 12),
                ]);
            }
            rules.extend([
                LayoutRule {
                    control_id: PANEL_PROMPT_LAB_TEMPLATE_HEADER_ROW,
                    parent_control_id: Some(PANEL_PROMPT_LAB),
                    dock_style: DockStyle::Top,
                    order: 13,
                    fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STANDARD),
                    margin: (0, 0, 2, 0),
                },
                LayoutRule {
                    control_id: CHK_PROMPT_LAB_SECTION_TEMPLATE,
                    parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_HEADER_ROW),
                    dock_style: DockStyle::Fill,
                    order: 0,
                    fixed_size: None,
                    margin: (0, 0, 0, 0),
                },
            ]);
            if visibility.show_template_row {
                rules.extend([
                    LayoutRule {
                        control_id: PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW,
                        parent_control_id: Some(PANEL_PROMPT_LAB),
                        dock_style: DockStyle::Top,
                        order: 14,
                        fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_ACTION),
                        margin: (0, 0, 2, 0),
                    },
                    LayoutRule {
                        control_id: CHK_PROMPT_LAB_TEMPLATE_OPEN,
                        parent_control_id: Some(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW),
                        dock_style: DockStyle::Left,
                        order: 0,
                        fixed_size: Some(PROMPT_LAB_TEMPLATE_TOGGLE_BUTTON_WIDTH),
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
                        order: 15,
                        fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STATUS),
                        margin: (0, 0, 2, 0),
                    },
                ]);
            } else {
                rules.extend([
                    collapsed_top_rule(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW, PANEL_PROMPT_LAB, 14),
                    collapsed_top_rule(LABEL_PROMPT_LAB_TEMPLATE_STATUS, PANEL_PROMPT_LAB, 15),
                ]);
            }
            rules.push(LayoutRule {
                control_id: PANEL_PROMPT_LAB_RUN_DETAILS_HEADER_ROW,
                parent_control_id: Some(PANEL_PROMPT_LAB),
                dock_style: DockStyle::Top,
                order: 18,
                fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_STANDARD),
                margin: (0, 0, 2, 0),
            });
            rules.push(LayoutRule {
                control_id: CHK_PROMPT_LAB_SECTION_RUN_DETAILS,
                parent_control_id: Some(PANEL_PROMPT_LAB_RUN_DETAILS_HEADER_ROW),
                dock_style: DockStyle::Fill,
                order: 0,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            });
            if visibility.show_run_details_row {
                rules.push(LayoutRule {
                    control_id: LABEL_PROMPT_LAB_METADATA,
                    parent_control_id: Some(PANEL_PROMPT_LAB),
                    dock_style: DockStyle::Top,
                    order: 19,
                    fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_RUN_DETAILS_BODY),
                    margin: (0, 0, 2, 0),
                });
            } else {
                rules.push(collapsed_top_rule(
                    LABEL_PROMPT_LAB_METADATA,
                    PANEL_PROMPT_LAB,
                    19,
                ));
            }
        } else {
            rules.extend([
                collapsed_top_rule(PANEL_PROMPT_LAB_COMPARE_HEADER_ROW, PANEL_PROMPT_LAB, 7),
                collapsed_top_rule(PANEL_PROMPT_LAB_COMPARE_ROW, PANEL_PROMPT_LAB, 8),
                collapsed_top_rule(PANEL_PROMPT_LAB_CONTEXT_HEADER_ROW, PANEL_PROMPT_LAB, 9),
                collapsed_top_rule(PANEL_PROMPT_LAB_CONTEXT_ROW, PANEL_PROMPT_LAB, 10),
                collapsed_top_rule(PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW, PANEL_PROMPT_LAB, 11),
                collapsed_top_rule(LABEL_PROMPT_LAB_CONTEXT_STATUS, PANEL_PROMPT_LAB, 12),
                collapsed_top_rule(PANEL_PROMPT_LAB_TEMPLATE_HEADER_ROW, PANEL_PROMPT_LAB, 13),
                collapsed_top_rule(PANEL_PROMPT_LAB_TEMPLATE_ACTION_ROW, PANEL_PROMPT_LAB, 14),
                collapsed_top_rule(LABEL_PROMPT_LAB_TEMPLATE_STATUS, PANEL_PROMPT_LAB, 15),
                collapsed_top_rule(PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW, PANEL_PROMPT_LAB, 16),
                collapsed_top_rule(PANEL_PROMPT_LAB_TEMPLATE_USER_ROW, PANEL_PROMPT_LAB, 17),
                collapsed_top_rule(
                    PANEL_PROMPT_LAB_RUN_DETAILS_HEADER_ROW,
                    PANEL_PROMPT_LAB,
                    18,
                ),
                collapsed_top_rule(LABEL_PROMPT_LAB_METADATA, PANEL_PROMPT_LAB, 19),
            ]);
        }

        if visibility.show_template_editor_rows {
            rules.extend([
                LayoutRule {
                    control_id: PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW,
                    parent_control_id: Some(PANEL_PROMPT_LAB),
                    dock_style: DockStyle::Top,
                    order: 16,
                    fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_TEMPLATE_EDITOR_INPUT),
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
                    order: 17,
                    fixed_size: Some(PROMPT_LAB_ROW_HEIGHT_TEMPLATE_EDITOR_INPUT),
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
        } else {
            // Always collapse template editor rows unless the editor is explicitly open.
            rules.extend([
                collapsed_top_rule(PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW, PANEL_PROMPT_LAB, 16),
                collapsed_top_rule(PANEL_PROMPT_LAB_TEMPLATE_USER_ROW, PANEL_PROMPT_LAB, 17),
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
        PANEL_TAB_TRIAGE,
        PANEL_TAB_SUMMARY,
        PANEL_TAB_BRIEFING,
        PANEL_TAB_TRENDS,
        PANEL_LEFT,
        PANEL_LEFT_JOBS,
        PANEL_LEFT_PROMPT_LAB,
        PANEL_PROMPT_LAB,
        PANEL_PROMPT_LAB_MODE_ROW,
        PANEL_PROMPT_LAB_MODEL_ROW,
        PANEL_PROMPT_LAB_STAGE_ROW,
        PANEL_PROMPT_LAB_SOURCE_ROW,
        PANEL_PROMPT_LAB_INPUT_ROW,
        PANEL_PROMPT_LAB_ACTION_ROW,
        PANEL_PROMPT_LAB_COMPARE_HEADER_ROW,
        PANEL_PROMPT_LAB_COMPARE_ROW,
        PANEL_PROMPT_LAB_CONTEXT_HEADER_ROW,
        PANEL_PROMPT_LAB_CONTEXT_ROW,
        PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW,
        PANEL_PROMPT_LAB_TEMPLATE_HEADER_ROW,
        PANEL_PROMPT_LAB_RUN_DETAILS_HEADER_ROW,
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
        control_id: COMBO_PROMPT_LAB_MODEL_SELECTOR,
        style_id: StyleId::ComboBox,
    });
    for control_id in [VIEWER_PREVIEW, VIEWER_TRIAGE, VIEWER_BRIEFING] {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id,
            style_id: StyleId::ViewerReadable,
        });
    }

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
        BTN_PROMPT_LAB_RESOLVE,
        BTN_PROMPT_LAB_RUN,
        BTN_PROMPT_LAB_CONTEXT_APPLY,
        BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN,
        BTN_PROMPT_LAB_CONTEXT_REVERT,
        BTN_PROMPT_LAB_CONTEXT_SAVE,
        BTN_PROMPT_LAB_CONTEXT_RELOAD,
        CHK_PROMPT_LAB_TEMPLATE_OPEN,
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
        BTN_SOURCE_FROM_TRIAGE,
        BTN_SOURCE_TYPE_URL,
    ] {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id,
            style_id: StyleId::DefaultButton,
        });
    }

    for control_id in [
        BTN_PROMPT_LAB_MODE_BASIC,
        BTN_PROMPT_LAB_MODE_ADVANCED,
        BTN_STAGE_TRIAGE,
        BTN_STAGE_SUMMARY,
        BTN_STAGE_BRIEFING,
    ] {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id,
            style_id: StyleId::RadioButton,
        });
    }
    for control_id in [
        CHK_JOBS_SCOPE_SINCE_CHECKPOINT,
        CHK_PROMPT_LAB_SECTION_COMPARE,
        CHK_PROMPT_LAB_SECTION_CONTEXT,
        CHK_PROMPT_LAB_SECTION_TEMPLATE,
        CHK_PROMPT_LAB_SECTION_RUN_DETAILS,
    ] {
        commands.push(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id,
            style_id: StyleId::CheckBox,
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

    fn layout_rules_for_prompt_lab(prompt_lab: PromptLabLayoutConfig) -> Vec<LayoutRule> {
        let cmd = build_layout_command(
            WindowId::new(99),
            LayoutConfig {
                left_panel_width: 600,
                input_panel_visible: true,
                active_tab: AppTab::Triage,
                left_tab: LeftTab::PromptLab,
                prompt_lab,
            },
        );
        match cmd {
            PlatformCommand::DefineLayout { rules, .. } => rules,
            _ => panic!("expected DefineLayout"),
        }
    }

    fn fixed_size_for(rules: &[LayoutRule], control_id: ControlId) -> i32 {
        rules
            .iter()
            .find(|r| r.control_id == control_id)
            .and_then(|r| r.fixed_size)
            .expect("control rule must exist")
    }

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
        assert!(
            commands.iter().any(|cmd| matches!(
                cmd,
                PlatformCommand::CreateComboBox { control_id, .. }
                    if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR
            )),
            "model selector combo box should be created"
        );
        assert!(
            commands.iter().any(|cmd| matches!(
                cmd,
                PlatformCommand::CreateCheckBox { control_id, .. }
                    if *control_id == CHK_PROMPT_LAB_SECTION_COMPARE
            )),
            "section toggle Compare should be created as CheckBox"
        );
        assert!(
            commands.iter().any(|cmd| matches!(
                cmd,
                PlatformCommand::CreateCheckBox { control_id, .. }
                    if *control_id == CHK_JOBS_SCOPE_SINCE_CHECKPOINT
            )),
            "jobs scope checkbox should be created"
        );
    }

    #[test]
    fn jobs_panel_visible_for_all_job_oriented_tabs() {
        for left_tab in [LeftTab::Jobs, LeftTab::TriageReview, LeftTab::TriageResults] {
            let cmd = build_layout_command(
                WindowId::new(42),
                LayoutConfig {
                    left_panel_width: 600,
                    input_panel_visible: true,
                    active_tab: AppTab::Summary,
                    left_tab,
                    prompt_lab: PromptLabLayoutConfig {
                        visible: false,
                        advanced_mode: false,
                        compare_section_open: false,
                        context_section_open: false,
                        template_section_open: false,
                        run_details_section_open: false,
                        template_editor_open: false,
                    },
                },
            );
            let rules = match cmd {
                PlatformCommand::DefineLayout { rules, .. } => rules,
                _ => panic!("expected DefineLayout"),
            };
            let jobs_rule = rules
                .iter()
                .find(|r| r.control_id == PANEL_LEFT_JOBS)
                .expect("jobs panel rule");
            assert_eq!(
                jobs_rule.dock_style,
                DockStyle::Fill,
                "jobs panel must fill for {left_tab:?}"
            );
            assert_eq!(
                jobs_rule.fixed_size, None,
                "jobs panel must be visible for {left_tab:?}"
            );
        }
    }

    #[test]
    fn combo_box_style_is_defined() {
        let commands = initial_commands(WindowId::new(2));
        assert!(
            commands.iter().any(|cmd| matches!(
                cmd,
                PlatformCommand::DefineStyle { style_id, .. }
                    if *style_id == StyleId::ComboBox
            )),
            "StyleId::ComboBox must be defined to avoid runtime warning"
        );
    }

    #[test]
    fn checkbox_style_is_defined() {
        let commands = initial_commands(WindowId::new(2));
        assert!(
            commands.iter().any(|cmd| matches!(
                cmd,
                PlatformCommand::DefineStyle { style_id, .. }
                    if *style_id == StyleId::CheckBox
            )),
            "StyleId::CheckBox must be defined so WM_CTLCOLOR returns the dark-theme brush"
        );
    }

    /// General regression guard: every StyleId that appears in an ApplyStyleToControl command
    /// must have a corresponding DefineStyle command earlier in the same sequence.
    /// A missing DefineStyle is a silent runtime failure — WM_CTLCOLOR gets no brush and
    /// the control renders with the system default (light) colors.
    #[test]
    fn every_applied_style_has_a_prior_definition() {
        use std::collections::HashSet;
        let cmds = initial_commands(WindowId::new(99));
        let mut defined: HashSet<StyleId> = HashSet::new();
        for cmd in &cmds {
            match cmd {
                PlatformCommand::DefineStyle { style_id, .. } => {
                    defined.insert(*style_id);
                }
                PlatformCommand::ApplyStyleToControl {
                    style_id,
                    control_id,
                    ..
                } => {
                    assert!(
                        defined.contains(style_id),
                        "StyleId::{style_id:?} is applied to control {} but was never defined with DefineStyle — \
                         WM_CTLCOLOR will get no brush and the control will render with system-default (light) colors",
                        control_id.raw()
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn combo_box_style_is_applied_to_model_selector() {
        let commands = initial_commands(WindowId::new(2));
        assert!(
            commands.iter().any(|cmd| matches!(
                cmd,
                PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
                    if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR
                       && *style_id == StyleId::ComboBox
            )),
            "model selector combo box should have ComboBox style applied"
        );
    }

    /// When a tab is not active, its panel is collapsed to zero; the active tab fills the space.
    #[test]
    fn inactive_tab_panel_is_collapsed() {
        let cmd = build_layout_command(
            WindowId::new(3),
            LayoutConfig {
                left_panel_width: 600,
                input_panel_visible: true,
                active_tab: AppTab::Summary,
                left_tab: LeftTab::Jobs,
                prompt_lab: PromptLabLayoutConfig {
                    visible: false,
                    advanced_mode: false,
                    compare_section_open: false,
                    context_section_open: false,
                    template_section_open: false,
                    run_details_section_open: false,
                    template_editor_open: false,
                },
            },
        );
        let rules = match cmd {
            PlatformCommand::DefineLayout { rules, .. } => rules,
            _ => panic!("expected DefineLayout"),
        };
        // Summary tab should be active (None = Fill).
        let summary_tab = rules
            .iter()
            .find(|r| r.control_id == PANEL_TAB_SUMMARY)
            .expect("summary tab panel rule");
        assert_eq!(summary_tab.fixed_size, None);
        // Other content tabs should also be collapsed.
        for &control_id in &[PANEL_TAB_TRIAGE, PANEL_TAB_BRIEFING, PANEL_TAB_TRENDS] {
            let tab = rules
                .iter()
                .find(|r| r.control_id == control_id)
                .expect("tab panel rule");
            assert_eq!(
                tab.fixed_size,
                Some(0),
                "tab {:?} should be collapsed",
                control_id
            );
        }
    }

    #[test]
    fn preview_tab_panels_use_single_fill_rule() {
        let cmd = build_layout_command(
            WindowId::new(31),
            LayoutConfig {
                left_panel_width: 600,
                input_panel_visible: true,
                active_tab: AppTab::Summary,
                left_tab: LeftTab::Jobs,
                prompt_lab: PromptLabLayoutConfig {
                    visible: false,
                    advanced_mode: false,
                    compare_section_open: false,
                    context_section_open: false,
                    template_section_open: false,
                    run_details_section_open: false,
                    template_editor_open: false,
                },
            },
        );
        let rules = match cmd {
            PlatformCommand::DefineLayout { rules, .. } => rules,
            _ => panic!("expected DefineLayout"),
        };
        let tab_panel_ids = [
            PANEL_TAB_TRIAGE,
            PANEL_TAB_SUMMARY,
            PANEL_TAB_BRIEFING,
            PANEL_TAB_TRENDS,
        ];

        let fill_count = rules
            .iter()
            .filter(|r| {
                r.parent_control_id == Some(PANEL_PREVIEW)
                    && tab_panel_ids.contains(&r.control_id)
                    && r.dock_style == DockStyle::Fill
            })
            .count();
        assert_eq!(
            fill_count, 1,
            "PANEL_PREVIEW tab panels must have exactly one Fill child; additional tabs should be collapsed with zero-size non-Fill rules"
        );
    }

    #[test]
    fn expanded_layout_includes_all_controls() {
        let cmd = build_layout_command(
            WindowId::new(4),
            LayoutConfig {
                left_panel_width: 600,
                input_panel_visible: true,
                active_tab: AppTab::Triage,
                left_tab: LeftTab::PromptLab,
                prompt_lab: PromptLabLayoutConfig {
                    visible: true,
                    advanced_mode: true,
                    compare_section_open: true,
                    context_section_open: true,
                    template_section_open: true,
                    run_details_section_open: true,
                    template_editor_open: true,
                },
            },
        );
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

    #[test]
    fn template_editor_rows_are_collapsed_when_editor_is_closed() {
        let cmd = build_layout_command(
            WindowId::new(5),
            LayoutConfig {
                left_panel_width: 600,
                input_panel_visible: true,
                active_tab: AppTab::Triage,
                left_tab: LeftTab::PromptLab,
                prompt_lab: PromptLabLayoutConfig {
                    visible: true,
                    advanced_mode: true,
                    compare_section_open: false,
                    context_section_open: false,
                    template_section_open: true,
                    run_details_section_open: false,
                    template_editor_open: false,
                },
            },
        );
        let rules = match cmd {
            PlatformCommand::DefineLayout { rules, .. } => rules,
            _ => panic!("expected DefineLayout"),
        };
        let system_row = rules
            .iter()
            .find(|r| r.control_id == PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW)
            .expect("template system row rule");
        assert_eq!(system_row.fixed_size, Some(0));

        let user_row = rules
            .iter()
            .find(|r| r.control_id == PANEL_PROMPT_LAB_TEMPLATE_USER_ROW)
            .expect("template user row rule");
        assert_eq!(user_row.fixed_size, Some(0));
    }

    #[test]
    fn template_toggle_button_width_fits_state_text() {
        let cmd = build_layout_command(
            WindowId::new(6),
            LayoutConfig {
                left_panel_width: 600,
                input_panel_visible: true,
                active_tab: AppTab::Triage,
                left_tab: LeftTab::PromptLab,
                prompt_lab: PromptLabLayoutConfig {
                    visible: true,
                    advanced_mode: true,
                    compare_section_open: false,
                    context_section_open: false,
                    template_section_open: true,
                    run_details_section_open: false,
                    template_editor_open: false,
                },
            },
        );
        let rules = match cmd {
            PlatformCommand::DefineLayout { rules, .. } => rules,
            _ => panic!("expected DefineLayout"),
        };
        let toggle_button = rules
            .iter()
            .find(|r| r.control_id == CHK_PROMPT_LAB_TEMPLATE_OPEN)
            .expect("template toggle button rule");
        assert_eq!(
            toggle_button.fixed_size,
            Some(PROMPT_LAB_TEMPLATE_TOGGLE_BUTTON_WIDTH)
        );
    }

    #[test]
    fn template_editor_visibility_matrix_enforces_row_sizes() {
        for advanced_mode in [false, true] {
            for template_section_open in [false, true] {
                for template_editor_open in [false, true] {
                    let rules = layout_rules_for_prompt_lab(PromptLabLayoutConfig {
                        visible: true,
                        advanced_mode,
                        compare_section_open: false,
                        context_section_open: false,
                        template_section_open,
                        run_details_section_open: false,
                        template_editor_open,
                    });

                    let expected_editor_rows =
                        advanced_mode && template_section_open && template_editor_open;
                    let expected_size = if expected_editor_rows {
                        PROMPT_LAB_ROW_HEIGHT_TEMPLATE_EDITOR_INPUT
                    } else {
                        0
                    };

                    assert_eq!(
                        fixed_size_for(&rules, PANEL_PROMPT_LAB_TEMPLATE_SYSTEM_ROW),
                        expected_size,
                        "system row mismatch: advanced_mode={advanced_mode} template_section_open={template_section_open} template_editor_open={template_editor_open}"
                    );
                    assert_eq!(
                        fixed_size_for(&rules, PANEL_PROMPT_LAB_TEMPLATE_USER_ROW),
                        expected_size,
                        "user row mismatch: advanced_mode={advanced_mode} template_section_open={template_section_open} template_editor_open={template_editor_open}"
                    );
                }
            }
        }
    }

    #[test]
    fn compare_visibility_matrix_enforces_row_sizes() {
        for advanced_mode in [false, true] {
            for compare_section_open in [false, true] {
                let rules = layout_rules_for_prompt_lab(PromptLabLayoutConfig {
                    visible: true,
                    advanced_mode,
                    compare_section_open,
                    context_section_open: false,
                    template_section_open: false,
                    run_details_section_open: false,
                    template_editor_open: false,
                });
                let expected_compare_header = if advanced_mode {
                    PROMPT_LAB_ROW_HEIGHT_STANDARD
                } else {
                    0
                };
                let expected_compare_row = if advanced_mode && compare_section_open {
                    PROMPT_LAB_ROW_HEIGHT_STANDARD
                } else {
                    0
                };
                assert_eq!(
                    fixed_size_for(&rules, PANEL_PROMPT_LAB_COMPARE_HEADER_ROW),
                    expected_compare_header
                );
                assert_eq!(
                    fixed_size_for(&rules, PANEL_PROMPT_LAB_COMPARE_ROW),
                    expected_compare_row
                );
            }
        }
    }

    #[test]
    fn context_visibility_matrix_enforces_row_sizes() {
        for advanced_mode in [false, true] {
            for context_section_open in [false, true] {
                let rules = layout_rules_for_prompt_lab(PromptLabLayoutConfig {
                    visible: true,
                    advanced_mode,
                    compare_section_open: false,
                    context_section_open,
                    template_section_open: false,
                    run_details_section_open: false,
                    template_editor_open: false,
                });
                let expected_context_header = if advanced_mode {
                    PROMPT_LAB_ROW_HEIGHT_STANDARD
                } else {
                    0
                };
                let expected_context_body = if advanced_mode && context_section_open {
                    PROMPT_LAB_ROW_HEIGHT_CONTEXT_INPUT
                } else {
                    0
                };
                let expected_context_actions = if advanced_mode && context_section_open {
                    PROMPT_LAB_ROW_HEIGHT_ACTION
                } else {
                    0
                };
                let expected_context_status = if advanced_mode && context_section_open {
                    PROMPT_LAB_ROW_HEIGHT_STATUS
                } else {
                    0
                };
                assert_eq!(
                    fixed_size_for(&rules, PANEL_PROMPT_LAB_CONTEXT_HEADER_ROW),
                    expected_context_header
                );
                assert_eq!(
                    fixed_size_for(&rules, PANEL_PROMPT_LAB_CONTEXT_ROW),
                    expected_context_body
                );
                assert_eq!(
                    fixed_size_for(&rules, PANEL_PROMPT_LAB_CONTEXT_ACTION_ROW),
                    expected_context_actions
                );
                assert_eq!(
                    fixed_size_for(&rules, LABEL_PROMPT_LAB_CONTEXT_STATUS),
                    expected_context_status
                );
            }
        }
    }

    #[test]
    fn run_details_visibility_matrix_enforces_row_sizes() {
        for advanced_mode in [false, true] {
            for run_details_section_open in [false, true] {
                let rules = layout_rules_for_prompt_lab(PromptLabLayoutConfig {
                    visible: true,
                    advanced_mode,
                    compare_section_open: false,
                    context_section_open: false,
                    template_section_open: false,
                    run_details_section_open,
                    template_editor_open: false,
                });
                let expected_run_details_header = if advanced_mode {
                    PROMPT_LAB_ROW_HEIGHT_STANDARD
                } else {
                    0
                };
                let expected_run_details_body = if advanced_mode && run_details_section_open {
                    PROMPT_LAB_ROW_HEIGHT_RUN_DETAILS_BODY
                } else {
                    0
                };
                assert_eq!(
                    fixed_size_for(&rules, PANEL_PROMPT_LAB_RUN_DETAILS_HEADER_ROW),
                    expected_run_details_header
                );
                assert_eq!(
                    fixed_size_for(&rules, LABEL_PROMPT_LAB_METADATA),
                    expected_run_details_body
                );
            }
        }
    }

    /// Guard: every RadioButton, CheckBox, and Button created in initial_commands must have a
    /// corresponding ApplyStyleToControl command.
    ///
    /// Without an applied style, handle_wm_ctlcolorbtn returns None and Windows falls back to
    /// system-default light colors, producing the "new button appears white" regression.
    /// Adding a control to this category and forgetting the style command will fail this test.
    #[test]
    fn every_button_like_control_has_a_style_applied() {
        use std::collections::HashSet;
        let cmds = initial_commands(WindowId::new(99));

        let mut created: HashSet<i32> = HashSet::new();
        for cmd in &cmds {
            match cmd {
                PlatformCommand::CreateButton { control_id, .. }
                | PlatformCommand::CreateRadioButton { control_id, .. }
                | PlatformCommand::CreateCheckBox { control_id, .. } => {
                    created.insert(control_id.raw());
                }
                _ => {}
            }
        }

        let mut styled: HashSet<i32> = HashSet::new();
        for cmd in &cmds {
            if let PlatformCommand::ApplyStyleToControl { control_id, .. } = cmd {
                styled.insert(control_id.raw());
            }
        }

        for id in &created {
            assert!(
                styled.contains(id),
                "Control {} (Button/RadioButton/CheckBox) has no ApplyStyleToControl — \
                 WM_CTLCOLOR will fall back to system default (light) colors. \
                 Add it to the style application loop in initial_commands().",
                id
            );
        }
    }
}
