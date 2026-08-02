use super::super::constants::*;
use super::super::render_list_box::{build_list_box_item, build_list_box_items};
use super::*;
use commanductui::{ListBoxItemId, ListBoxRowDensity};
use harvester_core::Stage;
use harvester_core::{
    JobFilterStatus, JobListScope, JobResultKind, JobRowView,
    LeftPaneHeaderView,
    SessionState, StopFinishButtonState, StopPolicy,
};
use harvester_core::{
    JobOrigin, ScoreBand, SignalCandidateOutcome, SignalCandidateRow, SignalCandidateRowState,
};
use std::sync::Once;
#[path = "render_tests_buttons.rs"]
mod button_tests;
#[path = "render_tests_list_box.rs"]
mod list_box_tests;
#[path = "render_tests_preview.rs"]
mod preview_tests;
#[path = "render_tests_prompt_lab.rs"]
mod prompt_lab_tests;
#[path = "render_tests_status_bar.rs"]
mod status_bar_tests;
#[path = "render_tests_trends.rs"]
mod trends_tests;

fn init_logging() {
    static INIT: Once = Once::new();
    INIT.call_once(engine_logging::initialize_for_tests);
}

fn make_job(
    job_id: u64,
    url: &str,
    stage: Stage,
    outcome: Option<JobResultKind>,
    tokens: Option<u32>,
    bytes: Option<u64>,
) -> JobRowView {
    JobRowView {
        job_id,
        url: url.to_string(),
        stage,
        outcome,
        tokens,
        bytes,
        link_count: 0,
        downloaded_link_count: 0,
        links: Vec::new(),
        triage_annotation: None,
        has_summary: false,
        summary_title: None,
        summary_tokens: None,
        filter_status: None,
        has_analysis: false,
        origin: JobOrigin::Direct,
        is_since_checkpoint: false,
    }
}

fn make_view(jobs: Vec<JobRowView>) -> AppViewModel {
    let visible_jobs_after_filter = jobs.iter().map(|job| job.job_id).collect::<Vec<_>>();
    let first_visible_job_id = visible_jobs_after_filter.first().copied();
    let mut view = AppViewModel {
        job_count: jobs.len(),
        jobs,
        ..AppViewModel::default()
    };
    view.left_pane.job_list_scope = JobListScope::All;
    view.left_pane.visible_jobs_after_filter = visible_jobs_after_filter;
    view.left_pane.first_visible_job_id = first_visible_job_id;
    view
}

#[test]
fn header_texts_do_not_reemit_when_unchanged() {
    init_logging();
    let window_id = WindowId::new(4);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![make_job(
        1,
        "https://epochai.substack.com/p/what",
        Stage::Done,
        Some(JobResultKind::Success),
        None,
        None,
    )]);
    view.left_pane.left_tab = LeftTab::TriageResults;
    view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;
    view.jobs[0].triage_annotation = Some(harvester_core::TriageAnnotationView {
        priority: 1,
        category: "keep".to_string(),
        tags: vec![],
    });

    let _ = render(window_id, &view, &mut tree_state);
    let commands = render(window_id, &view, &mut tree_state);

    assert!(!commands.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlText { control_id, .. }
                if *control_id == LABEL_JOBS_HEADER_TITLE
                    || *control_id == LABEL_JOBS_HEADER_META
                    || *control_id == LABEL_PREVIEW_HEADER
                    || *control_id == LABEL_PREVIEW_SOURCE_CAPTION
                    || *control_id == BUTTON_PREVIEW_SOURCE_LINK
                    || *control_id == LABEL_PREVIEW_STATUS
                    || *control_id == LABEL_PREVIEW_ATTENTION
        )
    }));
}

#[test]
fn jobs_search_input_text_resyncs_to_state() {
    init_logging();
    let window_id = WindowId::new(64);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);

    let initial_commands = render(window_id, &view, &mut tree_state);
    assert!(
        input_text(&initial_commands, INPUT_JOBS_SEARCH).is_none(),
        "empty default query should not emit a command before the control exists"
    );

    view.left_pane.jobs_search_query = "kube".to_string();
    let changed_commands = render(window_id, &view, &mut tree_state);
    assert_eq!(
        input_text(&changed_commands, INPUT_JOBS_SEARCH),
        Some("kube")
    );

    let unchanged_commands = render(window_id, &view, &mut tree_state);
    assert!(
        input_text(&unchanged_commands, INPUT_JOBS_SEARCH).is_none(),
        "unchanged query should not re-emit SetInputText"
    );
}

#[test]
fn tree_updates_text_without_repopulate_on_progress_change() {
    init_logging();
    let window_id = WindowId::new(1);
    let mut tree_state = TreeRenderState::new();

    let view_initial = make_view(vec![make_job(
        1,
        "https://example.com",
        Stage::Queued,
        None,
        None,
        None,
    )]);
    let commands_initial = render(window_id, &view_initial, &mut tree_state);
    assert!(commands_initial
        .iter()
        .any(|cmd| matches!(cmd, PlatformCommand::PopulateListBox { .. })));

    let view_updated = make_view(vec![make_job(
        1,
        "https://example.com",
        Stage::Downloading,
        None,
        Some(100),
        Some(2048),
    )]);
    let commands_updated = render(window_id, &view_updated, &mut tree_state);
    let populated = commands_updated
        .iter()
        .find_map(|cmd| match cmd {
            PlatformCommand::PopulateListBox { items, .. } => Some(items),
            _ => None,
        })
        .expect("updated list emitted");
    assert_eq!(populated.len(), 1);
    assert_eq!(populated[0].id, ListBoxItemId::new(1));
    assert_eq!(populated[0].title, "example.com");
    assert_eq!(populated[0].badges[0].text, "Fetch");
    assert_eq!(populated[0].metadata, "example.com · 100 · 2.0 KB");
}
#[test]
fn tree_repopulates_when_structure_changes() {
    init_logging();
    let window_id = WindowId::new(2);
    let mut tree_state = TreeRenderState::new();

    let view_initial = make_view(vec![make_job(
        1,
        "https://example.com",
        Stage::Queued,
        None,
        None,
        None,
    )]);
    let _ = render(window_id, &view_initial, &mut tree_state);

    let view_added = make_view(vec![
        make_job(1, "https://example.com", Stage::Queued, None, None, None),
        make_job(2, "https://two.example", Stage::Queued, None, None, None),
    ]);
    let commands_added = render(window_id, &view_added, &mut tree_state);
    assert!(commands_added
        .iter()
        .any(|cmd| matches!(cmd, PlatformCommand::PopulateListBox { .. })));
}

#[test]
fn splitter_resize_keeps_input_panel_fixed() {
    init_logging();
    let window_id = WindowId::new(5);
    let mut tree_state = TreeRenderState::new();
    let view = AppViewModel {
        left_panel_width: 760,
        input_panel_visible: true,
        ..Default::default()
    };

    let commands = render(window_id, &view, &mut tree_state);
    let rules = commands
        .iter()
        .find_map(|cmd| match cmd {
            PlatformCommand::DefineLayout { rules, .. } => Some(rules),
            _ => None,
        })
        .expect("DefineLayout emitted");

    let input_width = rules
        .iter()
        .find(|rule| rule.control_id == PANEL_INPUT)
        .and_then(|rule| rule.fixed_size)
        .expect("PANEL_INPUT fixed size");
    let left_width = rules
        .iter()
        .find(|rule| rule.control_id == PANEL_LEFT)
        .and_then(|rule| rule.fixed_size)
        .expect("PANEL_LEFT fixed size");
    let jobs_fill = rules
        .iter()
        .find(|rule| rule.control_id == PANEL_JOBS)
        .map(|rule| rule.fixed_size)
        .expect("PANEL_JOBS rule present");

    assert_eq!(input_width, harvester_core::INPUT_PANEL_FIXED_WIDTH);
    assert_eq!(left_width, 760);
    assert_eq!(jobs_fill, None, "PANEL_JOBS should Fill its parent");
}

#[test]
fn initial_render_emits_layout_for_default_view() {
    init_logging();
    let window_id = WindowId::new(51);
    let mut tree_state = TreeRenderState::new();

    let commands = render(window_id, &AppViewModel::default(), &mut tree_state);

    assert!(commands
        .iter()
        .any(|cmd| matches!(cmd, PlatformCommand::DefineLayout { .. })));
}

#[test]
fn render_layout_only_skips_tree_and_preview_updates() {
    init_logging();
    let window_id = WindowId::new(6);
    let mut tree_state = TreeRenderState::new();
    let view = AppViewModel {
        left_panel_width: 760,
        input_panel_visible: true,
        ..Default::default()
    };
    let _ = render(window_id, &view, &mut tree_state);

    let mut layout = layout_view_from_app_view(&view);
    layout.left_panel_width = 720;
    let commands = render_layout_only(window_id, &layout, &mut tree_state);

    assert!(commands
        .iter()
        .any(|cmd| matches!(cmd, PlatformCommand::DefineLayout { .. })));
    assert!(!commands.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::PopulateTreeView { .. }
                | PlatformCommand::UpdateTreeItemText { .. }
                | PlatformCommand::UpdateTreeItemVisualState { .. }
                | PlatformCommand::SetRichEditContent { .. }
        )
    }));
}

#[test]
fn jobs_scope_toggle_reflects_scope_state() {
    let window_id = WindowId::new(39);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetToggleSwitchState { control_id, checked: true, .. }
            if *control_id == TS_JOBS_SCOPE
        )
    }));

    view.left_pane.job_list_scope = JobListScope::All;
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetToggleSwitchState { control_id, checked: false, .. }
            if *control_id == TS_JOBS_SCOPE
        )
    }));
}

#[test]
fn left_header_only_renders_meta_row() {
    let window_id = WindowId::new(40);
    let mut tree_state = TreeRenderState::new();
    let empty_view = make_view(vec![JobRowView {
        job_id: 1,
        url: "https://example.com".to_string(),
        stage: Stage::Done,
        outcome: Some(JobResultKind::Success),
        tokens: None,
        bytes: None,
        link_count: 0,
        downloaded_link_count: 0,
        links: vec![],
        triage_annotation: None,
        has_summary: false,
        summary_title: None,
        summary_tokens: None,
        filter_status: None,
        has_analysis: false,
        origin: JobOrigin::Direct,
        is_since_checkpoint: true,
    }]);
    let mut empty_view = empty_view;
    empty_view.left_pane.left_tab = LeftTab::TriageResults;
    empty_view.left_pane_header = LeftPaneHeaderView {
        title: "Results".to_string(),
        scope_label: None,
        count_label: Some("no triage results yet".to_string()),
        state_label: None,
    };
    let empty_cmds = render(window_id, &empty_view, &mut tree_state);
    let empty_meta =
        control_text(&empty_cmds, LABEL_JOBS_HEADER_META).expect("empty triage meta rendered");
    assert_eq!(empty_meta, "no triage results yet");
    assert_eq!(
        control_text(&empty_cmds, LABEL_JOBS_HEADER_TITLE),
        Some("Results")
    );

    let mut populated_view = empty_view.clone();
    populated_view.jobs[0].triage_annotation = Some(harvester_core::TriageAnnotationView {
        priority: 1,
        category: "keep".to_string(),
        tags: vec![],
    });
    populated_view.left_pane_header = LeftPaneHeaderView {
        title: "Results".to_string(),
        scope_label: None,
        count_label: Some("1 with triage".to_string()),
        state_label: None,
    };
    let populated_cmds = render(window_id, &populated_view, &mut tree_state);
    let populated_meta = control_text(&populated_cmds, LABEL_JOBS_HEADER_META)
        .expect("populated triage meta rendered");
    assert_eq!(populated_meta, "1 with triage");
    assert_eq!(control_text(&populated_cmds, LABEL_JOBS_HEADER_TITLE), None);
}

#[test]
fn triage_results_meta_uses_ai_unavailable_copy_when_blocked() {
    let window_id = WindowId::new(42);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.left_pane.left_tab = LeftTab::TriageResults;
    view.ai_unavailable_message =
        Some("AI features unavailable: OPENAI_API_KEY is not set".to_string());
    view.left_pane_header = LeftPaneHeaderView {
        title: "Results".to_string(),
        scope_label: None,
        count_label: Some("no triage results yet".to_string()),
        state_label: Some("AI unavailable".to_string()),
    };

    let cmds = render(window_id, &view, &mut tree_state);
    let meta = control_text(&cmds, LABEL_JOBS_HEADER_META).expect("triage meta rendered");
    assert_eq!(meta, "no triage results yet · AI unavailable");
    assert_eq!(
        control_text(&cmds, LABEL_JOBS_HEADER_TITLE),
        Some("Results")
    );
}

#[test]
fn triage_results_meta_preserves_count_when_ai_unavailable() {
    let window_id = WindowId::new(43);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![JobRowView {
        job_id: 1,
        url: "https://example.com".to_string(),
        stage: Stage::Done,
        outcome: Some(JobResultKind::Success),
        tokens: None,
        bytes: None,
        link_count: 0,
        downloaded_link_count: 0,
        links: vec![],
        triage_annotation: Some(harvester_core::TriageAnnotationView {
            priority: 1,
            category: "keep".to_string(),
            tags: vec![],
        }),
        has_summary: false,
        summary_title: None,
        summary_tokens: None,
        filter_status: None,
        has_analysis: true,
        origin: JobOrigin::Direct,
        is_since_checkpoint: true,
    }]);
    view.left_pane.left_tab = LeftTab::TriageResults;
    view.ai_unavailable_message =
        Some("AI features unavailable: OPENAI_API_KEY is not set".to_string());
    view.left_pane_header = LeftPaneHeaderView {
        title: "Results".to_string(),
        scope_label: None,
        count_label: Some("1 with triage".to_string()),
        state_label: Some("AI unavailable".to_string()),
    };

    let cmds = render(window_id, &view, &mut tree_state);
    let meta = control_text(&cmds, LABEL_JOBS_HEADER_META).expect("triage meta rendered");
    assert_eq!(meta, "1 with triage · AI unavailable");
    assert_eq!(
        control_text(&cmds, LABEL_JOBS_HEADER_TITLE),
        Some("Results")
    );
}

fn control_text(
    cmds: &[PlatformCommand],
    control_id: commanductui::types::ControlId,
) -> Option<&str> {
    cmds.iter().find_map(|cmd| match cmd {
        PlatformCommand::SetControlText {
            control_id: rendered_id,
            text,
            ..
        } if *rendered_id == control_id => Some(text.as_str()),
        _ => None,
    })
}
fn input_text(
    cmds: &[PlatformCommand],
    control_id: commanductui::types::ControlId,
) -> Option<&str> {
    cmds.iter().find_map(|cmd| match cmd {
        PlatformCommand::SetInputText {
            control_id: rendered_id,
            text,
            ..
        } if *rendered_id == control_id => Some(text.as_str()),
        _ => None,
    })
}
