use super::*;

#[test]
fn render_enables_summarize_when_summaries_can_start() {
    init_logging();
    let mut view = make_view(vec![]);
    view.summaries_can_start = true;
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    let cmds = render(window_id, &view, &mut tree_state);
    let enabled = cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
            if *control_id == BUTTON_SUMMARIZE
        )
    });
    assert!(enabled, "BUTTON_SUMMARIZE should be enabled");
}

#[test]
fn render_disables_summarize_when_summaries_cannot_start() {
    init_logging();
    let view = make_view(vec![]);
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    let cmds = render(window_id, &view, &mut tree_state);
    let disabled = cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
            if *control_id == BUTTON_SUMMARIZE
        )
    });
    assert!(disabled, "BUTTON_SUMMARIZE should be disabled");
}

#[test]
fn render_enables_briefing_when_generate_enabled() {
    init_logging();
    let mut view = make_view(vec![]);
    view.briefing_generate_enabled = true;
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    let cmds = render(window_id, &view, &mut tree_state);
    let enabled = cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
            if *control_id == BUTTON_BRIEFING
        )
    });
    assert!(enabled, "BUTTON_BRIEFING should be enabled");
}

#[test]
fn render_disables_briefing_when_generate_disabled() {
    init_logging();
    let view = make_view(vec![]);
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    let cmds = render(window_id, &view, &mut tree_state);
    let disabled = cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
            if *control_id == BUTTON_BRIEFING
        )
    });
    assert!(disabled, "BUTTON_BRIEFING should be disabled");
}

#[test]
fn render_enables_next_item_when_available() {
    init_logging();
    let mut view = make_view(vec![]);
    view.next_item_enabled = true;
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    let cmds = render(window_id, &view, &mut tree_state);
    let enabled = cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
            if *control_id == BUTTON_NEXT_ITEM
        )
    });
    assert!(enabled, "BUTTON_NEXT_ITEM should be enabled");
}

#[test]
fn render_disables_next_item_when_unavailable() {
    init_logging();
    let view = make_view(vec![]);
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    let cmds = render(window_id, &view, &mut tree_state);
    let disabled = cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
            if *control_id == BUTTON_NEXT_ITEM
        )
    });
    assert!(disabled, "BUTTON_NEXT_ITEM should be disabled");
}

#[test]
fn stop_button_uses_secondary_style_when_not_running() {
    init_logging();
    let view = make_view(vec![]);
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
            if *control_id == BUTTON_STOP && *style_id == StyleId::SecondaryButton
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
            if *control_id == BUTTON_STOP
        )
    }));
}

#[test]
fn stop_button_uses_destructive_style_when_running() {
    init_logging();
    let mut view = make_view(vec![]);
    view.session = SessionState::Running;
    view.stop_finish_button = StopFinishButtonState::Enabled {
        policy: StopPolicy::Finish,
    };
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
            if *control_id == BUTTON_STOP && *style_id == StyleId::DestructiveButton
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
            if *control_id == BUTTON_STOP
        )
    }));
}

#[test]
fn stop_button_stays_neutral_when_session_running_but_work_is_idle() {
    init_logging();
    let mut view = make_view(vec![]);
    view.session = SessionState::Running;
    view.stop_finish_button = StopFinishButtonState::Disabled;
    let mut tree_state = TreeRenderState::new();
    let window_id = WindowId::new(1);
    let cmds = render(window_id, &view, &mut tree_state);
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
            if *control_id == BUTTON_STOP && *style_id == StyleId::SecondaryButton
        )
    }));
    assert!(cmds.iter().any(|cmd| {
        matches!(
            cmd,
            PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
            if *control_id == BUTTON_STOP
        )
    }));
}
