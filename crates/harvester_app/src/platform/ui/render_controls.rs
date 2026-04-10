use commanductui::{MessageSeverity, PlatformCommand, StyleId, WindowId};
use harvester_core::{
    AppViewModel, JobListScope, LeftPaneHeaderView, LlmModelUsageView, SessionState,
};

use super::constants::*;
use super::render::{emit_if_changed, TreeRenderState};
use super::render_text::format_compact_tokens;

/// Maximum number of models shown individually in the status bar before collapsing.
const MAX_STATUS_BAR_MODELS: usize = 2;

/// Builds the LLM usage segment for the status bar, or None if no usage recorded.
/// Shows up to MAX_STATUS_BAR_MODELS models individually; collapses the rest.
pub(super) fn format_llm_usage_status(rows: &[LlmModelUsageView]) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    let visible = rows.len().min(MAX_STATUS_BAR_MODELS);
    let mut parts: Vec<String> = rows[..visible]
        .iter()
        .map(|r| {
            format!(
                "{}: in={} out={}",
                r.model,
                format_compact_tokens(r.input_tokens),
                format_compact_tokens(r.output_tokens)
            )
        })
        .collect();
    let hidden = rows.len() - visible;
    if hidden > 0 {
        parts.push(format!("+{} models", hidden));
    }
    Some(parts.join(", "))
}

pub(super) fn render_tab_bar_section(
    window_id: WindowId,
    view: &AppViewModel,
    _tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let active = view.right_pane.active_tab;
    cmds.push(PlatformCommand::SetTabBarSelection {
        window_id,
        control_id: TAB_BAR_RIGHT,
        selected_index: active.to_index(),
    });
}

pub(super) fn render_left_tab_bar_section(
    window_id: WindowId,
    view: &AppViewModel,
    _tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let active = view.left_pane.left_tab;
    cmds.push(PlatformCommand::SetTabBarSelection {
        window_id,
        control_id: TAB_BAR_LEFT,
        selected_index: active.to_index(),
    });
}

pub(super) fn render_status_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let session_label = match view.session {
        SessionState::Idle => "Idle",
        SessionState::Running => "Running",
        SessionState::Finishing => "Finishing",
        SessionState::Finished => "Finished",
    };
    let status_base_text = match &view.last_paste_stats {
        Some(stats) => format!(
            "Session: {} | Jobs: {} | Last paste: enqueued {}, skipped {}",
            session_label, view.job_count, stats.enqueued, stats.skipped
        ),
        None => format!("Session: {} | Jobs: {}", session_label, view.job_count),
    };

    let mut status_parts = vec![status_base_text];
    if view.left_pane.job_list_scope == JobListScope::SinceCheckpoint {
        status_parts.push("Since checkpoint".to_string());
    }
    if let Some(progress) = view.briefing_progress.as_deref() {
        status_parts.push(progress.to_string());
    }
    if let Some(progress) = view.triage_progress.as_deref() {
        status_parts.push(progress.to_string());
    }
    if let Some(status) = view.checkpoint_status_message.as_deref() {
        status_parts.push(status.to_string());
    }
    if let Some(usage) = format_llm_usage_status(&view.llm_usage_by_model) {
        status_parts.push(usage);
    }
    let severity = if let Some(message) = view.ai_unavailable_message.as_deref() {
        status_parts.push(message.to_string());
        MessageSeverity::Warning
    } else {
        MessageSeverity::Information
    };
    emit_if_changed(
        &mut tree_state.controls.prev_status_label,
        (status_parts.join(" | "), severity),
        cmds,
        |(text, severity)| PlatformCommand::UpdateLabelText {
            window_id,
            control_id: LABEL_STATUS,
            text,
            severity,
        },
    );
    tree_state.controls.prev_briefing_progress = view.briefing_progress.clone();
    tree_state.controls.prev_triage_progress = view.triage_progress.clone();
}

pub(super) fn render_operation_progress_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let (text, range, pos) = match &view.operation_progress {
        Some(op) => (
            format!("{}: {}/{}", op.label, op.completed, op.total),
            (0u32, op.total),
            op.completed,
        ),
        None => (String::new(), (0u32, 0u32), 0u32),
    };

    emit_if_changed(
        &mut tree_state.controls.prev_operation_progress_text,
        text,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_OPERATION_PROGRESS,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_operation_progress_range,
        range,
        cmds,
        |(min, max)| PlatformCommand::SetProgressBarRange {
            window_id,
            control_id: PROGRESS_OPERATION,
            min,
            max,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_operation_progress_pos,
        pos,
        cmds,
        |position| PlatformCommand::SetProgressBarPosition {
            window_id,
            control_id: PROGRESS_OPERATION,
            position,
        },
    );
}

pub(super) fn render_token_progress_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let scoped_total_tokens = match view.left_pane.job_list_scope {
        JobListScope::All => view.total_tokens,
        JobListScope::SinceCheckpoint => view
            .jobs
            .iter()
            .filter(|job| job.is_since_checkpoint)
            .filter_map(|job| job.tokens.map(u64::from))
            .sum(),
    };
    let raw_limit = view.token_limit;
    let effective_limit = raw_limit.max(1);
    let bar_max = effective_limit.min(u32::MAX as u64);
    let clamped_tokens = scoped_total_tokens.min(bar_max);
    let percent = if raw_limit > 0 {
        (scoped_total_tokens.min(raw_limit) as f64 / raw_limit as f64) * 100.0
    } else {
        0.0
    };
    let progress_text = format!(
        "{} / {}",
        format_compact_tokens(scoped_total_tokens),
        format_compact_tokens(view.token_limit)
    );
    let progress_style = if percent >= 100.0 {
        StyleId::ProgressBar
    } else {
        StyleId::StatusMeter
    };

    emit_if_changed(
        &mut tree_state.controls.prev_progress_range,
        (0, bar_max as u32),
        cmds,
        |(min, max)| PlatformCommand::SetProgressBarRange {
            window_id,
            control_id: PROGRESS_TOKENS,
            min,
            max,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_progress_pos,
        clamped_tokens as u32,
        cmds,
        |position| PlatformCommand::SetProgressBarPosition {
            window_id,
            control_id: PROGRESS_TOKENS,
            position,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_token_progress_style,
        progress_style,
        cmds,
        |style_id| PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id: PROGRESS_TOKENS,
            style_id,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_progress_text,
        progress_text,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_TOKEN_PROGRESS,
            text,
        },
    );
}

pub(super) fn render_main_controls_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    emit_if_changed(
        &mut tree_state.controls.prev_jobs_header_meta_text,
        format_left_pane_header_meta(&view.left_pane_header),
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_JOBS_HEADER_META,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_stop_style,
        if matches!(view.session, SessionState::Running) {
            StyleId::DestructiveButton
        } else {
            StyleId::SecondaryButton
        },
        cmds,
        |style_id| PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id: BUTTON_STOP,
            style_id,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_stop_enabled,
        matches!(view.session, SessionState::Running),
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_STOP,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_briefing_enabled,
        view.briefing_can_start,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_BRIEFING,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_triage_enabled,
        view.triage_can_start,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_TRIAGE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_poll_enabled,
        view.poll_sources_enabled,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_POLL_SOURCES,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_open_browser_enabled,
        view.selected_url.is_some(),
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_OPEN_BROWSER,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.controls.prev_jobs_scope_since_checkpoint_checked,
        view.left_pane.job_list_scope == JobListScope::SinceCheckpoint,
        cmds,
        |checked| PlatformCommand::SetToggleSwitchState {
            window_id,
            control_id: TS_JOBS_SCOPE,
            checked,
        },
    );
}

pub(super) fn format_left_pane_header_meta(header: &LeftPaneHeaderView) -> String {
    let mut parts = Vec::<String>::new();
    if let Some(scope_label) = header.scope_label.as_deref() {
        parts.push(String::from(scope_label));
    }
    if let Some(count_label) = header.count_label.as_deref() {
        parts.push(String::from(count_label));
    }
    if let Some(state_label) = header.state_label.as_deref() {
        parts.push(String::from(state_label));
    }
    parts.join(" · ")
}
