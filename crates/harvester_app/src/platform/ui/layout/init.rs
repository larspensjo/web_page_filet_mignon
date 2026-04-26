use commanductui::types::{LabelClass, MenuItemConfig, SplitterOrientation};
use commanductui::{Color, FontDescription, FontWeight, PlatformCommand, WindowId};
use harvester_core::TOKEN_LIMIT;

use super::super::constants::*;

/// Creates all Win32 controls for the main window.
///
/// Called by `initial_commands`; the caller is responsible for defining and
/// applying dark-theme styles before/after this function.
pub(super) fn create_controls(window_id: WindowId, commands: &mut Vec<PlatformCommand>) {
    commands.push(PlatformCommand::CreateMainMenu {
        window_id,
        menu_items: vec![MenuItemConfig {
            action: None,
            text: "File".to_string(),
            children: vec![MenuItemConfig {
                action: Some(MENU_ACTION_ADD_URL),
                text: "Add URL...\tCtrl+L".to_string(),
                children: Vec::new(),
            }],
        }],
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: None,
        control_id: PANEL_TOOLBAR,
    });

    commands.push(PlatformCommand::CreateToggleSwitch {
        window_id,
        parent_control_id: Some(PANEL_TOOLBAR),
        control_id: TS_JOBS_SCOPE,
        label: "Since checkpoint".to_string(),
        checked: true, // initial state; synced by render on first tick
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_TOOLBAR),
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
            r: 0x14,
            g: 0x14,
            b: 0x13,
        },
        text_color: Color {
            r: 0xFA,
            g: 0xF9,
            b: 0xF5,
        },
        accent_color: Color {
            r: 0xC9,
            g: 0x64,
            b: 0x42,
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
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW),
        control_id: PANEL_AI_WARNING,
    });
    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW),
        control_id: PANEL_PREVIEW_CONTEXT,
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
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_AI_WARNING),
        control_id: LABEL_AI_WARNING_TITLE,
        initial_text: String::new(),
        class: LabelClass::Default,
    });
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_AI_WARNING),
        control_id: LABEL_AI_WARNING_BODY,
        initial_text: String::new(),
        class: LabelClass::Default,
    });
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW_CONTEXT),
        control_id: LABEL_PREVIEW_SOURCE,
        initial_text: String::new(),
        class: LabelClass::Default,
    });
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW_CONTEXT),
        control_id: LABEL_PREVIEW_STATUS,
        initial_text: String::new(),
        class: LabelClass::Default,
    });
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW_CONTEXT),
        control_id: LABEL_PREVIEW_ATTENTION,
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
            "Poll Stats".to_string(),
        ],
    });
    commands.push(PlatformCommand::SetTabBarStyle {
        window_id,
        control_id: TAB_BAR_RIGHT,
        background_color: Color {
            r: 0x14,
            g: 0x14,
            b: 0x13,
        },
        text_color: Color {
            r: 0x87,
            g: 0x86,
            b: 0x7F,
        },
        accent_color: Color {
            r: 0x3D,
            g: 0x3D,
            b: 0x3A,
        },
        font: Some(FontDescription {
            name: Some("Segoe UI".to_string()),
            size: Some(9),
            weight: Some(FontWeight::Normal),
        }),
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
            r: 0x14,
            g: 0x14,
            b: 0x13,
        },
        text_color: Color {
            r: 0x87,
            g: 0x86,
            b: 0x7F,
        },
        accent_color: Color {
            r: 0x3D,
            g: 0x3D,
            b: 0x3A,
        },
        font: Some(FontDescription {
            name: Some("Segoe UI".to_string()),
            size: Some(9),
            weight: Some(FontWeight::Normal),
        }),
    });
    commands.push(PlatformCommand::CreateChart {
        window_id,
        parent_control_id: Some(PANEL_TAB_TRENDS),
        control_id: CHART_TRENDS,
    });
    // Static description label shown below the trend-category selector bar.
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_TAB_TRENDS),
        control_id: LABEL_TRENDS_DESCRIPTION,
        initial_text: "Top 5 products by recent activity, last 13 weeks".to_string(),
        class: LabelClass::Default,
    });

    commands.push(PlatformCommand::CreatePanel {
        window_id,
        parent_control_id: Some(PANEL_PREVIEW),
        control_id: PANEL_TAB_POLL_STATS,
    });
    commands.push(PlatformCommand::CreateRichEdit {
        window_id,
        parent_control_id: Some(PANEL_TAB_POLL_STATS),
        control_id: VIEWER_POLL_STATS,
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_JOBS),
        control_id: LABEL_JOBS_HEADER_TITLE,
        initial_text: "Jobs".to_string(),
        class: LabelClass::Default,
    });
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_JOBS),
        control_id: LABEL_JOBS_HEADER_META,
        initial_text: String::new(),
        class: LabelClass::Default,
    });
    commands.push(PlatformCommand::CreateListBox {
        window_id,
        parent_control_id: Some(PANEL_JOBS),
        control_id: TREE_JOBS,
    });

    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_PROGRESS),
        control_id: LABEL_TOKEN_PROGRESS,
        initial_text: format!("0 / {}K", TOKEN_LIMIT / 1_000),
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
        control_id: BUTTON_POLL_SOURCES,
        text: "Poll sources".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_BUTTONS),
        control_id: BUTTON_TRIAGE,
        text: "Run Triage".to_string(),
    });
    commands.push(PlatformCommand::CreateButton {
        window_id,
        parent_control_id: Some(PANEL_BUTTONS),
        control_id: BUTTON_SUMMARIZE,
        text: "Summarize Articles".to_string(),
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
        control_id: BUTTON_OPEN_BROWSER,
        text: "Open in Browser".to_string(),
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
    commands.push(PlatformCommand::CreateLabel {
        window_id,
        parent_control_id: Some(PANEL_BOTTOM),
        control_id: LABEL_OPERATION_PROGRESS,
        initial_text: String::new(),
        class: LabelClass::Default,
    });
    commands.push(PlatformCommand::CreateProgressBar {
        window_id,
        parent_control_id: Some(PANEL_BOTTOM),
        control_id: PROGRESS_OPERATION,
    });
}
