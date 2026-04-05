use commanductui::types::{TreeItemDescriptor, TreeItemId};
use commanductui::{
    ChartDataPacket, ChartLineData, ChartLineEmphasis, CheckState, MessageSeverity,
    PlatformCommand, StyleId, WindowId,
};
use engine_logging::{engine_debug, engine_info, engine_warn};
use harvester_core::{
    AppTab, AppViewModel, JobFilterStatus, JobListScope, JobResultKind, JobRowView,
    LayoutViewModel, LeftTab, LlmModelUsageView, PreviewHeaderView, PromptLabStage, SessionState,
    Stage, TrendsTabView, DEFAULT_JOBS_PANEL_WIDTH,
};
use harvester_core::{LeftPaneHeaderView, PreviewContextView};
use harvester_engine::llm::ModelId;
use harvester_engine::LinkKind;

use super::constants::*;
use super::layout::{build_layout_command, LayoutConfig, PromptLabLayoutConfig};
use super::markdown_to_rtf::{convert_markdown_to_rtf, RTF_TRUNCATE_MARKER};
use super::tree_item_ids::{
    job_tree_item_id, link_tree_item_id, links_folder_tree_item_id, links_show_more_tree_item_id,
};
use std::collections::HashMap;

const MAX_VIEWER_CHARS: usize = 64 * 1024;
#[allow(dead_code)]
const VIEWER_TRUNCATE_MARKER: &str = "[display truncated]";
const SUMMARY_EMPTY_STATE_MARKDOWN: &str =
    "No article selected\n\nSelect a job/article from the list to view its summary.";
/// Maximum number of models shown individually in the status bar before collapsing.
const MAX_STATUS_BAR_MODELS: usize = 2;

fn format_compact_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
    }
}

/// Builds the LLM usage segment for the status bar, or None if no usage recorded.
/// Shows up to MAX_STATUS_BAR_MODELS models individually; collapses the rest.
fn format_llm_usage_status(rows: &[LlmModelUsageView]) -> Option<String> {
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

/// Helper: Convert optional ModelId to combo box selection index.
/// Returns 0 for None (default), or index+1 for a specific model in the catalog.
pub(crate) fn model_to_combo_index(
    selected: Option<&ModelId>,
    catalog: &[ModelId],
) -> Option<usize> {
    match selected {
        None => Some(0),
        Some(model) => catalog.iter().position(|m| m == model).map(|idx| idx + 1),
    }
}

/// Helper: Convert combo box selection index to optional ModelId.
/// Returns None for index 0 (default), or the model at (index-1) in the catalog.
pub(crate) fn combo_index_to_model(index: usize, catalog: &[ModelId]) -> Option<ModelId> {
    if index == 0 {
        None
    } else {
        catalog.get(index - 1).cloned()
    }
}

#[derive(Debug)]
pub struct TreeRenderState {
    initialized: bool,
    structure: Vec<TreeStructureItem>,
    text_by_id: HashMap<TreeItemId, String>,
    check_state_by_id: HashMap<TreeItemId, CheckState>,
    /// Tracks the previous left_panel_width to detect changes
    prev_left_panel_width: i32,
    prev_input_panel_visible: bool,
    prev_status_label: Option<(String, MessageSeverity)>,
    prev_progress_text: Option<String>,
    prev_preview_text: Option<String>,
    prev_stop_enabled: Option<bool>,
    prev_briefing_enabled: Option<bool>,
    prev_triage_enabled: Option<bool>,
    prev_poll_enabled: Option<bool>,
    prev_briefing_progress: Option<String>,
    prev_triage_progress: Option<String>,
    prev_progress_range: Option<(u32, u32)>,
    prev_progress_pos: Option<u32>,
    prev_token_progress_style: Option<StyleId>,
    prev_stop_style: Option<StyleId>,
    prev_operation_progress_visible: bool,
    prev_operation_progress_text: Option<String>,
    prev_operation_progress_range: Option<(u32, u32)>,
    prev_operation_progress_pos: Option<u32>,
    prev_open_browser_enabled: Option<bool>,
    prev_jobs_header_meta_text: Option<String>,
    prev_jobs_scope_since_checkpoint_checked: Option<bool>,
    prev_prompt_lab_visible: bool,
    prev_prompt_lab_advanced_mode: bool,
    prev_prompt_lab_mode_basic_checked: Option<bool>,
    prev_prompt_lab_mode_advanced_checked: Option<bool>,
    prev_prompt_lab_stage_triage_checked: Option<bool>,
    prev_prompt_lab_stage_summary_checked: Option<bool>,
    prev_prompt_lab_stage_briefing_checked: Option<bool>,
    prev_prompt_lab_compare_section_open: bool,
    prev_prompt_lab_context_section_open: bool,
    prev_prompt_lab_template_section_open: bool,
    prev_prompt_lab_run_details_section_open: bool,
    prev_prompt_lab_template_editor_open: bool,
    prev_prompt_lab_status_text: Option<String>,
    prev_prompt_lab_metadata_text: Option<String>,
    prev_prompt_lab_url_input: Option<String>,
    prev_prompt_lab_run_enabled: Option<bool>,
    prev_prompt_lab_resolve_enabled: Option<bool>,
    prev_prompt_lab_url_enabled: Option<bool>,
    prev_prompt_lab_context_text: Option<String>,
    prev_prompt_lab_context_status_text: Option<String>,
    prev_prompt_lab_context_apply_enabled: Option<bool>,
    prev_prompt_lab_context_apply_rerun_enabled: Option<bool>,
    prev_prompt_lab_context_revert_enabled: Option<bool>,
    prev_prompt_lab_context_save_enabled: Option<bool>,
    prev_prompt_lab_template_open_checked: Option<bool>,
    prev_prompt_lab_template_system_text: Option<String>,
    prev_prompt_lab_template_user_text: Option<String>,
    prev_prompt_lab_template_status_text: Option<String>,
    prev_prompt_lab_template_apply_enabled: Option<bool>,
    prev_prompt_lab_template_apply_rerun_enabled: Option<bool>,
    prev_prompt_lab_template_revert_enabled: Option<bool>,
    prev_prompt_lab_template_save_enabled: Option<bool>,
    prev_prompt_lab_template_system_enabled: Option<bool>,
    prev_prompt_lab_template_user_enabled: Option<bool>,
    prev_prompt_lab_compare_add_current_enabled: Option<bool>,
    prev_prompt_lab_compare_add_baseline_enabled: Option<bool>,
    prev_prompt_lab_compare_reset_draft_enabled: Option<bool>,
    prev_prompt_lab_compare_start_enabled: Option<bool>,
    prev_prompt_lab_compare_cancel_enabled: Option<bool>,
    prev_prompt_lab_compare_auto_select_enabled: Option<bool>,
    prev_prompt_lab_compare_winner_clear_enabled: Option<bool>,
    prev_prompt_lab_section_compare_checked: Option<bool>,
    prev_prompt_lab_section_context_checked: Option<bool>,
    prev_prompt_lab_section_template_checked: Option<bool>,
    prev_prompt_lab_section_run_details_checked: Option<bool>,
    prev_prompt_lab_model_catalog: Option<Vec<ModelId>>,
    prev_prompt_lab_selected_model: Option<String>,
    // Tab bar state
    prev_active_tab: AppTab,
    prev_left_tab: LeftTab,
    prev_triage_text: Option<String>,
    prev_briefing_text: Option<String>,
    prev_poll_stats_text: Option<String>,
    prev_preview_header_override_text: Option<String>,
    prev_preview_source_text: Option<String>,
    prev_preview_status_text: Option<String>,
    prev_preview_attention_text: Option<String>,
}

impl Default for TreeRenderState {
    fn default() -> Self {
        Self {
            initialized: false,
            structure: Vec::new(),
            text_by_id: HashMap::new(),
            check_state_by_id: HashMap::new(),
            prev_left_panel_width: DEFAULT_JOBS_PANEL_WIDTH,
            prev_input_panel_visible: false,
            prev_status_label: None,
            prev_progress_text: None,
            prev_preview_text: None,
            prev_stop_enabled: None,
            prev_briefing_enabled: None,
            prev_triage_enabled: None,
            prev_poll_enabled: None,
            prev_briefing_progress: None,
            prev_triage_progress: None,
            prev_progress_range: None,
            prev_progress_pos: None,
            prev_token_progress_style: None,
            prev_stop_style: None,
            prev_operation_progress_visible: false,
            prev_operation_progress_text: None,
            prev_operation_progress_range: None,
            prev_operation_progress_pos: None,
            prev_open_browser_enabled: None,
            prev_jobs_header_meta_text: None,
            prev_jobs_scope_since_checkpoint_checked: None,
            prev_prompt_lab_visible: false,
            prev_prompt_lab_advanced_mode: false,
            prev_prompt_lab_mode_basic_checked: None,
            prev_prompt_lab_mode_advanced_checked: None,
            prev_prompt_lab_stage_triage_checked: None,
            prev_prompt_lab_stage_summary_checked: None,
            prev_prompt_lab_stage_briefing_checked: None,
            prev_prompt_lab_compare_section_open: false,
            prev_prompt_lab_context_section_open: false,
            prev_prompt_lab_template_section_open: false,
            prev_prompt_lab_run_details_section_open: false,
            prev_prompt_lab_template_editor_open: false,
            prev_prompt_lab_status_text: None,
            prev_prompt_lab_metadata_text: None,
            prev_prompt_lab_url_input: None,
            prev_prompt_lab_run_enabled: None,
            prev_prompt_lab_resolve_enabled: None,
            prev_prompt_lab_url_enabled: None,
            prev_prompt_lab_context_text: None,
            prev_prompt_lab_context_status_text: None,
            prev_prompt_lab_context_apply_enabled: None,
            prev_prompt_lab_context_apply_rerun_enabled: None,
            prev_prompt_lab_context_revert_enabled: None,
            prev_prompt_lab_context_save_enabled: None,
            prev_prompt_lab_template_open_checked: None,
            prev_prompt_lab_template_system_text: None,
            prev_prompt_lab_template_user_text: None,
            prev_prompt_lab_template_status_text: None,
            prev_prompt_lab_template_apply_enabled: None,
            prev_prompt_lab_template_apply_rerun_enabled: None,
            prev_prompt_lab_template_revert_enabled: None,
            prev_prompt_lab_template_save_enabled: None,
            prev_prompt_lab_template_system_enabled: None,
            prev_prompt_lab_template_user_enabled: None,
            prev_prompt_lab_compare_add_current_enabled: None,
            prev_prompt_lab_compare_add_baseline_enabled: None,
            prev_prompt_lab_compare_reset_draft_enabled: None,
            prev_prompt_lab_compare_start_enabled: None,
            prev_prompt_lab_compare_cancel_enabled: None,
            prev_prompt_lab_compare_auto_select_enabled: None,
            prev_prompt_lab_compare_winner_clear_enabled: None,
            prev_prompt_lab_section_compare_checked: None,
            prev_prompt_lab_section_context_checked: None,
            prev_prompt_lab_section_template_checked: None,
            prev_prompt_lab_section_run_details_checked: None,
            prev_prompt_lab_model_catalog: None,
            prev_prompt_lab_selected_model: None,
            prev_active_tab: AppTab::Summary,
            prev_left_tab: LeftTab::default(),
            prev_triage_text: None,
            prev_briefing_text: None,
            prev_poll_stats_text: None,
            prev_preview_header_override_text: None,
            prev_preview_source_text: None,
            prev_preview_status_text: None,
            prev_preview_attention_text: None,
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

pub fn render(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
) -> Vec<PlatformCommand> {
    let mut cmds = Vec::new();
    render_layout_section(
        window_id,
        &layout_view_from_app_view(view),
        tree_state,
        &mut cmds,
    );
    render_tab_bar_section(window_id, view, tree_state, &mut cmds);
    render_left_tab_bar_section(window_id, view, tree_state, &mut cmds);
    render_status_section(window_id, view, tree_state, &mut cmds);
    render_operation_progress_section(window_id, view, tree_state, &mut cmds);
    render_token_progress_section(window_id, view, tree_state, &mut cmds);
    render_main_controls_section(window_id, view, tree_state, &mut cmds);
    render_prompt_lab_section(window_id, view, tree_state, &mut cmds);

    let job_items = build_job_tree(view);
    append_tree_commands(window_id, job_items, tree_state, &mut cmds);

    render_preview_section(window_id, view, tree_state, &mut cmds);

    cmds
}

pub(crate) fn render_layout_only(
    window_id: WindowId,
    layout: &LayoutViewModel,
    tree_state: &mut TreeRenderState,
) -> Vec<PlatformCommand> {
    let mut cmds = Vec::new();
    render_layout_section(window_id, layout, tree_state, &mut cmds);
    cmds
}

/// Converts a `TrendsTabView` into a `ChartDataPacket` for the chart control.
/// Uses a fixed 10-color warm-toned palette, assigned by entity index.
fn build_chart_data(trends: &TrendsTabView) -> ChartDataPacket {
    // COLORREF palette (0x00BBGGRR), warm-compatible chart colors.
    const COLORS: [u32; 10] = [
        0x004264C9, // #C96442 terracotta
        0x005777D9, // #D97757 coral
        0x00A5AEB0, // #B0AEA5 warm silver
        0x007F8687, // #87867F stone
        0x0059935E, // #5E9359 muted green
        0x005AACB8, // #B8AC5A warm gold
        0x008C6DAF, // #AF6D8C muted mauve
        0x0068A5C4, // #C4A568 sand
        0x00A0827A, // #7A82A0 cool-warm lavender
        0x006BB088, // #88B06B sage
    ];

    if trends.is_loading {
        return ChartDataPacket {
            lines: vec![],
            week_labels: vec![],
            is_loading: true,
            show_x_axis_labels: true,
            show_y_axis_labels: true,
            show_end_labels: false,
        };
    }
    let Some(cat_data) = &trends.category_data else {
        return ChartDataPacket {
            lines: vec![],
            week_labels: vec![],
            is_loading: false,
            show_x_axis_labels: true,
            show_y_axis_labels: true,
            show_end_labels: false,
        };
    };
    let lines = cat_data
        .lines
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, el)| ChartLineData {
            label: el.label.clone(),
            weekly_counts: el.weekly_counts.clone(),
            color: COLORS[i],
            end_label: Some(el.label.clone()),
            emphasis: if i < 2 {
                ChartLineEmphasis::Primary
            } else {
                ChartLineEmphasis::Secondary
            },
        })
        .collect();
    ChartDataPacket {
        lines,
        week_labels: cat_data.weeks.clone(),
        is_loading: false,
        show_x_axis_labels: true,
        show_y_axis_labels: true,
        show_end_labels: true,
    }
}

fn emit_if_changed<T, F>(prev: &mut Option<T>, next: T, cmds: &mut Vec<PlatformCommand>, emit: F)
where
    T: PartialEq + Clone,
    F: FnOnce(T) -> PlatformCommand,
{
    if prev.as_ref() != Some(&next) {
        cmds.push(emit(next.clone()));
        *prev = Some(next);
    }
}

fn layout_view_from_app_view(view: &AppViewModel) -> LayoutViewModel {
    LayoutViewModel {
        left_panel_width: view.left_panel_width,
        input_panel_visible: view.input_panel_visible,
        operation_progress_visible: view.operation_progress_visible,
        active_tab: view.right_pane.active_tab,
        left_tab: view.left_pane.left_tab,
        left_header_meta_visible: view.left_pane_header.scope_label.is_some()
            || view.left_pane_header.count_label.is_some()
            || view.left_pane_header.state_label.is_some(),
        preview_header_override_visible: view.preview_header_text.is_some(),
        preview_context_visible: view.preview_context.is_some()
            && view.preview_header_text.is_none(),
        preview_attention_visible: view
            .preview_context
            .as_ref()
            .and_then(|context| context.attention_label.as_ref())
            .is_some()
            && view.preview_header_text.is_none(),
        prompt_lab_advanced_mode: view.left_pane.prompt_lab.advanced_mode,
        prompt_lab_compare_section_open: view.left_pane.prompt_lab.compare_section_open,
        prompt_lab_context_section_open: view.left_pane.prompt_lab.context_section_open,
        prompt_lab_template_section_open: view.left_pane.prompt_lab.template_section_open,
        prompt_lab_run_details_section_open: view.left_pane.prompt_lab.run_details_section_open,
        prompt_lab_template_editor_open: view.left_pane.prompt_lab.template_editor_open,
    }
}

fn render_layout_section(
    window_id: WindowId,
    layout: &LayoutViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let prompt_lab_tab_visible = layout.left_tab == LeftTab::PromptLab;
    let layout_changed = layout.left_panel_width != tree_state.prev_left_panel_width
        || layout.input_panel_visible != tree_state.prev_input_panel_visible
        || layout.operation_progress_visible != tree_state.prev_operation_progress_visible
        || layout.active_tab != tree_state.prev_active_tab
        || layout.left_tab != tree_state.prev_left_tab
        || prompt_lab_tab_visible != tree_state.prev_prompt_lab_visible
        || layout.prompt_lab_advanced_mode != tree_state.prev_prompt_lab_advanced_mode
        || layout.prompt_lab_compare_section_open
            != tree_state.prev_prompt_lab_compare_section_open
        || layout.prompt_lab_context_section_open
            != tree_state.prev_prompt_lab_context_section_open
        || layout.prompt_lab_template_section_open
            != tree_state.prev_prompt_lab_template_section_open
        || layout.prompt_lab_run_details_section_open
            != tree_state.prev_prompt_lab_run_details_section_open
        || layout.prompt_lab_template_editor_open
            != tree_state.prev_prompt_lab_template_editor_open;
    if !layout_changed {
        return;
    }
    engine_debug!(
        "[Render] Layout update: left_panel_width {} -> {}, input_panel_visible: {} -> {}, active_tab: {:?} -> {:?}",
        tree_state.prev_left_panel_width,
        layout.left_panel_width,
        tree_state.prev_input_panel_visible,
        layout.input_panel_visible,
        tree_state.prev_active_tab,
        layout.active_tab,
    );
    cmds.push(build_layout_command(
        window_id,
        LayoutConfig {
            left_panel_width: layout.left_panel_width,
            input_panel_visible: layout.input_panel_visible,
            operation_progress_visible: layout.operation_progress_visible,
            left_header_meta_visible: layout.left_header_meta_visible,
            preview_header_override_visible: layout.preview_header_override_visible,
            preview_context_visible: layout.preview_context_visible,
            preview_attention_visible: layout.preview_attention_visible,
            active_tab: layout.active_tab,
            left_tab: layout.left_tab,
            prompt_lab: PromptLabLayoutConfig {
                visible: prompt_lab_tab_visible,
                advanced_mode: layout.prompt_lab_advanced_mode,
                compare_section_open: layout.prompt_lab_compare_section_open,
                context_section_open: layout.prompt_lab_context_section_open,
                template_section_open: layout.prompt_lab_template_section_open,
                run_details_section_open: layout.prompt_lab_run_details_section_open,
                template_editor_open: layout.prompt_lab_template_editor_open,
            },
        },
    ));
    if prompt_lab_tab_visible && !tree_state.prev_prompt_lab_visible {
        tree_state.prev_prompt_lab_model_catalog = None;
        tree_state.prev_prompt_lab_selected_model = None;
    }
    tree_state.prev_left_panel_width = layout.left_panel_width;
    tree_state.prev_input_panel_visible = layout.input_panel_visible;
    tree_state.prev_operation_progress_visible = layout.operation_progress_visible;
    tree_state.prev_active_tab = layout.active_tab;
    tree_state.prev_left_tab = layout.left_tab;
    tree_state.prev_prompt_lab_visible = prompt_lab_tab_visible;
    tree_state.prev_prompt_lab_advanced_mode = layout.prompt_lab_advanced_mode;
    tree_state.prev_prompt_lab_compare_section_open = layout.prompt_lab_compare_section_open;
    tree_state.prev_prompt_lab_context_section_open = layout.prompt_lab_context_section_open;
    tree_state.prev_prompt_lab_template_section_open = layout.prompt_lab_template_section_open;
    tree_state.prev_prompt_lab_run_details_section_open =
        layout.prompt_lab_run_details_section_open;
    tree_state.prev_prompt_lab_template_editor_open = layout.prompt_lab_template_editor_open;
}

fn render_tab_bar_section(
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

fn render_left_tab_bar_section(
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

fn render_status_section(
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
        &mut tree_state.prev_status_label,
        (status_parts.join(" | "), severity),
        cmds,
        |(text, severity)| PlatformCommand::UpdateLabelText {
            window_id,
            control_id: LABEL_STATUS,
            text,
            severity,
        },
    );
    tree_state.prev_briefing_progress = view.briefing_progress.clone();
    tree_state.prev_triage_progress = view.triage_progress.clone();
}

fn render_operation_progress_section(
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
        &mut tree_state.prev_operation_progress_text,
        text,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_OPERATION_PROGRESS,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_operation_progress_range,
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
        &mut tree_state.prev_operation_progress_pos,
        pos,
        cmds,
        |position| PlatformCommand::SetProgressBarPosition {
            window_id,
            control_id: PROGRESS_OPERATION,
            position,
        },
    );
}

fn render_token_progress_section(
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
        &mut tree_state.prev_progress_range,
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
        &mut tree_state.prev_progress_pos,
        clamped_tokens as u32,
        cmds,
        |position| PlatformCommand::SetProgressBarPosition {
            window_id,
            control_id: PROGRESS_TOKENS,
            position,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_token_progress_style,
        progress_style,
        cmds,
        |style_id| PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id: PROGRESS_TOKENS,
            style_id,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_progress_text,
        progress_text,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_TOKEN_PROGRESS,
            text,
        },
    );
}

fn render_main_controls_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    emit_if_changed(
        &mut tree_state.prev_jobs_header_meta_text,
        format_left_pane_header_meta(&view.left_pane_header),
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_JOBS_HEADER_META,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_stop_style,
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
        &mut tree_state.prev_stop_enabled,
        matches!(view.session, SessionState::Running),
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_STOP,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_briefing_enabled,
        view.briefing_can_start,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_BRIEFING,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_triage_enabled,
        view.triage_can_start,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_TRIAGE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_poll_enabled,
        view.poll_sources_enabled,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_POLL_SOURCES,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_open_browser_enabled,
        view.selected_url.is_some(),
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_OPEN_BROWSER,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_jobs_scope_since_checkpoint_checked,
        view.left_pane.job_list_scope == JobListScope::SinceCheckpoint,
        cmds,
        |checked| PlatformCommand::SetToggleSwitchState {
            window_id,
            control_id: TS_JOBS_SCOPE,
            checked,
        },
    );
}

fn format_left_pane_header_meta(header: &LeftPaneHeaderView) -> String {
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

fn render_prompt_lab_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_mode_basic_checked,
        !view.left_pane.prompt_lab.advanced_mode,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_PROMPT_LAB_MODE_BASIC,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_mode_advanced_checked,
        view.left_pane.prompt_lab.advanced_mode,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_PROMPT_LAB_MODE_ADVANCED,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_stage_triage_checked,
        view.left_pane.prompt_lab.selected_stage == PromptLabStage::Triage,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_STAGE_TRIAGE,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_stage_summary_checked,
        view.left_pane.prompt_lab.selected_stage == PromptLabStage::Summary,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_STAGE_SUMMARY,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_stage_briefing_checked,
        view.left_pane.prompt_lab.selected_stage == PromptLabStage::Briefing,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_STAGE_BRIEFING,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_section_compare_checked,
        view.left_pane.prompt_lab.compare_section_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_SECTION_COMPARE,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_section_context_checked,
        view.left_pane.prompt_lab.context_section_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_SECTION_CONTEXT,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_section_template_checked,
        view.left_pane.prompt_lab.template_section_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_SECTION_TEMPLATE,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_section_run_details_checked,
        view.left_pane.prompt_lab.run_details_section_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_SECTION_RUN_DETAILS,
            checked,
        },
    );

    let model_catalog = &view.left_pane.prompt_lab.model_catalog;
    let selected_model = view.left_pane.prompt_lab.selected_model_override.as_ref();
    if tree_state.prev_prompt_lab_model_catalog.as_deref() != Some(model_catalog.as_slice()) {
        engine_info!(
            "[prompt-lab-model] render updating combo items source={:?} count={}",
            view.left_pane.prompt_lab.model_catalog_source,
            model_catalog.len()
        );
        let mut items = vec!["Default".to_string()];
        items.extend(model_catalog.iter().map(|m| m.model_name().to_string()));
        cmds.push(PlatformCommand::SetComboBoxItems {
            window_id,
            control_id: COMBO_PROMPT_LAB_MODEL_SELECTOR,
            items,
        });
        tree_state.prev_prompt_lab_model_catalog = Some(model_catalog.clone());
    }
    let selected_model_key = selected_model
        .map(|m| m.model_name().to_string())
        .unwrap_or_else(|| "__DEFAULT__".to_string());
    if tree_state.prev_prompt_lab_selected_model.as_deref() != Some(selected_model_key.as_str()) {
        let index = model_to_combo_index(selected_model, model_catalog);
        engine_info!(
            "[prompt-lab-model] render updating combo selection key={} index={:?} catalog_count={}",
            selected_model_key,
            index,
            model_catalog.len()
        );
        cmds.push(PlatformCommand::SetComboBoxSelection {
            window_id,
            control_id: COMBO_PROMPT_LAB_MODEL_SELECTOR,
            selected_index: index,
        });
        tree_state.prev_prompt_lab_selected_model = Some(selected_model_key);
    }

    emit_if_changed(
        &mut tree_state.prev_prompt_lab_run_enabled,
        view.left_pane.prompt_lab.can_run,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_RUN,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_resolve_enabled,
        !view.left_pane.prompt_lab.resolve_pending,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_RESOLVE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_url_enabled,
        true,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: INPUT_PROMPT_LAB_URL,
            enabled,
        },
    );

    let compare_add_enabled = view.left_pane.prompt_lab.can_add_candidate;
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_compare_add_current_enabled,
        compare_add_enabled,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_ADD_CURRENT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_compare_add_baseline_enabled,
        compare_add_enabled,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_ADD_BASELINE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_compare_reset_draft_enabled,
        view.left_pane.prompt_lab.can_reset_draft,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_RESET_DRAFT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_compare_start_enabled,
        view.left_pane.prompt_lab.active_batch.is_none()
            && view.left_pane.prompt_lab.draft_candidates.len() >= 2,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_START,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_compare_cancel_enabled,
        view.left_pane
            .prompt_lab
            .active_batch
            .as_ref()
            .map(|batch| batch.can_cancel)
            .unwrap_or(false),
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_CANCEL,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_compare_auto_select_enabled,
        view.left_pane
            .prompt_lab
            .active_batch
            .as_ref()
            .map(|batch| batch.can_auto_select)
            .unwrap_or(false),
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_AUTO_SELECT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_compare_winner_clear_enabled,
        view.left_pane
            .prompt_lab
            .active_batch
            .as_ref()
            .map(|batch| {
                batch
                    .rows
                    .iter()
                    .any(|row| row.is_manual_winner || row.is_auto_winner)
            })
            .unwrap_or(false),
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_WINNER_CLEAR,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_context_apply_enabled,
        view.left_pane.prompt_lab.can_apply_context,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_APPLY,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_context_apply_rerun_enabled,
        view.left_pane.prompt_lab.can_apply_and_rerun,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_context_revert_enabled,
        view.left_pane.prompt_lab.can_revert_context,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_REVERT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_context_save_enabled,
        view.left_pane.prompt_lab.can_save_context,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_SAVE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_template_open_checked,
        view.left_pane.prompt_lab.template_editor_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_TEMPLATE_OPEN,
            checked,
        },
    );

    let can_apply_template = view.left_pane.prompt_lab.template_dirty
        && view
            .left_pane
            .prompt_lab
            .template_validation_errors
            .is_empty();
    let can_apply_template_and_rerun = can_apply_template && view.left_pane.prompt_lab.can_run;
    let can_revert_template =
        view.left_pane.prompt_lab.template_dirty || view.left_pane.prompt_lab.template_applied;
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_template_apply_enabled,
        can_apply_template,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_template_apply_rerun_enabled,
        can_apply_template_and_rerun,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_template_revert_enabled,
        can_revert_template,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_REVERT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_template_save_enabled,
        view.left_pane.prompt_lab.template_applied,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_SAVE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_template_system_text,
        view.left_pane.prompt_lab.template_system_draft.clone(),
        cmds,
        |text| PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_template_user_text,
        view.left_pane.prompt_lab.template_user_draft.clone(),
        cmds,
        |text| PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_template_system_enabled,
        view.left_pane.prompt_lab.template_editor_open,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_template_user_enabled,
        view.left_pane.prompt_lab.template_editor_open,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_url_input,
        view.left_pane.prompt_lab.url_input.clone(),
        cmds,
        |text| PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_URL,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_status_text,
        prompt_lab_status_text(&view.left_pane.prompt_lab),
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_STATUS,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_metadata_text,
        prompt_lab_metadata_text(&view.left_pane.prompt_lab),
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_METADATA,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_context_text,
        view.left_pane.prompt_lab.context_draft_text.clone(),
        cmds,
        |text| PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_CONTEXT,
            text,
        },
    );
    let context_status_text = if !view
        .left_pane
        .prompt_lab
        .context_validation_errors
        .is_empty()
    {
        view.left_pane
            .prompt_lab
            .context_validation_errors
            .join(" • ")
    } else {
        view.left_pane
            .prompt_lab
            .context_status_message
            .clone()
            .unwrap_or_default()
    };
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_context_status_text,
        context_status_text,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_CONTEXT_STATUS,
            text,
        },
    );
    let template_status_text = if !view
        .left_pane
        .prompt_lab
        .template_validation_errors
        .is_empty()
    {
        view.left_pane
            .prompt_lab
            .template_validation_errors
            .join(" • ")
    } else if let (Some(version), Some(path)) = (
        view.left_pane.prompt_lab.template_saved_version,
        view.left_pane.prompt_lab.template_saved_path.as_deref(),
    ) {
        format!("Saved template v{version} to {path}")
    } else if view.left_pane.prompt_lab.template_applied {
        "Template draft applied".to_string()
    } else if view.left_pane.prompt_lab.template_dirty {
        "Template draft has unapplied changes".to_string()
    } else {
        String::new()
    };
    emit_if_changed(
        &mut tree_state.prev_prompt_lab_template_status_text,
        template_status_text,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_TEMPLATE_STATUS,
            text,
        },
    );
}

fn render_preview_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    // Summary tab: only show the selected article summary; never fall back to shared preview text.
    let summary_markdown = view
        .right_pane
        .summary_markdown
        .as_deref()
        .unwrap_or(SUMMARY_EMPTY_STATE_MARKDOWN);
    if tree_state.prev_preview_text.as_deref() != Some(summary_markdown) {
        let (truncated_markdown, was_truncated) = truncate_markdown_for_preview(summary_markdown);
        let mut rtf_text = convert_markdown_to_rtf(&truncated_markdown);
        if was_truncated {
            engine_warn!(
                "[preview] summary markdown truncated from {} chars to {} chars",
                summary_markdown.chars().count(),
                truncated_markdown.chars().count()
            );
            if rtf_text.ends_with('}') {
                rtf_text.pop();
            }
            rtf_text.push_str("\\par ");
            rtf_text.push_str(RTF_TRUNCATE_MARKER);
            rtf_text.push('}');
        }
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_PREVIEW,
            rtf_text,
        });
        tree_state.prev_preview_text = Some(summary_markdown.to_string());
    }

    // Triage tab viewer.
    let triage_markdown = view
        .right_pane
        .triage_markdown
        .as_deref()
        .unwrap_or_default();
    if tree_state.prev_triage_text.as_deref() != Some(triage_markdown) {
        let (truncated, _) = truncate_markdown_for_preview(triage_markdown);
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_TRIAGE,
            rtf_text: convert_markdown_to_rtf(&truncated),
        });
        tree_state.prev_triage_text = Some(triage_markdown.to_string());
    }

    // Briefing tab viewer.
    let briefing_markdown = view
        .right_pane
        .briefing_markdown
        .as_deref()
        .unwrap_or_default();
    if tree_state.prev_briefing_text.as_deref() != Some(briefing_markdown) {
        let (truncated, _) = truncate_markdown_for_preview(briefing_markdown);
        let display = strip_leading_h1(&truncated);
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_BRIEFING,
            rtf_text: convert_markdown_to_rtf(display),
        });
        tree_state.prev_briefing_text = Some(briefing_markdown.to_string());
    }

    // Poll Stats tab viewer.
    let poll_stats_text = view
        .right_pane
        .poll_stats_markdown
        .as_deref()
        .unwrap_or("No poll data yet.");
    if tree_state.prev_poll_stats_text.as_deref() != Some(poll_stats_text) {
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_POLL_STATS,
            rtf_text: convert_markdown_to_rtf(poll_stats_text),
        });
        tree_state.prev_poll_stats_text = Some(poll_stats_text.to_string());
    }

    // Trends tab: category selector.
    let active_category = view.right_pane.trends.active_category;
    cmds.push(PlatformCommand::SetTabBarSelection {
        window_id,
        control_id: TAB_BAR_TRENDS,
        selected_index: active_category.to_index(),
    });

    // Trends tab: chart data (unconditional — mirrors radio button pattern; InvalidateRect is cheap).
    cmds.push(PlatformCommand::SetChartData {
        window_id,
        control_id: CHART_TRENDS,
        data: build_chart_data(&view.right_pane.trends),
    });

    if let Some(header_text) = view.preview_header_text.clone() {
        emit_if_changed(
            &mut tree_state.prev_preview_header_override_text,
            header_text,
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_HEADER,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.prev_preview_source_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_SOURCE,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.prev_preview_status_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_STATUS,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.prev_preview_attention_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_ATTENTION,
                text,
            },
        );
    } else if let Some(context) = view.preview_context.as_ref() {
        emit_if_changed(
            &mut tree_state.prev_preview_header_override_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_HEADER,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.prev_preview_source_text,
            context.source_label.clone(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_SOURCE,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.prev_preview_status_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_STATUS,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.prev_preview_attention_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_ATTENTION,
                text,
            },
        );
    } else {
        emit_if_changed(
            &mut tree_state.prev_preview_header_override_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_HEADER,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.prev_preview_source_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_SOURCE,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.prev_preview_status_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_STATUS,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.prev_preview_attention_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_ATTENTION,
                text,
            },
        );
    }
}

fn prompt_lab_status_text(prompt_lab: &harvester_core::PromptLabView) -> String {
    if prompt_lab.is_in_flight {
        return "Prompt Lab: Running...".to_string();
    }
    if let Some(reason) = prompt_lab.run_disabled_reason {
        if !prompt_lab.can_run {
            return format!("Prompt Lab: {reason}");
        }
    }
    if let Some(err) = prompt_lab.latest_validation_error.as_deref() {
        return format!("Prompt Lab validation: {err}");
    }
    if prompt_lab.url_resolve_failed {
        return "Prompt Lab: URL resolve failed".to_string();
    }
    if let Some(run) = prompt_lab.latest_run.as_ref() {
        if let Some(reason) = run.failure_reason.as_deref() {
            return format!("Prompt Lab failed: {reason}");
        }
        return format!("Prompt Lab: latest run {}", run.status_label);
    }
    "Prompt Lab ready".to_string()
}

fn prompt_lab_metadata_text(prompt_lab: &harvester_core::PromptLabView) -> String {
    let template_source = prompt_lab
        .template_snapshot_source
        .map(|source| format!("{source:?}").to_lowercase())
        .unwrap_or_else(|| "-".to_string());
    let template_version = prompt_lab
        .template_snapshot_version
        .map(|version| version.to_string())
        .unwrap_or_else(|| "-".to_string());
    let template_description = prompt_lab
        .template_snapshot_description
        .as_deref()
        .unwrap_or("-");
    if let Some(run) = prompt_lab.latest_run.as_ref() {
        return format!(
            "model={} in={} out={} cost={} wall={}ms parse_ok={} cache={} template_source={} template_version={} template_desc={}",
            run.resolved_model.as_deref().unwrap_or("-"),
            run.input_tokens
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run.output_tokens
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run.cost_microdollars
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run.wall_ms
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run.parse_ok
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            run.cache_status.as_deref().unwrap_or("-"),
            template_source,
            template_version,
            template_description
        );
    }
    format!(
        "model=- in=- out=- cost=- wall=- parse_ok=- cache=- template_source={} template_version={} template_desc={}",
        template_source,
        template_version,
        template_description
    )
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

enum JobRowPresentation {
    Jobs,
    TriageReview,
    TriageResults,
}

fn job_row_presentation(tab: LeftTab) -> JobRowPresentation {
    match tab {
        LeftTab::Jobs => JobRowPresentation::Jobs,
        LeftTab::TriageReview => JobRowPresentation::TriageReview,
        LeftTab::TriageResults => JobRowPresentation::TriageResults,
        LeftTab::PromptLab => JobRowPresentation::Jobs,
    }
}

fn job_row_check_policy(
    tab: LeftTab,
    is_pre_triage_reviewing: bool,
    job: &JobRowView,
) -> CheckState {
    match tab {
        LeftTab::TriageReview if is_pre_triage_reviewing => match job.filter_status {
            Some(JobFilterStatus::HardExcluded { .. })
            | Some(JobFilterStatus::ManuallyExcluded) => CheckState::Unchecked,
            Some(JobFilterStatus::ReviewNeeded { .. })
            | Some(JobFilterStatus::ManuallyIncluded)
            | Some(JobFilterStatus::AutoIncluded) => CheckState::Checked,
            None => CheckState::Unchecked,
        },
        _ => CheckState::Hidden,
    }
}

fn job_row_style_policy(tab: LeftTab, has_summary: bool) -> Option<StyleId> {
    match tab {
        LeftTab::Jobs => {
            if has_summary {
                None
            } else {
                Some(StyleId::TreeItemDisabled)
            }
        }
        LeftTab::TriageReview | LeftTab::TriageResults | LeftTab::PromptLab => None,
    }
}

fn build_job_tree(view: &AppViewModel) -> Vec<TreeItemDescriptor> {
    let tab = view.left_pane.left_tab;
    let presentation = job_row_presentation(tab);

    // Scope filter: SinceCheckpoint drops jobs not in the checkpoint window.
    let scope_filtered: Vec<&JobRowView> =
        if view.left_pane.job_list_scope == JobListScope::SinceCheckpoint {
            view.jobs.iter().filter(|j| j.is_since_checkpoint).collect()
        } else {
            view.jobs.iter().collect()
        };

    // TriageResults: sort by triage priority (desc), then job_id (asc) for tie-breaking,
    // but only after triage settles. Resorting on every in-flight result changes sibling
    // order, which currently forces a full TreeView repopulation and visible flicker.
    // All other tabs use stable job-id order from the view model.
    let mut sorted_buf: Vec<&JobRowView>;
    let jobs_iter: &[&JobRowView] =
        if matches!(tab, LeftTab::TriageResults) && view.triage_progress.is_none() {
            sorted_buf = scope_filtered;
            sorted_buf.sort_by(|a, b| {
                let p_a = a
                    .triage_annotation
                    .as_ref()
                    .map(|t| t.priority)
                    .unwrap_or(0);
                let p_b = b
                    .triage_annotation
                    .as_ref()
                    .map(|t| t.priority)
                    .unwrap_or(0);
                p_b.cmp(&p_a).then(a.job_id.cmp(&b.job_id))
            });
            &sorted_buf
        } else {
            sorted_buf = scope_filtered;
            &sorted_buf
        };

    jobs_iter
        .iter()
        .map(|job| {
            let mut children = Vec::new();
            if job.link_count > 0 {
                children.push(TreeItemDescriptor {
                    id: links_folder_tree_item_id(job.job_id),
                    text: format!("Links ({})", job.link_count),
                    is_folder: true,
                    state: CheckState::Hidden,
                    children: build_link_children(job),
                    style_override: None,
                });
            }
            let text = match presentation {
                JobRowPresentation::Jobs => format_job_row_legacy(job),
                JobRowPresentation::TriageReview => format_job_row_triage_review(job),
                JobRowPresentation::TriageResults => format_job_row_triage_results(job),
            };
            TreeItemDescriptor {
                id: job_tree_item_id(job.job_id),
                text,
                is_folder: true,
                state: job_row_check_policy(tab, view.is_pre_triage_reviewing, job),
                children,
                style_override: job_row_style_policy(tab, job.has_summary),
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
            state: CheckState::Hidden,
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
            state: CheckState::Hidden,
            children: Vec::new(),
            style_override: None,
        });
    }

    children
}

/// Jobs tab: stable row text — never changes based on triage results.
/// Triage annotation is intentionally omitted; use TriageResults tab for that.
fn format_job_row_legacy(job: &JobRowView) -> String {
    let mut metadata = Vec::new();
    if let Some(filter_status) = filter_status_label(job.filter_status.as_ref()) {
        metadata.push(filter_status.to_string());
    }
    metadata.push(job_status_label(job).to_string());
    if job.has_summary {
        metadata.push(job_source_label(job));
    }
    if let Some(tokens) = job.tokens {
        metadata.push(format!("{} tok", format_compact_tokens(tokens as u64)));
    }
    if let Some(bytes) = job.bytes {
        metadata.push(format_compact_bytes(bytes));
    }

    let primary = job_primary_label(job);
    format!("{primary} — {}", metadata.join(" · "))
}

/// Triage Review tab: shows the URL/title and review status cue.
fn format_job_row_triage_review(job: &JobRowView) -> String {
    let review_status = match &job.filter_status {
        Some(JobFilterStatus::HardExcluded { .. }) => "[AUTO EXCLUDED] ",
        Some(JobFilterStatus::ReviewNeeded { .. }) => "[REVIEW NEEDED] ",
        Some(JobFilterStatus::ManuallyExcluded) => "[EXCLUDED] ",
        Some(JobFilterStatus::ManuallyIncluded) => "[INCLUDED] ",
        Some(JobFilterStatus::AutoIncluded) => "[AUTO INCLUDED] ",
        None => "",
    };
    let label = job_display_label(job);
    format!("{review_status}{label}")
}

/// Triage Results tab: title first, with triage metadata kept compact.
fn format_job_row_triage_results(job: &JobRowView) -> String {
    if let Some(annotation) = &job.triage_annotation {
        let primary = triage_result_primary_label(job);
        let mut metadata = vec![format!(
            "P{} {}",
            annotation.priority,
            title_case_label(&annotation.category)
        )];
        metadata.push(job_source_label(job));
        if let Some(tags) = compact_triage_tag_count(&annotation.tags) {
            metadata.push(tags);
        }
        format!("{primary} — {}", metadata.join(" · "))
    } else {
        let primary = if job.has_summary {
            job_primary_label(job)
        } else {
            compact_url_label(&job.url, 48)
        };
        let mut metadata = vec!["No triage".to_string()];
        if job.has_summary {
            metadata.push(job_source_label(job));
        }
        format!("{primary} — {}", metadata.join(" · "))
    }
}

/// Returns the best short display label for a job (summary title or URL).
fn job_display_label(job: &JobRowView) -> String {
    if job.has_summary {
        format!("{} — {}", job_primary_label(job), job_source_label(job))
    } else {
        job_primary_label(job)
    }
}

fn job_primary_label(job: &JobRowView) -> String {
    job_primary_label_with_limit(job, 64)
}

fn job_primary_label_with_limit(job: &JobRowView, max_chars: usize) -> String {
    if job.has_summary {
        let title = job
            .summary_title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("(summary available)");
        truncate_with_ellipsis(title, max_chars)
    } else {
        compact_url_label(&job.url, max_chars.min(56))
    }
}

fn job_source_label(job: &JobRowView) -> String {
    let domain = domain_from_url(&job.url);
    if domain.is_empty() {
        compact_url_label(&job.url, 32)
    } else {
        truncate_with_ellipsis(&domain, 32)
    }
}

fn triage_result_primary_label(job: &JobRowView) -> String {
    if job.has_summary {
        return job_primary_label_with_limit(job, 58);
    }
    url_slug_label(&job.url)
        .map(|label| title_case_label(&label))
        .map(|label| truncate_with_ellipsis(&label, 58))
        .unwrap_or_else(|| compact_url_label(&job.url, 48))
}

fn filter_status_label(filter_status: Option<&JobFilterStatus>) -> Option<&'static str> {
    match filter_status {
        Some(JobFilterStatus::HardExcluded { .. }) => Some("Auto-excluded"),
        Some(JobFilterStatus::ReviewNeeded { .. }) => Some("Review"),
        Some(JobFilterStatus::ManuallyExcluded) => Some("Excluded"),
        Some(JobFilterStatus::ManuallyIncluded) => Some("Included"),
        Some(JobFilterStatus::AutoIncluded) => Some("Auto"),
        None => None,
    }
}

fn job_status_label(job: &JobRowView) -> &'static str {
    match &job.outcome {
        Some(JobResultKind::Success) => "OK",
        Some(JobResultKind::Failed { .. }) => "ERR",
        None => match job.stage {
            Stage::Queued => "Queued",
            Stage::Downloading => "Fetch",
            Stage::Sanitizing => "Clean",
            Stage::Converting => "Convert",
            Stage::Tokenizing => "Tokens",
            Stage::Writing => "Write",
            Stage::Done => "Done",
        },
    }
}

#[allow(dead_code)]
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

fn compact_url_label(url: &str, max_chars: usize) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return "(untitled source)".to_string();
    }

    let without_scheme = trimmed
        .find("://")
        .map(|pos| &trimmed[pos + 3..])
        .unwrap_or(trimmed);
    let without_query = without_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    let mut segments = without_query
        .split('/')
        .filter(|segment| !segment.is_empty());
    let Some(host) = segments.next() else {
        return truncate_with_ellipsis(trimmed, max_chars);
    };
    let path_segments: Vec<&str> = segments.collect();
    let compact = match path_segments.as_slice() {
        [] => host.to_string(),
        [only] => format!("{host}/{only}"),
        [first, second] => format!("{host}/{first}/{second}"),
        [first, .., last] => format!("{host}/{first}/.../{last}"),
    };
    truncate_with_ellipsis(&compact, max_chars)
}

fn url_slug_label(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .find("://")
        .map(|pos| &trimmed[pos + 3..])
        .unwrap_or(trimmed);
    let without_query = without_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    let slug = without_query
        .rsplit('/')
        .next()
        .unwrap_or(without_query)
        .trim();
    if slug.is_empty() || !slug.contains('-') {
        return None;
    }

    let label = humanize_slug_with_limit(slug, 60);
    if label.is_empty() || label.eq_ignore_ascii_case(slug) && !label.contains(' ') {
        None
    } else {
        Some(label)
    }
}

fn compact_triage_tag_count(tags: &[String]) -> Option<String> {
    if tags.is_empty() {
        return None;
    }
    if tags.len() == 1 {
        Some("1 tag".to_string())
    } else {
        Some(format!("{} tags", tags.len()))
    }
}

fn title_case_label(value: &str) -> String {
    let mut out = Vec::new();
    for word in value.split(['-', '_', ' ']).filter(|word| !word.is_empty()) {
        let mut chars = word.chars();
        let Some(first) = chars.next() else {
            continue;
        };
        let rest: String = chars.collect();
        out.push(format!(
            "{}{}",
            first.to_uppercase(),
            rest.to_ascii_lowercase()
        ));
    }
    if out.is_empty() {
        value.trim().to_string()
    } else {
        out.join(" ")
    }
}

fn humanize_slug_with_limit(value: &str, max_chars: usize) -> String {
    let words: Vec<&str> = value
        .split(['-', '_'])
        .filter(|word| !word.is_empty())
        .collect();
    if words.is_empty() {
        value.to_string()
    } else {
        truncate_with_ellipsis(&words.join(" "), max_chars)
    }
}

fn domain_from_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .find("://")
        .map(|pos| &trimmed[pos + 3..])
        .unwrap_or(trimmed);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    if host.is_empty() {
        trimmed.to_string()
    } else {
        host.to_string()
    }
}

fn truncate_with_ellipsis(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    let char_count = trimmed.chars().count();
    if char_count <= max_chars {
        return trimmed.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let prefix: String = trimmed.chars().take(max_chars - 3).collect();
    format!("{prefix}...")
}

fn format_compact_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;

    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[allow(dead_code)]
fn format_preview_context(header: &PreviewHeaderView) -> PreviewContextView {
    let source_label = if header.domain.is_empty() {
        "(unknown source)".to_string()
    } else {
        header.domain.clone()
    };
    let status_label = match &header.outcome {
        Some(JobResultKind::Failed { reason }) => format!("Failed ({reason})"),
        Some(JobResultKind::Success) => "Done".to_string(),
        None => stage_label(header.stage).to_string(),
    };
    let attention_label = if header.nav_heavy {
        Some("navigation-heavy".to_string())
    } else {
        None
    };

    PreviewContextView {
        source_label,
        status_label,
        attention_label,
    }
}

#[allow(dead_code)]
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

#[allow(dead_code)]
fn shape_for_viewer(text: &str) -> String {
    let text = add_spacing_before_headings(text);
    let text = normalize_bullets(&text);
    let text = strip_bold_markers(&text);
    let text = cap_blank_line_runs(&text);
    truncate_for_viewer(&text)
}

#[allow(dead_code)]
fn add_spacing_before_headings(text: &str) -> String {
    let mut output: Vec<&str> = Vec::new();
    for line in text.lines() {
        let is_heading = line.starts_with('#');
        if is_heading && !output.is_empty() && !output.last().unwrap_or(&"").trim().is_empty() {
            output.push("");
        }
        output.push(line);
    }
    output.join("\n")
}

#[allow(dead_code)]
fn normalize_bullets(text: &str) -> String {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent_len = line.len().saturating_sub(trimmed.len());
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let mut rebuilt = String::new();
            rebuilt.push_str(&line[..indent_len]);
            rebuilt.push_str("• ");
            rebuilt.push_str(rest);
            out.push(rebuilt);
        } else {
            out.push(line.to_string());
        }
    }
    out.join("\n")
}

#[allow(dead_code)]
fn strip_bold_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '*' && matches!(chars.peek(), Some('*')) {
            chars.next();
            continue;
        }
        out.push(ch);
    }
    out
}

#[allow(dead_code)]
fn cap_blank_line_runs(text: &str) -> String {
    let mut out = Vec::new();
    let mut blank_run = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 2 {
                out.push("");
            }
        } else {
            blank_run = 0;
            out.push(line);
        }
    }
    out.join("\n")
}

#[allow(dead_code)]
fn truncate_for_viewer(text: &str) -> String {
    let total_chars = text.chars().count();
    if total_chars <= MAX_VIEWER_CHARS {
        return text.to_string();
    }

    let marker = format!("\r\n{VIEWER_TRUNCATE_MARKER}");
    let marker_chars = marker.chars().count();
    if marker_chars >= MAX_VIEWER_CHARS {
        return marker;
    }
    let keep_chars = MAX_VIEWER_CHARS - marker_chars;
    let cutoff = text
        .char_indices()
        .nth(keep_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    let mut truncated = text[..cutoff].to_string();
    truncated.push_str(&marker);
    truncated
}

fn strip_leading_h1(text: &str) -> &str {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix("# ") {
        let end = rest.find('\n').map_or(rest.len(), |i| i + 1);
        rest[end..].trim_start_matches('\n')
    } else {
        trimmed
    }
}

fn truncate_markdown_for_preview(text: &str) -> (String, bool) {
    let total_chars = text.chars().count();
    if total_chars <= MAX_VIEWER_CHARS {
        return (text.to_string(), false);
    }

    let cutoff = text
        .char_indices()
        .nth(MAX_VIEWER_CHARS)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    (text[..cutoff].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::Stage;
    use harvester_core::{
        LinkDownloadState, LinkRowView, PromptLabRunId, PromptLabRunSummaryView, PromptLabView,
    };
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
            triage_annotation: None,
            has_summary: false,
            summary_title: None,
            filter_status: None,
            has_analysis: false,
            is_since_checkpoint: false,
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
        let mut view = AppViewModel {
            job_count: jobs.len(),
            jobs,
            ..AppViewModel::default()
        };
        view.left_pane.job_list_scope = JobListScope::All;
        view
    }

    fn completed_prompt_lab_view() -> PromptLabView {
        PromptLabView {
            visible: true,
            latest_run: Some(PromptLabRunSummaryView {
                run_id: PromptLabRunId(1),
                stage: PromptLabStage::Summary,
                status_label: "completed",
                output_json: Some("{\"ok\":true}".to_string()),
                failure_reason: None,
                input_tokens: Some(10),
                output_tokens: Some(20),
                cost_microdollars: Some(30),
                wall_ms: Some(40),
                resolved_model: Some(harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI.to_string()),
                parse_ok: Some(true),
                cache_status: Some("miss".to_string()),
            }),
            ..PromptLabView::default()
        }
    }

    #[test]
    fn preview_context_exposes_source_status_and_ignores_analytics_fields() {
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
        let context = format_preview_context(&header);
        assert_eq!(context.source_label, "example.com");
        assert_eq!(context.status_label, "Done");
        assert_eq!(context.attention_label, None);
    }

    #[test]
    fn preview_context_appends_nav_heavy_indicator() {
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
        let context = format_preview_context(&header);
        assert_eq!(context.source_label, "dense.example");
        assert_eq!(context.status_label, "Converting");
        assert_eq!(context.attention_label.as_deref(), Some("navigation-heavy"));
    }

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
        assert_eq!(control_text(&commands, LABEL_PREVIEW_SOURCE), Some(""));
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
        assert_eq!(control_text(&commands, LABEL_PREVIEW_SOURCE), Some(""));
        assert_eq!(control_text(&commands, LABEL_PREVIEW_STATUS), Some(""));
        assert_eq!(control_text(&commands, LABEL_PREVIEW_ATTENTION), Some(""));
    }

    #[test]
    fn header_texts_do_not_reemit_when_unchanged() {
        init_logging();
        let window_id = WindowId::new(3);
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
                        || *control_id == LABEL_PREVIEW_SOURCE
                        || *control_id == LABEL_PREVIEW_STATUS
                        || *control_id == LABEL_PREVIEW_ATTENTION
            )
        }));
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
        assert_eq!(text, &format_job_row_legacy(&view_updated.jobs[0]));
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
            triage_annotation: None,
            has_summary: false,
            summary_title: None,
            filter_status: None,
            has_analysis: false,
            is_since_checkpoint: false,
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
        assert_eq!(folder.state, CheckState::Hidden);
        assert_eq!(folder.children.len(), 2);
        assert_eq!(folder.children[0].id, link_tree_item_id(42, 0));
        assert_eq!(folder.children[0].state, CheckState::Hidden);
        let show_more = &folder.children[1];
        assert_eq!(show_more.id, links_show_more_tree_item_id(42));
        assert_eq!(show_more.text, "(show more… 3 remaining)");
        assert_eq!(show_more.state, CheckState::Hidden);
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
    fn shape_adds_blank_line_before_heading() {
        let shaped = shape_for_viewer("text\n# Heading");
        assert_eq!(shaped, "text\n\n# Heading");
    }

    #[test]
    fn shape_heading_already_preceded_by_blank_not_doubled() {
        let shaped = shape_for_viewer("text\n\n# Heading");
        assert_eq!(shaped, "text\n\n# Heading");
    }

    #[test]
    fn shape_bullet_normalized() {
        let shaped = shape_for_viewer("- item");
        assert_eq!(shaped, "• item");
    }

    #[test]
    fn shape_bold_markers_stripped() {
        let shaped = shape_for_viewer("**term**");
        assert_eq!(shaped, "term");
    }

    #[test]
    fn shape_blank_line_runs_capped() {
        let shaped = shape_for_viewer("a\n\n\n\nb");
        assert_eq!(shaped, "a\n\n\nb");
    }

    #[test]
    fn shape_length_guard_truncates() {
        let source = "x".repeat(MAX_VIEWER_CHARS + 10);
        let shaped = shape_for_viewer(&source);
        assert!(shaped.ends_with(VIEWER_TRUNCATE_MARKER));
        assert_eq!(shaped.chars().count(), MAX_VIEWER_CHARS);
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
    fn job_without_summary_gets_tree_item_disabled_style_override() {
        init_logging();
        let mut job = make_job(1, "https://example.com", Stage::Done, None, None, None);
        job.has_summary = false;
        let view = make_view(vec![job]);
        let items = build_job_tree(&view);
        assert_eq!(items[0].style_override, Some(StyleId::TreeItemDisabled));
    }

    #[test]
    fn job_with_summary_has_no_style_override() {
        init_logging();
        let mut job = make_job(1, "https://example.com", Stage::Done, None, None, None);
        job.has_summary = true;
        job.summary_title = Some("Summary headline".to_string());
        let view = make_view(vec![job]);
        let items = build_job_tree(&view);
        assert_eq!(items[0].style_override, None);
    }

    #[test]
    fn format_job_row_prefers_summary_headline_layout_after_summary() {
        let mut job = make_job(
            7,
            "https://example.com/path?q=1",
            Stage::Done,
            Some(JobResultKind::Success),
            Some(123),
            Some(456),
        );
        job.has_summary = true;
        job.summary_title = Some("Headline from summary".to_string());
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 4,
            category: "security".to_string(),
            tags: vec!["tag1".to_string()],
        });

        let row = format_job_row_legacy(&job);
        // Triage annotation must NOT appear in the Jobs tab — row is stable pre/post triage.
        assert_eq!(
            row,
            "Headline from summary — OK · example.com · 123 tok · 456 B"
        );
        assert!(!row.contains("P4"));
        assert!(!row.contains("[#7]"));
        assert!(!row.contains("https://example.com/path?q=1"));
    }

    #[test]
    fn format_job_row_keeps_legacy_layout_before_summary_exists() {
        let mut job = make_job(
            9,
            "https://example.com/path",
            Stage::Done,
            Some(JobResultKind::Success),
            Some(100),
            Some(200),
        );
        job.has_summary = false;
        job.summary_title = Some("Should not be used yet".to_string());

        let row = format_job_row_legacy(&job);
        assert_eq!(row, "example.com/path — OK · 100 tok · 200 B");
        assert!(!row.contains("https://"));
    }

    #[test]
    fn format_job_row_legacy_compacts_long_url_when_no_summary_exists() {
        let job = make_job(
            10,
            "https://example.com/very-long-section/2026/04/03/story-name?utm_source=test",
            Stage::Downloading,
            None,
            None,
            Some(2_048),
        );

        let row = format_job_row_legacy(&job);
        assert_eq!(
            row,
            "example.com/very-long-section/.../story-name — Fetch · 2.0 KB"
        );
        assert!(!row.contains('?'));
    }

    // ── Per-tab formatter tests ───────────────────────────────────────────────

    #[test]
    fn format_job_row_triage_review_shows_review_needed_prefix() {
        let mut job = make_job(
            1,
            "https://example.com/a",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job.has_summary = true;
        job.summary_title = Some("Article Title".to_string());
        job.filter_status = Some(JobFilterStatus::ReviewNeeded { reasons: vec![] });
        let row = format_job_row_triage_review(&job);
        assert!(
            row.starts_with("[REVIEW NEEDED] "),
            "expected review prefix, got: {row}"
        );
        assert!(row.contains("Article Title"));
    }

    #[test]
    fn format_job_row_triage_review_shows_excluded_prefix() {
        let mut job = make_job(2, "https://example.com/b", Stage::Done, None, None, None);
        job.filter_status = Some(JobFilterStatus::ManuallyExcluded);
        let row = format_job_row_triage_review(&job);
        assert!(
            row.starts_with("[EXCLUDED] "),
            "expected excluded prefix, got: {row}"
        );
    }

    #[test]
    fn format_job_row_triage_results_shows_annotation() {
        let mut job = make_job(
            3,
            "https://example.com/c",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job.has_summary = true;
        job.summary_title = Some("Summary Headline".to_string());
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 2,
            category: "tech".to_string(),
            tags: vec!["ai".to_string(), "ml".to_string()],
        });
        let row = format_job_row_triage_results(&job);
        assert_eq!(row, "Summary Headline — P2 Tech · example.com · 2 tags");
    }

    #[test]
    fn format_job_row_triage_results_no_annotation_shows_placeholder() {
        let mut job = make_job(4, "https://example.com/d", Stage::Done, None, None, None);
        job.triage_annotation = None;
        let row = format_job_row_triage_results(&job);
        assert_eq!(row, "example.com/d — No triage");
    }

    #[test]
    fn format_job_row_triage_results_uses_slug_and_compact_tag_count_without_summary() {
        let mut job = make_job(
            5,
            "https://epochai.substack.com/p/hyperscaler-capex-has-quadrupled",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "business".to_string(),
            tags: vec![
                "capex".to_string(),
                "resource-grab".to_string(),
                "hyperscalers".to_string(),
                "sovereign-ai".to_string(),
            ],
        });

        let row = format_job_row_triage_results(&job);
        assert_eq!(
            row,
            "Hyperscaler Capex Has Quadrupled — P5 Business · epochai.substack.com · 4 tags"
        );
    }

    // ── Scope filter tests ────────────────────────────────────────────────────

    #[test]
    fn scope_since_checkpoint_filters_jobs_in_tree() {
        init_logging();
        let window_id = WindowId::new(60);
        let mut tree_state = TreeRenderState::new();

        let mut job_in = make_job(
            1,
            "https://a.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job_in.is_since_checkpoint = true;
        job_in.has_summary = true;
        job_in.summary_title = Some("In Scope".to_string());

        let mut job_out = make_job(
            2,
            "https://b.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job_out.is_since_checkpoint = false;
        job_out.has_summary = true;
        job_out.summary_title = Some("Out of Scope".to_string());

        let mut view = make_view(vec![job_in, job_out]);
        view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;

        let cmds = render(window_id, &view, &mut tree_state);
        let populated = cmds
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::PopulateTreeView { items, .. } => Some(items),
                _ => None,
            })
            .expect("PopulateTreeView emitted");

        assert_eq!(populated.len(), 1, "only the in-scope job should appear");
        assert!(
            populated[0].text.contains("In Scope"),
            "wrong item: {}",
            populated[0].text
        );
    }

    #[test]
    fn scope_all_shows_all_jobs() {
        init_logging();
        let window_id = WindowId::new(61);
        let mut tree_state = TreeRenderState::new();

        let mut job1 = make_job(
            1,
            "https://a.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job1.is_since_checkpoint = true;
        job1.has_summary = true;
        job1.summary_title = Some("Job A".to_string());

        let mut job2 = make_job(
            2,
            "https://b.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        job2.is_since_checkpoint = false;
        job2.has_summary = true;
        job2.summary_title = Some("Job B".to_string());

        let mut view = make_view(vec![job1, job2]);
        view.left_pane.job_list_scope = JobListScope::All;

        let cmds = render(window_id, &view, &mut tree_state);
        let populated = cmds
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::PopulateTreeView { items, .. } => Some(items),
                _ => None,
            })
            .expect("PopulateTreeView emitted");

        assert_eq!(populated.len(), 2, "all jobs should appear with All scope");
    }

    #[test]
    fn triage_results_tab_sorts_by_priority_descending() {
        // Jobs arrive in job_id order (low priority first), but TriageResults should
        // show highest priority first once triage has settled.
        init_logging();
        let window_id = WindowId::new(62);
        let mut tree_state = TreeRenderState::new();

        let mut low = make_job(
            1,
            "https://low.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        low.has_summary = true;
        low.summary_title = Some("Low Priority".to_string());
        low.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 2,
            category: "misc".to_string(),
            tags: vec![],
        });

        let mut high = make_job(
            2,
            "https://high.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        high.has_summary = true;
        high.summary_title = Some("High Priority".to_string());
        high.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "tech".to_string(),
            tags: vec![],
        });

        // View model has jobs in job_id order (low=1, high=2) — stable, not triage-sorted.
        let mut view = make_view(vec![low, high]);
        view.left_pane.left_tab = LeftTab::TriageResults;

        let cmds = render(window_id, &view, &mut tree_state);
        let populated = cmds
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::PopulateTreeView { items, .. } => Some(items),
                _ => None,
            })
            .expect("PopulateTreeView emitted");

        assert_eq!(populated.len(), 2);
        // TriageResults render must reorder: highest priority (P5) first.
        assert!(
            populated[0].text.contains("P5"),
            "first item should be P5, got: {}",
            populated[0].text
        );
        assert!(
            populated[1].text.contains("P2"),
            "second item should be P2, got: {}",
            populated[1].text
        );
    }

    #[test]
    fn triage_results_tab_keeps_stable_order_while_triage_in_progress() {
        init_logging();
        let window_id = WindowId::new(64);
        let mut tree_state = TreeRenderState::new();

        let low = make_job(
            1,
            "https://low.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );

        let mut high = make_job(
            2,
            "https://high.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        high.has_summary = true;
        high.summary_title = Some("High Priority".to_string());
        high.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "tech".to_string(),
            tags: vec![],
        });

        let mut view = make_view(vec![low, high]);
        view.left_pane.left_tab = LeftTab::TriageResults;
        view.triage_progress = Some("Triaging 1/2 articles...".to_string());

        let cmds = render(window_id, &view, &mut tree_state);
        let populated = cmds
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::PopulateTreeView { items, .. } => Some(items),
                _ => None,
            })
            .expect("PopulateTreeView emitted");

        assert_eq!(populated.len(), 2);
        assert!(
            populated[0].text.starts_with("low.com"),
            "first should stay job 1 while triage is running, got: {}",
            populated[0].text
        );
        assert!(
            populated[0].text.contains("No triage"),
            "first should still show no-triage state, got: {}",
            populated[0].text
        );
        assert!(
            populated[1].text.contains("P5"),
            "second should stay job 2 while triage is running, got: {}",
            populated[1].text
        );
    }

    #[test]
    fn triage_results_in_progress_updates_rows_without_repopulating_tree() {
        init_logging();
        let window_id = WindowId::new(65);
        let mut tree_state = TreeRenderState::new();

        let low_initial = make_job(
            1,
            "https://low.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );

        let mut high_initial = make_job(
            2,
            "https://high.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        high_initial.has_summary = true;
        high_initial.summary_title = Some("High Priority".to_string());
        high_initial.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "tech".to_string(),
            tags: vec![],
        });

        let mut initial_view = make_view(vec![low_initial, high_initial]);
        initial_view.left_pane.left_tab = LeftTab::TriageResults;
        initial_view.triage_progress = Some("Triaging 1/2 articles...".to_string());
        let _ = render(window_id, &initial_view, &mut tree_state);

        let mut low_updated = make_job(
            1,
            "https://low.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        low_updated.has_summary = true;
        low_updated.summary_title = Some("Now Highest".to_string());
        low_updated.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 6,
            category: "finance".to_string(),
            tags: vec![],
        });

        let mut high_updated = make_job(
            2,
            "https://high.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        high_updated.has_summary = true;
        high_updated.summary_title = Some("High Priority".to_string());
        high_updated.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "tech".to_string(),
            tags: vec![],
        });

        let mut updated_view = make_view(vec![low_updated, high_updated]);
        updated_view.left_pane.left_tab = LeftTab::TriageResults;
        updated_view.triage_progress = Some("Triaging 2/2 articles...".to_string());

        let cmds = render(window_id, &updated_view, &mut tree_state);

        assert!(
            !cmds
                .iter()
                .any(|cmd| matches!(cmd, PlatformCommand::PopulateTreeView { .. })),
            "in-flight triage updates should not repopulate the whole tree"
        );
        assert!(
            cmds.iter().any(|cmd| matches!(
                cmd,
                PlatformCommand::UpdateTreeItemText { item_id, text, .. }
                    if *item_id == job_tree_item_id(1)
                        && text.contains("Now Highest")
                        && text.contains("P6 Finance")
            )),
            "updated triage row should refresh in place"
        );
    }

    #[test]
    fn jobs_tab_stable_order_unaffected_by_triage_priority() {
        // Jobs tab must show items in job_id order, not reordered by triage priority.
        init_logging();
        let window_id = WindowId::new(63);
        let mut tree_state = TreeRenderState::new();

        let mut low = make_job(
            1,
            "https://low.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        low.has_summary = true;
        low.summary_title = Some("Low Priority".to_string());
        low.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 2,
            category: "misc".to_string(),
            tags: vec![],
        });

        let mut high = make_job(
            2,
            "https://high.com/",
            Stage::Done,
            Some(JobResultKind::Success),
            None,
            None,
        );
        high.has_summary = true;
        high.summary_title = Some("High Priority".to_string());
        high.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "tech".to_string(),
            tags: vec![],
        });

        let mut view = make_view(vec![low, high]);
        view.left_pane.left_tab = LeftTab::Jobs; // stable order

        let cmds = render(window_id, &view, &mut tree_state);
        let populated = cmds
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::PopulateTreeView { items, .. } => Some(items),
                _ => None,
            })
            .expect("PopulateTreeView emitted");

        assert_eq!(populated.len(), 2);
        // Jobs tab must preserve insertion order (job_id 1 first, then job_id 2).
        assert!(
            populated[0].text.contains("Low Priority"),
            "first should be Low Priority (job 1), got: {}",
            populated[0].text
        );
        assert!(
            populated[1].text.contains("High Priority"),
            "second should be High Priority (job 2), got: {}",
            populated[1].text
        );
    }

    // ── Per-tab style policy tests ────────────────────────────────────────────

    #[test]
    fn jobs_tab_applies_disabled_style_when_no_summary() {
        assert_eq!(
            job_row_style_policy(LeftTab::Jobs, false),
            Some(StyleId::TreeItemDisabled)
        );
        assert_eq!(job_row_style_policy(LeftTab::Jobs, true), None);
    }

    #[test]
    fn triage_tabs_never_apply_disabled_style() {
        assert_eq!(job_row_style_policy(LeftTab::TriageReview, false), None);
        assert_eq!(job_row_style_policy(LeftTab::TriageReview, true), None);
        assert_eq!(job_row_style_policy(LeftTab::TriageResults, false), None);
        assert_eq!(job_row_style_policy(LeftTab::TriageResults, true), None);
    }

    // ── Per-tab check policy tests ────────────────────────────────────────────

    #[test]
    fn check_policy_only_active_in_triage_review_during_interactive_phase() {
        let mut job = make_job(1, "https://x.com/", Stage::Done, None, None, None);
        job.filter_status = Some(JobFilterStatus::AutoIncluded);

        // TriageReview + interactive → Checked
        assert_eq!(
            job_row_check_policy(LeftTab::TriageReview, true, &job),
            CheckState::Checked
        );
        // TriageReview + not interactive → Unchecked
        assert_eq!(
            job_row_check_policy(LeftTab::TriageReview, false, &job),
            CheckState::Hidden
        );
        // Jobs tab → always Hidden
        assert_eq!(
            job_row_check_policy(LeftTab::Jobs, true, &job),
            CheckState::Hidden
        );
        // TriageResults → always Hidden
        assert_eq!(
            job_row_check_policy(LeftTab::TriageResults, true, &job),
            CheckState::Hidden
        );
    }

    #[test]
    fn triage_results_use_hidden_state_lane_for_markers() {
        let mut job = make_job(1, "https://example.com", Stage::Done, None, None, None);
        job.has_summary = true;
        job.summary_title = Some("Example headline".to_string());
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 5,
            category: "Business".to_string(),
            tags: vec!["capex".to_string()],
        });
        job.links = vec![make_link_row(
            0,
            "Related link",
            LinkDownloadState::Downloaded {
                path: "cache/article.html".into(),
            },
        )];
        job.link_count = job.links.len();
        let mut view = make_view(vec![job]);
        view.left_pane.left_tab = LeftTab::TriageResults;

        let items = build_job_tree(&view);
        assert_eq!(items[0].state, CheckState::Hidden);
        assert_eq!(items[0].children[0].state, CheckState::Hidden);
    }

    #[test]
    fn render_enables_open_browser_when_selected_url_is_some() {
        init_logging();
        let mut view = make_view(vec![]);
        view.selected_url = Some("https://example.com".to_string());
        let mut tree_state = TreeRenderState::new();
        let window_id = WindowId::new(1);
        let cmds = render(window_id, &view, &mut tree_state);
        let enabled = cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
                if *control_id == BUTTON_OPEN_BROWSER
            )
        });
        assert!(enabled, "BUTTON_OPEN_BROWSER should be enabled");
    }

    #[test]
    fn render_disables_open_browser_when_selected_url_is_none() {
        init_logging();
        let view = make_view(vec![]);
        let mut tree_state = TreeRenderState::new();
        let window_id = WindowId::new(1);
        let cmds = render(window_id, &view, &mut tree_state);
        let disabled = cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BUTTON_OPEN_BROWSER
            )
        });
        assert!(disabled, "BUTTON_OPEN_BROWSER should be disabled");
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
    fn render_is_idempotent_for_open_browser_state() {
        init_logging();
        let view = make_view(vec![]);
        let mut tree_state = TreeRenderState::new();
        let window_id = WindowId::new(1);
        // First render sets initial state
        render(window_id, &view, &mut tree_state);
        // Second render should not emit SetControlEnabled for BUTTON_OPEN_BROWSER
        let cmds = render(window_id, &view, &mut tree_state);
        let changed = cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, .. }
                if *control_id == BUTTON_OPEN_BROWSER
            )
        });
        assert!(
            !changed,
            "BUTTON_OPEN_BROWSER state should not change on second render"
        );
    }

    #[test]
    fn run_button_disabled_when_can_run_false() {
        let window_id = WindowId::new(10);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.can_run = false;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BTN_PROMPT_LAB_RUN
            )
        }));
    }

    #[test]
    fn template_apply_button_disabled_when_validation_errors_exist() {
        let window_id = WindowId::new(15);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.template_editor_open = true;
        view.left_pane.prompt_lab.template_dirty = true;
        view.left_pane.prompt_lab.template_validation_errors = vec!["unknown var".to_string()];
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BTN_PROMPT_LAB_TEMPLATE_APPLY
            )
        }));
    }

    #[test]
    fn template_save_button_disabled_when_not_applied() {
        let window_id = WindowId::new(16);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.template_editor_open = true;
        view.left_pane.prompt_lab.template_applied = false;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BTN_PROMPT_LAB_TEMPLATE_SAVE
            )
        }));
    }

    #[test]
    fn template_errors_are_rendered_in_template_status_label() {
        let window_id = WindowId::new(17);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.template_editor_open = true;
        view.left_pane.prompt_lab.template_validation_errors = vec![
            "first validation problem".to_string(),
            "second validation problem".to_string(),
        ];
        let cmds = render(window_id, &view, &mut tree_state);
        let status = control_text(&cmds, LABEL_PROMPT_LAB_TEMPLATE_STATUS)
            .expect("template status text rendered");
        assert!(status.contains("first validation problem"));
        assert!(status.contains("second validation problem"));
    }

    #[test]
    fn compare_start_disabled_by_default() {
        let window_id = WindowId::new(18);
        let mut tree_state = TreeRenderState::new();
        let view = make_view(vec![]);
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BTN_COMPARE_START
            )
        }));
    }

    #[test]
    fn compare_start_enabled_when_two_draft_candidates() {
        let window_id = WindowId::new(19);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.draft_candidates = vec![
            harvester_core::PromptLabCompareCandidateView {
                candidate_id: 1,
                label: "c1".to_string(),
                stage_label: "Triage".to_string(),
                model_label: "default".to_string(),
                prompt_version_label: "active".to_string(),
                has_context_override: false,
                has_template_override: false,
            },
            harvester_core::PromptLabCompareCandidateView {
                candidate_id: 2,
                label: "c2".to_string(),
                stage_label: "Triage".to_string(),
                model_label: "default".to_string(),
                prompt_version_label: "active".to_string(),
                has_context_override: false,
                has_template_override: false,
            },
        ];
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
                if *control_id == BTN_COMPARE_START
            )
        }));
    }

    #[test]
    fn prompt_lab_section_checkbox_states_reflect_view_state() {
        let window_id = WindowId::new(33);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.compare_section_open = true;
        view.left_pane.prompt_lab.context_section_open = false;
        view.left_pane.prompt_lab.template_section_open = true;
        view.left_pane.prompt_lab.run_details_section_open = true;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetCheckBoxChecked { control_id, checked: true, .. }
                if *control_id == CHK_PROMPT_LAB_SECTION_COMPARE
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetCheckBoxChecked { control_id, checked: false, .. }
                if *control_id == CHK_PROMPT_LAB_SECTION_CONTEXT
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetCheckBoxChecked { control_id, checked: true, .. }
                if *control_id == CHK_PROMPT_LAB_SECTION_RUN_DETAILS
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
    fn token_progress_uses_since_checkpoint_scope_total_when_enabled() {
        let window_id = WindowId::new(41);
        let mut tree_state = TreeRenderState::new();
        let mut since_job = make_job(
            1,
            "https://since.example",
            Stage::Done,
            Some(JobResultKind::Success),
            Some(50),
            None,
        );
        since_job.is_since_checkpoint = true;
        let all_time_job = make_job(
            2,
            "https://all.example",
            Stage::Done,
            Some(JobResultKind::Success),
            Some(150),
            None,
        );
        let mut view = make_view(vec![since_job, all_time_job]);
        view.total_tokens = 200;
        view.token_limit = 200_000;
        view.left_pane.job_list_scope = JobListScope::SinceCheckpoint;

        let cmds = render(window_id, &view, &mut tree_state);

        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlText { control_id, text, .. }
                if *control_id == LABEL_TOKEN_PROGRESS
                    && text == "50 / 200K"
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetProgressBarPosition { control_id, position, .. }
                if *control_id == PROGRESS_TOKENS && *position == 50
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::ApplyStyleToControl { control_id, style_id, .. }
                if *control_id == PROGRESS_TOKENS && *style_id == StyleId::StatusMeter
            )
        }));
    }

    #[test]
    fn token_progress_stays_muted_below_limit_even_when_high() {
        let window_id = WindowId::new(42);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(Vec::new());
        view.total_tokens = 97_002;
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
        view.total_tokens = 100_000;
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
            filter_status: None,
            has_analysis: false,
            is_since_checkpoint: true,
        }]);
        let mut empty_view = empty_view;
        empty_view.left_pane.left_tab = LeftTab::TriageResults;
        let empty_cmds = render(window_id, &empty_view, &mut tree_state);
        let empty_meta =
            control_text(&empty_cmds, LABEL_JOBS_HEADER_META).expect("empty triage meta rendered");
        assert_eq!(empty_meta, "no triage results yet");
        assert!(control_text(&empty_cmds, LABEL_JOBS_HEADER_TITLE).is_none());

        let mut populated_view = empty_view.clone();
        populated_view.jobs[0].triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: 1,
            category: "keep".to_string(),
            tags: vec![],
        });
        let populated_cmds = render(window_id, &populated_view, &mut tree_state);
        let populated_meta = control_text(&populated_cmds, LABEL_JOBS_HEADER_META)
            .expect("populated triage meta rendered");
        assert_eq!(populated_meta, "1 with triage");
        assert!(control_text(&populated_cmds, LABEL_JOBS_HEADER_TITLE).is_none());
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
    fn triage_results_meta_uses_ai_unavailable_copy_when_blocked() {
        let window_id = WindowId::new(42);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.left_tab = LeftTab::TriageResults;
        view.ai_unavailable_message =
            Some("AI features unavailable: OPENAI_API_KEY is not set".to_string());

        let cmds = render(window_id, &view, &mut tree_state);
        let meta = control_text(&cmds, LABEL_JOBS_HEADER_META).expect("triage meta rendered");
        assert_eq!(meta, "no triage results yet · AI unavailable");
        assert!(control_text(&cmds, LABEL_JOBS_HEADER_TITLE).is_none());
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
            filter_status: None,
            has_analysis: true,
            is_since_checkpoint: true,
        }]);
        view.left_pane.left_tab = LeftTab::TriageResults;
        view.ai_unavailable_message =
            Some("AI features unavailable: OPENAI_API_KEY is not set".to_string());

        let cmds = render(window_id, &view, &mut tree_state);
        let meta = control_text(&cmds, LABEL_JOBS_HEADER_META).expect("triage meta rendered");
        assert_eq!(meta, "1 with triage · AI unavailable");
        assert!(control_text(&cmds, LABEL_JOBS_HEADER_TITLE).is_none());
    }

    #[test]
    fn prompt_lab_mode_and_stage_radio_states_reflect_view_state() {
        let window_id = WindowId::new(38);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.advanced_mode = true;
        view.left_pane.prompt_lab.selected_stage = PromptLabStage::Summary;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetRadioButtonChecked { control_id, checked: true, .. }
                if *control_id == BTN_PROMPT_LAB_MODE_ADVANCED
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetRadioButtonChecked { control_id, checked: false, .. }
                if *control_id == BTN_PROMPT_LAB_MODE_BASIC
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetRadioButtonChecked { control_id, checked: true, .. }
                if *control_id == BTN_STAGE_SUMMARY
            )
        }));
    }

    #[test]
    fn prompt_lab_tab_advanced_layout_does_not_depend_on_legacy_visible_flag() {
        let window_id = WindowId::new(39);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.left_tab = LeftTab::PromptLab;
        view.left_pane.prompt_lab.visible = false;
        view.left_pane.prompt_lab.advanced_mode = true;

        let cmds = render(window_id, &view, &mut tree_state);
        let rules = cmds
            .iter()
            .find_map(|cmd| match cmd {
                PlatformCommand::DefineLayout { rules, .. } => Some(rules),
                _ => None,
            })
            .expect("DefineLayout emitted");

        let compare_header_size = rules
            .iter()
            .find(|rule| rule.control_id == PANEL_PROMPT_LAB_COMPARE_HEADER_ROW)
            .and_then(|rule| rule.fixed_size)
            .expect("compare header fixed size");
        assert_ne!(compare_header_size, 0);
    }

    #[test]
    fn prompt_lab_model_selector_defaults_to_index_zero_on_first_render() {
        let window_id = WindowId::new(34);
        let mut tree_state = TreeRenderState::new();
        let view = make_view(vec![]);

        let cmds = render(window_id, &view, &mut tree_state);

        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetComboBoxSelection {
                    control_id,
                    selected_index: Some(0),
                    ..
                } if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR
            )
        }));
    }

    #[test]
    fn prompt_lab_model_selector_emits_catalog_items() {
        let window_id = WindowId::new(35);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.model_catalog = vec![
            ModelId::new(
                harvester_engine::llm::ProviderKind::OpenAi,
                harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI,
            ),
            ModelId::new(harvester_engine::llm::ProviderKind::OpenAi, "o3-mini"),
        ];

        let cmds = render(window_id, &view, &mut tree_state);

        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetComboBoxItems {
                    control_id,
                    items,
                    ..
                } if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR
                    && items == &vec![
                        "Default".to_string(),
                        harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI.to_string(),
                        "o3-mini".to_string(),
                    ]
            )
        }));
    }

    #[test]
    fn prompt_lab_model_selector_replays_on_hidden_to_visible_transition() {
        let window_id = WindowId::new(36);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.left_tab = LeftTab::PromptLab;
        view.left_pane.prompt_lab.model_catalog = vec![ModelId::new(
            harvester_engine::llm::ProviderKind::OpenAi,
            harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI,
        )];

        let initial_cmds = render(window_id, &view, &mut tree_state);
        assert!(initial_cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxItems { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));
        assert!(initial_cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxSelection { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));

        view.left_pane.left_tab = LeftTab::Jobs;
        let _ = render(window_id, &view, &mut tree_state);

        view.left_pane.left_tab = LeftTab::PromptLab;
        let reopen_cmds = render(window_id, &view, &mut tree_state);
        assert!(reopen_cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxItems { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));
        assert!(reopen_cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxSelection { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));
    }

    #[test]
    fn prompt_lab_model_selector_unchanged_visible_state_is_idempotent() {
        let window_id = WindowId::new(37);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.visible = true;
        view.left_pane.prompt_lab.model_catalog = vec![ModelId::new(
            harvester_engine::llm::ProviderKind::OpenAi,
            harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI,
        )];

        let _ = render(window_id, &view, &mut tree_state);
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(!cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxItems { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));
        assert!(!cmds.iter().any(|cmd| {
            matches!(cmd, PlatformCommand::SetComboBoxSelection { control_id, .. }
                if *control_id == COMBO_PROMPT_LAB_MODEL_SELECTOR)
        }));
    }

    #[test]
    fn resolve_button_enabled_when_not_pending() {
        let window_id = WindowId::new(12);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab.resolve_pending = true;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BTN_PROMPT_LAB_RESOLVE
            )
        }));

        view.left_pane.prompt_lab.resolve_pending = false;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
                if *control_id == BTN_PROMPT_LAB_RESOLVE
            )
        }));
    }

    /// The Prompt Lab now lives in its own tab; a completed run does NOT override VIEWER_PREVIEW.
    #[test]
    fn prompt_lab_run_does_not_override_summary_viewer() {
        let window_id = WindowId::new(13);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab = completed_prompt_lab_view();
        let cmds = render(window_id, &view, &mut tree_state);
        // The Prompt Lab output JSON should NOT appear in VIEWER_PREVIEW.
        assert!(!cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetRichEditContent { control_id, rtf_text, .. }
                if *control_id == VIEWER_PREVIEW && rtf_text.contains("ok")
            )
        }));
        // LABEL_PREVIEW_HEADER should not say "Prompt Lab".
        assert!(!cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlText { control_id, text, .. }
                if *control_id == LABEL_PREVIEW_HEADER && text.contains("Prompt Lab")
            )
        }));
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
            tree_state.prev_preview_text.as_deref(),
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

    #[test]
    fn render_idempotent_on_unchanged_prompt_lab_view() {
        let window_id = WindowId::new(14);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.left_pane.prompt_lab = completed_prompt_lab_view();
        let _ = render(window_id, &view, &mut tree_state);
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(!cmds
            .iter()
            .any(|cmd| matches!(cmd, PlatformCommand::SetRichEditContent { .. })));
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

    #[test]
    fn trends_chart_data_emits_set_chart_data() {
        use harvester_core::{CategoryTrendView, EntityLineView, TrendCategory, TrendsTabView};
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let mut view = AppViewModel::default();
        view.right_pane.trends = TrendsTabView {
            is_loading: false,
            active_category: TrendCategory::Companies,
            category_data: Some(CategoryTrendView {
                weeks: vec!["W1".to_string(), "W2".to_string(), "W3".to_string()],
                lines: vec![
                    EntityLineView {
                        label: "Acme".to_string(),
                        weekly_counts: vec![1, 2, 3],
                        total_count: 6,
                    },
                    EntityLineView {
                        label: "Beta".to_string(),
                        weekly_counts: vec![3, 2, 1],
                        total_count: 6,
                    },
                ],
                total_entity_count: 2,
            }),
        };
        let cmds = render(window_id, &view, &mut state);
        let chart_cmd = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        });
        assert!(
            chart_cmd.is_some(),
            "SetChartData not emitted for CHART_TRENDS"
        );
        if let Some(PlatformCommand::SetChartData { data, .. }) = chart_cmd {
            assert_eq!(data.lines.len(), 2);
            assert_eq!(data.lines[0].label, "Acme");
            assert_eq!(data.week_labels, vec!["W1", "W2", "W3"]);
            assert!(!data.is_loading);
        }
    }

    #[test]
    fn trends_chart_loading_state_emits_empty_packet() {
        use harvester_core::TrendsTabView;
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let mut view = AppViewModel::default();
        view.right_pane.trends = TrendsTabView {
            is_loading: true,
            ..Default::default()
        };
        let cmds = render(window_id, &view, &mut state);
        let chart_cmd = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        });
        assert!(
            chart_cmd.is_some(),
            "SetChartData not emitted during loading"
        );
        if let Some(PlatformCommand::SetChartData { data, .. }) = chart_cmd {
            assert!(data.is_loading);
            assert!(data.lines.is_empty());
        }
    }

    #[test]
    fn trends_chart_data_truncates_to_five_lines() {
        use harvester_core::{CategoryTrendView, EntityLineView, TrendCategory, TrendsTabView};
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let mut view = AppViewModel::default();
        let lines: Vec<EntityLineView> = (0..10)
            .map(|i| EntityLineView {
                label: format!("Entity{i}"),
                weekly_counts: vec![i as u32, i as u32 + 1],
                total_count: (2 * i) as u32,
            })
            .collect();
        view.right_pane.trends = TrendsTabView {
            is_loading: false,
            active_category: TrendCategory::Companies,
            category_data: Some(CategoryTrendView {
                weeks: vec!["W1".to_string(), "W2".to_string()],
                lines,
                total_entity_count: 10,
            }),
        };
        let cmds = render(window_id, &view, &mut state);
        let chart_cmd = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        });
        assert!(chart_cmd.is_some(), "SetChartData not emitted");
        if let Some(PlatformCommand::SetChartData { data, .. }) = chart_cmd {
            assert_eq!(data.lines.len(), 5, "expected at most 5 lines from take(5)");
        }
    }

    fn make_five_line_trends_view() -> AppViewModel {
        use harvester_core::{CategoryTrendView, EntityLineView, TrendCategory, TrendsTabView};
        let mut view = AppViewModel::default();
        let lines: Vec<EntityLineView> = (0..5)
            .map(|i| EntityLineView {
                label: format!("Entity{i}"),
                weekly_counts: vec![i as u32, i as u32 + 1],
                total_count: (2 * i) as u32,
            })
            .collect();
        view.right_pane.trends = TrendsTabView {
            is_loading: false,
            active_category: TrendCategory::Companies,
            category_data: Some(CategoryTrendView {
                weeks: vec!["W1".to_string(), "W2".to_string()],
                lines,
                total_entity_count: 5,
            }),
        };
        view
    }

    #[test]
    fn trends_top_two_lines_are_primary_emphasis() {
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let view = make_five_line_trends_view();
        let cmds = render(window_id, &view, &mut state);
        if let Some(PlatformCommand::SetChartData { data, .. }) = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        }) {
            assert!(
                matches!(data.lines[0].emphasis, ChartLineEmphasis::Primary),
                "line 0 should be Primary"
            );
            assert!(
                matches!(data.lines[1].emphasis, ChartLineEmphasis::Primary),
                "line 1 should be Primary"
            );
        } else {
            panic!("SetChartData not emitted");
        }
    }

    #[test]
    fn trends_lines_2_to_4_are_secondary_emphasis() {
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let view = make_five_line_trends_view();
        let cmds = render(window_id, &view, &mut state);
        if let Some(PlatformCommand::SetChartData { data, .. }) = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        }) {
            for i in 2..5 {
                assert!(
                    matches!(data.lines[i].emphasis, ChartLineEmphasis::Secondary),
                    "line {i} should be Secondary"
                );
            }
        } else {
            panic!("SetChartData not emitted");
        }
    }

    #[test]
    fn trends_all_lines_have_end_label() {
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let view = make_five_line_trends_view();
        let cmds = render(window_id, &view, &mut state);
        if let Some(PlatformCommand::SetChartData { data, .. }) = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        }) {
            for (i, line) in data.lines.iter().enumerate() {
                assert!(
                    line.end_label.is_some(),
                    "line {i} should have end_label set"
                );
            }
        } else {
            panic!("SetChartData not emitted");
        }
    }

    #[test]
    fn trends_show_end_labels_is_true() {
        let window_id = WindowId::new(99);
        let mut state = TreeRenderState::default();
        let view = make_five_line_trends_view();
        let cmds = render(window_id, &view, &mut state);
        if let Some(PlatformCommand::SetChartData { data, .. }) = cmds.iter().find(|c| {
            matches!(c, PlatformCommand::SetChartData { control_id, .. } if *control_id == CHART_TRENDS)
        }) {
            assert!(data.show_end_labels, "show_end_labels should be true");
        } else {
            panic!("SetChartData not emitted");
        }
    }
}
