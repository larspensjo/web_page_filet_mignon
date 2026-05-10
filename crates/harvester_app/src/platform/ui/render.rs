use commanductui::{MessageSeverity, PlatformCommand, StyleId, WindowId};
use engine_logging::engine_debug;
use harvester_core::{AppTab, AppViewModel, LayoutViewModel, LeftTab, DEFAULT_JOBS_PANEL_WIDTH};
use harvester_engine::llm::ModelId;

use super::groups::bottom_buttons::BottomButtonsRenderState;
use super::groups::prompt_lab_actions::PromptLabActionsRenderState;
use super::groups::prompt_lab_sections::PromptLabSectionsRenderState;
use super::layout::{build_layout_command, LayoutConfig, PromptLabLayoutConfig};
use super::render_controls::{
    render_left_tab_bar_section, render_llm_quota_progress_section, render_main_controls_section,
    render_operation_progress_section, render_status_section, render_tab_bar_section,
    render_token_progress_section,
};
use super::render_list_box::{append_list_box_commands, ListBoxRenderModel};
use super::render_preview::render_preview_section;
pub(crate) use super::render_prompt_lab::combo_index_to_model;
use super::render_prompt_lab::render_prompt_lab_section;

#[derive(Debug)]
pub(super) struct LayoutRenderState {
    pub(super) layout_initialized: bool,
    pub(super) prev_left_panel_width: i32,
    pub(super) prev_input_panel_visible: bool,
    pub(super) prev_operation_progress_visible: bool,
    pub(super) prev_ai_warning_banner_visible: bool,
    pub(super) prev_active_tab: AppTab,
    pub(super) prev_left_tab: LeftTab,
    pub(super) prev_prompt_lab_visible: bool,
    pub(super) prev_prompt_lab_advanced_mode: bool,
    pub(super) prev_prompt_lab_compare_section_open: bool,
    pub(super) prev_prompt_lab_context_section_open: bool,
    pub(super) prev_prompt_lab_template_section_open: bool,
    pub(super) prev_prompt_lab_run_details_section_open: bool,
    pub(super) prev_prompt_lab_template_editor_open: bool,
}

impl Default for LayoutRenderState {
    fn default() -> Self {
        Self {
            layout_initialized: false,
            prev_left_panel_width: DEFAULT_JOBS_PANEL_WIDTH,
            prev_input_panel_visible: false,
            prev_operation_progress_visible: false,
            prev_ai_warning_banner_visible: false,
            prev_active_tab: AppTab::Summary,
            prev_left_tab: LeftTab::default(),
            prev_prompt_lab_visible: false,
            prev_prompt_lab_advanced_mode: false,
            prev_prompt_lab_compare_section_open: false,
            prev_prompt_lab_context_section_open: false,
            prev_prompt_lab_template_section_open: false,
            prev_prompt_lab_run_details_section_open: false,
            prev_prompt_lab_template_editor_open: false,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ControlsRenderState {
    pub(super) prev_status_label: Option<(String, MessageSeverity)>,
    pub(super) prev_progress_text: Option<String>,
    pub(super) prev_briefing_progress: Option<String>,
    pub(super) prev_triage_progress: Option<String>,
    pub(super) prev_progress_range: Option<(u32, u32)>,
    pub(super) prev_progress_pos: Option<u32>,
    pub(super) prev_token_progress_style: Option<StyleId>,
    pub(super) prev_operation_progress_text: Option<String>,
    pub(super) prev_operation_progress_range: Option<(u32, u32)>,
    pub(super) prev_operation_progress_pos: Option<u32>,
    pub(super) prev_llm_quota_text: Option<String>,
    pub(super) prev_llm_quota_range: Option<(u32, u32)>,
    pub(super) prev_llm_quota_pos: Option<u32>,
    pub(super) prev_llm_quota_style: Option<StyleId>,
    pub(super) prev_jobs_header_meta_text: Option<String>,
    pub(super) prev_jobs_scope_since_checkpoint_checked: Option<bool>,
}

#[derive(Debug, Default)]
pub(super) struct PromptLabRenderState {
    pub(super) prev_prompt_lab_mode_basic_checked: Option<bool>,
    pub(super) prev_prompt_lab_mode_advanced_checked: Option<bool>,
    pub(super) prev_prompt_lab_stage_triage_checked: Option<bool>,
    pub(super) prev_prompt_lab_stage_summary_checked: Option<bool>,
    pub(super) prev_prompt_lab_stage_briefing_checked: Option<bool>,
    pub(super) prev_prompt_lab_status_text: Option<String>,
    pub(super) prev_prompt_lab_metadata_text: Option<String>,
    pub(super) prev_prompt_lab_url_input: Option<String>,
    pub(super) prev_prompt_lab_url_enabled: Option<bool>,
    pub(super) prev_prompt_lab_context_text: Option<String>,
    pub(super) prev_prompt_lab_context_status_text: Option<String>,
    pub(super) prev_prompt_lab_template_system_text: Option<String>,
    pub(super) prev_prompt_lab_template_user_text: Option<String>,
    pub(super) prev_prompt_lab_template_status_text: Option<String>,
    pub(super) prev_prompt_lab_template_system_enabled: Option<bool>,
    pub(super) prev_prompt_lab_template_user_enabled: Option<bool>,
    pub(super) prev_prompt_lab_model_catalog: Option<Vec<ModelId>>,
    pub(super) prev_prompt_lab_selected_model: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct PreviewRenderState {
    pub(super) prev_preview_text: Option<String>,
    pub(super) prev_triage_text: Option<String>,
    pub(super) prev_briefing_text: Option<String>,
    pub(super) prev_poll_stats_text: Option<String>,
    pub(super) prev_ai_warning_title_text: Option<String>,
    pub(super) prev_ai_warning_body_text: Option<String>,
    pub(super) prev_preview_header_override_text: Option<String>,
    pub(super) prev_preview_source_caption_text: Option<String>,
    pub(super) prev_preview_source_link_text: Option<String>,
    pub(super) prev_preview_source_link_enabled: Option<bool>,
    pub(super) prev_preview_status_text: Option<String>,
    pub(super) prev_preview_attention_text: Option<String>,
}

#[derive(Debug, Default)]
pub struct TreeRenderState {
    pub(super) layout: LayoutRenderState,
    pub(super) controls: ControlsRenderState,
    pub(super) bottom_buttons: BottomButtonsRenderState,
    pub(super) prompt_lab_actions: PromptLabActionsRenderState,
    pub(super) prompt_lab_sections: PromptLabSectionsRenderState,
    pub(super) prompt_lab: PromptLabRenderState,
    pub(super) preview: PreviewRenderState,
}

impl TreeRenderState {
    pub fn new() -> Self {
        Self::default()
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
    render_llm_quota_progress_section(window_id, view, tree_state, &mut cmds);
    render_token_progress_section(window_id, view, tree_state, &mut cmds);
    render_main_controls_section(window_id, view, tree_state, &mut cmds);
    super::groups::bottom_buttons::render(
        window_id,
        view,
        &mut tree_state.bottom_buttons,
        &mut cmds,
    );
    super::groups::prompt_lab_actions::render(
        window_id,
        view,
        &mut tree_state.prompt_lab_actions,
        &mut cmds,
    );
    render_prompt_lab_section(window_id, view, tree_state, &mut cmds);

    let list_box = ListBoxRenderModel::from_view(view);
    append_list_box_commands(window_id, list_box, &mut cmds);

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
pub(super) fn emit_if_changed<T, F>(
    prev: &mut Option<T>,
    next: T,
    cmds: &mut Vec<PlatformCommand>,
    emit: F,
) where
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
        ai_warning_banner_visible: view.ai_warning_banner.is_some(),
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
    let layout_changed = !tree_state.layout.layout_initialized
        || layout.left_panel_width != tree_state.layout.prev_left_panel_width
        || layout.input_panel_visible != tree_state.layout.prev_input_panel_visible
        || layout.operation_progress_visible != tree_state.layout.prev_operation_progress_visible
        || layout.ai_warning_banner_visible != tree_state.layout.prev_ai_warning_banner_visible
        || layout.active_tab != tree_state.layout.prev_active_tab
        || layout.left_tab != tree_state.layout.prev_left_tab
        || prompt_lab_tab_visible != tree_state.layout.prev_prompt_lab_visible
        || layout.prompt_lab_advanced_mode != tree_state.layout.prev_prompt_lab_advanced_mode
        || layout.prompt_lab_compare_section_open
            != tree_state.layout.prev_prompt_lab_compare_section_open
        || layout.prompt_lab_context_section_open
            != tree_state.layout.prev_prompt_lab_context_section_open
        || layout.prompt_lab_template_section_open
            != tree_state.layout.prev_prompt_lab_template_section_open
        || layout.prompt_lab_run_details_section_open
            != tree_state.layout.prev_prompt_lab_run_details_section_open
        || layout.prompt_lab_template_editor_open
            != tree_state.layout.prev_prompt_lab_template_editor_open;
    if !layout_changed {
        return;
    }
    engine_debug!(
        "[Render] Layout update: left_panel_width {} -> {}, input_panel_visible: {} -> {}, active_tab: {:?} -> {:?}",
        tree_state.layout.prev_left_panel_width,
        layout.left_panel_width,
        tree_state.layout.prev_input_panel_visible,
        layout.input_panel_visible,
        tree_state.layout.prev_active_tab,
        layout.active_tab,
    );
    cmds.push(build_layout_command(
        window_id,
        LayoutConfig {
            left_panel_width: layout.left_panel_width,
            input_panel_visible: layout.input_panel_visible,
            operation_progress_visible: layout.operation_progress_visible,
            left_header_meta_visible: layout.left_header_meta_visible,
            ai_warning_banner_visible: layout.ai_warning_banner_visible,
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
    if prompt_lab_tab_visible && !tree_state.layout.prev_prompt_lab_visible {
        tree_state.prompt_lab.prev_prompt_lab_model_catalog = None;
        tree_state.prompt_lab.prev_prompt_lab_selected_model = None;
    }
    tree_state.layout.prev_left_panel_width = layout.left_panel_width;
    tree_state.layout.prev_input_panel_visible = layout.input_panel_visible;
    tree_state.layout.prev_operation_progress_visible = layout.operation_progress_visible;
    tree_state.layout.prev_ai_warning_banner_visible = layout.ai_warning_banner_visible;
    tree_state.layout.prev_active_tab = layout.active_tab;
    tree_state.layout.prev_left_tab = layout.left_tab;
    tree_state.layout.prev_prompt_lab_visible = prompt_lab_tab_visible;
    tree_state.layout.prev_prompt_lab_advanced_mode = layout.prompt_lab_advanced_mode;
    tree_state.layout.prev_prompt_lab_compare_section_open = layout.prompt_lab_compare_section_open;
    tree_state.layout.prev_prompt_lab_context_section_open = layout.prompt_lab_context_section_open;
    tree_state.layout.prev_prompt_lab_template_section_open =
        layout.prompt_lab_template_section_open;
    tree_state.layout.prev_prompt_lab_run_details_section_open =
        layout.prompt_lab_run_details_section_open;
    tree_state.layout.prev_prompt_lab_template_editor_open = layout.prompt_lab_template_editor_open;
    tree_state.layout.layout_initialized = true;
}


#[cfg(test)]
#[rustfmt::skip]
#[path = "render_tests.rs"]
mod tests;
