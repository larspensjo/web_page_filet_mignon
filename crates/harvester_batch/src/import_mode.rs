use crate::cli::Args;
use crate::no_progress::{pipeline_operation, NoProgressWatchdog};
use crate::runner::{
    apply_signal_candidate_selection_settings, batch_host_llm_defaults, exit_code_with_shutdown,
    should_log_batch_msg, summarize_batch_msg, CycleOutcome, CycleStartWorkReporter,
    DispatchLoopOptions, BATCH_EMPTY_API_KEY_WARNING, BATCH_MISSING_API_KEY_WARNING,
    MAX_DISPATCH_INBOX_BATCH,
};
use chrono::Utc;
use engine_logging::{engine_debug, engine_info, engine_warn};
use harvester_core::{update, AppState, BatchObservation, CompletedJobSnapshot, ImportPhase, Msg};
use harvester_io::{
    host_bootstrap::{build_effect_runner, pump_pre_triage_refresh},
    load_runtime_hydration, load_signal_candidate_overrides, persist_completed_jobs, EffectRunner,
    NoOpPlatformHandler, PersistenceWorker, RuntimePaths,
};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

fn import_is_terminal(obs: &BatchObservation) -> bool {
    !obs.import_in_flight
        && matches!(
            obs.import_phase,
            ImportPhase::Complete | ImportPhase::Failed
        )
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
    let hydration = load_runtime_hydration(&paths.state_path, &paths.output_dir);
    let existing_completed_jobs = hydration.jobs;

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let mut state = AppState::new();
    state.set_llm_max_in_flight(args.llm_concurrency);
    apply_signal_candidate_selection_settings(&mut state, args);

    let platform_handler = Box::new(NoOpPlatformHandler);
    let defaults = batch_host_llm_defaults();
    let (effect_runner, _, availability) = build_effect_runner(
        paths,
        msg_tx.clone(),
        args.llm_concurrency,
        &defaults,
        platform_handler,
        Box::new(PersistenceWorker::new(
            paths.state_path.clone(),
            paths.blacklist_path.clone(),
        )),
        BATCH_MISSING_API_KEY_WARNING,
        Some(BATCH_EMPTY_API_KEY_WARNING),
    )?;
    state = crate::runner::apply_llm_availability(state, availability);

    for message in hydration.notices {
        (state, _) = update(state, Msg::RuntimeStateNotice { message });
    }
    if hydration.recovery_needs_persist {
        // Import intentionally processes only arrivals. Persist startup recovery via
        // the same reducer/effect path without admitting the old corpus to this run.
        let (recovered, _) = update(
            AppState::new(),
            Msg::RestoreCompletedJobs(existing_completed_jobs.clone()),
        );
        let (recovered, _) = update(
            recovered,
            Msg::RestorePendingIntake(harvester_io::load_pending_intake(&paths.state_path)),
        );
        let (recovered, _) = update(
            recovered,
            Msg::BlacklistHydrated {
                state: harvester_io::load_blacklist(&paths.blacklist_path),
            },
        );
        let (_, effects) = update(recovered, Msg::FetchTimeRecoveryCompleted);
        effect_runner.enqueue(effects);
    }

    // Hydrate prompt/template metadata needed for downstream work.
    effect_runner.enqueue(vec![harvester_core::Effect::LoadPromptTemplateFiles]);
    let (new_state, startup_effects) = update(state, Msg::StartupHydrationRequested);
    state = new_state;
    if !startup_effects.is_empty() {
        effect_runner.enqueue(startup_effects);
    }

    let (next, effects) = harvester_io::host_bootstrap::hydrate_result_stores(state, paths);
    state = next;
    effect_runner.enqueue(effects);
    if let Some(reason) = state.result_store_failure() {
        eprintln!("AI features unavailable: {reason}");
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
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            tick_interval: Duration::from_millis(75),
            ..DispatchLoopOptions::default()
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

    // Ordering contract: flush and stop the runner's persistence sink before
    // this import-only path writes its authoritative merged job snapshot.
    let result_save_error = effect_runner.flush_results().err();
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
    let slim_jobs = merged_completed_jobs
        .into_iter()
        .map(|job| harvester_core::SlimJobRecord {
            url: job.url,
            tokens: job.tokens,
            bytes: job.bytes,
            fetched_utc: job.fetched_utc,
        })
        .collect::<Vec<_>>();
    persist_completed_jobs(&paths.state_path, &slim_jobs);

    Ok(exit_code_with_shutdown(
        if let Some(reason) = state
            .result_store_failure()
            .map(str::to_owned)
            .or_else(|| result_save_error.map(|e| e.to_string()))
        {
            eprintln!("Final summary: AI features unavailable: {reason}");
            1
        } else {
            match outcome {
                CycleOutcome::Success => 0,
                CycleOutcome::PartialFailure => 1,
                CycleOutcome::TotalFailure => 1,
            }
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

/// Inner dispatch loop for import mode. It starts a reducer-owned Resume run
/// once import is terminal and uses the shared pipeline settlement query.
fn run_import_dispatch_loop(
    state: &mut AppState,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
    progress: Option<&mut crate::progress::ImportProgressReporter>,
) -> Result<CycleOutcome, String> {
    run_import_dispatch_loop_with_sink(
        state,
        msg_rx,
        &mut |effects| effect_runner.enqueue(effects),
        shutdown_flag,
        options,
        progress,
        &mut |_| {},
    )
}

fn run_import_dispatch_loop_with_sink(
    state: &mut AppState,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_sink: &mut dyn FnMut(Vec<harvester_core::Effect>),
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
    mut progress: Option<&mut crate::progress::ImportProgressReporter>,
    iteration_observer: &mut dyn FnMut(usize),
) -> Result<CycleOutcome, String> {
    let timeout = options.receive_timeout;
    let mut iterations = 0;
    let mut last_tick = Instant::now();
    let mut last_progress_render = Instant::now();
    let mut resume_requested = false;
    let mut cycle_start_work = CycleStartWorkReporter::default();
    let mut watchdog = NoProgressWatchdog::new(Instant::now());

    loop {
        iterations += 1;

        if shutdown_flag.load(Ordering::Relaxed) {
            engine_info!("[import] Shutdown signal detected");
            let obs = state.batch_observation();
            return Ok(classify_import_cycle_outcome(&obs));
        }

        let mut received_message = false;
        match msg_rx.recv_timeout(timeout) {
            Ok(first_msg) => {
                received_message = true;
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
                    let (new_state, effects) = update(std::mem::take(state), msg);
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

                let current_state = std::mem::take(state);
                let (next_state, effects, _) = pump_pre_triage_refresh(current_state);
                *state = next_state;
                queued_effects.extend(effects);

                if !queued_effects.is_empty() {
                    effect_sink(queued_effects);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Message channel disconnected unexpectedly".to_string());
            }
        }

        if state.pipeline_run_phase() != harvester_core::PipelineRunPhase::Idle {
            let (new_state, effects) = update(std::mem::take(state), Msg::PipelineRunAdvance);
            *state = new_state;
            if !effects.is_empty() {
                effect_sink(effects);
            }
        }

        if resume_requested {
            let lines = cycle_start_work.pending_lines(state);
            if !lines.is_empty() {
                if let Some(p) = progress.as_deref_mut() {
                    p.suspend_for_output(&mut std::io::stdout());
                }
                for line in lines {
                    println!("{line}");
                }
            }
        }

        if (state.pipeline_run_phase() != harvester_core::PipelineRunPhase::Idle
            || state.pipeline_activity().intake_refresh_pending)
            && last_tick.elapsed() >= options.tick_interval
        {
            let (new_state, tick_effects) = update(std::mem::take(state), Msg::tick_at(Utc::now()));
            *state = new_state;
            if !tick_effects.is_empty() {
                effect_sink(tick_effects);
            }
            last_tick = Instant::now();
        }

        let obs = state.batch_observation();
        if !resume_requested
            && import_is_terminal(&obs)
            && !state.pipeline_activity().intake_refresh_pending
        {
            if let Some(p) = progress.as_deref_mut() {
                p.suspend_for_output(&mut std::io::stdout());
            }
            if let Some(line) = cycle_start_work.pending_count_line(state) {
                println!("{line}");
            }
            if let Some(p) = progress.as_deref_mut() {
                p.update_from_obs(&obs, &mut std::io::stdout(), &mut std::io::stderr());
            }
            engine_info!(
                "[import] Requesting resume pipeline run imported={} failed={}",
                obs.imports_completed,
                obs.imports_failed
            );
            let (new_state, effects) = update(
                std::mem::take(state),
                Msg::PipelineRunRequested {
                    scope: harvester_core::PipelineRunScope::Resume,
                },
            );
            *state = new_state;
            if !effects.is_empty() {
                effect_sink(effects);
            }
            resume_requested = true;
        }

        let obs = state.batch_observation();
        if let Some(p) = progress.as_deref_mut() {
            p.update_from_obs(&obs, &mut std::io::stdout(), &mut std::io::stderr());
            last_progress_render = Instant::now();
        }

        if resume_requested && harvester_core::BatchStatus::Settled == state.batch_status() {
            engine_info!("[import] Cycle settled after {} iterations", iterations);
            return Ok(classify_import_cycle_outcome(&obs));
        }
        watchdog.check(
            Instant::now(),
            received_message,
            state.pipeline_has_in_flight_work(),
            || {
                format!(
                    "importing saved browser pages phase={:?}; {}",
                    obs.import_phase,
                    pipeline_operation(state)
                )
            },
        )?;
        iteration_observer(iterations);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::{Effect, PipelineRunScope};
    use harvester_engine::llm::PromptId;
    use harvester_engine::{ImportReport, ImportedArchiveRef};
    use std::path::PathBuf;

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
            signal_total: 0,
            signal_pending_or_in_flight: 0,
            signal_completed: 0,
            signal_failed: 0,
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
    fn import_loop_survives_more_than_ten_thousand_quiet_import_iterations() {
        let (_msg_tx, msg_rx) = mpsc::channel();
        let (mut state, _) = update(
            AppState::new(),
            Msg::ImportSavedWebpagesRequested {
                dir: PathBuf::from("quiet-import"),
            },
        );
        assert!(state.batch_observation().import_in_flight);
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut iterations = 0;
        run_import_dispatch_loop_with_sink(
            &mut state,
            &msg_rx,
            &mut |_: Vec<Effect>| {},
            &shutdown,
            DispatchLoopOptions {
                receive_timeout: Duration::ZERO,
                tick_interval: Duration::ZERO,
            },
            None,
            &mut |count| {
                iterations = count;
                if count == 10_010 {
                    shutdown.store(true, Ordering::Relaxed);
                }
            },
        )
        .expect("quiet in-flight import must outlast the old iteration cap");
        assert_eq!(iterations, 10_010);
        assert!(state.batch_observation().import_in_flight);
    }

    #[test]
    fn import_resume_waits_for_import_terminal_without_owning_stage_settlement() {
        let mut obs = idle_import_obs();
        assert!(!import_is_terminal(&obs));

        obs.import_phase = ImportPhase::Importing;
        assert!(!import_is_terminal(&obs));

        obs.import_phase = ImportPhase::Complete;
        obs.import_in_flight = true;
        assert!(!import_is_terminal(&obs));

        obs.import_in_flight = false;
        obs.triage_pending = 1;
        // Import completion starts Resume; reducer-owned pipeline activity then
        // determines when the shared host loop settles.
        assert!(import_is_terminal(&obs));

        obs.import_phase = ImportPhase::Failed;
        assert!(import_is_terminal(&obs));
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

    #[test]
    fn imported_articles_enter_resume_run_and_reach_summaries() {
        let article = harvester_engine::LoadedArticle {
            url: "https://import.example/article".into(),
            source_title: Some("Imported article".into()),
            prepared_text: std::iter::repeat_n("importedword", 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: "imported-hash".into(),
            fetched_utc: Some("2026-09-27T00:00:00Z".into()),
        };
        let (state, _) = update(
            AppState::new(),
            Msg::LlmMetadataLoaded {
                active_versions: [
                    (PromptId::ArticleTriage, 1),
                    (PromptId::ArticleSummary, 1),
                    (PromptId::ArticleSignalCandidate, 1),
                ]
                .into_iter()
                .collect(),
                effective_models: [
                    (PromptId::ArticleTriage, "test-triage-model".into()),
                    (PromptId::ArticleSummary, "test-summary-model".into()),
                    (PromptId::ArticleSignalCandidate, "test-signal-model".into()),
                ]
                .into_iter()
                .collect(),
            },
        );
        let (state, _) = update(state, Msg::PromptTemplateFilesLoaded);
        let (state, _) = update(
            state,
            Msg::PromptContextsLoaded {
                contexts: Default::default(),
            },
        );
        let (state, effects) = update(
            state,
            Msg::ImportSavedWebpagesRequested {
                dir: PathBuf::from("/saved-pages"),
            },
        );
        let request_id = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::ImportSavedWebpages { request_id, .. } => Some(*request_id),
                _ => None,
            })
            .unwrap();
        let (state, _) = update(
            state,
            Msg::ImportSavedWebpagesCompleted {
                request_id,
                report: ImportReport {
                    scanned_count: 1,
                    imported_entries: vec![ImportedArchiveRef {
                        persisted_path: PathBuf::from("/archive/imported.md"),
                        canonical_url: article.url.clone(),
                        content_hash: article.content_hash.clone(),
                        fetched_utc: article.fetched_utc.clone().unwrap(),
                    }],
                    warnings: Vec::new(),
                    failures: Vec::new(),
                    duplicate_url_count: 0,
                    duplicate_content_count: 0,
                },
            },
        );
        let (state, effects) = update(
            state,
            Msg::PipelineRunRequested {
                scope: PipelineRunScope::Resume,
            },
        );
        let (state, effects) = harvester_core::fixture_support::complete_processing_configuration(
            state, effects, 100_000,
        );
        let load_id = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            })
            .expect("the import Resume run loads its completed archive window");
        let (state, effects) = update(
            state,
            Msg::TriageArticlesLoaded {
                request_id: load_id,
                delta: harvester_engine::TriageArticleDelta::full_window(
                    vec![article.clone()],
                    100_000,
                ),
            },
        );
        let triage_id = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::RequestLlmCompletion {
                    request_id,
                    prompt_id: PromptId::ArticleTriage,
                    ..
                } => Some(*request_id),
                _ => None,
            })
            .expect("the imported article is triaged");
        let (state, effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: triage_id,
                result: harvester_core::LlmResultKind::Success {
                    output_json: r#"{"category":"news","priority":3,"tags":["import"],"rationale":"imported article"}"#.into(),
                    input_tokens: 20,
                    output_tokens: 8,
                    prompt_version: 1,
                    resolved_model: "test-triage-model".into(),
                },
                metadata: None,
            },
        );
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::ArticleSummary,
                ..
            }
        )));
        assert_eq!(state.batch_observation().imports_completed, 1);
        assert_eq!(state.batch_observation().summary_total, 1);
    }
}
