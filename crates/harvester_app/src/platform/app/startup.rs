use super::ui;
use commanductui::{PlatformCommand, WindowId};
use engine_logging::engine_warn;
use harvester_core::{update, AiAvailability, AppState, AppViewModel, Effect, LlmQuotaLimits, Msg};
use harvester_io::{
    load_completed_jobs, load_pre_triage_overrides, load_signal_candidate_cache,
    load_signal_candidate_overrides, load_summary_cache, load_triage_cache, RuntimePaths,
};

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

    // Asynchronous startup hydration begins here. Reducer-owned startup
    // scheduling stays adjacent to the state transition that emits those effects.
    state = apply_startup_msg(state, Msg::StartupHydrationRequested, &mut startup_effects);

    let completed = load_completed_jobs(&paths.state_path);
    if !completed.is_empty() {
        state = apply_startup_msg(
            state,
            Msg::RestoreCompletedJobs(completed),
            &mut startup_effects,
        );
    }

    let summary_cache = load_summary_cache(&paths.summary_cache_path);
    if !summary_cache.is_empty() {
        state = apply_startup_msg(
            state,
            Msg::SummaryCacheHydrated {
                cache: summary_cache,
            },
            &mut startup_effects,
        );
    }

    let triage_cache = load_triage_cache(&paths.triage_cache_path);
    if !triage_cache.is_empty() {
        state = apply_startup_msg(
            state,
            Msg::TriageCacheHydrated {
                cache: triage_cache,
            },
            &mut startup_effects,
        );
    }

    match load_signal_candidate_cache(&paths.signal_candidate_cache_path) {
        Ok(signal_candidate_cache) if !signal_candidate_cache.is_empty() => {
            state = apply_startup_msg(
                state,
                Msg::SignalCandidateCacheLoaded {
                    cache: signal_candidate_cache,
                },
                &mut startup_effects,
            );
        }
        Ok(_) => {}
        Err(err) => {
            engine_warn!(
                "[signal-cache] failed to hydrate {}: {}",
                paths.signal_candidate_cache_path.display(),
                err
            );
        }
    }

    match load_signal_candidate_overrides(&paths.signal_candidate_overrides_path) {
        Ok(signal_candidate_overrides) if !signal_candidate_overrides.is_empty() => {
            state = apply_startup_msg(
                state,
                Msg::SignalCandidateOverridesLoaded {
                    overrides: signal_candidate_overrides,
                },
                &mut startup_effects,
            );
        }
        Ok(_) => {}
        Err(err) => {
            engine_warn!(
                "[signal-overrides] failed to hydrate {}: {}",
                paths.signal_candidate_overrides_path.display(),
                err
            );
        }
    }

    let overrides = load_pre_triage_overrides(&paths.state_path);
    if !overrides.is_empty() {
        state = apply_startup_msg(
            state,
            Msg::PreTriageOverridesHydrated { overrides },
            &mut startup_effects,
        );
    }

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
