use commanductui::{MessageSeverity, PlatformCommand, WindowId};
use harvester_core::{AppViewModel, SessionState};

use super::constants::*;

pub fn render(window_id: WindowId, view: &AppViewModel) -> Vec<PlatformCommand> {
    let session_label = match view.session {
        SessionState::Idle => "Idle",
        SessionState::Running => "Running",
        SessionState::Finishing => "Finishing",
        SessionState::Finished => "Finished",
    };

    let status_text = format!(
        "Session: {session_label} | URLs queued: {} | Jobs: {}",
        view.queued_urls.len(),
        view.job_count
    );

    let mut cmds = Vec::new();

    cmds.push(PlatformCommand::UpdateLabelText {
        window_id,
        control_id: LABEL_STATUS,
        text: status_text,
        severity: MessageSeverity::Information,
    });

    cmds.push(PlatformCommand::SetControlEnabled {
        window_id,
        control_id: BUTTON_START,
        enabled: matches!(view.session, SessionState::Idle),
    });
    cmds.push(PlatformCommand::SetControlEnabled {
        window_id,
        control_id: BUTTON_STOP,
        enabled: matches!(view.session, SessionState::Running),
    });

    cmds
}
