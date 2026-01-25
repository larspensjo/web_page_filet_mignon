use commanductui::types::{TreeItemDescriptor, TreeItemId};
use commanductui::{CheckState, MessageSeverity, PlatformCommand, StyleId, WindowId};
use harvester_core::{
    AppViewModel, JobResultKind, JobRowView, PreviewHeaderView, SessionState, Stage,
};

use super::constants::*;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct TreeRenderState {
    initialized: bool,
    structure: Vec<TreeStructureItem>,
    text_by_id: HashMap<TreeItemId, String>,
    check_state_by_id: HashMap<TreeItemId, CheckState>,
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

    cmds.push(PlatformCommand::SetControlEnabled {
        window_id,
        control_id: BUTTON_ARCHIVE,
        enabled: view.job_count > 0,
    });

    let job_items = build_job_tree(view);
    append_tree_commands(window_id, job_items, tree_state, &mut cmds);

    let preview_text = view
        .preview_text
        .as_deref()
        .map(normalize_windows_newlines)
        .unwrap_or_default();
    cmds.push(PlatformCommand::SetViewerContent {
        window_id,
        control_id: VIEWER_PREVIEW,
        text: preview_text,
    });

    let header_text = view
        .preview_header
        .as_ref()
        .map(format_preview_header)
        .unwrap_or_else(|| "(no selection)".to_string());
    cmds.push(PlatformCommand::SetControlText {
        window_id,
        control_id: LABEL_PREVIEW_HEADER,
        text: header_text,
    });

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
    let stage_desc = match header.outcome {
        Some(JobResultKind::Failed) => "Failed".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::Stage;
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
}
