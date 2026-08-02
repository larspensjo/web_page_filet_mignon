use super::super::super::render_controls::format_llm_usage_status;
use super::*;
use harvester_core::{
    ArchivePartialCoverageView, LlmModelUsageView, LlmQuotaSeverity, LlmQuotaView,
};

#[test]
fn token_progress_uses_archive_estimate_regardless_of_scope() {
    let window_id = WindowId::new(41);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.archive_token_estimate = 50;
    view.token_limit = 200_000;
    view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlText { control_id, text, .. }
            if *control_id == LABEL_TOKEN_PROGRESS && text == "50 / 200K"
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetProgressBarPosition { control_id, position, .. }
            if *control_id == PROGRESS_TOKENS && *position == 50
        )
    }));
}

#[test]
fn llm_quota_progress_renders_label_range_and_position() {
    let window_id = WindowId::new(45);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(Vec::new());
    view.llm_quota = LlmQuotaView {
        label: "LLM calls 37 / 100".to_string(),
        used: 37,
        limit: Some(100),
        percent: Some(37),
        severity: LlmQuotaSeverity::Normal,
    };

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlText { control_id, text, .. }
            if *control_id == LABEL_LLM_QUOTA && text == "LLM calls 37 / 100"
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetProgressBarRange { control_id, min, max, .. }
            if *control_id == PROGRESS_LLM_QUOTA && *min == 0 && *max == 100
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetProgressBarPosition { control_id, position, .. }
            if *control_id == PROGRESS_LLM_QUOTA && *position == 37
        )
    }));
}

#[test]
fn llm_quota_progress_uses_accent_style_when_exhausted() {
    let window_id = WindowId::new(46);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(Vec::new());
    view.llm_quota = LlmQuotaView {
        label: "LLM calls 100 / 100".to_string(),
        used: 100,
        limit: Some(100),
        percent: Some(100),
        severity: LlmQuotaSeverity::Exhausted,
    };

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
            if *control_id == PROGRESS_LLM_QUOTA && *style_id == StyleId::ProgressBar
        )
    }));
}

#[test]
fn token_progress_stays_muted_below_limit_even_when_high() {
    let window_id = WindowId::new(42);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(Vec::new());
    view.archive_token_estimate = 97_002;
    view.token_limit = 100_000;

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
            if *control_id == PROGRESS_TOKENS && *style_id == StyleId::StatusMeter
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlText { control_id, text, .. }
            if *control_id == LABEL_TOKEN_PROGRESS
                && text == "97K / 100K"
        )
    }));
}

#[test]
fn token_progress_escalates_to_accent_at_limit() {
    let window_id = WindowId::new(43);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(Vec::new());
    view.archive_token_estimate = 100_000;
    view.token_limit = 100_000;

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
            if *control_id == PROGRESS_TOKENS && *style_id == StyleId::ProgressBar
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlText { control_id, text, .. }
            if *control_id == LABEL_TOKEN_PROGRESS
                && text == "100K / 100K"
        )
    }));
}

#[test]
fn token_counts_label_shows_filtered_and_raw_counts() {
    let window_id = WindowId::new(47);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.archive_filtered_count = 12;
    view.raw_unprocessed_count = 3;

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlText { control_id, text, .. }
            if *control_id == LABEL_TOKEN_COUNTS && text == "12 filtered · 3 raw"
        )
    }));
}

#[test]
fn token_counts_label_shows_partial_archive_coverage() {
    let window_id = WindowId::new(48);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.archive_partial_coverage = Some(ArchivePartialCoverageView {
        triaged: 3,
        actionable_total: 10,
    });

    let cmds = render(window_id, &view, &mut tree_state);

    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlText { control_id, text, .. }
            if *control_id == LABEL_TOKEN_COUNTS
                && text == "3 of 10 triaged — run triage to export"
        )
    }));
}

#[test]
fn status_bar_uses_warning_severity_for_ai_unavailable_message() {
    let window_id = WindowId::new(41);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.ai_unavailable_message =
        Some("AI features unavailable: OPENAI_API_KEY is not set".to_string());

    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::UpdateLabelText { control_id, severity: MessageSeverity::Warning, text, .. }
                if *control_id == LABEL_STATUS && text.contains("OPENAI_API_KEY is not set")
            )
        }));
}

#[test]
fn status_bar_renders_dense_operational_context() {
    let window_id = WindowId::new(43);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.session = SessionState::Running;
    view.job_count = 7;
    view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;
    view.checkpoint_status_message = Some("Checkpoint saved".to_string());
    view.llm_usage_by_model = vec![LlmModelUsageView {
        model: "model-a".to_string(),
        input_tokens: 12,
        output_tokens: 34,
    }];
    view.ai_unavailable_message =
        Some("AI features unavailable: OPENAI_API_KEY is not set".to_string());

    let cmds = render(window_id, &view, &mut tree_state);
    let (status_text, severity) = cmds
        .iter()
        .find_map(|cmd| match cmd {
            PlatformCommand::UpdateLabelText {
                control_id,
                text,
                severity,
                ..
            } if *control_id == LABEL_STATUS => Some((text.as_str(), *severity)),
            _ => None,
        })
        .expect("status label rendered");

    assert_eq!(severity, MessageSeverity::Warning);
    assert!(status_text.contains("Session: Running | Jobs: 7"));
    assert!(status_text.contains("Since checkpoint"));
    assert!(status_text.contains("Checkpoint saved"));
    assert!(status_text.contains("model-a: in=12 out=34"));
    assert!(status_text.contains("OPENAI_API_KEY is not set"));
}

#[test]
fn status_bar_omits_summary_progress_when_operation_progress_shows_it() {
    let window_id = WindowId::new(42);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.operation_progress_visible = true;
    view.operation_progress = Some(harvester_core::OperationProgress {
        label: "Summarizing".to_string(),
        completed: 3,
        total: 5,
    });

    let cmds = render(window_id, &view, &mut tree_state);
    let status_text = cmds
        .iter()
        .find_map(|cmd| match cmd {
            PlatformCommand::UpdateLabelText {
                control_id, text, ..
            } if *control_id == LABEL_STATUS => Some(text.as_str()),
            _ => None,
        })
        .expect("status label rendered");
    assert!(!status_text.contains("Summarizing"));
    assert_eq!(
        control_text(&cmds, LABEL_OPERATION_PROGRESS),
        Some("Summarizing: 3/5")
    );
}

#[test]
fn ai_warning_banner_text_is_emitted_when_present() {
    let window_id = WindowId::new(44);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.ai_warning_banner = Some(harvester_core::InlineWarningView {
        title: "AI features are disabled".to_string(),
        body: "Set OPENAI_API_KEY in the launch environment and restart to enable triage and briefing.".to_string(),
    });

    let cmds = render(window_id, &view, &mut tree_state);
    assert_eq!(
        control_text(&cmds, LABEL_AI_WARNING_TITLE),
        Some("AI features are disabled")
    );
    assert_eq!(
        control_text(&cmds, LABEL_AI_WARNING_BODY),
        Some("Set OPENAI_API_KEY in the launch environment and restart to enable triage and briefing.")
    );
}

#[test]
fn ai_warning_banner_text_clears_when_hidden() {
    let window_id = WindowId::new(45);
    let mut tree_state = TreeRenderState::new();
    let mut view = make_view(vec![]);
    view.ai_warning_banner = Some(harvester_core::InlineWarningView {
        title: "AI features are disabled".to_string(),
        body: "Set OPENAI_API_KEY in the launch environment and restart to enable triage and briefing.".to_string(),
    });

    let _ = render(window_id, &view, &mut tree_state);

    view.ai_warning_banner = None;
    let cmds = render(window_id, &view, &mut tree_state);
    assert_eq!(control_text(&cmds, LABEL_AI_WARNING_TITLE), Some(""));
    assert_eq!(control_text(&cmds, LABEL_AI_WARNING_BODY), Some(""));
}

#[test]
fn status_bar_omits_llm_usage_when_empty() {
    assert_eq!(format_llm_usage_status(&[]), None);
}

#[test]
fn status_bar_includes_llm_usage_segment() {
    let rows = vec![LlmModelUsageView {
        model: "alpha".to_string(),
        input_tokens: 12_000,
        output_tokens: 3_000,
    }];
    let result = format_llm_usage_status(&rows).expect("Some");
    assert!(result.contains("alpha"));
    assert!(result.contains("in=12K"));
    assert!(result.contains("out=3K"));
    assert!(!result.contains('+'));
}

#[test]
fn status_bar_collapses_when_model_count_exceeds_limit() {
    let rows = vec![
        LlmModelUsageView {
            model: "alpha".to_string(),
            input_tokens: 1_000,
            output_tokens: 500,
        },
        LlmModelUsageView {
            model: "beta".to_string(),
            input_tokens: 2_000,
            output_tokens: 1_000,
        },
        LlmModelUsageView {
            model: "gamma".to_string(),
            input_tokens: 3_000,
            output_tokens: 1_500,
        },
    ];
    let result = format_llm_usage_status(&rows).expect("Some");
    assert!(result.contains("alpha: in=1K out=500"));
    assert!(result.contains("beta: in=2K out=1K"));
    assert!(result.contains("+1 models"));
    assert!(!result.contains("gamma"));
}
