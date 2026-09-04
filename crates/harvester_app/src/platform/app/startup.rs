use super::ui;
use commanductui::{PlatformCommand, WindowId};
use harvester_core::AppViewModel;

pub(super) fn assemble_startup_commands(
    window_id: WindowId,
    initial_view: &AppViewModel,
    tree_render_state: &mut ui::render::TreeRenderState,
) -> Vec<PlatformCommand> {
    let mut initial_commands = ui::layout::initial_commands(window_id);
    initial_commands.extend(ui::render::render(
        window_id,
        initial_view,
        tree_render_state,
    ));

    // Reveal ownership stays at the app layer so first render and first reveal
    // remain one explicit, testable contract.
    initial_commands.push(PlatformCommand::SignalMainWindowUISetupComplete { window_id });
    initial_commands.push(PlatformCommand::ShowWindow { window_id });
    initial_commands
}
