use super::super::super::markdown_to_rtf::RTF_TRUNCATE_MARKER;
use super::super::super::render_preview::SUMMARY_EMPTY_STATE_MARKDOWN;
use super::super::super::render_text::MAX_VIEWER_CHARS;
use super::*;
use harvester_core::{PreviewContextView, PreviewHeaderView};

#[test]
fn preview_header_text_override_wins_over_article_header() {
    init_logging();
    let window_id = WindowId::new(1);
    let mut tree_state = TreeRenderState::new();
    let view = AppViewModel {
        preview_header: Some(PreviewHeaderView {
            domain: "example.com".to_string(),
            tokens: Some(123),
            bytes: Some(456),
            stage: Stage::Done,
            outcome: Some(JobResultKind::Success),
            heading_count: 2,
            link_density: 0.0,
            nav_heavy: false,
        }),
        preview_context: Some(PreviewContextView {
            source_label: "example.com".to_string(),
            status_label: "Done".to_string(),
            attention_label: None,
        }),
        preview_header_text: Some("Executive Briefing | 3 articles | Done".to_string()),
        ..AppViewModel::default()
    };

    let commands = render(window_id, &view, &mut tree_state);

    let header = commands.iter().find_map(|cmd| match cmd {
        PlatformCommand::SetControlText {
            control_id, text, ..
        } if *control_id == LABEL_PREVIEW_HEADER => Some(text.as_str()),
        _ => None,
    });
    assert_eq!(header, Some("Executive Briefing | 3 articles | Done"));
    assert_eq!(
        control_text(&commands, LABEL_PREVIEW_SOURCE_CAPTION),
        Some("")
    );
    assert_eq!(
        control_text(&commands, BUTTON_PREVIEW_SOURCE_LINK),
        Some("")
    );
    assert_eq!(control_text(&commands, LABEL_PREVIEW_STATUS), Some(""));
    assert_eq!(control_text(&commands, LABEL_PREVIEW_ATTENTION), Some(""));
}

#[test]
fn preview_metadata_clears_when_no_article_is_selected() {
    init_logging();
    let window_id = WindowId::new(2);
    let mut tree_state = TreeRenderState::new();
    let view = make_view(vec![]);

    let commands = render(window_id, &view, &mut tree_state);

    assert_eq!(control_text(&commands, LABEL_PREVIEW_HEADER), Some(""));
    assert_eq!(
        control_text(&commands, LABEL_PREVIEW_SOURCE_CAPTION),
        Some("")
    );
    assert_eq!(
        control_text(&commands, BUTTON_PREVIEW_SOURCE_LINK),
        Some("")
    );
    assert_eq!(control_text(&commands, LABEL_PREVIEW_STATUS), Some(""));
    assert_eq!(control_text(&commands, LABEL_PREVIEW_ATTENTION), Some(""));
}

#[test]
fn preview_source_renders_caption_and_link_text() {
    init_logging();
    let window_id = WindowId::new(3);
    let mut tree_state = TreeRenderState::new();
    let view = AppViewModel {
        preview_context: Some(PreviewContextView {
            source_label: "epochai.substack.com".to_string(),
            status_label: "Done".to_string(),
            attention_label: None,
        }),
        selected_url: Some("https://epochai.substack.com/p/what".to_string()),
        ..AppViewModel::default()
    };

    let commands = render(window_id, &view, &mut tree_state);

    assert_eq!(
        control_text(&commands, LABEL_PREVIEW_SOURCE_CAPTION),
        Some("Source")
    );
    assert_eq!(
        control_text(&commands, BUTTON_PREVIEW_SOURCE_LINK),
        Some("epochai.substack.com")
    );
}

#[test]
fn preview_text_is_sent_as_rtf_to_rich_edit() {
    init_logging();
    let window_id = WindowId::new(3);
    let mut tree_state = TreeRenderState::new();
    let mut view = AppViewModel::default();
    view.right_pane.summary_markdown = Some("first\nsecond\r\nthird\rfourth".to_string());

    let commands = render(window_id, &view, &mut tree_state);
    let viewer_text = commands
        .iter()
        .find_map(|cmd| match cmd {
            PlatformCommand::SetRichEditContent { rtf_text, .. } => Some(rtf_text),
            _ => None,
        })
        .expect("SetRichEditContent emitted");
    assert!(viewer_text.contains("first"));
    assert!(viewer_text.contains("second"));
}

#[test]
fn render_preview_uses_rtf_converter() {
    init_logging();
    let window_id = WindowId::new(6);
    let mut tree_state = TreeRenderState::new();
    let view = AppViewModel {
        preview_text: Some("## Heading".to_string()),
        ..Default::default()
    };

    let commands = render(window_id, &view, &mut tree_state);
    let viewer_text = commands
        .iter()
        .find_map(|cmd| match cmd {
            PlatformCommand::SetRichEditContent { rtf_text, .. } => Some(rtf_text),
            _ => None,
        })
        .expect("SetRichEditContent emitted");
    assert!(viewer_text.contains("\\b"));
}

#[test]
fn render_preview_marks_bold_in_rtf() {
    init_logging();
    let window_id = WindowId::new(7);
    let mut tree_state = TreeRenderState::new();
    let mut view = AppViewModel::default();
    view.right_pane.summary_markdown = Some("**bold**".to_string());

    let commands = render(window_id, &view, &mut tree_state);
    let viewer_text = commands
        .iter()
        .find_map(|cmd| match cmd {
            PlatformCommand::SetRichEditContent { rtf_text, .. } => Some(rtf_text),
            _ => None,
        })
        .expect("SetRichEditContent emitted");
    assert!(viewer_text.contains("\\b "));
    assert!(viewer_text.contains("\\b0 "));
}

#[test]
fn render_preview_idempotent_when_text_unchanged() {
    init_logging();
    let window_id = WindowId::new(8);
    let mut tree_state = TreeRenderState::new();
    let view = AppViewModel {
        preview_text: Some("same".to_string()),
        ..Default::default()
    };
    let _ = render(window_id, &view, &mut tree_state);
    let commands = render(window_id, &view, &mut tree_state);
    assert!(!commands
        .iter()
        .any(|cmd| matches!(cmd, PlatformCommand::SetRichEditContent { .. })));
}

#[test]
fn render_preview_truncation_adds_marker() {
    init_logging();
    let window_id = WindowId::new(9);
    let mut tree_state = TreeRenderState::new();
    let long_text = "x".repeat(MAX_VIEWER_CHARS + 1);
    let mut view = AppViewModel::default();
    view.right_pane.summary_markdown = Some(long_text);
    let commands = render(window_id, &view, &mut tree_state);
    let viewer_text = commands
        .iter()
        .find_map(|cmd| match cmd {
            PlatformCommand::SetRichEditContent { rtf_text, .. } => Some(rtf_text),
            _ => None,
        })
        .expect("SetRichEditContent emitted");
    assert!(viewer_text.contains(RTF_TRUNCATE_MARKER));
}

#[test]
fn render_enables_preview_source_link_when_selected_url_is_some() {
    init_logging();
    let mut view = make_view(vec![]);
    view.selected_url = Some("https://example.com".to_string());
    view.preview_context = Some(PreviewContextView {
        source_label: "example.com".to_string(),
        status_label: "Done".to_string(),
        attention_label: None,
    });
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    let cmds = render(window_id, &view, &mut tree_state);
    let enabled = cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
            if *control_id == BUTTON_PREVIEW_SOURCE_LINK
        )
    });
    assert!(enabled, "BUTTON_PREVIEW_SOURCE_LINK should be enabled");
}

#[test]
fn render_disables_preview_source_link_when_selected_url_is_none() {
    init_logging();
    let mut view = make_view(vec![]);
    view.preview_context = Some(PreviewContextView {
        source_label: "example.com".to_string(),
        status_label: "Done".to_string(),
        attention_label: None,
    });
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    let cmds = render(window_id, &view, &mut tree_state);
    let disabled = cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
            if *control_id == BUTTON_PREVIEW_SOURCE_LINK
        )
    });
    assert!(disabled, "BUTTON_PREVIEW_SOURCE_LINK should be disabled");
}

#[test]
fn render_is_idempotent_for_preview_source_link_state() {
    init_logging();
    let mut view = make_view(vec![]);
    view.preview_context = Some(PreviewContextView {
        source_label: "example.com".to_string(),
        status_label: "Done".to_string(),
        attention_label: None,
    });
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    // First render sets initial state
    render(window_id, &view, &mut tree_state);
    // Second render should not emit SetControlEnabled for BUTTON_PREVIEW_SOURCE_LINK
    let cmds = render(window_id, &view, &mut tree_state);
    let changed = cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, .. }
            if *control_id == BUTTON_PREVIEW_SOURCE_LINK
        )
    });
    assert!(
        !changed,
        "BUTTON_PREVIEW_SOURCE_LINK state should not change on second render"
    );
}

#[test]
fn summary_tab_without_selected_article_shows_empty_state_not_briefing_preview() {
    let window_id = WindowId::new(40);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.right_pane.active_tab = AppTab::Summary;
    view.right_pane.summary_markdown = None;
    view.preview_text = Some("Briefing content should not leak".to_string());
    view.right_pane.briefing_markdown = Some("Briefing content should not leak".to_string());

    let cmds = render(window_id, &view, &mut tree_state);

    assert_eq!(
        tree_state.preview.prev_preview_text.as_deref(),
        Some(SUMMARY_EMPTY_STATE_MARKDOWN)
    );
    assert!(!cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetRichEditContent { control_id, rtf_text, .. }
                if *control_id == VIEWER_PREVIEW && rtf_text.contains("Briefing content should not leak")
            )
        }));
}
