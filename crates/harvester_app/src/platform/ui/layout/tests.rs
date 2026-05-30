use commanductui::types::{ControlId, DockStyle, LayoutRule};
use commanductui::{Color, PlatformCommand, StyleId, WindowId};
use harvester_core::{AppTab, LeftTab};

use super::super::constants::*;
use super::super::groups::bottom_buttons::FOOTER_BUTTON_VERTICAL_MARGIN;
use super::rules::{
    AI_WARNING_ROW_HEIGHT, LLM_QUOTA_BAR_WIDTH, LLM_QUOTA_LABEL_WIDTH, PREVIEW_CONTEXT_ROW_HEIGHT,
    PROMPT_LAB_ROW_HEIGHT_ACTION, PROMPT_LAB_ROW_HEIGHT_CONTEXT_INPUT,
    PROMPT_LAB_ROW_HEIGHT_RUN_DETAILS_BODY, PROMPT_LAB_ROW_HEIGHT_STANDARD,
    PROMPT_LAB_ROW_HEIGHT_STATUS, PROMPT_LAB_ROW_HEIGHT_TEMPLATE_EDITOR_INPUT,
    PROMPT_LAB_TEMPLATE_TOGGLE_BUTTON_WIDTH, TOKEN_COUNTS_LABEL_WIDTH, TOKEN_METER_BAR_WIDTH,
    TOKEN_METER_LABEL_WIDTH,
};
use super::{build_layout_command, initial_commands, LayoutConfig, PromptLabLayoutConfig};

fn layout_rules_for_prompt_lab(prompt_lab: PromptLabLayoutConfig) -> Vec<LayoutRule> {
    let cmd = build_layout_command(
        WindowId::new(99),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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

fn layout_rules_for_left_tab(left_tab: LeftTab) -> Vec<LayoutRule> {
    let cmd = build_layout_command(
        WindowId::new(98),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
            active_tab: AppTab::Summary,
            left_tab,
            prompt_lab: PromptLabLayoutConfig {
                visible: left_tab == LeftTab::PromptLab,
                advanced_mode: false,
                compare_section_open: false,
                context_section_open: false,
                template_section_open: false,
                run_details_section_open: false,
                template_editor_open: false,
            },
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
fn operation_controls_have_width_when_visible() {
    let cmd = build_layout_command(
        WindowId::new(88),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: true,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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
    assert_eq!(fixed_size_for(&rules, PROGRESS_OPERATION), 80);
    assert_eq!(fixed_size_for(&rules, LABEL_OPERATION_PROGRESS), 170);
    assert_eq!(
        fixed_size_for(&rules, PROGRESS_LLM_QUOTA),
        LLM_QUOTA_BAR_WIDTH
    );
    assert_eq!(
        fixed_size_for(&rules, LABEL_LLM_QUOTA),
        LLM_QUOTA_LABEL_WIDTH
    );
}

#[test]
fn operation_controls_collapse_when_hidden() {
    let cmd = build_layout_command(
        WindowId::new(89),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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
    assert_eq!(fixed_size_for(&rules, PROGRESS_OPERATION), 0);
    assert_eq!(fixed_size_for(&rules, LABEL_OPERATION_PROGRESS), 0);
    assert_eq!(
        fixed_size_for(&rules, PROGRESS_LLM_QUOTA),
        LLM_QUOTA_BAR_WIDTH
    );
    assert_eq!(
        fixed_size_for(&rules, LABEL_LLM_QUOTA),
        LLM_QUOTA_LABEL_WIDTH
    );
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
            PlatformCommand::CreateToggleSwitch {
                control_id,
                checked: true,
                ..
            } if *control_id == TS_JOBS_SCOPE
        )),
        "jobs scope toggle switch should be created"
    );
    assert!(
        commands.iter().any(|cmd| matches!(
            cmd,
            PlatformCommand::CreateInput { control_id, .. }
                if *control_id == INPUT_JOBS_SEARCH
        )),
        "jobs search input should be created"
    );
}

#[test]
fn jobs_search_input_has_layout_rule_on_jobs_tab() {
    let rules = layout_rules_for_left_tab(LeftTab::Jobs);
    let search_rule = rules
        .iter()
        .find(|rule| rule.control_id == INPUT_JOBS_SEARCH)
        .expect("jobs search rule");
    let tree_rule = rules
        .iter()
        .find(|rule| rule.control_id == TREE_JOBS)
        .expect("jobs tree rule");

    assert_eq!(search_rule.parent_control_id, Some(PANEL_JOBS));
    assert_eq!(search_rule.dock_style, DockStyle::Top);
    assert_eq!(search_rule.fixed_size, Some(24));
    assert!(search_rule.order < tree_rule.order);

    for tab in [
        LeftTab::TriageReview,
        LeftTab::TriageResults,
        LeftTab::PromptLab,
    ] {
        let rules = layout_rules_for_left_tab(tab);
        assert_eq!(fixed_size_for(&rules, INPUT_JOBS_SEARCH), 0);
    }
}

#[test]
fn jobs_search_input_receives_default_input_style() {
    let commands = initial_commands(WindowId::new(2));
    assert!(commands.iter().any(|cmd| matches!(
        cmd,
        PlatformCommand::ApplyStyleToControl {
            control_id,
            style_id,
            ..
        } if *control_id == INPUT_JOBS_SEARCH && *style_id == StyleId::DefaultInput
    )));
}

#[test]
fn initial_commands_do_not_show_window_directly() {
    let commands = initial_commands(WindowId::new(3));
    assert!(!commands
        .iter()
        .any(|cmd| matches!(cmd, PlatformCommand::ShowWindow { .. })));
    assert!(!commands
        .iter()
        .any(|cmd| matches!(cmd, PlatformCommand::SignalMainWindowUISetupComplete { .. })));
}

#[test]
fn toolbar_contains_scope_and_token_controls_on_same_row() {
    let cmd = build_layout_command(
        WindowId::new(77),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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

    let panel_progress = rules
        .iter()
        .find(|r| r.control_id == PANEL_PROGRESS)
        .expect("progress panel rule");
    assert_eq!(panel_progress.parent_control_id, Some(PANEL_TOOLBAR));

    let token_label = rules
        .iter()
        .find(|r| r.control_id == LABEL_TOKEN_PROGRESS)
        .expect("token label rule");
    assert_eq!(token_label.parent_control_id, Some(PANEL_PROGRESS));
    assert_eq!(token_label.dock_style, DockStyle::Right);
    assert_eq!(token_label.fixed_size, Some(TOKEN_METER_LABEL_WIDTH));

    let token_bar = rules
        .iter()
        .find(|r| r.control_id == PROGRESS_TOKENS)
        .expect("token bar rule");
    assert_eq!(token_bar.parent_control_id, Some(PANEL_PROGRESS));
    assert_eq!(token_bar.dock_style, DockStyle::Right);
    assert_eq!(token_bar.fixed_size, Some(TOKEN_METER_BAR_WIDTH));

    let token_counts = rules
        .iter()
        .find(|r| r.control_id == LABEL_TOKEN_COUNTS)
        .expect("token counts label rule");
    assert_eq!(token_counts.parent_control_id, Some(PANEL_PROGRESS));
    assert_eq!(token_counts.dock_style, DockStyle::Right);
    assert_eq!(token_counts.fixed_size, Some(TOKEN_COUNTS_LABEL_WIDTH));
    // Order 2 docks further left than the bar (order 1) and label (order 0).
    assert_eq!(token_counts.order, 2);
}

#[test]
fn jobs_panel_visible_for_all_job_oriented_tabs() {
    for left_tab in [LeftTab::Jobs, LeftTab::TriageReview, LeftTab::TriageResults] {
        let cmd = build_layout_command(
            WindowId::new(42),
            LayoutConfig {
                left_panel_width: 600,
                input_panel_visible: true,
                operation_progress_visible: false,
                left_header_meta_visible: true,
                ai_warning_banner_visible: false,
                preview_header_override_visible: false,
                preview_context_visible: false,
                preview_attention_visible: false,
                signal_candidate_preview_visible: false,
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

#[test]
fn metadata_style_is_transparent_and_applied_to_metadata_labels() {
    let commands = initial_commands(WindowId::new(2));
    let metadata_style = commands.iter().find_map(|cmd| match cmd {
        PlatformCommand::DefineStyle { style_id, style } if *style_id == StyleId::MetadataText => {
            Some(style)
        }
        _ => None,
    });
    let metadata_style = metadata_style.expect("MetadataText style should be defined");
    assert_eq!(metadata_style.background_color, None);
    assert_eq!(
        metadata_style.text_color,
        Some(Color {
            r: 0x87,
            g: 0x86,
            b: 0x7F,
        })
    );

    for label_id in [
        LABEL_JOBS_HEADER_META,
        LABEL_PREVIEW_SOURCE_CAPTION,
        LABEL_PREVIEW_STATUS,
        LABEL_PREVIEW_ATTENTION,
        LABEL_TRENDS_DESCRIPTION,
    ] {
        assert!(
            commands.iter().any(|cmd| matches!(
                cmd,
                PlatformCommand::ApplyStyleToControl {
                    control_id,
                    style_id,
                    ..
                } if *control_id == label_id && *style_id == StyleId::MetadataText
            )),
            "metadata label {:?} should use MetadataText",
            label_id
        );
    }
}

#[test]
fn preview_source_link_uses_link_button_style() {
    let commands = initial_commands(WindowId::new(3));
    let link_style = commands.iter().find_map(|cmd| match cmd {
        PlatformCommand::DefineStyle { style_id, style } if *style_id == StyleId::LinkButton => {
            Some(style)
        }
        _ => None,
    });
    let link_style = link_style.expect("LinkButton style should be defined");
    assert_eq!(
        link_style.text_alignment,
        Some(commanductui::TextAlignment::Left)
    );
    assert_eq!(
        link_style.text_color,
        Some(Color {
            r: 0xC9,
            g: 0x64,
            b: 0x42,
        })
    );
    assert!(commands.iter().any(|cmd| matches!(
        cmd,
        PlatformCommand::ApplyStyleToControl {
            control_id,
            style_id,
            ..
        } if *control_id == BUTTON_PREVIEW_SOURCE_LINK && *style_id == StyleId::LinkButton
    )));
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
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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
    for &control_id in &[
        PANEL_TAB_TRIAGE,
        PANEL_TAB_BRIEFING,
        PANEL_TAB_TRENDS,
        PANEL_TAB_POLL_STATS,
    ] {
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
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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
        PANEL_TAB_POLL_STATS,
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
fn preview_context_row_is_tall_enough_for_metadata_label() {
    let cmd = build_layout_command(
        WindowId::new(32),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: true,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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
    let context_row = rules
        .iter()
        .find(|r| r.control_id == PANEL_PREVIEW_CONTEXT)
        .expect("preview context row rule");
    assert_eq!(context_row.fixed_size, Some(PREVIEW_CONTEXT_ROW_HEIGHT));
    assert!(
        context_row.fixed_size.unwrap_or_default() - context_row.margin.1 - context_row.margin.3
            >= 26,
        "preview metadata row should leave enough vertical room for styled label text"
    );
}

#[test]
fn ai_warning_controls_created_in_initial_commands() {
    let commands = initial_commands(WindowId::new(33));
    assert!(commands.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::CreatePanel { control_id, .. } if *control_id == PANEL_AI_WARNING
        )
    }));
    assert!(commands.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::CreateLabel { control_id, .. } if *control_id == LABEL_AI_WARNING_TITLE
        )
    }));
    assert!(commands.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::CreateLabel { control_id, .. } if *control_id == LABEL_AI_WARNING_BODY
        )
    }));
}

#[test]
fn ai_warning_row_sits_immediately_below_right_tab_bar() {
    let cmd = build_layout_command(
        WindowId::new(34),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: true,
            preview_header_override_visible: true,
            preview_context_visible: true,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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

    let tab_bar = rules
        .iter()
        .find(|r| r.control_id == TAB_BAR_RIGHT)
        .expect("right tab bar rule");
    let ai_warning = rules
        .iter()
        .find(|r| r.control_id == PANEL_AI_WARNING)
        .expect("ai warning rule");
    let preview_header = rules
        .iter()
        .find(|r| r.control_id == LABEL_PREVIEW_HEADER)
        .expect("preview header rule");

    assert_eq!(tab_bar.parent_control_id, Some(PANEL_PREVIEW));
    assert_eq!(tab_bar.order, 0);
    assert_eq!(ai_warning.parent_control_id, Some(PANEL_PREVIEW));
    assert_eq!(ai_warning.order, 1);
    assert_eq!(preview_header.parent_control_id, Some(PANEL_PREVIEW));
    assert_eq!(preview_header.order, 2);
}

#[test]
fn ai_warning_row_collapses_when_hidden() {
    let hidden = build_layout_command(
        WindowId::new(35),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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
    let hidden_rules = match hidden {
        PlatformCommand::DefineLayout { rules, .. } => rules,
        _ => panic!("expected DefineLayout"),
    };
    assert_eq!(fixed_size_for(&hidden_rules, PANEL_AI_WARNING), 0);

    let visible = build_layout_command(
        WindowId::new(36),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: true,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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
    let visible_rules = match visible {
        PlatformCommand::DefineLayout { rules, .. } => rules,
        _ => panic!("expected DefineLayout"),
    };
    assert_eq!(
        fixed_size_for(&visible_rules, PANEL_AI_WARNING),
        AI_WARNING_ROW_HEIGHT
    );
}

#[test]
fn expanded_layout_includes_all_controls() {
    let cmd = build_layout_command(
        WindowId::new(4),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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

#[test]
fn all_tab_panels_receive_panel_background_dark_theme_style() {
    let cmds = initial_commands(WindowId::new(99));
    for panel_id in [
        PANEL_TAB_TRIAGE,
        PANEL_TAB_SUMMARY,
        PANEL_TAB_BRIEFING,
        PANEL_TAB_TRENDS,
        PANEL_TAB_POLL_STATS,
    ] {
        let has_style = cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::ApplyStyleToControl {
                    control_id,
                    style_id: StyleId::PanelBackground,
                    ..
                } if *control_id == panel_id
            )
        });
        assert!(
            has_style,
            "PANEL {:?} should receive PanelBackground style in initial_commands",
            panel_id
        );
    }
}

#[test]
fn all_right_pane_viewers_receive_viewer_readable_style() {
    let cmds = initial_commands(WindowId::new(99));
    for viewer_id in [
        VIEWER_PREVIEW,
        VIEWER_TRIAGE,
        VIEWER_BRIEFING,
        VIEWER_POLL_STATS,
    ] {
        let has_style = cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::ApplyStyleToControl {
                    control_id,
                    style_id: StyleId::ViewerReadable,
                    ..
                } if *control_id == viewer_id
            )
        });
        assert!(
            has_style,
            "VIEWER {:?} should receive ViewerReadable style",
            viewer_id
        );
    }
}

#[test]
fn secondary_footer_buttons_use_secondary_button_style() {
    let cmds = initial_commands(WindowId::new(99));
    for button_id in [
        BUTTON_TRIAGE,
        BUTTON_SUMMARIZE,
        BUTTON_BRIEFING,
        BUTTON_ARCHIVE,
    ] {
        let has_style = cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::ApplyStyleToControl {
                    control_id,
                    style_id: StyleId::SecondaryButton,
                    ..
                } if *control_id == button_id
            )
        });
        assert!(
            has_style,
            "BUTTON {:?} should receive SecondaryButton style",
            button_id
        );
    }
}

#[test]
fn poll_sources_uses_primary_button_style() {
    let cmds = initial_commands(WindowId::new(100));
    let has_style = cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::ApplyStyleToControl {
                control_id,
                style_id: StyleId::PrimaryButton,
                ..
            } if *control_id == BUTTON_POLL_SOURCES
        )
    });
    assert!(
        has_style,
        "BUTTON_POLL_SOURCES should receive PrimaryButton style"
    );
}

#[test]
fn preview_viewers_use_editorial_inner_margins() {
    let cmd = build_layout_command(
        WindowId::new(109),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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

    for viewer_id in [
        VIEWER_PREVIEW,
        VIEWER_TRIAGE,
        VIEWER_BRIEFING,
        VIEWER_POLL_STATS,
    ] {
        let rule = rules
            .iter()
            .find(|r| r.control_id == viewer_id)
            .expect("viewer rule");
        assert_eq!(
            rule.margin,
            (18, 16, 18, 24),
            "VIEWER {:?} should use the roomier reading margins",
            viewer_id
        );
    }
}

#[test]
fn footer_buttons_share_a_common_vertical_alignment() {
    let cmd = build_layout_command(
        WindowId::new(110),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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

    for button_id in [
        BUTTON_STOP,
        BUTTON_BRIEFING,
        BUTTON_SUMMARIZE,
        BUTTON_TRIAGE,
        BUTTON_POLL_SOURCES,
        BUTTON_ARCHIVE,
    ] {
        let rule = rules
            .iter()
            .find(|r| r.control_id == button_id)
            .expect("button rule");
        assert_eq!(
            (rule.margin.0, rule.margin.2),
            FOOTER_BUTTON_VERTICAL_MARGIN,
            "BUTTON {:?} should share the footer row baseline",
            button_id
        );
    }
}

#[test]
fn footer_buttons_follow_workflow_order() {
    let cmd = build_layout_command(
        WindowId::new(111),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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

    let ordered_buttons: Vec<_> = rules
        .iter()
        .filter(|rule| rule.parent_control_id == Some(PANEL_BUTTONS))
        .map(|rule| (rule.order, rule.control_id))
        .collect();

    assert_eq!(
        ordered_buttons,
        vec![
            (0, BUTTON_STOP),
            (1, BUTTON_POLL_SOURCES),
            (2, BUTTON_TRIAGE),
            (3, BUTTON_SUMMARIZE),
            (4, BUTTON_BRIEFING),
            (5, BUTTON_ARCHIVE),
        ]
    );
}

#[test]
fn preview_context_row_uses_caption_then_fill_link_layout() {
    let cmd = build_layout_command(
        WindowId::new(113),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: true,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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

    let caption = rules
        .iter()
        .find(|rule| rule.control_id == LABEL_PREVIEW_SOURCE_CAPTION)
        .expect("caption rule");
    assert_eq!(caption.dock_style, DockStyle::Left);
    assert_eq!(caption.fixed_size, Some(68));

    let link = rules
        .iter()
        .find(|rule| rule.control_id == BUTTON_PREVIEW_SOURCE_LINK)
        .expect("link rule");
    assert_eq!(link.dock_style, DockStyle::Fill);
    assert_eq!(link.margin, (0, 0, 12, 0));
}

#[test]
fn stop_button_width_matches_standard_footer_buttons() {
    let cmd = build_layout_command(
        WindowId::new(112),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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
    let stop_width = rules
        .iter()
        .find(|r| r.control_id == BUTTON_STOP)
        .and_then(|r| r.fixed_size)
        .expect("stop button width");
    let poll_width = rules
        .iter()
        .find(|r| r.control_id == BUTTON_POLL_SOURCES)
        .and_then(|r| r.fixed_size)
        .expect("poll button width");
    assert_eq!(stop_width, poll_width);
}

#[test]
fn footer_button_row_is_tall_enough_for_primary_action_presence() {
    let cmd = build_layout_command(
        WindowId::new(113),
        LayoutConfig {
            left_panel_width: 600,
            input_panel_visible: true,
            operation_progress_visible: false,
            left_header_meta_visible: true,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
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
    let panel_buttons = rules
        .iter()
        .find(|r| r.control_id == PANEL_BUTTONS)
        .expect("buttons panel rule");
    assert_eq!(panel_buttons.fixed_size, Some(56));
}
