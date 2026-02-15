use commanductui::types::{TreeItemDescriptor, TreeItemId};
use commanductui::{CheckState, MessageSeverity, PlatformCommand, StyleId, WindowId};
use engine_logging::{engine_debug, engine_warn};
use harvester_core::{
    AppViewModel, JobFilterStatus, JobResultKind, JobRowView, LinkDownloadState, PreviewHeaderView,
    PromptLabInputSource, PromptLabStage, SessionState, Stage, DEFAULT_JOBS_PANEL_WIDTH,
};
use harvester_engine::LinkKind;

use super::constants::*;
use super::layout::build_layout_command;
use super::markdown_to_rtf::{convert_markdown_to_rtf, RTF_TRUNCATE_MARKER};
use super::tree_item_ids::{
    job_tree_item_id, link_tree_item_id, links_folder_tree_item_id, links_show_more_tree_item_id,
};
use std::collections::HashMap;

const MAX_VIEWER_CHARS: usize = 64 * 1024;
#[allow(dead_code)]
const VIEWER_TRUNCATE_MARKER: &str = "[display truncated]";

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
    prev_prompt_lab_template_editor_open: bool,
    prev_prompt_lab_status_text: Option<String>,
    prev_prompt_lab_metadata_text: Option<String>,
    prev_prompt_lab_url_input: Option<String>,
    prev_prompt_lab_run_enabled: Option<bool>,
    prev_prompt_lab_rerun_enabled: Option<bool>,
    prev_prompt_lab_resolve_enabled: Option<bool>,
    prev_prompt_lab_url_enabled: Option<bool>,
    prev_prompt_lab_clear_enabled: Option<bool>,
    prev_prompt_lab_stage_triage_text: Option<String>,
    prev_prompt_lab_stage_summary_text: Option<String>,
    prev_prompt_lab_stage_briefing_text: Option<String>,
    prev_prompt_lab_source_triage_text: Option<String>,
    prev_prompt_lab_source_url_text: Option<String>,
    prev_prompt_lab_context_text: Option<String>,
    prev_prompt_lab_context_status_text: Option<String>,
    prev_prompt_lab_context_apply_enabled: Option<bool>,
    prev_prompt_lab_context_apply_rerun_enabled: Option<bool>,
    prev_prompt_lab_context_revert_enabled: Option<bool>,
    prev_prompt_lab_context_save_enabled: Option<bool>,
    prev_prompt_lab_template_open_text: Option<String>,
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
            prev_prompt_lab_template_editor_open: false,
            prev_prompt_lab_status_text: None,
            prev_prompt_lab_metadata_text: None,
            prev_prompt_lab_url_input: None,
            prev_prompt_lab_run_enabled: None,
            prev_prompt_lab_rerun_enabled: None,
            prev_prompt_lab_resolve_enabled: None,
            prev_prompt_lab_url_enabled: None,
            prev_prompt_lab_clear_enabled: None,
            prev_prompt_lab_stage_triage_text: None,
            prev_prompt_lab_stage_summary_text: None,
            prev_prompt_lab_stage_briefing_text: None,
            prev_prompt_lab_source_triage_text: None,
            prev_prompt_lab_source_url_text: None,
            prev_prompt_lab_context_text: None,
            prev_prompt_lab_context_status_text: None,
            prev_prompt_lab_context_apply_enabled: None,
            prev_prompt_lab_context_apply_rerun_enabled: None,
            prev_prompt_lab_context_revert_enabled: None,
            prev_prompt_lab_context_save_enabled: None,
            prev_prompt_lab_template_open_text: None,
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
    let layout_changed = view.left_panel_width != tree_state.prev_left_panel_width
        || view.input_panel_visible != tree_state.prev_input_panel_visible
        || view.prompt_lab.visible != tree_state.prev_prompt_lab_visible
        || view.prompt_lab.template_editor_open != tree_state.prev_prompt_lab_template_editor_open;
    if layout_changed {
        engine_debug!(
            "[Render] Layout update: left_panel_width {} -> {}, input_panel_visible: {} -> {}",
            tree_state.prev_left_panel_width,
            view.left_panel_width,
            tree_state.prev_input_panel_visible,
            view.input_panel_visible
        );
        cmds.push(build_layout_command(
            window_id,
            view.left_panel_width,
            view.input_panel_visible,
            view.prompt_lab.visible,
            view.prompt_lab.template_editor_open,
        ));
        tree_state.prev_left_panel_width = view.left_panel_width;
        tree_state.prev_input_panel_visible = view.input_panel_visible;
        tree_state.prev_prompt_lab_visible = view.prompt_lab.visible;
        tree_state.prev_prompt_lab_template_editor_open = view.prompt_lab.template_editor_open;
    }

    let mut status_parts = vec![status_base_text.clone()];
    if let Some(progress) = view.briefing_progress.as_deref() {
        status_parts.push(progress.to_string());
    }
    if let Some(progress) = view.triage_progress.as_deref() {
        status_parts.push(progress.to_string());
    }
    let status_text = status_parts.join(" | ");

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
    tree_state.prev_triage_progress = view.triage_progress.clone();

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

    let briefing_enabled = view.briefing_can_start;
    if tree_state.prev_briefing_enabled != Some(briefing_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_BRIEFING,
            enabled: briefing_enabled,
        });
        tree_state.prev_briefing_enabled = Some(briefing_enabled);
    }

    let triage_enabled = view.triage_can_start;
    if tree_state.prev_triage_enabled != Some(triage_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_TRIAGE,
            enabled: triage_enabled,
        });
        tree_state.prev_triage_enabled = Some(triage_enabled);
    }

    let poll_enabled = view.poll_sources_enabled;
    if tree_state.prev_poll_enabled != Some(poll_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_POLL_SOURCES,
            enabled: poll_enabled,
        });
        tree_state.prev_poll_enabled = Some(poll_enabled);
    }

    let open_browser_enabled = view.selected_url.is_some();
    if tree_state.prev_open_browser_enabled != Some(open_browser_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BUTTON_OPEN_BROWSER,
            enabled: open_browser_enabled,
        });
        tree_state.prev_open_browser_enabled = Some(open_browser_enabled);
    }

    let stage_triage_text = select_label(
        "Triage",
        view.prompt_lab.selected_stage == PromptLabStage::Triage,
    );
    if tree_state.prev_prompt_lab_stage_triage_text.as_deref() != Some(stage_triage_text.as_str()) {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: BTN_STAGE_TRIAGE,
            text: stage_triage_text.clone(),
        });
        tree_state.prev_prompt_lab_stage_triage_text = Some(stage_triage_text);
    }
    let stage_summary_text = select_label(
        "Summary",
        view.prompt_lab.selected_stage == PromptLabStage::Summary,
    );
    if tree_state.prev_prompt_lab_stage_summary_text.as_deref() != Some(stage_summary_text.as_str())
    {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: BTN_STAGE_SUMMARY,
            text: stage_summary_text.clone(),
        });
        tree_state.prev_prompt_lab_stage_summary_text = Some(stage_summary_text);
    }
    let stage_briefing_text = select_label(
        "Briefing",
        view.prompt_lab.selected_stage == PromptLabStage::Briefing,
    );
    if tree_state.prev_prompt_lab_stage_briefing_text.as_deref()
        != Some(stage_briefing_text.as_str())
    {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: BTN_STAGE_BRIEFING,
            text: stage_briefing_text.clone(),
        });
        tree_state.prev_prompt_lab_stage_briefing_text = Some(stage_briefing_text);
    }

    let source_from_triage_text = select_label(
        "From triage",
        view.prompt_lab.selected_input_source == PromptLabInputSource::FromTriageArticles,
    );
    if tree_state.prev_prompt_lab_source_triage_text.as_deref()
        != Some(source_from_triage_text.as_str())
    {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: BTN_SOURCE_FROM_TRIAGE,
            text: source_from_triage_text.clone(),
        });
        tree_state.prev_prompt_lab_source_triage_text = Some(source_from_triage_text);
    }
    let source_type_url_text = select_label(
        "Type URL",
        view.prompt_lab.selected_input_source == PromptLabInputSource::TypeUrl,
    );
    if tree_state.prev_prompt_lab_source_url_text.as_deref() != Some(source_type_url_text.as_str())
    {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: BTN_SOURCE_TYPE_URL,
            text: source_type_url_text.clone(),
        });
        tree_state.prev_prompt_lab_source_url_text = Some(source_type_url_text);
    }

    if tree_state.prev_prompt_lab_run_enabled != Some(view.prompt_lab.can_run) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_RUN,
            enabled: view.prompt_lab.can_run,
        });
        tree_state.prev_prompt_lab_run_enabled = Some(view.prompt_lab.can_run);
    }
    if tree_state.prev_prompt_lab_rerun_enabled != Some(view.prompt_lab.can_rerun) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_RERUN,
            enabled: view.prompt_lab.can_rerun,
        });
        tree_state.prev_prompt_lab_rerun_enabled = Some(view.prompt_lab.can_rerun);
    }
    let type_url_selected = view.prompt_lab.selected_input_source == PromptLabInputSource::TypeUrl;
    let resolve_enabled = type_url_selected && !view.prompt_lab.resolve_pending;
    if tree_state.prev_prompt_lab_resolve_enabled != Some(resolve_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_RESOLVE,
            enabled: resolve_enabled,
        });
        tree_state.prev_prompt_lab_resolve_enabled = Some(resolve_enabled);
    }
    if tree_state.prev_prompt_lab_url_enabled != Some(type_url_selected) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: INPUT_PROMPT_LAB_URL,
            enabled: type_url_selected,
        });
        tree_state.prev_prompt_lab_url_enabled = Some(type_url_selected);
    }
    let clear_enabled = view.prompt_lab.run_count > 0;
    if tree_state.prev_prompt_lab_clear_enabled != Some(clear_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CLEAR,
            enabled: clear_enabled,
        });
        tree_state.prev_prompt_lab_clear_enabled = Some(clear_enabled);
    }
    let compare_add_enabled = view.prompt_lab.can_add_candidate;
    if tree_state.prev_prompt_lab_compare_add_current_enabled != Some(compare_add_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_ADD_CURRENT,
            enabled: compare_add_enabled,
        });
        tree_state.prev_prompt_lab_compare_add_current_enabled = Some(compare_add_enabled);
    }
    if tree_state.prev_prompt_lab_compare_add_baseline_enabled != Some(compare_add_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_ADD_BASELINE,
            enabled: compare_add_enabled,
        });
        tree_state.prev_prompt_lab_compare_add_baseline_enabled = Some(compare_add_enabled);
    }
    let compare_reset_enabled = view.prompt_lab.can_reset_draft;
    if tree_state.prev_prompt_lab_compare_reset_draft_enabled != Some(compare_reset_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_RESET_DRAFT,
            enabled: compare_reset_enabled,
        });
        tree_state.prev_prompt_lab_compare_reset_draft_enabled = Some(compare_reset_enabled);
    }
    let compare_start_enabled =
        view.prompt_lab.active_batch.is_none() && view.prompt_lab.draft_candidates.len() >= 2;
    if tree_state.prev_prompt_lab_compare_start_enabled != Some(compare_start_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_START,
            enabled: compare_start_enabled,
        });
        tree_state.prev_prompt_lab_compare_start_enabled = Some(compare_start_enabled);
    }
    let compare_cancel_enabled = view
        .prompt_lab
        .active_batch
        .as_ref()
        .map(|batch| batch.can_cancel)
        .unwrap_or(false);
    if tree_state.prev_prompt_lab_compare_cancel_enabled != Some(compare_cancel_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_CANCEL,
            enabled: compare_cancel_enabled,
        });
        tree_state.prev_prompt_lab_compare_cancel_enabled = Some(compare_cancel_enabled);
    }
    let compare_auto_select_enabled = view
        .prompt_lab
        .active_batch
        .as_ref()
        .map(|batch| batch.can_auto_select)
        .unwrap_or(false);
    if tree_state.prev_prompt_lab_compare_auto_select_enabled != Some(compare_auto_select_enabled) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_AUTO_SELECT,
            enabled: compare_auto_select_enabled,
        });
        tree_state.prev_prompt_lab_compare_auto_select_enabled = Some(compare_auto_select_enabled);
    }
    let compare_winner_clear_enabled = view
        .prompt_lab
        .active_batch
        .as_ref()
        .map(|batch| {
            batch
                .rows
                .iter()
                .any(|row| row.is_manual_winner || row.is_auto_winner)
        })
        .unwrap_or(false);
    if tree_state.prev_prompt_lab_compare_winner_clear_enabled != Some(compare_winner_clear_enabled)
    {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_WINNER_CLEAR,
            enabled: compare_winner_clear_enabled,
        });
        tree_state.prev_prompt_lab_compare_winner_clear_enabled =
            Some(compare_winner_clear_enabled);
    }
    if tree_state.prev_prompt_lab_context_apply_enabled != Some(view.prompt_lab.can_apply_context) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_APPLY,
            enabled: view.prompt_lab.can_apply_context,
        });
        tree_state.prev_prompt_lab_context_apply_enabled = Some(view.prompt_lab.can_apply_context);
    }
    if tree_state.prev_prompt_lab_context_apply_rerun_enabled
        != Some(view.prompt_lab.can_apply_and_rerun)
    {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN,
            enabled: view.prompt_lab.can_apply_and_rerun,
        });
        tree_state.prev_prompt_lab_context_apply_rerun_enabled =
            Some(view.prompt_lab.can_apply_and_rerun);
    }
    if tree_state.prev_prompt_lab_context_revert_enabled != Some(view.prompt_lab.can_revert_context)
    {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_REVERT,
            enabled: view.prompt_lab.can_revert_context,
        });
        tree_state.prev_prompt_lab_context_revert_enabled =
            Some(view.prompt_lab.can_revert_context);
    }
    if tree_state.prev_prompt_lab_context_save_enabled != Some(view.prompt_lab.can_save_context) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_SAVE,
            enabled: view.prompt_lab.can_save_context,
        });
        tree_state.prev_prompt_lab_context_save_enabled = Some(view.prompt_lab.can_save_context);
    }
    let template_open_text = if view.prompt_lab.template_editor_open {
        "[x] Edit Templates".to_string()
    } else {
        "[ ] Edit Templates".to_string()
    };
    if tree_state.prev_prompt_lab_template_open_text.as_deref() != Some(template_open_text.as_str())
    {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_OPEN,
            text: template_open_text.clone(),
        });
        tree_state.prev_prompt_lab_template_open_text = Some(template_open_text);
    }
    let can_apply_template =
        view.prompt_lab.template_dirty && view.prompt_lab.template_validation_errors.is_empty();
    let can_apply_template_and_rerun = can_apply_template && view.prompt_lab.can_run;
    let can_revert_template = view.prompt_lab.template_dirty || view.prompt_lab.template_applied;
    if tree_state.prev_prompt_lab_template_apply_enabled != Some(can_apply_template) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY,
            enabled: can_apply_template,
        });
        tree_state.prev_prompt_lab_template_apply_enabled = Some(can_apply_template);
    }
    if tree_state.prev_prompt_lab_template_apply_rerun_enabled != Some(can_apply_template_and_rerun)
    {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN,
            enabled: can_apply_template_and_rerun,
        });
        tree_state.prev_prompt_lab_template_apply_rerun_enabled =
            Some(can_apply_template_and_rerun);
    }
    if tree_state.prev_prompt_lab_template_revert_enabled != Some(can_revert_template) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_REVERT,
            enabled: can_revert_template,
        });
        tree_state.prev_prompt_lab_template_revert_enabled = Some(can_revert_template);
    }
    if tree_state.prev_prompt_lab_template_save_enabled != Some(view.prompt_lab.template_applied) {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_SAVE,
            enabled: view.prompt_lab.template_applied,
        });
        tree_state.prev_prompt_lab_template_save_enabled = Some(view.prompt_lab.template_applied);
    }
    if tree_state.prev_prompt_lab_template_system_text.as_deref()
        != Some(view.prompt_lab.template_system_draft.as_str())
    {
        cmds.push(PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
            text: view.prompt_lab.template_system_draft.clone(),
        });
        tree_state.prev_prompt_lab_template_system_text =
            Some(view.prompt_lab.template_system_draft.clone());
    }
    if tree_state.prev_prompt_lab_template_user_text.as_deref()
        != Some(view.prompt_lab.template_user_draft.as_str())
    {
        cmds.push(PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
            text: view.prompt_lab.template_user_draft.clone(),
        });
        tree_state.prev_prompt_lab_template_user_text =
            Some(view.prompt_lab.template_user_draft.clone());
    }
    if tree_state.prev_prompt_lab_template_system_enabled
        != Some(view.prompt_lab.template_editor_open)
    {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
            enabled: view.prompt_lab.template_editor_open,
        });
        tree_state.prev_prompt_lab_template_system_enabled =
            Some(view.prompt_lab.template_editor_open);
    }
    if tree_state.prev_prompt_lab_template_user_enabled
        != Some(view.prompt_lab.template_editor_open)
    {
        cmds.push(PlatformCommand::SetControlEnabled {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
            enabled: view.prompt_lab.template_editor_open,
        });
        tree_state.prev_prompt_lab_template_user_enabled =
            Some(view.prompt_lab.template_editor_open);
    }

    if tree_state.prev_prompt_lab_url_input.as_deref() != Some(view.prompt_lab.url_input.as_str()) {
        cmds.push(PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_URL,
            text: view.prompt_lab.url_input.clone(),
        });
        tree_state.prev_prompt_lab_url_input = Some(view.prompt_lab.url_input.clone());
    }

    let status_text = prompt_lab_status_text(&view.prompt_lab);
    if tree_state.prev_prompt_lab_status_text.as_deref() != Some(status_text.as_str()) {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_STATUS,
            text: status_text.clone(),
        });
        tree_state.prev_prompt_lab_status_text = Some(status_text);
    }
    let metadata_text = prompt_lab_metadata_text(&view.prompt_lab);
    if tree_state.prev_prompt_lab_metadata_text.as_deref() != Some(metadata_text.as_str()) {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_METADATA,
            text: metadata_text.clone(),
        });
        tree_state.prev_prompt_lab_metadata_text = Some(metadata_text);
    }
    let context_text = view.prompt_lab.context_draft_text.clone();
    if tree_state.prev_prompt_lab_context_text.as_deref() != Some(context_text.as_str()) {
        cmds.push(PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_CONTEXT,
            text: context_text.clone(),
        });
        tree_state.prev_prompt_lab_context_text = Some(context_text);
    }
    let context_status_text = if !view.prompt_lab.context_validation_errors.is_empty() {
        view.prompt_lab.context_validation_errors.join(" • ")
    } else {
        view.prompt_lab
            .context_status_message
            .clone()
            .unwrap_or_default()
    };
    if tree_state.prev_prompt_lab_context_status_text != Some(context_status_text.clone()) {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_CONTEXT_STATUS,
            text: context_status_text.clone(),
        });
        tree_state.prev_prompt_lab_context_status_text = Some(context_status_text);
    }
    let template_status_text = if !view.prompt_lab.template_validation_errors.is_empty() {
        view.prompt_lab.template_validation_errors.join(" • ")
    } else if let (Some(version), Some(path)) = (
        view.prompt_lab.template_saved_version,
        view.prompt_lab.template_saved_path.as_deref(),
    ) {
        format!("Saved template v{version} to {path}")
    } else if view.prompt_lab.template_applied {
        "Template draft applied".to_string()
    } else if view.prompt_lab.template_dirty {
        "Template draft has unapplied changes".to_string()
    } else {
        String::new()
    };
    if tree_state.prev_prompt_lab_template_status_text != Some(template_status_text.clone()) {
        cmds.push(PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_TEMPLATE_STATUS,
            text: template_status_text.clone(),
        });
        tree_state.prev_prompt_lab_template_status_text = Some(template_status_text);
    }

    let job_items = build_job_tree(view);
    append_tree_commands(window_id, job_items, tree_state, &mut cmds);

    let (preview_markdown, preview_header_text) = prompt_lab_preview_override(view);
    let preview_text_changed = match tree_state.prev_preview_text.as_deref() {
        Some(prev) => prev != preview_markdown,
        None => true,
    };
    if preview_text_changed {
        let (truncated_markdown, was_truncated) = truncate_markdown_for_preview(preview_markdown);
        let mut rtf_text = convert_markdown_to_rtf(&truncated_markdown);
        if was_truncated {
            engine_warn!(
                "[preview] markdown preview truncated from {} chars to {} chars",
                preview_markdown.chars().count(),
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
        tree_state.prev_preview_text = Some(preview_markdown.to_string());
    }

    let header_text = preview_header_text.unwrap_or_else(|| {
        view.preview_header
            .as_ref()
            .map(format_preview_header)
            .unwrap_or_else(|| "(no selection)".to_string())
    });
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

fn prompt_lab_preview_override(view: &AppViewModel) -> (&str, Option<String>) {
    if view.prompt_lab.visible {
        if let Some(run) = view.prompt_lab.latest_run.as_ref() {
            if run.status_label == "completed" {
                let header = format!(
                    "Prompt Lab - {}{}",
                    prompt_lab_stage_label(run.stage),
                    run.resolved_model
                        .as_ref()
                        .map(|model| format!(" ({model})"))
                        .unwrap_or_default()
                );
                if let Some(output) = run.output_json.as_deref() {
                    return (output, Some(header));
                }
            }
        }
    }
    (view.preview_text.as_deref().unwrap_or_default(), None)
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

fn select_label(label: &str, selected: bool) -> String {
    if selected {
        format!("[x] {label}")
    } else {
        format!("[ ] {label}")
    }
}

fn prompt_lab_stage_label(stage: PromptLabStage) -> &'static str {
    match stage {
        PromptLabStage::Triage => "Triage",
        PromptLabStage::Summary => "Summary",
        PromptLabStage::Briefing => "Briefing",
    }
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
                state: job_check_state(view, job),
                children,
                style_override: if job.has_summary {
                    None
                } else {
                    Some(StyleId::TreeItemDisabled)
                },
            }
        })
        .collect()
}

fn job_check_state(view: &AppViewModel, job: &JobRowView) -> CheckState {
    if !view.is_pre_triage_reviewing {
        return CheckState::Unchecked;
    }
    match job.filter_status {
        Some(JobFilterStatus::HardExcluded { .. }) | Some(JobFilterStatus::ManuallyExcluded) => {
            CheckState::Unchecked
        }
        Some(JobFilterStatus::ReviewNeeded { .. })
        | Some(JobFilterStatus::ManuallyIncluded)
        | Some(JobFilterStatus::AutoIncluded) => CheckState::Checked,
        None => CheckState::Unchecked,
    }
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
            triage_annotation: None,
            has_summary: false,
            summary_title: None,
            filter_status: None,
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
        let view = AppViewModel {
            preview_text: Some("first\nsecond\r\nthird\rfourth".to_string()),
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
        let view = AppViewModel {
            preview_text: Some("**bold**".to_string()),
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
        let view = AppViewModel {
            preview_text: Some(long_text),
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
        let jobs_width = rules
            .iter()
            .find(|rule| rule.control_id == PANEL_JOBS)
            .and_then(|rule| rule.fixed_size)
            .expect("PANEL_JOBS fixed size");

        assert_eq!(input_width, harvester_core::INPUT_PANEL_FIXED_WIDTH);
        assert_eq!(jobs_width, 760 - harvester_core::INPUT_PANEL_FIXED_WIDTH);
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

        let row = format_job_row(&job);
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

        let row = format_job_row(&job);
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
        view.prompt_lab.can_run = false;
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
        view.prompt_lab.template_editor_open = true;
        view.prompt_lab.template_dirty = true;
        view.prompt_lab.template_validation_errors = vec!["unknown var".to_string()];
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
        view.prompt_lab.template_editor_open = true;
        view.prompt_lab.template_applied = false;
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
        view.prompt_lab.template_editor_open = true;
        view.prompt_lab.template_validation_errors = vec![
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
    fn rerun_button_enabled_with_completed_run() {
        let window_id = WindowId::new(11);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.prompt_lab.can_rerun = true;
        view.prompt_lab.latest_run = completed_prompt_lab_view().latest_run;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
                if *control_id == BTN_PROMPT_LAB_RERUN
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
        view.prompt_lab.draft_candidates = vec![
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
    fn resolve_button_enabled_only_in_typeurl_mode() {
        let window_id = WindowId::new(12);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.prompt_lab.selected_input_source = PromptLabInputSource::FromTriageArticles;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: false, .. }
                if *control_id == BTN_PROMPT_LAB_RESOLVE
            )
        }));

        view.prompt_lab.selected_input_source = PromptLabInputSource::TypeUrl;
        view.prompt_lab.resolve_pending = false;
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlEnabled { control_id, enabled: true, .. }
                if *control_id == BTN_PROMPT_LAB_RESOLVE
            )
        }));
    }

    #[test]
    fn preview_override_emitted_when_lab_run_completed() {
        let window_id = WindowId::new(13);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.prompt_lab = completed_prompt_lab_view();
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetRichEditContent { rtf_text, .. }
                if rtf_text.contains("ok")
            )
        }));
        assert!(cmds.iter().any(|cmd| {
            matches!(
                cmd,
                PlatformCommand::SetControlText { control_id, text, .. }
                if *control_id == LABEL_PREVIEW_HEADER && text.contains("Prompt Lab")
            )
        }));
    }

    #[test]
    fn render_idempotent_on_unchanged_prompt_lab_view() {
        let window_id = WindowId::new(14);
        let mut tree_state = TreeRenderState::new();
        let mut view = make_view(vec![]);
        view.prompt_lab = completed_prompt_lab_view();
        let _ = render(window_id, &view, &mut tree_state);
        let cmds = render(window_id, &view, &mut tree_state);
        assert!(!cmds
            .iter()
            .any(|cmd| matches!(cmd, PlatformCommand::SetRichEditContent { .. })));
    }
}
