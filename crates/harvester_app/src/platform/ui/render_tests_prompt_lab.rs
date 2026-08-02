use super::*;
use harvester_core::{PromptLabRunId, PromptLabRunSummaryView, PromptLabStage, PromptLabView};
use harvester_engine::llm::ModelId;

fn completed_prompt_lab_view() -> PromptLabView {
    PromptLabView {
        visible: true,
        latest_run: Some(PromptLabRunSummaryView {
            run_id: PromptLabRunId(1),
            stage: PromptLabStage::Summary,
            status_label: "completed",
            output_json: Some("{\"ok\":true}".to_string()),
            failure_reason: None,
            input_tokens: Some(10),
            output_tokens: Some(20),
            cost_microdollars: Some(30),
            wall_ms: Some(40),
            resolved_model: Some(harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI.to_string()),
            parse_ok: Some(true),
            cache_status: Some("miss".to_string()),
        }),
        ..PromptLabView::default()
    }
}

#[test]
fn run_button_disabled_when_can_run_false() {
    let window_id = WindowId::new(10);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab.can_run = false;
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
            if *control_id == BTN_PROMPT_LAB_RUN
        )
    }));
}

#[test]
fn template_apply_button_disabled_when_validation_errors_exist() {
    let window_id = WindowId::new(15);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab.template_editor_open = true;
    view.left_pane.prompt_lab.template_dirty = true;
    view.left_pane.prompt_lab.template_validation_errors = vec!["unknown var".to_string()];
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
            if *control_id == BTN_PROMPT_LAB_TEMPLATE_APPLY
        )
    }));
}

#[test]
fn template_save_button_disabled_when_not_applied() {
    let window_id = WindowId::new(16);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab.template_editor_open = true;
    view.left_pane.prompt_lab.template_applied = false;
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
            if *control_id == BTN_PROMPT_LAB_TEMPLATE_SAVE
        )
    }));
}

#[test]
fn template_errors_are_rendered_in_template_status_label() {
    let window_id = WindowId::new(17);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab.template_editor_open = true;
    view.left_pane.prompt_lab.template_validation_errors = vec![
        "first validation problem".to_string(),
        "second validation problem".to_string(),
    ];
    let cmds = render(window_id, &view, &mut tree_state);
    let status = control_text(&cmds, LABEL_PROMPT_LAB_TEMPLATE_STATUS)
        .expect("template status text rendered");
    assert!(status.contains("first validation problem"));
    assert!(status.contains("second validation problem"));
}

#[test]
fn compare_start_disabled_by_default() {
    let window_id = WindowId::new(18);
    let mut tree_state = TreeRenderState::new();
    let view = make_view(vec![]);
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
            if *control_id == BTN_COMPARE_START
        )
    }));
}

#[test]
fn compare_start_enabled_when_two_draft_candidates() {
    let window_id = WindowId::new(19);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab.draft_candidates = vec![
        harvester_core::PromptLabCompareCandidateView {
            candidate_id: 1,
            label: "c1".to_string(),
            stage_label: "Triage".to_string(),
            model_label: "default".to_string(),
            prompt_version_label: "active".to_string(),
            has_context_override: false,
            has_template_override: false,
        },
        harvester_core::PromptLabCompareCandidateView {
            candidate_id: 2,
            label: "c2".to_string(),
            stage_label: "Triage".to_string(),
            model_label: "default".to_string(),
            prompt_version_label: "active".to_string(),
            has_context_override: false,
            has_template_override: false,
        },
    ];
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
            if *control_id == BTN_COMPARE_START
        )
    }));
}

#[test]
fn prompt_lab_section_checkbox_states_reflect_view_state() {
    let window_id = WindowId::new(33);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab.compare_section_open = true;
    view.left_pane.prompt_lab.context_section_open = false;
    view.left_pane.prompt_lab.template_section_open = true;
    view.left_pane.prompt_lab.run_details_section_open = true;
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetCheckBoxChecked { control_id, checked: true, .. }
            if *control_id == CHK_PROMPT_LAB_SECTION_COMPARE
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetCheckBoxChecked { control_id, checked: false, .. }
            if *control_id == CHK_PROMPT_LAB_SECTION_CONTEXT
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetCheckBoxChecked { control_id, checked: true, .. }
            if *control_id == CHK_PROMPT_LAB_SECTION_RUN_DETAILS
        )
    }));
}

#[test]
fn prompt_lab_mode_and_stage_radio_states_reflect_view_state() {
    let window_id = WindowId::new(38);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab.advanced_mode = true;
    view.left_pane.prompt_lab.selected_stage = PromptLabStage::Summary;
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetRadioButtonChecked { control_id, checked: true, .. }
            if *control_id == BTN_PROMPT_LAB_MODE_ADVANCED
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetRadioButtonChecked { control_id, checked: false, .. }
            if *control_id == BTN_PROMPT_LAB_MODE_BASIC
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetRadioButtonChecked { control_id, checked: true, .. }
            if *control_id == BTN_STAGE_SUMMARY
        )
    }));
}

#[test]
fn prompt_lab_tab_advanced_layout_does_not_depend_on_legacy_visible_flag() {
    let window_id = WindowId::new(39);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.left_tab = LeftTab::PromptLab;
    view.left_pane.prompt_lab.visible = false;
    view.left_pane.prompt_lab.advanced_mode = true;

    let cmds = render(window_id, &view, &mut tree_state);
    let rules = cmds
        .iter()
        .find_map(|cmd| match cmd {
            PlatformCommand::DefineLayout { rules, .. } => Some(rules),
            _ => None,
        })
        .expect("DefineLayout emitted");

    let compare_header_size = rules
        .iter()
        .find(|rule| rule.control_id == PANEL_PROMPT_LAB_COMPARE_HEADER_ROW)
        .and_then(|rule| rule.fixed_size)
        .expect("compare header fixed size");
    assert_ne!(compare_header_size, 0);
}

#[test]
fn prompt_lab_model_selector_defaults_to_index_zero_on_first_render() {
    let window_id = WindowId::new(34);
    let mut tree_state = TreeRenderState::new();
    let view = make_view(vec![]);

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetComboBoxSelection {
                control_id,
                selected_index: Some(0),
                ..
            } if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR
        )
    }));
}

#[test]
fn prompt_lab_model_selector_emits_catalog_items() {
    let window_id = WindowId::new(35);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab.model_catalog = vec![
        ModelId::new(
            harvester_engine::llm::ProviderKind::OpenAi,
            harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI,
        ),
        ModelId::new(harvester_engine::llm::ProviderKind::OpenAi, "o3-mini"),
    ];

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetComboBoxItems {
                control_id,
                items,
                ..
            } if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR
                && items == &vec![
                    "Default".to_string(),
                    harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI.to_string(),
                    "o3-mini".to_string(),
                ]
        )
    }));
}

#[test]
fn prompt_lab_model_selector_replays_on_hidden_to_visible_transition() {
    let window_id = WindowId::new(36);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.left_tab = LeftTab::PromptLab;
    view.left_pane.prompt_lab.model_catalog = vec![ModelId::new(
        harvester_engine::llm::ProviderKind::OpenAi,
        harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI,
    )];

    let initial_cmds = render(window_id, &view, &mut tree_state);
    assert!(initial_cmds.iter().any(|cmd| {
        matches!(cmd, PlatformCommand::SetComboBoxItems { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
    }));
    assert!(initial_cmds.iter().any(|cmd| {
        matches!(cmd, PlatformCommand::SetComboBoxSelection { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
    }));

    view.left_pane.left_tab = LeftTab::Jobs;
    let _ = render(window_id, &view, &mut tree_state);

    view.left_pane.left_tab = LeftTab::PromptLab;
    let reopen_cmds = render(window_id, &view, &mut tree_state);
    assert!(reopen_cmds.iter().any(|cmd| {
        matches!(cmd, PlatformCommand::SetComboBoxItems { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
    }));
    assert!(reopen_cmds.iter().any(|cmd| {
        matches!(cmd, PlatformCommand::SetComboBoxSelection { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
    }));
}

#[test]
fn prompt_lab_model_selector_unchanged_visible_state_is_idempotent() {
    let window_id = WindowId::new(37);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab.visible = true;
    view.left_pane.prompt_lab.model_catalog = vec![ModelId::new(
        harvester_engine::llm::ProviderKind::OpenAi,
        harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI,
    )];

    let _ = render(window_id, &view, &mut tree_state);
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(!cmds.iter().any(|cmd| {
        matches!(cmd, PlatformCommand::SetComboBoxItems { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
    }));
    assert!(!cmds.iter().any(|cmd| {
        matches!(cmd, PlatformCommand::SetComboBoxSelection { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
    }));
}

#[test]
fn resolve_button_enabled_when_not_pending() {
    let window_id = WindowId::new(12);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab.resolve_pending = true;
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
            if *control_id == BTN_PROMPT_LAB_RESOLVE
        )
    }));

    view.left_pane.prompt_lab.resolve_pending = false;
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
            if *control_id == BTN_PROMPT_LAB_RESOLVE
        )
    }));
}

/// The Prompt Lab now lives in its own tab; a completed run does NOT override VIEWER_PREVIEW.
#[test]
fn prompt_lab_run_does_not_override_summary_viewer() {
    let window_id = WindowId::new(13);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab = completed_prompt_lab_view();
    let cmds = render(window_id, &view, &mut tree_state);
    // The Prompt Lab output JSON should NOT appear in VIEWER_PREVIEW.
    assert!(!cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetRichEditContent { control_id, rtf_text, .. }
            if *control_id == VIEWER_PREVIEW && rtf_text.contains("ok")
        )
    }));
    // LABEL_PREVIEW_HEADER should not say "Prompt Lab".
    assert!(!cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlText { control_id, text, .. }
            if *control_id == LABEL_PREVIEW_HEADER && text.contains("Prompt Lab")
        )
    }));
}

#[test]
fn render_idempotent_on_unchanged_prompt_lab_view() {
    let window_id = WindowId::new(14);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.prompt_lab = completed_prompt_lab_view();
    let _ = render(window_id, &view, &mut tree_state);
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(!cmds
        .iter()
        .any(|cmd| matches!(cmd, PlatformCommand::SetRichEditContent { .. })));
}
