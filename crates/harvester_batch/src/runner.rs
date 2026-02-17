use crate::cli::Args;
use crate::lock;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{update, AppState, Msg};
use harvester_io::{
    load_completed_jobs, load_sources, EffectRunner, NoOpPlatformHandler, RuntimePaths,
};
use std::sync::mpsc;

/// Run the batch orchestration loop
pub fn run(args: Args) -> Result<i32, String> {
    engine_info!("[batch] Initializing runtime paths");

    let paths = RuntimePaths::new(
        args.output_dir.clone(),
        args.sources.clone(),
        args.contexts_dir.clone(),
        args.prompts_dir.clone(),
    );

    engine_info!("[batch] Acquiring lock");
    let _lock_guard = lock::acquire_lock(&paths.output_dir, args.force_unlock)?;

    if args.dry_run {
        engine_info!("[batch] Dry-run mode: single poll only");
        return run_dry_run(&paths, &args);
    }

    // Validate source configuration
    engine_info!("[batch] Loading source registry from {:?}", paths.sources_path);
    let source_registry = load_sources(&paths.sources_path);

    if !args.allow_unsupported_sources {
        let unsupported: Vec<_> = source_registry
            .sources
            .iter()
            .filter_map(|s| match &s.source_type {
                harvester_engine::SourceType::Script { .. } => Some(s.id.to_string()),
                _ => None,
            })
            .collect();

        if !unsupported.is_empty() {
            return Err(format!(
                "Unsupported source types detected: {:?}. Use --allow-unsupported-sources to override.",
                unsupported
            ));
        }
    } else {
        let unsupported_count = source_registry
            .sources
            .iter()
            .filter(|s| matches!(&s.source_type, harvester_engine::SourceType::Script { .. }))
            .count();
        if unsupported_count > 0 {
            engine_warn!(
                "[batch] Running with {} unsupported source(s) (Script type)",
                unsupported_count
            );
        }
    }

    // Create message channel
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();

    // Hydrate state
    engine_info!("[batch] Hydrating state from disk");
    let mut state = AppState::new();
    state.set_triage_max_in_flight(args.llm_concurrency);
    state.set_summary_max_in_flight(args.llm_concurrency);

    // Restore completed jobs
    let completed_jobs = load_completed_jobs(&paths.state_path);
    if !completed_jobs.is_empty() {
        engine_info!("[batch] Restoring {} completed jobs", completed_jobs.len());
        let (new_state, effects) = update(state, Msg::RestoreCompletedJobs(completed_jobs));
        state = new_state;
        // Effects from restore are cache loads which will be executed shortly
        for effect in effects {
            engine_info!("[batch] Restore effect queued: {:?}", effect);
        }
    } else {
        engine_info!("[batch] No previous state found, starting fresh");
    }

    // Build EffectRunner (without LLM for now - TODO: add LLM support)
    engine_info!("[batch] Building EffectRunner");
    let platform_handler = Box::new(NoOpPlatformHandler);
    let _effect_runner = EffectRunner::new(paths.clone(), msg_tx.clone(), platform_handler);

    // TODO: Implement event loop
    engine_info!("[batch] Full batch mode event loop not yet implemented");

    // Clean shutdown
    drop(msg_rx);

    Ok(0)
}

fn run_dry_run(_paths: &RuntimePaths, _args: &Args) -> Result<i32, String> {
    // TODO: Implement dry-run mode
    println!("[dry-run] Not yet implemented");
    Ok(0)
}
