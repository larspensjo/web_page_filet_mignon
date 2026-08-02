use crate::cli::Args;
use crate::runner::{
    apply_signal_candidate_selection_settings, build_effect_runner, exit_code_with_shutdown,
    is_ai_orchestration_enabled, maybe_dispatch_batch_ai_orchestration, should_log_batch_msg,
    summarize_batch_msg, CycleOutcome, DispatchLoopOptions, MAX_DISPATCH_INBOX_BATCH,
};
use engine_logging::{engine_debug, engine_info, engine_warn};
use harvester_core::{update, AppState, BatchObservation, CompletedJobSnapshot, ImportPhase, Msg};
use harvester_io::{
    load_completed_jobs, load_signal_candidate_cache, load_signal_candidate_overrides,
    load_summary_cache, persist_completed_jobs, EffectRunner, NoOpPlatformHandler, RuntimePaths,
};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Determines if an import-mode cycle should settle.
/// Import mode ignores poll/triage/job state; only waits for the import and its
/// downstream work (summaries or briefing) to complete.
fn should_settle_import_cycle(obs: &BatchObservation) -> bool {
    !obs.import_in_flight
        && !matches!(obs.import_phase, ImportPhase::Importing)
        && !matches!(
            obs.pre_triage_phase,
            harvester_core::PreTriagePhase::LoadingArticles
                | harvester_core::PreTriagePhase::Reviewing
                | harvester_core::PreTriagePhase::ReadyToTriage
        )
        && obs.triage_in_flight == 0
        && obs.triage_pending == 0
        && obs.summary_in_flight == 0
        && obs.summary_pending == 0
}

/// Classifies the outcome of a completed import-mode cycle.
fn classify_import_cycle_outcome(obs: &BatchObservation) -> CycleOutcome {
    let has_import_success = obs.imports_completed > 0;
    let has_import_failure =
        obs.imports_failed > 0 || matches!(obs.import_phase, ImportPhase::Failed);

    match (has_import_success, has_import_failure) {
        (true, false) => CycleOutcome::Success,
        (true, true) => CycleOutcome::PartialFailure,
        (false, true) => CycleOutcome::TotalFailure,
        // Idle means nothing was even attempted — treat as total failure.
        (false, false) => CycleOutcome::TotalFailure,
    }
}

/// Runs the import-mode workflow for browser-saved webpage imports.
///
/// Branches before source loading and drives only the import pipeline.
/// Exits after the import and any requested downstream work (summaries/briefing) settles.
pub(crate) fn run_import_mode(
    paths: &RuntimePaths,
    args: &Args,
    import_dir: std::path::PathBuf,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<i32, String> {
    engine_info!("[import] Starting import mode");
    let existing_completed_jobs = load_completed_jobs(&paths.state_path);

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(args.llm_concurrency);
    state.set_summary_max_in_flight(args.llm_concurrency);
    apply_signal_candidate_selection_settings(&mut state, args);

    let enable_ai_orchestration = is_ai_orchestration_enabled();
    let platform_handler = Box::new(NoOpPlatformHandler);
    let (effect_runner, _) = build_effect_runner(
        paths,
        msg_tx.clone(),
        args.llm_concurrency,
        platform_handler,
        false,
    )?;

    // Hydrate prompt/template metadata needed for downstream work.
    effect_runner.enqueue(vec![
        harvester_core::Effect::LoadPromptTemplateFiles,
        harvester_core::Effect::LoadLlmMetadata,
    ]);
    let (new_state, startup_effects) = update(state, Msg::StartupHydrationRequested);
    state = new_state;
    if !startup_effects.is_empty() {
        effect_runner.enqueue(startup_effects);
    }

    // Hydrate summary cache for cache-hit reuse during summaries.
    let summary_cache = load_summary_cache(&paths.summary_cache_path);
    if !summary_cache.is_empty() {
        let (new_state, effects) = update(
            state,
            Msg::SummaryCacheHydrated {
                cache: summary_cache,
            },
        );
        state = new_state;
        if !effects.is_empty() {
            effect_runner.enqueue(effects);
        }
    }
    match load_signal_candidate_cache(&paths.signal_candidate_cache_path) {
        Ok(signal_candidate_cache) if !signal_candidate_cache.is_empty() => {
            let (new_state, effects) = update(
                state,
                Msg::SignalCandidateCacheLoaded {
                    cache: signal_candidate_cache,
                },
            );
            state = new_state;
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
        }
        Ok(_) => {}
        Err(err) => engine_warn!(
            "[signal-cache] failed to hydrate {}: {}",
            paths.signal_candidate_cache_path.display(),
            err
        ),
    }
    match load_signal_candidate_overrides(&paths.signal_candidate_overrides_path) {
        Ok(signal_candidate_overrides) if !signal_candidate_overrides.is_empty() => {
            let (new_state, effects) = update(
                state,
                Msg::SignalCandidateOverridesLoaded {
                    overrides: signal_candidate_overrides,
                },
            );
            state = new_state;
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
        }
        Ok(_) => {}
        Err(err) => engine_warn!(
            "[signal-overrides] failed to hydrate {}: {}",
            paths.signal_candidate_overrides_path.display(),
            err
        ),
    }

    // Dispatch the import request.
    let (new_state, import_effects) =
        update(state, Msg::ImportSavedWebpagesRequested { dir: import_dir });
    state = new_state;
    effect_runner.enqueue(import_effects);

    let progress_enabled = std::io::stdout().is_terminal() && std::io::stderr().is_terminal();
    let mut progress = crate::progress::ImportProgressReporter::new(progress_enabled);
    progress.startup_line(&mut std::io::stdout());

    // Run the import dispatch loop until settled.
    let outcome = run_import_dispatch_loop(
        &mut state,
        &msg_tx,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            enable_ai_orchestration,
            require_new_jobs_since: None,
            tick_interval: Duration::from_millis(75),
        },
        Some(&mut progress),
    )?;

    let obs = state.batch_observation();
    engine_info!(
        "[import] Settled: phase={:?} imported={} failed={}",
        obs.import_phase,
        obs.imports_completed,
        obs.imports_failed,
    );

    // LlmHandle is owned by effect_runner; usage totals are not accessible here.
    // Printing "$0.00" would be incorrect when triage/summaries actually ran.
    let cost_display = "unavailable".to_string();
    progress.finish(&cost_display, &mut std::io::stdout());

    drop(effect_runner);
    let imported_completed_jobs = state.completed_jobs_snapshot();
    let merged_completed_jobs =
        merge_completed_jobs_for_import(existing_completed_jobs, imported_completed_jobs);
    engine_info!(
        "[import] Persisting completed jobs existing={} imported={} merged={}",
        merged_completed_jobs
            .len()
            .saturating_sub(obs.imports_completed),
        obs.imports_completed,
        merged_completed_jobs.len()
    );
    persist_completed_jobs(&paths.state_path, &merged_completed_jobs);

    Ok(exit_code_with_shutdown(
        match outcome {
            CycleOutcome::Success => 0,
            CycleOutcome::PartialFailure => 1,
            CycleOutcome::TotalFailure => 1,
        },
        shutdown_flag.load(Ordering::Relaxed),
    ))
}

fn merge_completed_jobs_for_import(
    existing_completed_jobs: Vec<CompletedJobSnapshot>,
    imported_completed_jobs: Vec<CompletedJobSnapshot>,
) -> Vec<CompletedJobSnapshot> {
    let mut merged = existing_completed_jobs;
    merged.extend(imported_completed_jobs);
    merged
}

/// Inner dispatch loop for import mode. Uses `should_settle_import_cycle` instead of
/// `should_settle_cycle`, and `classify_import_cycle_outcome` for the final result.
fn run_import_dispatch_loop(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
    mut progress: Option<&mut crate::progress::ImportProgressReporter>,
) -> Result<CycleOutcome, String> {
    let timeout = Duration::from_millis(100);
    let mut iterations = 0;
    let mut last_tick = Instant::now();
    let mut last_progress_render = Instant::now();
    const MAX_ITERATIONS: usize = 10_000;

    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            return Err(format!(
                "Import dispatch loop exceeded maximum iterations ({})",
                MAX_ITERATIONS
            ));
        }

        if shutdown_flag.load(Ordering::Relaxed) {
            engine_info!("[import] Shutdown signal detected");
            let obs = state.batch_observation();
            return Ok(classify_import_cycle_outcome(&obs));
        }

        match msg_rx.recv_timeout(timeout) {
            Ok(first_msg) => {
                let mut inbox = vec![first_msg];
                while inbox.len() < MAX_DISPATCH_INBOX_BATCH {
                    let Ok(next_msg) = msg_rx.try_recv() else {
                        break;
                    };
                    inbox.push(next_msg);
                }

                let mut queued_effects = Vec::new();
                for msg in inbox {
                    if should_log_batch_msg(&msg) {
                        engine_debug!("[import] Processing message: {}", summarize_batch_msg(&msg));
                    }
                    let (new_state, effects) = update(state.clone(), msg);
                    *state = new_state;
                    queued_effects.extend(effects);
                    if last_progress_render.elapsed() >= Duration::from_millis(250) {
                        if let Some(p) = progress.as_deref_mut() {
                            let obs = state.batch_observation();
                            p.update_from_obs(&obs, &mut std::io::stdout(), &mut std::io::stderr());
                        }
                        last_progress_render = Instant::now();
                    }
                }

                if let Some(triggered_by_job_done) =
                    state.take_pre_triage_refresh_evaluation_request()
                {
                    let ordered_urls = state.ordered_completed_job_urls_snapshot();
                    let (new_state, effects) = update(
                        state.clone(),
                        Msg::EvaluatePreTriageRefresh {
                            ordered_urls,
                            triggered_by_job_done,
                        },
                    );
                    *state = new_state;
                    queued_effects.extend(effects);
                }

                if !queued_effects.is_empty() {
                    effect_runner.enqueue(queued_effects);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Message channel disconnected unexpectedly".to_string());
            }
        }

        if options.enable_ai_orchestration && last_tick.elapsed() >= options.tick_interval {
            let (new_state, tick_effects) = update(state.clone(), Msg::Tick);
            *state = new_state;
            if !tick_effects.is_empty() {
                effect_runner.enqueue(tick_effects);
            }
            last_tick = Instant::now();
        }

        let mut orchestrated = false;
        if options.enable_ai_orchestration {
            if let Some(next_msg) = maybe_dispatch_batch_ai_orchestration(state) {
                msg_tx.send(next_msg).map_err(|e| {
                    format!("Failed to dispatch import orchestration message: {}", e)
                })?;
                orchestrated = true;
            }
        }

        let obs = state.batch_observation();
        if let Some(p) = progress.as_deref_mut() {
            p.update_from_obs(&obs, &mut std::io::stdout(), &mut std::io::stderr());
            last_progress_render = Instant::now();
        }

        if !orchestrated && should_settle_import_cycle(&obs) {
            engine_info!("[import] Cycle settled after {} iterations", iterations);
            return Ok(classify_import_cycle_outcome(&obs));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn observation_with_import(
        jobs_total: usize,
        jobs_done: usize,
        jobs_failed: usize,
        triage_completed: usize,
        triage_failed: usize,
        summary_completed: usize,
        summary_failed: usize,
        imports_completed: usize,
        imports_failed: usize,
    ) -> BatchObservation {
        BatchObservation {
            poll_in_progress: false,
            session_state: harvester_core::SessionState::Idle,
            jobs_total,
            jobs_done,
            jobs_failed,
            jobs_in_flight: 0,
            pre_triage_phase: harvester_core::PreTriagePhase::Idle,
            pre_triage_total: 0,
            pre_triage_included: 0,
            pre_triage_review: 0,
            pre_triage_filtered: 0,
            triage_phase: harvester_core::TriagePhase::Idle,
            triage_total: 0,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed,
            triage_failed,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed,
            summary_failed,
            triage_deferred: 0,
            summary_deferred: 0,
            signal_total: 0,
            signal_pending_or_in_flight: 0,
            signal_completed: 0,
            signal_failed: 0,
            signal_deferred: 0,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
            import_phase: harvester_core::ImportPhase::Idle,
            imports_completed,
            imports_failed,
            import_in_flight: false,
            source_poll_stats: vec![],
        }
    }

    fn idle_import_obs() -> BatchObservation {
        observation_with_import(0, 0, 0, 0, 0, 0, 0, 0, 0)
    }

    #[test]
    fn import_cycle_does_not_settle_while_triage_in_flight() {
        let mut obs = observation_with_import(0, 0, 0, 0, 0, 0, 0, 1, 0);
        obs.import_phase = harvester_core::ImportPhase::Idle;
        obs.import_in_flight = false;
        obs.triage_in_flight = 2;
        obs.triage_pending = 0;
        obs.summary_in_flight = 0;
        obs.summary_pending = 0;
        assert!(!should_settle_import_cycle(&obs));
    }

    #[test]
    fn import_cycle_does_not_settle_while_triage_pending() {
        let mut obs = observation_with_import(0, 0, 0, 0, 0, 0, 0, 1, 0);
        obs.import_phase = harvester_core::ImportPhase::Idle;
        obs.import_in_flight = false;
        obs.triage_in_flight = 0;
        obs.triage_pending = 3;
        obs.summary_in_flight = 0;
        obs.summary_pending = 0;
        assert!(!should_settle_import_cycle(&obs));
    }

    #[test]
    fn import_cycle_settles_when_all_phases_drained() {
        let mut obs = observation_with_import(0, 0, 0, 0, 0, 0, 0, 1, 0);
        obs.import_phase = harvester_core::ImportPhase::Idle;
        obs.import_in_flight = false;
        obs.triage_in_flight = 0;
        obs.triage_pending = 0;
        obs.summary_in_flight = 0;
        obs.summary_pending = 0;
        assert!(should_settle_import_cycle(&obs));
    }

    #[test]
    fn import_saved_web_dir_flag_is_parsed() {
        let args = crate::cli::Args::parse_from(&[
            "harvester_batch",
            "--import-saved-web-dir",
            "/tmp/saved",
        ]);
        assert_eq!(
            args.import_saved_web_dir,
            Some(std::path::PathBuf::from("/tmp/saved"))
        );
    }

    #[test]
    fn import_saved_web_dir_conflicts_with_dry_run() {
        let result = <crate::cli::Args as clap::Parser>::try_parse_from([
            "harvester_batch",
            "--import-saved-web-dir",
            "/tmp/saved",
            "--dry-run",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn import_saved_web_dir_conflicts_with_single_shot() {
        let result = <crate::cli::Args as clap::Parser>::try_parse_from([
            "harvester_batch",
            "--import-saved-web-dir",
            "/tmp/saved",
            "--single-shot",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn import_mode_persistence_merge_preserves_existing_jobs_and_appends_imports() {
        let existing = vec![harvester_core::CompletedJobSnapshot {
            url: "https://example.com/existing".to_string(),
            tokens: Some(10),
            bytes: Some(100),
            links: Vec::new(),
            fetched_utc: Some("2026-03-08T06:00:00Z".to_string()),
        }];
        let imported = vec![harvester_core::CompletedJobSnapshot {
            url: "https://example.com/imported".to_string(),
            tokens: None,
            bytes: None,
            links: Vec::new(),
            fetched_utc: Some("2026-03-08T06:01:56Z".to_string()),
        }];

        let merged = merge_completed_jobs_for_import(existing, imported);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].url, "https://example.com/existing");
        assert_eq!(merged[1].url, "https://example.com/imported");
        assert_eq!(
            merged[1].fetched_utc.as_deref(),
            Some("2026-03-08T06:01:56Z")
        );
    }

    #[test]
    fn should_settle_import_cycle_when_idle() {
        let obs = idle_import_obs();
        assert!(should_settle_import_cycle(&obs));
    }

    #[test]
    fn should_not_settle_import_cycle_when_in_flight() {
        let mut obs = idle_import_obs();
        obs.import_in_flight = true;
        obs.import_phase = harvester_core::ImportPhase::Importing;
        assert!(!should_settle_import_cycle(&obs));
    }

    #[test]
    fn should_not_settle_import_cycle_when_summaries_pending() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Complete;
        obs.summary_pending = 3;
        assert!(!should_settle_import_cycle(&obs));
    }

    #[test]
    fn should_settle_import_cycle_when_complete_and_no_pending() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Complete;
        obs.imports_completed = 2;
        assert!(should_settle_import_cycle(&obs));
    }

    #[test]
    fn should_not_settle_import_cycle_when_pre_triage_loading() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Complete;
        obs.imports_completed = 2;
        obs.pre_triage_phase = harvester_core::PreTriagePhase::LoadingArticles;
        assert!(!should_settle_import_cycle(&obs));
    }

    #[test]
    fn should_not_settle_import_cycle_when_pre_triage_reviewing() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Complete;
        obs.imports_completed = 2;
        obs.pre_triage_phase = harvester_core::PreTriagePhase::Reviewing;
        assert!(!should_settle_import_cycle(&obs));
    }

    #[test]
    fn should_not_settle_import_cycle_when_pre_triage_ready_to_triage() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Complete;
        obs.imports_completed = 2;
        obs.pre_triage_phase = harvester_core::PreTriagePhase::ReadyToTriage;
        assert!(!should_settle_import_cycle(&obs));
    }

    #[test]
    fn classify_import_cycle_success_when_all_imported() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Complete;
        obs.imports_completed = 3;
        assert_eq!(classify_import_cycle_outcome(&obs), CycleOutcome::Success);
    }

    #[test]
    fn classify_import_cycle_partial_when_some_failed() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Complete;
        obs.imports_completed = 2;
        obs.imports_failed = 1;
        assert_eq!(
            classify_import_cycle_outcome(&obs),
            CycleOutcome::PartialFailure
        );
    }

    #[test]
    fn classify_import_cycle_total_failure_when_zero_imported() {
        let mut obs = idle_import_obs();
        obs.import_phase = harvester_core::ImportPhase::Failed;
        obs.imports_failed = 2;
        assert_eq!(
            classify_import_cycle_outcome(&obs),
            CycleOutcome::TotalFailure
        );
    }
}
