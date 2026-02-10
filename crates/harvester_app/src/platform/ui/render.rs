use commanductui::types::{DockStyle, LayoutRule, TreeItemDescriptor, TreeItemId};
use commanductui::{CheckState, MessageSeverity, PlatformCommand, StyleId, WindowId};
use engine_logging::engine_debug;
use harvester_core::{
    AppViewModel, JobResultKind, JobRowView, LinkDownloadState, PreviewHeaderView, SessionState,
    Stage, DEFAULT_LEFT_PANEL_WIDTH, INPUT_PANEL_FIXED_WIDTH,
};
use harvester_engine::LinkKind;

use super::constants::*;
use super::tree_item_ids::{
    job_tree_item_id, link_tree_item_id, links_folder_tree_item_id, links_show_more_tree_item_id,
};
use std::collections::HashMap;

#[derive(Debug)]
pub struct TreeRenderState {
    initialized: bool,
    structure: Vec<TreeStructureItem>,
    text_by_id: HashMap<TreeItemId, String>,
    check_state_by_id: HashMap<TreeItemId, CheckState>,
    /// Tracks the previous left_panel_width to detect changes
    prev_left_panel_width: i32,
    prev_status_text: Option<String>,
    prev_progress_text: Option<String>,
    prev_preview_text: Option<String>,
    prev_header_text: Option<String>,
    prev_stop_enabled: Option<bool>,
    prev_archive_enabled: Option<bool>,
    prev_briefing_enabled: Option<bool>,
    prev_briefing_progress: Option<String>,
    prev_progress_range: Option<(u32, u32)>,
    prev_progress_pos: Option<u32>,
}

impl Default for TreeRenderState {
    fn default() -> Self {
        Self {
            initialized: false,
            structure: Vec::new(),
            text_by_id: HashMap::new(),
            check_state_by_id: HashMap::new(),
            prev_left_panel_width: DEFAULT_LEFT_PANEL_WIDTH,
            prev_status_text: None,
            prev_progress_text: None,
            prev_preview_text: None,
            prev_header_text: None,
            prev_stop_enabled: None,
            prev_archive_enabled: None,
            prev_briefing_enabled: None,
            prev_briefing_progress: None,
            prev_progress_range: None,
            prev_progress_pos: None,
        }
    }
}

impl TreeRenderState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeStructureItem {
    id: TreeItemId,
    parent_id: Option<TreeItemId>,
    is_folder: bool,
    child_count: usize,
    style_override: Option<StyleId>,
}

#[derive(Debug)]
struct TreeSnapshot {
    structure: Vec<TreeStructureItem>,
    text_by_id: HashMap<TreeItemId, String>,
    check_state_by_id: HashMap<TreeItemId, CheckState>,
}

impl TreeSnapshot {
    fn from_items(items: &[TreeItemDescriptor]) -> Self {
        let mut snapshot = Self {
            structure: Vec::new(),
            text_by_id: HashMap::new(),
            check_state_by_id: HashMap::new(),
        };
        snapshot.push_items(items, None);
        snapshot
    }

    fn push_items(&mut self, items: &[TreeItemDescriptor], parent_id: Option<TreeItemId>) {
        for item in items {
            self.structure.push(TreeStructureItem {
                id: item.id,
                parent_id,
                is_folder: item.is_folder,
                child_count: item.children.len(),
                style_override: item.style_override,
            });
            self.text_by_id.insert(item.id, item.text.clone());
            self.check_state_by_id.insert(item.id, item.state);
            if !item.children.is_empty() {
                self.push_items(&item.children, Some(item.id));
            }
        }
    }
}

#[allow(clippy::vec_init_then_push)]
pub fn render(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
) -> Vec<PlatformCommand> {
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

    // Check if left_panel_width changed and emit updated layout
    if view.left_panel_width != tree_state.prev_left_panel_width {
        engine_debug!(
            "[Render] Layout update: left_panel_width {} -> {}",
            tree_state.prev_left_panel_width,
            view.left_panel_width
        );
        cmds.push(build_layout_command(window_id, view.left_panel_width));
        tree_state.prev_left_panel_width = view.left_panel_width;
    }

    let briefing_progress = view.briefing_progress.as_deref();
    let status_text = if let Some(progress) = briefing_progress {
        format!("{} | {}", status_base_text, progress)
    } else {
        status_base_text.clone()
    };

    let status_changed = match tree_state.prev_status_text.as_deref() {
        Some(prev) => prev != status_text.as_str(),
        None => true,
    };
    if status_changed {
        let updated_text = status_text.clone();
        cmds.push(PlatformCommand::UpdateLabelText {
            window_id,
            control_id: LABEL_STATUS,
            text: updated_text.clone(),
            severity: MessageSeverity::Information,
        });
        tree_state.prev_status_text = Some(updated_text);
    }
    tree_state.prev_briefing_progress = view.briefing_progress.clone();

    let range = (0, bar_max as u32);
    if tree_state.prev_progress_range != Some(range) {
        cmds.push(PlatformCommand::SetProgressBarRange {
            window_id,
            control_id: PROGRESS_TOKENS,
            min: range.0,
            max: range.1,
        });
        tree_state.prev_progress_range = Some(range);
    }
    let pos = clamped_tokens as u32;
    if tree_state.prev_progress_pos != Some(pos) {
        cmds.push(PlatformCommand::SetProgressBarPosition {
            window_id,
            control_id: PROGRESS_TOKENS,
            position: pos,
        });
        tree_state.prev_progress_pos = Some(pos);
    }
    let progress_text_changed = match tree_state.prev_progress_text.as_deref() {
        Some(prev) => prev != progress_text,
        None => true,
    };
    if progress_text_changed {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_TOKEN_PROGRESS,
            text: progress_text.to_string(),
        });
        tree_state.prev_progress_text = Some(progress_text.to_string());
    }

    let stop_enabled = matches!(view.session, SessionState::Running);
    if tree_state.prev_stop_enabled != Some(stop_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_STOP,
            enabled: stop_enabled,
        });
        tree_state.prev_stop_enabled = Some(stop_enabled);
    }

    let archive_enabled = view.job_count > 0;
    if tree_state.prev_archive_enabled != Some(archive_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_ARCHIVE,
            enabled: archive_enabled,
        });
        tree_state.prev_archive_enabled = Some(archive_enabled);
    }

    let briefing_enabled = view.briefing_can_start;
    if tree_state.prev_briefing_enabled != Some(briefing_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_BRIEFING,
            enabled: briefing_enabled,
        });
        tree_state.prev_briefing_enabled = Some(briefing_enabled);
    }

    let job_items = build_job_tree(view);
    append_tree_commands(window_id, job_items, tree_state, &mut cmds);

    let preview_text = view
        .preview_text
        .as_deref()
        .map(normalize_windows_newlines)
        .unwrap_or_default();
    let preview_text_changed = match tree_state.prev_preview_text.as_deref() {
        Some(prev) => prev != preview_text,
        None => true,
    };
    if preview_text_changed {
        cmds.push(PlatformCommand::SetViewerContent {
            window_id,
            control_id: VIEWER_PREVIEW,
            text: preview_text.to_string(),
        });
        tree_state.prev_preview_text = Some(preview_text.to_string());
    }

    let header_text = view
        .preview_header
        .as_ref()
        .map(format_preview_header)
        .unwrap_or_else(|| "(no selection)".to_string());
    let header_text_changed = match tree_state.prev_header_text.as_deref() {
        Some(prev) => prev != header_text,
        None => true,
    };
    if header_text_changed {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PREVIEW_HEADER,
            text: header_text.to_string(),
        });
        tree_state.prev_header_text = Some(header_text.to_string());
    }

    cmds
}

fn append_tree_commands(
    window_id: WindowId,
    items: Vec<TreeItemDescriptor>,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let snapshot = TreeSnapshot::from_items(&items);
    if !tree_state.initialized || tree_state.structure != snapshot.structure {
        cmds.push(PlatformCommand::PopulateTreeView {
            window_id,
            control_id: TREE_JOBS,
            items,
        });
        tree_state.initialized = true;
        tree_state.structure = snapshot.structure;
        tree_state.text_by_id = snapshot.text_by_id;
        tree_state.check_state_by_id = snapshot.check_state_by_id;
        return;
    }

    for item in &snapshot.structure {
        if let Some(new_text) = snapshot.text_by_id.get(&item.id) {
            if tree_state.text_by_id.get(&item.id) != Some(new_text) {
                cmds.push(PlatformCommand::UpdateTreeItemText {
                    window_id,
                    control_id: TREE_JOBS,
                    item_id: item.id,
                    text: new_text.clone(),
                });
            }
        }

        if let Some(new_state) = snapshot.check_state_by_id.get(&item.id) {
            if tree_state.check_state_by_id.get(&item.id) != Some(new_state) {
                cmds.push(PlatformCommand::UpdateTreeItemVisualState {
                    window_id,
                    control_id: TREE_JOBS,
                    item_id: item.id,
                    new_state: *new_state,
                });
            }
        }
    }

    tree_state.structure = snapshot.structure;
    tree_state.text_by_id = snapshot.text_by_id;
    tree_state.check_state_by_id = snapshot.check_state_by_id;
}

fn build_job_tree(view: &AppViewModel) -> Vec<TreeItemDescriptor> {
    view.jobs
        .iter()
        .map(|job| {
            let mut children = Vec::new();
            if job.link_count > 0 {
                children.push(TreeItemDescriptor {
                    id: links_folder_tree_item_id(job.job_id),
                    text: format!("Links ({})", job.link_count),
                    is_folder: true,
                    state: CheckState::Unchecked,
                    children: build_link_children(job),
                    style_override: None,
                });
            }
            TreeItemDescriptor {
                id: job_tree_item_id(job.job_id),
                text: format_job_row(job),
                is_folder: true,
                state: CheckState::Unchecked,
                children,
                style_override: None,
            }
        })
        .collect()
}

fn build_link_children(job: &JobRowView) -> Vec<TreeItemDescriptor> {
    let mut children: Vec<_> = job
        .links
        .iter()
        .filter(|link| link.kind == LinkKind::Hyperlink)
        .map(|link| TreeItemDescriptor {
            id: link_tree_item_id(job.job_id, link.index),
            text: link.label.clone(),
            is_folder: false,
            state: match link.download_state {
                LinkDownloadState::Downloaded { .. } => CheckState::Checked,
                _ => CheckState::Unchecked,
            },
            children: Vec::new(),
            style_override: None,
        })
        .collect();

    let remaining = job.link_count.saturating_sub(job.links.len());
    if remaining > 0 {
        children.push(TreeItemDescriptor {
            id: links_show_more_tree_item_id(job.job_id),
            text: format!("(show more… {} remaining)", remaining),
            is_folder: false,
            state: CheckState::Unchecked,
            children: Vec::new(),
            style_override: None,
        });
    }

    children
}

fn format_job_row(job: &JobRowView) -> String {
    let status = match &job.outcome {
        Some(JobResultKind::Success) => "OK".to_string(),
        Some(JobResultKind::Failed { reason }) => format!("ERR ({})", reason),
        None => stage_label(job.stage).to_string(),
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

fn format_preview_header(header: &PreviewHeaderView) -> String {
    let mut parts = Vec::new();
    if !header.domain.is_empty() {
        parts.push(header.domain.clone());
    }
    if let Some(tokens) = header.tokens {
        parts.push(format!("{} tokens", format_with_commas(tokens as u64)));
    }
    if let Some(bytes) = header.bytes {
        parts.push(format!("{bytes} B"));
    }
    parts.push(format!("{count} headings", count = header.heading_count));
    let stage_desc = match &header.outcome {
        Some(JobResultKind::Failed { reason }) => format!("Failed ({})", reason),
        Some(JobResultKind::Success) => "Done".to_string(),
        None => stage_label(header.stage).to_string(),
    };
    parts.push(stage_desc);
    if header.nav_heavy {
        parts.push("[nav-heavy]".to_string());
    }
    parts.join(" | ")
}

fn normalize_windows_newlines(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if matches!(chars.peek(), Some('\n')) {
                    chars.next();
                }
                normalized.push_str("\r\n");
            }
            '\n' => normalized.push_str("\r\n"),
            other => normalized.push(other),
        }
    }
    normalized
}

/// Builds a DefineLayout command with updated panel widths based on left_panel_width.
/// PANEL_INPUT keeps a fixed width, while PANEL_JOBS consumes the remaining left-region width.
fn build_layout_command(window_id: WindowId, left_panel_width: i32) -> PlatformCommand {
    let input_width = INPUT_PANEL_FIXED_WIDTH;
    let jobs_width = (left_panel_width - input_width).max(0);

    PlatformCommand::DefineLayout {
        window_id,
        rules: vec![
            // Progress panel at the top
            LayoutRule {
                control_id: PANEL_PROGRESS,
                parent_control_id: None,
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(64),
                margin: (0, 0, 0, 0),
            },
            // Progress label and bar inside the panel
            LayoutRule {
                control_id: LABEL_TOKEN_PROGRESS,
                parent_control_id: Some(PANEL_PROGRESS),
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(22),
                margin: (8, 8, 4, 8),
            },
            LayoutRule {
                control_id: PROGRESS_TOKENS,
                parent_control_id: Some(PANEL_PROGRESS),
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 8, 8, 8),
            },
            // Status bar panel at the very bottom
            LayoutRule {
                control_id: PANEL_BOTTOM,
                parent_control_id: None,
                dock_style: DockStyle::Bottom,
                order: 100,
                fixed_size: Some(32),
                margin: (0, 0, 0, 0),
            },
            // Buttons panel above the status bar
            LayoutRule {
                control_id: PANEL_BUTTONS,
                parent_control_id: None,
                dock_style: DockStyle::Bottom,
                order: 110,
                fixed_size: Some(44),
                margin: (0, 0, 0, 0),
            },
            // URL drop box on the left (fixed width)
            LayoutRule {
                control_id: PANEL_INPUT,
                parent_control_id: None,
                dock_style: DockStyle::Left,
                order: 200,
                fixed_size: Some(input_width),
                margin: (6, 6, 6, 6),
            },
            // Jobs panel consumes the remaining width in the left region
            LayoutRule {
                control_id: PANEL_JOBS,
                parent_control_id: None,
                dock_style: DockStyle::Left,
                order: 300,
                fixed_size: Some(jobs_width),
                margin: (6, 6, 6, 6),
            },
            // Jobs header label
            LayoutRule {
                control_id: LABEL_JOBS_HEADER,
                parent_control_id: Some(PANEL_JOBS),
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(28),
                margin: (0, 0, 4, 0),
            },
            // Jobs tree fills remaining space in panel
            LayoutRule {
                control_id: TREE_JOBS,
                parent_control_id: Some(PANEL_JOBS),
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            // Splitter between left panels and preview
            LayoutRule {
                control_id: SPLITTER_MAIN,
                parent_control_id: None,
                dock_style: DockStyle::Left,
                order: 305,
                fixed_size: Some(4),
                margin: (6, 0, 6, 0),
            },
            LayoutRule {
                control_id: PANEL_PREVIEW,
                parent_control_id: None,
                dock_style: DockStyle::Fill,
                order: 310,
                fixed_size: None,
                margin: (6, 6, 6, 6),
            },
            LayoutRule {
                control_id: LABEL_PREVIEW_HEADER,
                parent_control_id: Some(PANEL_PREVIEW),
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(28),
                margin: (6, 6, 4, 0),
            },
            LayoutRule {
                control_id: VIEWER_PREVIEW,
                parent_control_id: Some(PANEL_PREVIEW),
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            // Input hint label above the text box
            LayoutRule {
                control_id: LABEL_INPUT_HINT,
                parent_control_id: Some(PANEL_INPUT),
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(28),
                margin: (0, 0, 4, 0),
            },
            // URL input fills remaining space
            LayoutRule {
                control_id: INPUT_URLS,
                parent_control_id: Some(PANEL_INPUT),
                dock_style: DockStyle::Fill,
                order: 1,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            },
            // Status label fills the panel
            LayoutRule {
                control_id: LABEL_STATUS,
                parent_control_id: Some(PANEL_BOTTOM),
                dock_style: DockStyle::Fill,
                order: 0,
                fixed_size: None,
                margin: (6, 6, 6, 6),
            },
            // Buttons placed horizontally with fixed width
            LayoutRule {
                control_id: BUTTON_ARCHIVE,
                parent_control_id: Some(PANEL_BUTTONS),
                dock_style: DockStyle::Left,
                order: 0,
                fixed_size: Some(160),
                margin: (6, 6, 6, 6),
            },
            LayoutRule {
                control_id: BUTTON_STOP,
                parent_control_id: Some(PANEL_BUTTONS),
                dock_style: DockStyle::Left,
                order: 1,
                fixed_size: Some(160),
                margin: (6, 6, 6, 0),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::LinkRowView;
    use harvester_core::Stage;
    use std::path::PathBuf;
    use std::sync::Once;

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
        }
    }

    fn make_link_row(index: u32, label: &str, download_state: LinkDownloadState) -> LinkRowView {
        LinkRowView {
            index,
            url: format!("https://links.example/{index}"),
            label: label.to_string(),
            kind: LinkKind::Hyperlink,
            download_state,
            age_suspect: false,
        }
    }

    fn make_view(jobs: Vec<JobRowView>) -> AppViewModel {
        AppViewModel {
            job_count: jobs.len(),
            jobs,
            ..AppViewModel::default()
        }
    }

    #[test]
    fn preview_header_includes_headings_and_tokens() {
        init_logging();
        let header = PreviewHeaderView {
            domain: "example.com".to_string(),
            tokens: Some(1234),
            bytes: Some(2048),
            stage: Stage::Done,
            outcome: Some(JobResultKind::Success),
            heading_count: 8,
            link_density: 0.0,
            nav_heavy: false,
        };
        assert_eq!(
            format_preview_header(&header),
            "example.com | 1,234 tokens | 2048 B | 8 headings | Done"
        );
    }

    #[test]
    fn preview_header_appends_nav_heavy_indicator() {
        init_logging();
        let header = PreviewHeaderView {
            domain: "dense.example".to_string(),
            tokens: None,
            bytes: None,
            stage: Stage::Converting,
            outcome: None,
            heading_count: 0,
            link_density: 1.0,
            nav_heavy: true,
        };
        assert_eq!(
            format_preview_header(&header),
            "dense.example | 0 headings | Converting | [nav-heavy]"
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
            .any(|cmd| matches!(cmd, PlatformCommand::PopulateTreeView { .. })));

        let view_updated = make_view(vec![make_job(
            1,
            "https://example.com",
            Stage::Downloading,
            None,
            Some(100),
            Some(2048),
        )]);
        let commands_updated = render(window_id, &view_updated, &mut tree_state);

        assert!(!commands_updated
            .iter()
            .any(|cmd| matches!(cmd, PlatformCommand::PopulateTreeView { .. })));

        let mut text_updates = commands_updated
            .iter()
            .filter_map(|cmd| match cmd {
                PlatformCommand::UpdateTreeItemText { item_id, text, .. } => Some((item_id, text)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(text_updates.len(), 1);
        let (item_id, text) = text_updates.pop().expect("update exists");
        assert_eq!(*item_id, TreeItemId(1));
        assert_eq!(text, &format_job_row(&view_updated.jobs[0]));
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
            .any(|cmd| matches!(cmd, PlatformCommand::PopulateTreeView { .. })));
    }

    #[test]
    fn links_folder_and_show_more_children_rendered() {
        init_logging();
        let window_id = WindowId::new(4);
        let mut tree_state = TreeRenderState::new();
        let link = make_link_row(
            0,
            "Example",
            LinkDownloadState::Downloaded {
                path: PathBuf::from("linked/example.md"),
            },
        );
        let job = JobRowView {
            job_id: 42,
            url: "https://example.com".to_string(),
            stage: Stage::Done,
            outcome: Some(JobResultKind::Success),
            tokens: None,
            bytes: None,
            link_count: 4,
            downloaded_link_count: 1,
            links: vec![link],
        };
        let view = make_view(vec![job]);
        let commands = render(window_id, &view, &mut tree_state);
        let items = commands
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::PopulateTreeView { items, .. } => Some(items),
                _ => None,
            })
            .expect("populate emitted");
        let job_item = &items[0];
        assert_eq!(job_item.children.len(), 1);
        let folder = &job_item.children[0];
        assert_eq!(folder.text, "Links (4)");
        assert_eq!(folder.children.len(), 2);
        assert_eq!(folder.children[0].id, link_tree_item_id(42, 0));
        assert_eq!(folder.children[0].state, CheckState::Checked);
        let show_more = &folder.children[1];
        assert_eq!(show_more.id, links_show_more_tree_item_id(42));
        assert_eq!(show_more.text, "(show more… 3 remaining)");
    }

    #[test]
    fn normalize_windows_newlines_handles_various_sequences() {
        assert_eq!(normalize_windows_newlines("line1\nline2"), "line1\r\nline2");
        assert_eq!(normalize_windows_newlines("line1\rline2"), "line1\r\nline2");
        assert_eq!(
            normalize_windows_newlines("line1\r\nline2"),
            "line1\r\nline2"
        );
        assert_eq!(
            normalize_windows_newlines("line1\r\nline2\nline3\rline4"),
            "line1\r\nline2\r\nline3\r\nline4"
        );
    }

    #[test]
    fn preview_text_newlines_are_normalized_before_set_viewer_content() {
        init_logging();
        let window_id = WindowId::new(3);
        let mut tree_state = TreeRenderState::new();
        let view = AppViewModel {
            preview_text: Some("first\nsecond\r\nthird\rfourth".to_string()),
            ..Default::default()
        };

        let commands = render(window_id, &view, &mut tree_state);
        let viewer_text = commands
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::SetViewerContent { text, .. } => Some(text),
                _ => None,
            })
            .expect("SetViewerContent emitted");
        assert_eq!(viewer_text, "first\r\nsecond\r\nthird\r\nfourth");
    }

    #[test]
    fn splitter_resize_keeps_input_panel_fixed() {
        init_logging();
        let window_id = WindowId::new(5);
        let mut tree_state = TreeRenderState::new();
        let view = AppViewModel {
            left_panel_width: 760,
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
        let jobs_width = rules
            .iter()
            .find(|rule| rule.control_id == PANEL_JOBS)
            .and_then(|rule| rule.fixed_size)
            .expect("PANEL_JOBS fixed size");

        assert_eq!(input_width, 160);
        assert_eq!(jobs_width, 760 - 160);
    }
}
