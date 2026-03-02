use commanductui::types::{TreeItemDescriptor, TreeItemId};
use commanductui::{
    ChartDataPacket, ChartLineData, CheckState, MessageSeverity, PlatformCommand, StyleId, WindowId,
};
use engine_logging::{engine_debug, engine_info, engine_warn};
use harvester_core::{
    AppTab, AppViewModel, JobFilterStatus, JobListScope, JobResultKind, JobRowView, LeftTab,
    LinkDownloadState, LlmModelUsageView, PreviewHeaderView, PromptLabStage, SessionState, Stage,
    TrendsTabView, DEFAULT_JOBS_PANEL_WIDTH,
};
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
    prev_status_text: Option<String>,
    prev_progress_text: Option<String>,
    prev_preview_text: Option<String>,
    prev_header_text: Option<String>,
    prev_stop_enabled: Option<bool>,
    prev_briefing_enabled: Option<bool>,
    prev_triage_enabled: Option<bool>,
    prev_poll_enabled: Option<bool>,
    prev_briefing_progress: Option<String>,
    prev_triage_progress: Option<String>,
    prev_progress_range: Option<(u32, u32)>,
    prev_progress_pos: Option<u32>,
    prev_open_browser_enabled: Option<bool>,
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
            prev_status_text: None,
            prev_progress_text: None,
            prev_preview_text: None,
            prev_header_text: None,
            prev_stop_enabled: None,
            prev_briefing_enabled: None,
            prev_triage_enabled: None,
            prev_poll_enabled: None,
            prev_briefing_progress: None,
            prev_triage_progress: None,
            prev_progress_range: None,
            prev_progress_pos: None,
            prev_open_browser_enabled: None,
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
    render_layout_section(window_id, view, tree_state, &mut cmds);
    render_tab_bar_section(window_id, view, tree_state, &mut cmds);
    render_left_tab_bar_section(window_id, view, tree_state, &mut cmds);
    render_status_section(window_id, view, tree_state, &mut cmds);
    render_token_progress_section(window_id, view, tree_state, &mut cmds);
    render_main_controls_section(window_id, view, tree_state, &mut cmds);
    render_prompt_lab_section(window_id, view, tree_state, &mut cmds);

    let job_items = build_job_tree(view);
    append_tree_commands(window_id, job_items, tree_state, &mut cmds);

    render_preview_section(window_id, view, tree_state, &mut cmds);

    cmds
}

/// Converts a `TrendsTabView` into a `ChartDataPacket` for the chart control.
/// Uses a fixed 10-color VS Code dark-theme palette, assigned by entity index.
fn build_chart_data(trends: &TrendsTabView) -> ChartDataPacket {
    // COLORREF palette (0x00BBGGRR), up to 10 entity lines.
    const COLORS: [u32; 10] = [
        0x00B0C94E, // #4EC9B0 teal
        0x007891CE, // #CE9178 salmon
        0x00FEDC9C, // #9CDCFE light blue
        0x00AADCDC, // #DCDCAA yellow
        0x00C086C5, // #C586C0 purple
        0x004747F4, // #F44747 red
        0x007DBAD7, // #D7BA7D gold
        0x0055996A, // #6A9955 green
        0x00D69C56, // #569CD6 blue
        0x00A8CEB5, // #B5CEA8 light green
    ];

    if trends.is_loading {
        return ChartDataPacket {
            lines: vec![],
            week_labels: vec![],
            is_loading: true,
        };
    }
    let Some(cat_data) = &trends.category_data else {
        return ChartDataPacket {
            lines: vec![],
            week_labels: vec![],
            is_loading: false,
        };
    };
    let lines = cat_data
        .lines
        .iter()
        .take(10)
        .enumerate()
        .map(|(i, el)| ChartLineData {
            label: el.label.clone(),
            weekly_counts: el.weekly_counts.clone(),
            color: COLORS[i],
        })
        .collect();
    ChartDataPacket {
        lines,
        week_labels: cat_data.weeks.clone(),
        is_loading: false,
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

fn render_layout_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let prompt_lab_tab_visible = view.left_pane.left_tab == LeftTab::PromptLab;
    let layout_changed = view.left_panel_width != tree_state.prev_left_panel_width
        || view.input_panel_visible != tree_state.prev_input_panel_visible
        || view.right_pane.active_tab != tree_state.prev_active_tab
        || view.left_pane.left_tab != tree_state.prev_left_tab
        || prompt_lab_tab_visible != tree_state.prev_prompt_lab_visible
        || view.left_pane.prompt_lab.advanced_mode != tree_state.prev_prompt_lab_advanced_mode
        || view.left_pane.prompt_lab.compare_section_open
            != tree_state.prev_prompt_lab_compare_section_open
        || view.left_pane.prompt_lab.context_section_open
            != tree_state.prev_prompt_lab_context_section_open
        || view.left_pane.prompt_lab.template_section_open
            != tree_state.prev_prompt_lab_template_section_open
        || view.left_pane.prompt_lab.template_editor_open
            != tree_state.prev_prompt_lab_template_editor_open;
    if !layout_changed {
        return;
    }
    engine_debug!(
        "[Render] Layout update: left_panel_width {} -> {}, input_panel_visible: {} -> {}, active_tab: {:?} -> {:?}",
        tree_state.prev_left_panel_width,
        view.left_panel_width,
        tree_state.prev_input_panel_visible,
        view.input_panel_visible,
        tree_state.prev_active_tab,
        view.right_pane.active_tab,
    );
    if view.left_pane.left_tab != tree_state.prev_left_tab {
        let visible_count = match view.left_pane.left_tab {
            LeftTab::Jobs | LeftTab::TriageReview | LeftTab::TriageResults => {
                match view.left_pane.job_list_scope {
                    JobListScope::SinceCheckpoint => {
                        view.jobs.iter().filter(|j| j.is_since_checkpoint).count()
                    }
                    JobListScope::All => view.jobs.len(),
                }
            }
            LeftTab::PromptLab => 0,
        };
        engine_info!(
            "[jobs-ui] visible rows: {} (tab={:?}, scope={:?})",
            visible_count,
            view.left_pane.left_tab,
            view.left_pane.job_list_scope,
        );
    }
    cmds.push(build_layout_command(
        window_id,
        LayoutConfig {
            left_panel_width: view.left_panel_width,
            input_panel_visible: view.input_panel_visible,
            active_tab: view.right_pane.active_tab,
            left_tab: view.left_pane.left_tab,
            prompt_lab: PromptLabLayoutConfig {
                visible: prompt_lab_tab_visible,
                advanced_mode: view.left_pane.prompt_lab.advanced_mode,
                compare_section_open: view.left_pane.prompt_lab.compare_section_open,
                context_section_open: view.left_pane.prompt_lab.context_section_open,
                template_section_open: view.left_pane.prompt_lab.template_section_open,
                run_details_section_open: view.left_pane.prompt_lab.run_details_section_open,
                template_editor_open: view.left_pane.prompt_lab.template_editor_open,
            },
        },
    ));
    if prompt_lab_tab_visible && !tree_state.prev_prompt_lab_visible {
        tree_state.prev_prompt_lab_model_catalog = None;
        tree_state.prev_prompt_lab_selected_model = None;
    }
    tree_state.prev_left_panel_width = view.left_panel_width;
    tree_state.prev_input_panel_visible = view.input_panel_visible;
    tree_state.prev_active_tab = view.right_pane.active_tab;
    tree_state.prev_left_tab = view.left_pane.left_tab;
    tree_state.prev_prompt_lab_visible = prompt_lab_tab_visible;
    tree_state.prev_prompt_lab_advanced_mode = view.left_pane.prompt_lab.advanced_mode;
    tree_state.prev_prompt_lab_compare_section_open =
        view.left_pane.prompt_lab.compare_section_open;
    tree_state.prev_prompt_lab_context_section_open =
        view.left_pane.prompt_lab.context_section_open;
    tree_state.prev_prompt_lab_template_section_open =
        view.left_pane.prompt_lab.template_section_open;
    tree_state.prev_prompt_lab_template_editor_open =
        view.left_pane.prompt_lab.template_editor_open;
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
    if let Some(progress) = view.briefing_progress.as_deref() {
        status_parts.push(progress.to_string());
    }
    if let Some(progress) = view.triage_progress.as_deref() {
        status_parts.push(progress.to_string());
    }
    if let Some(usage) = format_llm_usage_status(&view.llm_usage_by_model) {
        status_parts.push(usage);
    }
    emit_if_changed(
        &mut tree_state.prev_status_text,
        status_parts.join(" | "),
        cmds,
        |text| PlatformCommand::UpdateLabelText {
            window_id,
            control_id: LABEL_STATUS,
            text,
            severity: MessageSeverity::Information,
        },
    );
    tree_state.prev_briefing_progress = view.briefing_progress.clone();
    tree_state.prev_triage_progress = view.triage_progress.clone();
}

fn render_token_progress_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
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
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_BRIEFING,
            rtf_text: convert_markdown_to_rtf(&truncated),
        });
        tree_state.prev_briefing_text = Some(briefing_markdown.to_string());
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

    let header_text = view
        .preview_header
        .as_ref()
        .map(format_preview_header)
        .unwrap_or_else(|| "(no selection)".to_string());
    emit_if_changed(
        &mut tree_state.prev_header_text,
        header_text,
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PREVIEW_HEADER,
            text,
        },
    );
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

fn job_row_check_policy(tab: LeftTab, is_pre_triage_reviewing: bool, job: &JobRowView) -> CheckState {
    match tab {
        LeftTab::TriageReview if is_pre_triage_reviewing => match job.filter_status {
            Some(JobFilterStatus::HardExcluded { .. })
            | Some(JobFilterStatus::ManuallyExcluded) => CheckState::Unchecked,
            Some(JobFilterStatus::ReviewNeeded { .. })
            | Some(JobFilterStatus::ManuallyIncluded)
            | Some(JobFilterStatus::AutoIncluded) => CheckState::Checked,
            None => CheckState::Unchecked,
        },
        _ => CheckState::Unchecked,
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
    let jobs_iter: Box<dyn Iterator<Item = &JobRowView>> =
        if view.left_pane.job_list_scope == JobListScope::SinceCheckpoint {
            Box::new(view.jobs.iter().filter(|j| j.is_since_checkpoint))
        } else {
            Box::new(view.jobs.iter())
        };
    let tab = view.left_pane.left_tab;
    let presentation = job_row_presentation(tab);
    jobs_iter
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

/// Jobs tab: preserves the original pre/post-triage row layout.
fn format_job_row_legacy(job: &JobRowView) -> String {
    let filter_prefix = match &job.filter_status {
        Some(JobFilterStatus::HardExcluded { .. }) => "[AUTO EXCLUDED] ",
        Some(JobFilterStatus::ReviewNeeded { .. }) => "[REVIEW] ",
        Some(JobFilterStatus::ManuallyExcluded) => "[EXCLUDED] ",
        Some(JobFilterStatus::ManuallyIncluded) => "",
        _ => "",
    };
    if job.has_summary {
        let title = job
            .summary_title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("(summary available)");
        let domain = domain_from_url(&job.url);
        let source = if domain.is_empty() { &job.url } else { &domain };
        let triage_prefix = job
            .triage_annotation
            .as_ref()
            .map(|annotation| format!("P{} [{}] ", annotation.priority, annotation.category))
            .unwrap_or_default();
        return format!("{filter_prefix}{triage_prefix}{title} — {source}");
    }

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
    let annotation = job.triage_annotation.as_ref().map(|annotation| {
        let mut prefix = format!("P{} [{}]", annotation.priority, annotation.category);
        if !annotation.tags.is_empty() {
            let tags = annotation.tags.join(", ");
            prefix.push_str(&format!(" ({tags})"));
        }
        prefix.push_str(" — ");
        prefix
    });
    let annotated_url = if let Some(prefix) = annotation {
        format!("{prefix}{}", job.url)
    } else {
        job.url.clone()
    };
    let base = if metrics.is_empty() {
        format!(
            "[#{id}] {status} — {annotated_url}",
            id = job.job_id,
            status = status,
            annotated_url = annotated_url
        )
    } else {
        format!(
            "[#{id}] {status} — {annotated_url} ({metrics})",
            id = job.job_id,
            status = status,
            annotated_url = annotated_url,
            metrics = metrics
        )
    };
    format!("{filter_prefix}{base}")
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

/// Triage Results tab: shows triage annotation (priority/category/tags) prominently.
fn format_job_row_triage_results(job: &JobRowView) -> String {
    if let Some(annotation) = &job.triage_annotation {
        let tag_suffix = if annotation.tags.is_empty() {
            String::new()
        } else {
            format!(" ({})", annotation.tags.join(", "))
        };
        let label = job_display_label(job);
        format!(
            "P{} [{}]{} — {}",
            annotation.priority, annotation.category, tag_suffix, label
        )
    } else {
        let label = job_display_label(job);
        format!("[no triage] {label}")
    }
}

/// Returns the best short display label for a job (summary title or URL).
fn job_display_label(job: &JobRowView) -> String {
    if job.has_summary {
        let title = job
            .summary_title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or("(summary available)");
        let domain = domain_from_url(&job.url);
        let source = if domain.is_empty() { job.url.as_str() } else { domain.as_str() };
        format!("{title} — {source}")
    } else {
        job.url.clone()
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
    use harvester_core::{LinkRowView, PromptLabRunId, PromptLabRunSummaryView, PromptLabView};
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
        AppViewModel {
            job_count: jobs.len(),
            jobs,
            ..AppViewModel::default()
        }
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
                resolved_model: Some("gpt-4o-mini".to_string()),
                parse_ok: Some(true),
                cache_status: Some("miss".to_string()),
            }),
            ..PromptLabView::default()
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
        assert_eq!(row, "P4 [security] Headline from summary — example.com");
        assert!(!row.contains("[#7]"));
        assert!(!row.contains("OK"));
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
        assert!(row.contains("[#9] OK"));
        assert!(row.contains("https://example.com/path"));
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
            "missing {{context}}".to_string(),
            "unknown {{foo}}".to_string(),
        ];
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlText { control_id, text, .. }
                if *control_id == LABEL_PROMPT_LAB_TEMPLATE_STATUS
                    && text.contains("missing {{context}}")
                    && text.contains("unknown {{foo}}")
            )
        }));
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
            ModelId::new(harvester_engine::llm::ProviderKind::OpenAi, "gpt-4o-mini"),
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
                        "gpt-4o-mini".to_string(),
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
            "gpt-4o-mini",
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
            "gpt-4o-mini",
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

    #[test]
    fn status_bar_omits_llm_usage_when_empty() {
        assert_eq!(format_llm_usage_status(&[]), None);
    }

    #[test]
    fn status_bar_includes_llm_usage_segment() {
        let rows = vec![LlmModelUsageView {
            model: "gpt-4o-mini".to_string(),
            input_tokens: 12_000,
            output_tokens: 3_000,
        }];
        let result = format_llm_usage_status(&rows);
        assert_eq!(result, Some("gpt-4o-mini: in=12K out=3K".to_string()));
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
}
