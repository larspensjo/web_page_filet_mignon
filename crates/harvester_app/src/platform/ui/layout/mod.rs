use commanductui::{PlatformCommand, WindowId};
use harvester_core::{AppTab, LeftTab, DEFAULT_JOBS_PANEL_WIDTH};

mod init;
mod rules;
mod theme;

#[derive(Debug, Clone)]
pub(crate) struct PromptLabLayoutConfig {
    pub visible: bool,
    pub advanced_mode: bool,
    pub compare_section_open: bool,
    pub context_section_open: bool,
    pub template_section_open: bool,
    pub run_details_section_open: bool,
    pub template_editor_open: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct LayoutConfig {
    pub left_panel_width: i32,
    pub input_panel_visible: bool,
    pub operation_progress_visible: bool,
    pub left_header_meta_visible: bool,
    pub ai_warning_banner_visible: bool,
    pub preview_header_override_visible: bool,
    pub preview_context_visible: bool,
    pub preview_attention_visible: bool,
    pub signal_candidate_preview_visible: bool,
    pub prompt_lab: PromptLabLayoutConfig,
    pub active_tab: AppTab,
    pub left_tab: LeftTab,
}
pub fn initial_commands(window_id: WindowId) -> Vec<PlatformCommand> {
    let mut commands = Vec::new();
    theme::define_dark_theme_styles(&mut commands);
    init::create_controls(window_id, &mut commands);
    theme::apply_dark_theme(window_id, &mut commands);
    commands.push(build_layout_command(
        window_id,
        LayoutConfig {
            left_panel_width: DEFAULT_JOBS_PANEL_WIDTH,
            input_panel_visible: false,
            operation_progress_visible: false,
            left_header_meta_visible: false,
            ai_warning_banner_visible: false,
            preview_header_override_visible: false,
            preview_context_visible: false,
            preview_attention_visible: false,
            signal_candidate_preview_visible: false,
            active_tab: AppTab::Summary,
            left_tab: LeftTab::Jobs,
            prompt_lab: PromptLabLayoutConfig {
                visible: false,
                advanced_mode: false,
                compare_section_open: false,
                context_section_open: false,
                template_section_open: false,
                run_details_section_open: false,
                template_editor_open: false,
            },
        },
    ));
    commands
}
pub(crate) fn build_layout_command(window_id: WindowId, config: LayoutConfig) -> PlatformCommand {
    PlatformCommand::DefineLayout {
        window_id,
        rules: rules::build_layout_rules(
            config.left_panel_width,
            config.input_panel_visible,
            config.operation_progress_visible,
            config.left_header_meta_visible,
            config.ai_warning_banner_visible,
            config.preview_header_override_visible,
            config.preview_context_visible,
            config.preview_attention_visible,
            config.signal_candidate_preview_visible,
            config.prompt_lab,
            config.active_tab,
            config.left_tab,
        ),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
