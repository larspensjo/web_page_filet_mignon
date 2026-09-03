use super::ui;
use commanductui::{PlatformCommand, WindowId};
use harvester_core::{update, AiAvailability, AppState, AppViewModel, Effect, LlmQuotaLimits, Msg};
use harvester_io::{host_bootstrap::hydrate_state_from_disk, RuntimePaths};

fn apply_startup_msg(state: AppState, msg: Msg, startup_effects: &mut Vec<Effect>) -> AppState {
    let (next_state, effects) = update(state, msg);
    startup_effects.extend(effects);
    next_state
}

pub(super) fn prepare_startup_state(
    mut state: AppState,
    paths: &RuntimePaths,
    initial_width: i32,
    llm_max_concurrent_requests: usize,
    startup_ai_availability: Option<AiAvailability>,
    llm_quota_limits: Option<LlmQuotaLimits>,
) -> (AppState, Vec<Effect>) {
    let mut startup_effects = Vec::new();

    // Synchronous startup preparation: seed all cheap, local facts before the
    // first view snapshot so the first visible frame is already correct.
    let (mut next_state, _) = update(
        state,
        Msg::WindowResized {
            window_width: initial_width,
        },
    );
    next_state.set_triage_max_in_flight(llm_max_concurrent_requests);
    next_state.set_summary_max_in_flight(llm_max_concurrent_requests);
    state = next_state;

    if let Some(availability) = startup_ai_availability {
        state = apply_startup_msg(
            state,
            Msg::AiAvailabilityDetected { availability },
            &mut startup_effects,
        );
    }
    if let Some(limits) = llm_quota_limits {
        state = apply_startup_msg(
            state,
            Msg::LlmQuotaConfigured { limits },
            &mut startup_effects,
        );
    }

    let (hydrated_state, hydration_effects) = hydrate_state_from_disk(state, paths);
    state = hydrated_state;
    startup_effects.extend(hydration_effects);

    (state, startup_effects)
}

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
