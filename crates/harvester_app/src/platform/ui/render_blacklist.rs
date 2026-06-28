use commanductui::{PlatformCommand, WindowId};
use harvester_core::{AppViewModel, LeftTab};

use super::constants::*;
use super::markdown_to_rtf::convert_markdown_to_rtf;
use super::render::emit_if_changed;

#[derive(Debug, Default)]
pub(super) struct BlacklistRenderState {
    pub(super) prev_blacklist_text: Option<String>,
}

pub(super) fn render_blacklist_section(
    window_id: WindowId,
    view: &AppViewModel,
    state: &mut BlacklistRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    if view.left_pane.left_tab != LeftTab::Blacklist {
        return;
    }
    let content = format_blacklist_content(view);
    emit_if_changed(&mut state.prev_blacklist_text, content, cmds, |text| {
        PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_BLACKLIST,
            rtf_text: convert_markdown_to_rtf(&text),
        }
    });
}

fn format_blacklist_content(view: &AppViewModel) -> String {
    let bl = &view.blacklist;
    if bl.rows.is_empty() {
        return "No domains blacklisted yet.".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!(
        "**Blacklisted domains: {}**\n",
        bl.blacklisted_count
    ));
    for row in &bl.rows {
        lines.push(format!(
            "**{}** · {} strikes · {} · {} · {}",
            row.domain, row.strikes, row.status, row.last_failure, row.next_retry
        ));
    }
    lines.join("\n\n")
}
