use commanductui::types::{TreeItemDescriptor, TreeItemId};
use commanductui::{MessageSeverity, PlatformCommand, WindowId};
use harvester_core::{AppViewModel, JobResultKind, JobRowView, SessionState, Stage};

use super::constants::*;

#[allow(clippy::vec_init_then_push)]
pub fn render(window_id: WindowId, view: &AppViewModel) -> Vec<PlatformCommand> {
    let session_label = match view.session {
        SessionState::Idle => "Idle",
        SessionState::Running => "Running",
        SessionState::Finishing => "Finishing",
        SessionState::Finished => "Finished",
    };

    let status_text = match &view.last_paste_stats {
        Some(stats) => format!(
            "Session: {} | Jobs: {} | Last paste: enqueued {}, skipped {}",
            session_label, view.job_count, stats.enqueued, stats.skipped
        ),
        None => format!("Session: {} | Jobs: {}", session_label, view.job_count),
    };

    let raw_limit = view.token_limit;
    let effective_limit = raw_limit.max(1);
    let bar_max = effective_limit.min(u32::MAX as u64);
    let clamped_tokens = view.total_tokens.min(bar_max);
    let percent = if raw_limit > 0 {
        (view.total_tokens.min(raw_limit) as f64 / raw_limit as f64) * 100.0
    } else {
        0.0
    };
    let progress_text = format!(
        "Tokens: {} / {} ({:.1}%)",
        format_with_commas(view.total_tokens),
        format_with_commas(view.token_limit),
        percent
    );

    let mut cmds = Vec::new();

    cmds.push(PlatformCommand::UpdateLabelText {
        window_id,
        control_id: LABEL_STATUS,
        text: status_text,
        severity: MessageSeverity::Information,
    });

    cmds.push(PlatformCommand::SetProgressBarRange {
        window_id,
        control_id: PROGRESS_TOKENS,
        min: 0,
        max: bar_max as u32,
    });
    cmds.push(PlatformCommand::SetProgressBarPosition {
        window_id,
        control_id: PROGRESS_TOKENS,
        position: clamped_tokens as u32,
    });
    cmds.push(PlatformCommand::SetControlText {
        window_id,
        control_id: LABEL_TOKEN_PROGRESS,
        text: progress_text,
    });

    cmds.push(PlatformCommand::SetControlEnabled {
        window_id,
        control_id: BUTTON_STOP,
        enabled: matches!(view.session, SessionState::Running),
    });

    cmds.push(PlatformCommand::PopulateTreeView {
        window_id,
        control_id: TREE_JOBS,
        items: build_job_tree(view),
    });

    cmds
}

fn build_job_tree(view: &AppViewModel) -> Vec<TreeItemDescriptor> {
    view.jobs
        .iter()
        .map(|job| TreeItemDescriptor {
            id: TreeItemId(job.job_id),
            text: format_job_row(job),
            is_folder: false,
            state: commanductui::types::CheckState::Unchecked,
            children: Vec::new(),
            style_override: None,
        })
        .collect()
}

fn format_job_row(job: &JobRowView) -> String {
    let status = match job.outcome {
        Some(JobResultKind::Success) => "OK",
        Some(JobResultKind::Failed) => "ERR",
        None => stage_label(job.stage),
    };
    let tokens = job.tokens.map(|t| format!("{t} tok"));
    let bytes = job.bytes.map(|b| format!("{b} B"));
    let metrics = match (tokens, bytes) {
        (Some(t), Some(b)) => format!("{t}, {b}"),
        (Some(t), None) => t,
        (None, Some(b)) => b,
        _ => String::new(),
    };
    if metrics.is_empty() {
        format!(
            "[#{id}] {status} — {url}",
            id = job.job_id,
            status = status,
            url = job.url
        )
    } else {
        format!(
            "[#{id}] {status} — {url} ({metrics})",
            id = job.job_id,
            status = status,
            url = job.url,
            metrics = metrics
        )
    }
}

fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Queued => "Queued",
        Stage::Downloading => "Downloading",
        Stage::Sanitizing => "Sanitizing",
        Stage::Converting => "Converting",
        Stage::Tokenizing => "Tokenizing",
        Stage::Writing => "Writing",
        Stage::Done => "Done",
    }
}

fn format_with_commas(value: u64) -> String {
    let mut out = String::new();
    for (i, ch) in value.to_string().chars().rev().enumerate() {
        if i != 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}
