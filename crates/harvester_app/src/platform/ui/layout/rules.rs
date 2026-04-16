use commanductui::types::{ControlId, DockStyle, LayoutRule};
use harvester_core::{AppTab, LeftTab, INPUT_PANEL_FIXED_WIDTH};

use super::super::constants::*;
use super::PromptLabLayoutConfig;

pub(super) const PROMPT_LAB_ROW_HEIGHT_STANDARD: i32 = 26;
pub(super) const PROMPT_LAB_ROW_HEIGHT_ACTION: i32 = 28;
pub(super) const PROMPT_LAB_ROW_HEIGHT_CONTEXT_INPUT: i32 = 150;
pub(super) const PROMPT_LAB_ROW_HEIGHT_STATUS: i32 = 24;
pub(super) const PROMPT_LAB_ROW_HEIGHT_TEMPLATE_EDITOR_INPUT: i32 = 120;
pub(super) const PROMPT_LAB_ROW_HEIGHT_RUN_DETAILS_BODY: i32 = 42;
pub(super) const PROMPT_LAB_TEMPLATE_TOGGLE_BUTTON_WIDTH: i32 = 120;
pub(super) const AI_WARNING_ROW_HEIGHT: i32 = 42;
pub(super) const PREVIEW_CONTEXT_ROW_HEIGHT: i32 = 32;
pub(super) const TOKEN_METER_BAR_WIDTH: i32 = 190;
pub(super) const TOKEN_METER_LABEL_WIDTH: i32 = 120;

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

#[allow(clippy::too_many_arguments)]
pub(super) fn build_layout_rules(
    left_panel_width: i32,
    input_panel_visible: bool,
    operation_progress_visible: bool,
    left_header_meta_visible: bool,
    ai_warning_banner_visible: bool,
    preview_header_override_visible: bool,
    preview_context_visible: bool,
    _preview_attention_visible: bool,
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
    let operation_progress_bar_width = if operation_progress_visible { 80 } else { 0 };
    let operation_progress_label_width = if operation_progress_visible { 120 } else { 0 };
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
        // PANEL_TOOLBAR: topmost docked row for scope filter + token usage.
        LayoutRule {
            control_id: PANEL_TOOLBAR,
            parent_control_id: None,
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(42),
            margin: (0, 0, 0, 0),
        },
        // TS_JOBS_SCOPE: left side of toolbar.
        LayoutRule {
            control_id: TS_JOBS_SCOPE,
            parent_control_id: Some(PANEL_TOOLBAR),
            dock_style: DockStyle::Left,
            order: 10,
            fixed_size: Some(188),
            margin: (16, 8, 12, 8),
        },
        // PANEL_PROGRESS: container for the token controls on the same toolbar row.
        LayoutRule {
            control_id: PANEL_PROGRESS,
            parent_control_id: Some(PANEL_TOOLBAR),
            dock_style: DockStyle::Fill,
            order: 20,
            fixed_size: None,
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: LABEL_TOKEN_PROGRESS,
            parent_control_id: Some(PANEL_PROGRESS),
            dock_style: DockStyle::Right,
            order: 0,
            fixed_size: Some(TOKEN_METER_LABEL_WIDTH),
            margin: (10, 11, 16, 9),
        },
        LayoutRule {
            control_id: PROGRESS_TOKENS,
            parent_control_id: Some(PANEL_PROGRESS),
            dock_style: DockStyle::Right,
            order: 1,
            fixed_size: Some(TOKEN_METER_BAR_WIDTH),
            margin: (0, 14, 18, 14),
        },
        LayoutRule {
            control_id: PANEL_BOTTOM,
            parent_control_id: None,
            dock_style: DockStyle::Bottom,
            order: 100,
            fixed_size: Some(32),
            margin: (0, 8, 0, 0),
        },
        LayoutRule {
            control_id: PANEL_BUTTONS,
            parent_control_id: None,
            dock_style: DockStyle::Bottom,
            order: 110,
            fixed_size: Some(56),
            margin: (0, 0, 0, 0),
        },
        // PANEL_LEFT replaces the old root-level PANEL_INPUT + PANEL_JOBS.
        LayoutRule {
            control_id: PANEL_LEFT,
            parent_control_id: None,
            dock_style: DockStyle::Left,
            order: 200,
            fixed_size: Some(left_panel_width),
            margin: (12, 12, 6, 14),
        },
        // Left-pane tab bar.
        LayoutRule {
            control_id: TAB_BAR_LEFT,
            parent_control_id: Some(PANEL_LEFT),
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(28),
            margin: (0, 0, 6, 0),
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
                if show {
                    DockStyle::Fill
                } else {
                    DockStyle::Top
                }
            },
            order: 1,
            fixed_size: {
                let show = matches!(
                    left_tab,
                    LeftTab::Jobs | LeftTab::TriageReview | LeftTab::TriageResults
                );
                if show {
                    None
                } else {
                    Some(0)
                }
            },
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: PANEL_INPUT,
            parent_control_id: Some(PANEL_LEFT_JOBS),
            dock_style: DockStyle::Left,
            order: 0,
            fixed_size: Some(input_width),
            margin: (0, 0, 10, 0),
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
            control_id: LABEL_JOBS_HEADER_TITLE,
            parent_control_id: Some(PANEL_JOBS),
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(0),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: LABEL_JOBS_HEADER_META,
            parent_control_id: Some(PANEL_JOBS),
            dock_style: DockStyle::Top,
            order: 1,
            fixed_size: if left_header_meta_visible {
                Some(18)
            } else {
                Some(0)
            },
            margin: (2, 2, 8, 0),
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
            margin: (0, 6, 0, 6),
        },
        LayoutRule {
            control_id: SPLITTER_MAIN,
            parent_control_id: None,
            dock_style: DockStyle::Left,
            order: 205,
            fixed_size: Some(2),
            margin: (4, 20, 4, 20),
        },
        LayoutRule {
            control_id: PANEL_PREVIEW,
            parent_control_id: None,
            dock_style: DockStyle::Fill,
            order: 310,
            fixed_size: None,
            margin: (8, 20, 16, 20),
        },
        LayoutRule {
            control_id: LABEL_PREVIEW_HEADER,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: DockStyle::Top,
            order: 2,
            fixed_size: if preview_header_override_visible {
                Some(18)
            } else {
                Some(0)
            },
            margin: (2, 2, 8, 0),
        },
        LayoutRule {
            control_id: PANEL_AI_WARNING,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: DockStyle::Top,
            order: 1,
            fixed_size: if ai_warning_banner_visible {
                Some(AI_WARNING_ROW_HEIGHT)
            } else {
                Some(0)
            },
            margin: (0, 2, 0, 6),
        },
        LayoutRule {
            control_id: PANEL_PREVIEW_CONTEXT,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: DockStyle::Top,
            order: 3,
            fixed_size: if preview_context_visible {
                Some(PREVIEW_CONTEXT_ROW_HEIGHT)
            } else {
                Some(0)
            },
            margin: (2, 4, 4, 2),
        },
        LayoutRule {
            control_id: LABEL_AI_WARNING_TITLE,
            parent_control_id: Some(PANEL_AI_WARNING),
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(18),
            margin: (12, 6, 12, 0),
        },
        LayoutRule {
            control_id: LABEL_AI_WARNING_BODY,
            parent_control_id: Some(PANEL_AI_WARNING),
            dock_style: DockStyle::Fill,
            order: 1,
            fixed_size: None,
            margin: (12, 0, 12, 8),
        },
        LayoutRule {
            control_id: LABEL_PREVIEW_SOURCE,
            parent_control_id: Some(PANEL_PREVIEW_CONTEXT),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (0, 0, 8, 0),
        },
        LayoutRule {
            control_id: LABEL_PREVIEW_ATTENTION,
            parent_control_id: Some(PANEL_PREVIEW_CONTEXT),
            dock_style: DockStyle::Right,
            order: 10,
            fixed_size: Some(0),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: LABEL_PREVIEW_STATUS,
            parent_control_id: Some(PANEL_PREVIEW_CONTEXT),
            dock_style: DockStyle::Right,
            order: 20,
            fixed_size: Some(0),
            margin: (0, 0, 8, 0),
        },
        // Right-pane tab bar (custom TabBar widget).
        LayoutRule {
            control_id: TAB_BAR_RIGHT,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: DockStyle::Top,
            order: 0,
            fixed_size: Some(28),
            margin: (0, 0, 6, 0),
        },
        // Tab content panels — active tab fills remaining space; inactive ones collapse.
        LayoutRule {
            control_id: PANEL_TAB_TRIAGE,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: tab_dock(AppTab::Triage),
            order: 4,
            fixed_size: tab_size(AppTab::Triage),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: VIEWER_TRIAGE,
            parent_control_id: Some(PANEL_TAB_TRIAGE),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (18, 16, 18, 24),
        },
        LayoutRule {
            control_id: PANEL_TAB_SUMMARY,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: tab_dock(AppTab::Summary),
            order: 5,
            fixed_size: tab_size(AppTab::Summary),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: VIEWER_PREVIEW,
            parent_control_id: Some(PANEL_TAB_SUMMARY),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (18, 16, 18, 24),
        },
        LayoutRule {
            control_id: PANEL_TAB_BRIEFING,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: tab_dock(AppTab::Briefing),
            order: 6,
            fixed_size: tab_size(AppTab::Briefing),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: VIEWER_BRIEFING,
            parent_control_id: Some(PANEL_TAB_BRIEFING),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (18, 16, 18, 24),
        },
        LayoutRule {
            control_id: PANEL_TAB_TRENDS,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: tab_dock(AppTab::Trends),
            order: 7,
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
        // Static description label between the category selector and the chart.
        LayoutRule {
            control_id: LABEL_TRENDS_DESCRIPTION,
            parent_control_id: Some(PANEL_TAB_TRENDS),
            dock_style: DockStyle::Top,
            order: 1,
            fixed_size: Some(20),
            margin: (18, 10, 8, 16),
        },
        LayoutRule {
            control_id: CHART_TRENDS,
            parent_control_id: Some(PANEL_TAB_TRENDS),
            dock_style: DockStyle::Fill,
            order: 2,
            fixed_size: None,
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: PANEL_TAB_POLL_STATS,
            parent_control_id: Some(PANEL_PREVIEW),
            dock_style: tab_dock(AppTab::PollStats),
            order: 8,
            fixed_size: tab_size(AppTab::PollStats),
            margin: (0, 0, 0, 0),
        },
        LayoutRule {
            control_id: VIEWER_POLL_STATS,
            parent_control_id: Some(PANEL_TAB_POLL_STATS),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (18, 16, 18, 24),
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
            margin: (10, 14, 8, 14),
        },
        LayoutRule {
            control_id: PROGRESS_OPERATION,
            parent_control_id: Some(PANEL_BOTTOM),
            dock_style: DockStyle::Right,
            order: 10,
            fixed_size: Some(operation_progress_bar_width),
            margin: (12, 10, 8, 4),
        },
        LayoutRule {
            control_id: LABEL_OPERATION_PROGRESS,
            parent_control_id: Some(PANEL_BOTTOM),
            dock_style: DockStyle::Right,
            order: 20,
            fixed_size: Some(operation_progress_label_width),
            margin: (8, 10, 6, 10),
        },
        LayoutRule {
            control_id: BUTTON_STOP,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 0,
            fixed_size: Some(144),
            margin: (14, 6, 22, 6),
        },
        LayoutRule {
            control_id: BUTTON_POLL_SOURCES,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 1,
            fixed_size: Some(144),
            margin: (0, 6, 6, 6),
        },
        LayoutRule {
            control_id: BUTTON_TRIAGE,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 2,
            fixed_size: Some(144),
            margin: (0, 6, 6, 6),
        },
        LayoutRule {
            control_id: BUTTON_BRIEFING,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 3,
            fixed_size: Some(168),
            margin: (0, 6, 6, 6),
        },
        LayoutRule {
            control_id: BUTTON_OPEN_BROWSER,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 4,
            fixed_size: Some(144),
            margin: (0, 6, 6, 6),
        },
        LayoutRule {
            control_id: BUTTON_ARCHIVE,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 5,
            fixed_size: Some(112),
            margin: (0, 6, 6, 6),
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
