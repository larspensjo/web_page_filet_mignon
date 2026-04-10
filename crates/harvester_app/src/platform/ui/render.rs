use commanductui::{
    ChartDataPacket, ChartLineData, ChartLineEmphasis, MessageSeverity, PlatformCommand, StyleId,
    WindowId,
};
use engine_logging::{engine_debug, engine_info, engine_warn};
use harvester_core::{
    AppTab, AppViewModel, LayoutViewModel, LeftTab, PromptLabStage, TrendsTabView,
    DEFAULT_JOBS_PANEL_WIDTH,
};
use harvester_engine::llm::ModelId;

use super::constants::*;
use super::layout::{build_layout_command, LayoutConfig, PromptLabLayoutConfig};
use super::markdown_to_rtf::{convert_markdown_to_rtf, RTF_TRUNCATE_MARKER};
use super::render_controls::{
    render_left_tab_bar_section, render_main_controls_section, render_operation_progress_section,
    render_status_section, render_tab_bar_section, render_token_progress_section,
};
use super::render_list_box::{append_list_box_commands, ListBoxRenderModel};
use super::render_text::{strip_leading_h1, truncate_markdown_for_preview};

const SUMMARY_EMPTY_STATE_MARKDOWN: &str =
    "No article selected\n\nSelect a job/article from the list to view its summary.";

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
pub(super) struct LayoutRenderState {
    pub(super) layout_initialized: bool,
    pub(super) prev_left_panel_width: i32,
    pub(super) prev_input_panel_visible: bool,
    pub(super) prev_operation_progress_visible: bool,
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
    pub(super) prev_stop_enabled: Option<bool>,
    pub(super) prev_briefing_enabled: Option<bool>,
    pub(super) prev_triage_enabled: Option<bool>,
    pub(super) prev_poll_enabled: Option<bool>,
    pub(super) prev_briefing_progress: Option<String>,
    pub(super) prev_triage_progress: Option<String>,
    pub(super) prev_progress_range: Option<(u32, u32)>,
    pub(super) prev_progress_pos: Option<u32>,
    pub(super) prev_token_progress_style: Option<StyleId>,
    pub(super) prev_stop_style: Option<StyleId>,
    pub(super) prev_operation_progress_text: Option<String>,
    pub(super) prev_operation_progress_range: Option<(u32, u32)>,
    pub(super) prev_operation_progress_pos: Option<u32>,
    pub(super) prev_open_browser_enabled: Option<bool>,
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
    pub(super) prev_prompt_lab_section_compare_checked: Option<bool>,
    pub(super) prev_prompt_lab_section_context_checked: Option<bool>,
    pub(super) prev_prompt_lab_section_template_checked: Option<bool>,
    pub(super) prev_prompt_lab_section_run_details_checked: Option<bool>,
    pub(super) prev_prompt_lab_status_text: Option<String>,
    pub(super) prev_prompt_lab_metadata_text: Option<String>,
    pub(super) prev_prompt_lab_url_input: Option<String>,
    pub(super) prev_prompt_lab_run_enabled: Option<bool>,
    pub(super) prev_prompt_lab_resolve_enabled: Option<bool>,
    pub(super) prev_prompt_lab_url_enabled: Option<bool>,
    pub(super) prev_prompt_lab_context_text: Option<String>,
    pub(super) prev_prompt_lab_context_status_text: Option<String>,
    pub(super) prev_prompt_lab_context_apply_enabled: Option<bool>,
    pub(super) prev_prompt_lab_context_apply_rerun_enabled: Option<bool>,
    pub(super) prev_prompt_lab_context_revert_enabled: Option<bool>,
    pub(super) prev_prompt_lab_context_save_enabled: Option<bool>,
    pub(super) prev_prompt_lab_template_open_checked: Option<bool>,
    pub(super) prev_prompt_lab_template_system_text: Option<String>,
    pub(super) prev_prompt_lab_template_user_text: Option<String>,
    pub(super) prev_prompt_lab_template_status_text: Option<String>,
    pub(super) prev_prompt_lab_template_apply_enabled: Option<bool>,
    pub(super) prev_prompt_lab_template_apply_rerun_enabled: Option<bool>,
    pub(super) prev_prompt_lab_template_revert_enabled: Option<bool>,
    pub(super) prev_prompt_lab_template_save_enabled: Option<bool>,
    pub(super) prev_prompt_lab_template_system_enabled: Option<bool>,
    pub(super) prev_prompt_lab_template_user_enabled: Option<bool>,
    pub(super) prev_prompt_lab_compare_add_current_enabled: Option<bool>,
    pub(super) prev_prompt_lab_compare_add_baseline_enabled: Option<bool>,
    pub(super) prev_prompt_lab_compare_reset_draft_enabled: Option<bool>,
    pub(super) prev_prompt_lab_compare_start_enabled: Option<bool>,
    pub(super) prev_prompt_lab_compare_cancel_enabled: Option<bool>,
    pub(super) prev_prompt_lab_compare_auto_select_enabled: Option<bool>,
    pub(super) prev_prompt_lab_compare_winner_clear_enabled: Option<bool>,
    pub(super) prev_prompt_lab_model_catalog: Option<Vec<ModelId>>,
    pub(super) prev_prompt_lab_selected_model: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct PreviewRenderState {
    pub(super) prev_preview_text: Option<String>,
    pub(super) prev_triage_text: Option<String>,
    pub(super) prev_briefing_text: Option<String>,
    pub(super) prev_poll_stats_text: Option<String>,
    pub(super) prev_preview_header_override_text: Option<String>,
    pub(super) prev_preview_source_text: Option<String>,
    pub(super) prev_preview_status_text: Option<String>,
    pub(super) prev_preview_attention_text: Option<String>,
}

#[derive(Debug, Default)]
pub struct TreeRenderState {
    pub(super) layout: LayoutRenderState,
    pub(super) controls: ControlsRenderState,
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
    render_token_progress_section(window_id, view, tree_state, &mut cmds);
    render_main_controls_section(window_id, view, tree_state, &mut cmds);
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

fn render_prompt_lab_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_mode_basic_checked,
        !view.left_pane.prompt_lab.advanced_mode,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_PROMPT_LAB_MODE_BASIC,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_mode_advanced_checked,
        view.left_pane.prompt_lab.advanced_mode,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_PROMPT_LAB_MODE_ADVANCED,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_stage_triage_checked,
        view.left_pane.prompt_lab.selected_stage == PromptLabStage::Triage,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_STAGE_TRIAGE,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_stage_summary_checked,
        view.left_pane.prompt_lab.selected_stage == PromptLabStage::Summary,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_STAGE_SUMMARY,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_stage_briefing_checked,
        view.left_pane.prompt_lab.selected_stage == PromptLabStage::Briefing,
        cmds,
        |checked| PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: BTN_STAGE_BRIEFING,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_section_compare_checked,
        view.left_pane.prompt_lab.compare_section_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_SECTION_COMPARE,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_section_context_checked,
        view.left_pane.prompt_lab.context_section_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_SECTION_CONTEXT,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_section_template_checked,
        view.left_pane.prompt_lab.template_section_open,
        cmds,
        |checked| PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: CHK_PROMPT_LAB_SECTION_TEMPLATE,
            checked,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_section_run_details_checked,
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
    if tree_state
        .prompt_lab
        .prev_prompt_lab_model_catalog
        .as_deref()
        != Some(model_catalog.as_slice())
    {
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
        tree_state.prompt_lab.prev_prompt_lab_model_catalog = Some(model_catalog.clone());
    }
    let selected_model_key = selected_model
        .map(|m| m.model_name().to_string())
        .unwrap_or_else(|| "__DEFAULT__".to_string());
    if tree_state
        .prompt_lab
        .prev_prompt_lab_selected_model
        .as_deref()
        != Some(selected_model_key.as_str())
    {
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
        tree_state.prompt_lab.prev_prompt_lab_selected_model = Some(selected_model_key);
    }

    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_run_enabled,
        view.left_pane.prompt_lab.can_run,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_RUN,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_resolve_enabled,
        !view.left_pane.prompt_lab.resolve_pending,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_RESOLVE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_url_enabled,
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
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_compare_add_current_enabled,
        compare_add_enabled,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_ADD_CURRENT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_compare_add_baseline_enabled,
        compare_add_enabled,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_ADD_BASELINE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_compare_reset_draft_enabled,
        view.left_pane.prompt_lab.can_reset_draft,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_COMPARE_RESET_DRAFT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_compare_start_enabled,
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
        &mut tree_state.prompt_lab.prev_prompt_lab_compare_cancel_enabled,
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
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_compare_auto_select_enabled,
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
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_compare_winner_clear_enabled,
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
        &mut tree_state.prompt_lab.prev_prompt_lab_context_apply_enabled,
        view.left_pane.prompt_lab.can_apply_context,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_APPLY,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_context_apply_rerun_enabled,
        view.left_pane.prompt_lab.can_apply_and_rerun,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_context_revert_enabled,
        view.left_pane.prompt_lab.can_revert_context,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_REVERT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_context_save_enabled,
        view.left_pane.prompt_lab.can_save_context,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_CONTEXT_SAVE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_open_checked,
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
        &mut tree_state.prompt_lab.prev_prompt_lab_template_apply_enabled,
        can_apply_template,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_template_apply_rerun_enabled,
        can_apply_template_and_rerun,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_template_revert_enabled,
        can_revert_template,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_REVERT,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_save_enabled,
        view.left_pane.prompt_lab.template_applied,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: BTN_PROMPT_LAB_TEMPLATE_SAVE,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_system_text,
        view.left_pane.prompt_lab.template_system_draft.clone(),
        cmds,
        |text| PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_user_text,
        view.left_pane.prompt_lab.template_user_draft.clone(),
        cmds,
        |text| PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state
            .prompt_lab
            .prev_prompt_lab_template_system_enabled,
        view.left_pane.prompt_lab.template_editor_open,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_SYSTEM,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_template_user_enabled,
        view.left_pane.prompt_lab.template_editor_open,
        cmds,
        |enabled| PlatformCommand::SetControlEnabled {
            window_id,
            control_id: INPUT_PROMPT_LAB_TEMPLATE_USER,
            enabled,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_url_input,
        view.left_pane.prompt_lab.url_input.clone(),
        cmds,
        |text| PlatformCommand::SetInputText {
            window_id,
            control_id: INPUT_PROMPT_LAB_URL,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_status_text,
        prompt_lab_status_text(&view.left_pane.prompt_lab),
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_STATUS,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_metadata_text,
        prompt_lab_metadata_text(&view.left_pane.prompt_lab),
        cmds,
        |text| PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_PROMPT_LAB_METADATA,
            text,
        },
    );
    emit_if_changed(
        &mut tree_state.prompt_lab.prev_prompt_lab_context_text,
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
        &mut tree_state.prompt_lab.prev_prompt_lab_context_status_text,
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
        &mut tree_state.prompt_lab.prev_prompt_lab_template_status_text,
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
    if tree_state.preview.prev_preview_text.as_deref() != Some(summary_markdown) {
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
        tree_state.preview.prev_preview_text = Some(summary_markdown.to_string());
    }

    // Triage tab viewer.
    let triage_markdown = view
        .right_pane
        .triage_markdown
        .as_deref()
        .unwrap_or_default();
    if tree_state.preview.prev_triage_text.as_deref() != Some(triage_markdown) {
        let (truncated, _) = truncate_markdown_for_preview(triage_markdown);
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_TRIAGE,
            rtf_text: convert_markdown_to_rtf(&truncated),
        });
        tree_state.preview.prev_triage_text = Some(triage_markdown.to_string());
    }

    // Briefing tab viewer.
    let briefing_markdown = view
        .right_pane
        .briefing_markdown
        .as_deref()
        .unwrap_or_default();
    if tree_state.preview.prev_briefing_text.as_deref() != Some(briefing_markdown) {
        let (truncated, _) = truncate_markdown_for_preview(briefing_markdown);
        let display = strip_leading_h1(&truncated);
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_BRIEFING,
            rtf_text: convert_markdown_to_rtf(display),
        });
        tree_state.preview.prev_briefing_text = Some(briefing_markdown.to_string());
    }

    // Poll Stats tab viewer.
    let poll_stats_text = view
        .right_pane
        .poll_stats_markdown
        .as_deref()
        .unwrap_or("No poll data yet.");
    if tree_state.preview.prev_poll_stats_text.as_deref() != Some(poll_stats_text) {
        cmds.push(PlatformCommand::SetRichEditContent {
            window_id,
            control_id: VIEWER_POLL_STATS,
            rtf_text: convert_markdown_to_rtf(poll_stats_text),
        });
        tree_state.preview.prev_poll_stats_text = Some(poll_stats_text.to_string());
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
            &mut tree_state.preview.prev_preview_header_override_text,
            header_text,
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_HEADER,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_source_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_SOURCE,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_status_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_STATUS,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_attention_text,
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
            &mut tree_state.preview.prev_preview_header_override_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_HEADER,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_source_text,
            context.source_label.clone(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_SOURCE,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_status_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_STATUS,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_attention_text,
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
            &mut tree_state.preview.prev_preview_header_override_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_HEADER,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_source_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_SOURCE,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_status_text,
            String::new(),
            cmds,
            |text| PlatformCommand::SetControlText {
                window_id,
                control_id: LABEL_PREVIEW_STATUS,
                text,
            },
        );
        emit_if_changed(
            &mut tree_state.preview.prev_preview_attention_text,
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


#[cfg(test)]
#[rustfmt::skip]
#[path = "render_tests.rs"]
mod tests;
