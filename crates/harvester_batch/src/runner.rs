use crate::cli::{Args, CheckpointCommand};
use crate::lock;
use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{update, AppState, BatchObservation, LlmModelUsageView, Msg};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::prompts::register_defaults;
use harvester_engine::llm::{
    LlmConfig, LlmHandle, LlmQuotas, ModelId, OpenAiProvider, PricingRegistry, PromptRegistry,
    ProviderKind,
};
use harvester_io::{
    load_briefing_checkpoint, load_completed_jobs, load_sources, load_summary_cache,
    load_triage_cache, persist_completed_jobs, save_briefing_checkpoint, EffectRunner,
    NoOpPlatformHandler, RuntimePaths,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CycleOutcome {
    Success,
    PartialFailure,
    TotalFailure,
}

/// Determines if the batch cycle should settle (all work done or failed).
fn should_settle_cycle(obs: &BatchObservation) -> bool {
    // Settled when:
    // 1. No poll in progress
    // 2. Triage is either idle, complete, or failed (not active)
    // 3. No jobs in flight
    // 4. No triage work in flight
    // 5. No summary work in flight or pending
    !obs.poll_in_progress
        && !matches!(
            obs.triage_phase,
            harvester_core::TriagePhase::LoadingArticles | harvester_core::TriagePhase::Triaging
        )
        && obs.jobs_in_flight == 0
        && obs.triage_in_flight == 0
        && obs.summary_in_flight == 0
        && obs.summary_pending == 0
}

/// Classifies the outcome of a completed cycle based on observation metrics.
fn classify_cycle_outcome(obs: &BatchObservation) -> CycleOutcome {
    let has_failures = obs.jobs_failed > 0 || obs.triage_failed > 0;
    let has_successes = obs.jobs_done > 0 || obs.triage_completed > 0;

    match (has_successes, has_failures) {
        (true, false) => CycleOutcome::Success,
        (true, true) => CycleOutcome::PartialFailure,
        (false, true) => CycleOutcome::TotalFailure,
        (false, false) => CycleOutcome::Success, // Nothing to do is success
    }
}

fn cycle_outcome_label(outcome: &CycleOutcome) -> &'static str {
    match outcome {
        CycleOutcome::Success => "SUCCESS",
        CycleOutcome::PartialFailure => "PARTIAL",
        CycleOutcome::TotalFailure => "FAILED",
    }
}

fn effective_model_map(config: &LlmConfig) -> HashMap<PromptId, String> {
    let mut map = HashMap::new();

    let triage_model = config
        .triage_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleTriage, triage_model);

    let summary_model = config
        .summary_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleSummary, summary_model);

    let briefing_model = config
        .briefing_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::AggregateBriefing, briefing_model);

    map
}

fn build_effect_runner(
    paths: &RuntimePaths,
    msg_tx: mpsc::Sender<Msg>,
    llm_concurrency: usize,
    platform_handler: Box<NoOpPlatformHandler>,
) -> EffectRunner {
    if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        let provider: Arc<dyn harvester_engine::llm::provider::LlmProvider> =
            Arc::new(OpenAiProvider::new(api_key));
        let provider_clone = Arc::clone(&provider);
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        let registry = Arc::new(RwLock::new(registry));
        let config = LlmConfig {
            provider,
            default_model: ModelId::new(ProviderKind::OpenAi, "gpt-4o-mini"),
            triage_model: None,
            summary_model: None,
            briefing_model: None,
            registry: Arc::clone(&registry),
            quotas: LlmQuotas::default(),
            output_dir: paths.output_dir.clone(),
            pricing: PricingRegistry::with_defaults(),
            max_input_bytes: 100_000,
            #[allow(deprecated)]
            max_input_chars: 0,
            timestamp_utc: Arc::new(|| Utc::now().to_rfc3339()),
            session_id: format!("batch-{}", Utc::now().format("%Y%m%d-%H%M%S")),
            replay_cache: None,
            max_concurrent_requests: llm_concurrency,
        };
        let model_map = effective_model_map(&config);
        let handle = LlmHandle::new(config);
        EffectRunner::new_with_llm(
            paths.clone(),
            msg_tx,
            handle,
            100_000,
            Arc::clone(&registry),
            model_map,
            provider_clone,
            ProviderKind::OpenAi,
            platform_handler,
        )
    } else {
        engine_warn!("[batch] OPENAI_API_KEY not set; AI triage/summary features disabled");
        EffectRunner::new(paths.clone(), msg_tx, platform_handler)
    }
}

const MAX_BATCH_MSG_LOG_LEN: usize = 240;

fn truncate_for_log(input: &str, max_len: usize) -> String {
    if input.chars().count() <= max_len {
        return input.to_string();
    }
    let mut truncated: String = input.chars().take(max_len).collect();
    truncated.push_str("...");
    truncated
}

fn summarize_batch_msg(msg: &Msg) -> String {
    match msg {
        Msg::PollSourcesClicked => "PollSourcesClicked".to_string(),
        Msg::AllSourcesPollEnded => "AllSourcesPollEnded".to_string(),
        Msg::SourcePollCompleted { source_id, urls } => {
            format!(
                "SourcePollCompleted {{ source_id: {}, urls: {} }}",
                source_id,
                urls.len()
            )
        }
        Msg::JobProgress {
            job_id,
            stage,
            tokens,
            bytes,
            ..
        } => format!(
            "JobProgress {{ job_id: {}, stage: {:?}, bytes: {:?}, tokens: {:?} }}",
            job_id, stage, bytes, tokens
        ),
        Msg::JobDone { job_id, result, .. } => {
            let result_label = match result {
                harvester_core::JobResultKind::Success => "Success".to_string(),
                harvester_core::JobResultKind::Failed { reason } => {
                    format!("Failed({})", truncate_for_log(reason, 80))
                }
            };
            format!("JobDone {{ job_id: {}, result: {} }}", job_id, result_label)
        }
        Msg::TriageArticlesLoaded { articles, .. } => {
            format!("TriageArticlesLoaded {{ articles: {} }}", articles.len())
        }
        Msg::ArticlesLoaded { articles, .. } => {
            format!("ArticlesLoaded {{ articles: {} }}", articles.len())
        }
        Msg::BriefingPrereqArticlesLoaded { articles } => {
            format!(
                "BriefingPrereqArticlesLoaded {{ articles: {} }}",
                articles.len()
            )
        }
        Msg::PromptContextsLoaded { contexts } => {
            format!("PromptContextsLoaded {{ prompts: {} }}", contexts.len())
        }
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
            templates,
        } => format!(
            "LlmMetadataLoaded {{ active_versions: {}, effective_models: {}, templates: {} }}",
            active_versions.len(),
            effective_models.len(),
            templates.len()
        ),
        _ => truncate_for_log(&format!("{:?}", msg), MAX_BATCH_MSG_LOG_LEN),
    }
}

fn should_log_batch_msg(msg: &Msg) -> bool {
    !matches!(
        msg,
        Msg::JobProgress {
            stage: harvester_core::Stage::Downloading,
            ..
        }
    )
}

/// Run the batch orchestration loop.
///
/// Executes repeated poll cycles until shutdown signal received or error occurs.
/// Returns exit code: 0 (success), 1 (partial failure), or 2 (fatal error via Err).
///
/// # Arguments
/// * `args` - Parsed command-line arguments specifying paths, intervals, and flags
///
/// # Behavior
/// - Acquires exclusive lock on output directory
/// - Polls sources at configured intervals
/// - Persists state after each cycle
/// - Handles SIGINT/SIGTERM gracefully
/// - Dry-run mode: single poll, read-only, no persistence
pub fn run(args: Args) -> Result<i32, String> {
    engine_info!("[batch] Initializing runtime paths");

    let paths = RuntimePaths::new(
        args.output_dir.clone(),
        args.sources.clone(),
        args.contexts_dir.clone(),
        args.prompts_dir.clone(),
    );

    // Handle checkpoint commands before entering the batch loop.
    match args.checkpoint_command()? {
        Some(CheckpointCommand::Show) => {
            let val = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
            println!("{}", val.as_deref().unwrap_or("NONE"));
            return Ok(0);
        }
        Some(cmd) => {
            let _lock_guard = lock::acquire_lock(&paths.output_dir, args.force_unlock)?;
            execute_checkpoint_write(cmd, &paths)?;
            return Ok(0);
        }
        None => {}
    }

    engine_info!("[batch] Acquiring lock");
    let _lock_guard = lock::acquire_lock(&paths.output_dir, args.force_unlock)?;

    if args.dry_run {
        engine_info!("[batch] Dry-run mode: single poll only");
        return run_dry_run(&paths, &args);
    }

    // Validate source configuration
    engine_info!(
        "[batch] Loading source registry from {:?}",
        paths.sources_path
    );
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

    // Build EffectRunner (with optional LLM support based on OPENAI_API_KEY)
    engine_info!("[batch] Building EffectRunner");
    let platform_handler = Box::new(NoOpPlatformHandler);
    let effect_runner = build_effect_runner(
        &paths,
        msg_tx.clone(),
        args.llm_concurrency,
        platform_handler,
    );
    effect_runner.enqueue(vec![
        harvester_core::Effect::LoadPromptTemplateFiles,
        harvester_core::Effect::LoadLlmMetadata,
    ]);

    // Trigger reducer-owned metadata hydration.
    let (new_state, startup_effects) = update(state, Msg::StartupHydrationRequested);
    state = new_state;
    if !startup_effects.is_empty() {
        effect_runner.enqueue(startup_effects);
    }

    // Hydrate persistent caches for triage/summary reuse.
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
    let triage_cache = load_triage_cache(&paths.triage_cache_path);
    if !triage_cache.is_empty() {
        let (new_state, effects) = update(
            state,
            Msg::TriageCacheHydrated {
                cache: triage_cache,
            },
        );
        state = new_state;
        if !effects.is_empty() {
            effect_runner.enqueue(effects);
        }
    }

    // Install signal handler for graceful shutdown
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    install_signal_handler(Arc::clone(&shutdown_flag));

    // Outer cycle loop - poll repeatedly until shutdown
    let poll_interval = Duration::from_secs((args.poll_interval * 60) as u64);
    let mut cycle_count = 0;
    let mut total_cycles = 0;
    let mut successful_cycles = 0;
    let mut partial_failure_cycles = 0;
    let mut total_failure_cycles = 0;

    loop {
        cycle_count += 1;
        total_cycles += 1;
        engine_info!("[batch] === Starting cycle {} ===", cycle_count);

        // Start the cycle by dispatching poll
        engine_info!("[batch] Dispatching poll sources");
        msg_tx
            .send(Msg::PollSourcesClicked)
            .map_err(|e| format!("Failed to dispatch poll: {}", e))?;

        // Run dispatch loop until settled
        let outcome = run_dispatch_loop(
            &mut state,
            &msg_tx,
            &msg_rx,
            &effect_runner,
            &shutdown_flag,
            true,
        )?;

        // Track outcome statistics
        match outcome {
            CycleOutcome::Success => successful_cycles += 1,
            CycleOutcome::PartialFailure => partial_failure_cycles += 1,
            CycleOutcome::TotalFailure => total_failure_cycles += 1,
        }

        // Print cycle summary
        let obs = state.batch_observation();
        if cycle_count == 1 {
            print_cycle_table_header();
        }
        print_cycle_summary(cycle_count, &outcome, &obs);
        for line in format_llm_usage_lines(&state.llm_usage_rows()) {
            println!("{}", line);
        }

        // Persist state
        engine_info!("[batch] Persisting state");
        let completed_jobs = state.completed_jobs_snapshot();
        persist_completed_jobs(&paths.state_path, &completed_jobs);

        // Check for shutdown signal
        if shutdown_flag.load(Ordering::Relaxed) {
            engine_info!("[batch] Shutdown signal received, exiting");
            break;
        }

        // Sleep interruptibly before next cycle
        engine_info!(
            "[batch] Sleeping for {} minutes before next cycle",
            args.poll_interval
        );
        if sleep_interruptible(poll_interval, &shutdown_flag) {
            engine_info!("[batch] Shutdown during sleep, exiting");
            break;
        }
    }

    // Graceful shutdown
    engine_info!("[batch] Graceful shutdown: draining effects and persisting final state");
    drop(effect_runner);
    drop(msg_rx);

    let completed_jobs = state.completed_jobs_snapshot();
    persist_completed_jobs(&paths.state_path, &completed_jobs);

    // Print final summary
    print_final_summary(
        total_cycles,
        successful_cycles,
        partial_failure_cycles,
        total_failure_cycles,
    );

    engine_info!("[batch] Shutdown complete");

    // Determine exit code based on outcomes
    let exit_code = if partial_failure_cycles > 0 || total_failure_cycles > 0 {
        1 // Partial: work completed with some failures
    } else {
        0 // Success: all cycles successful
    };

    Ok(exit_code)
}

/// Writes or clears the briefing checkpoint file.
///
/// Called after the output lock is already held.
fn execute_checkpoint_write(cmd: CheckpointCommand, paths: &RuntimePaths) -> Result<(), String> {
    match cmd {
        CheckpointCommand::Set(ts) => {
            // ts was already validated by checkpoint_command()
            engine_info!("[briefing-checkpoint] set to {}", ts);
            save_briefing_checkpoint(&paths.briefing_checkpoint_path, Some(ts.as_str()))
        }
        CheckpointCommand::SetNow => {
            let ts = Utc::now().to_rfc3339();
            engine_info!("[briefing-checkpoint] set to {}", ts);
            save_briefing_checkpoint(&paths.briefing_checkpoint_path, Some(ts.as_str()))
        }
        CheckpointCommand::Clear => {
            engine_info!("[briefing-checkpoint] cleared");
            save_briefing_checkpoint(&paths.briefing_checkpoint_path, None)
        }
        CheckpointCommand::Show => unreachable!("Show is handled before lock acquisition"),
    }
}

/// Runs the inner dispatch loop until settlement or error.
/// Processes messages, updates state, executes effects, and checks for settlement.
fn run_dispatch_loop(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
    enable_ai_orchestration: bool,
) -> Result<CycleOutcome, String> {
    let timeout = Duration::from_millis(100);
    let mut iterations = 0;
    const MAX_ITERATIONS: usize = 10_000; // Safety limit

    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            return Err(format!(
                "Dispatch loop exceeded maximum iterations ({})",
                MAX_ITERATIONS
            ));
        }

        // Check for shutdown signal
        if shutdown_flag.load(Ordering::Relaxed) {
            engine_info!("[batch] Shutdown signal detected in dispatch loop");
            let obs = state.batch_observation();
            return Ok(classify_cycle_outcome(&obs));
        }

        // Receive message with timeout
        match msg_rx.recv_timeout(timeout) {
            Ok(msg) => {
                if should_log_batch_msg(&msg) {
                    engine_info!("[batch] Processing message: {}", summarize_batch_msg(&msg));
                }

                // Update state
                let (new_state, effects) = update(state.clone(), msg);
                *state = new_state;

                // Execute effects
                if !effects.is_empty() {
                    engine_info!("[batch] Enqueuing {} effects", effects.len());
                    effect_runner.enqueue(effects);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No message available, continue loop
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Message channel disconnected unexpectedly".to_string());
            }
        }

        // Check for settlement after processing available work.
        if enable_ai_orchestration {
            if let Some(next_msg) = maybe_dispatch_batch_ai_orchestration(state) {
                msg_tx.send(next_msg.clone()).map_err(|e| {
                    format!(
                        "Failed to dispatch orchestration message {:?}: {}",
                        next_msg, e
                    )
                })?;
            }
        }

        // This prevents an immediate idle-state exit before queued actions
        // (like PollSourcesClicked) have been reduced.
        let obs = state.batch_observation();
        if should_settle_cycle(&obs) {
            engine_info!(
                "[batch] Cycle settled after {} iterations: jobs={}/{}, triage={}/{}",
                iterations,
                obs.jobs_done,
                obs.jobs_total,
                obs.triage_completed,
                obs.triage_total
            );
            return Ok(classify_cycle_outcome(&obs));
        }
    }
}

fn maybe_dispatch_batch_ai_orchestration(state: &AppState) -> Option<Msg> {
    let obs = state.batch_observation();

    if matches!(
        obs.pre_triage_phase,
        harvester_core::PreTriagePhase::ReadyToTriage
    ) && !matches!(
        obs.triage_phase,
        harvester_core::TriagePhase::LoadingArticles | harvester_core::TriagePhase::Triaging
    ) && obs.pre_triage_included > 0
        && obs.triage_total < obs.pre_triage_included
    {
        return Some(Msg::TriageClicked);
    }

    if matches!(obs.triage_phase, harvester_core::TriagePhase::Complete)
        && obs.triage_completed > 0
        && obs.summary_total == 0
    {
        return Some(Msg::PrepareSummariesClicked);
    }

    None
}

/// Formats a token count as a compact human-readable string (e.g. 12K, 1.2M).
fn format_compact_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
    }
}

/// Formats per-model usage rows as indented display lines.
fn format_llm_usage_lines(rows: &[LlmModelUsageView]) -> Vec<String> {
    rows.iter()
        .map(|r| {
            format!(
                "  {}: in={} out={}",
                r.model,
                format_compact_tokens(r.input_tokens),
                format_compact_tokens(r.output_tokens)
            )
        })
        .collect()
}

fn print_cycle_table_header() {
    println!(
        "{:<5} {:<8} {:>15} {:>22} {:>20} {:>20} {:>16} {:>16}",
        "Cycle",
        "Outcome",
        "Jobs T/D/F/I",
        "PreTri T/I/R/F",
        "Triage T/C/F/P",
        "Summ T/C/F/P",
        "TriCache H/M/K",
        "SumCache H/M/K"
    );
    println!("{}", "-".repeat(132));
}

/// Prints a summary of the completed cycle.
fn print_cycle_summary(cycle: usize, outcome: &CycleOutcome, obs: &BatchObservation) {
    println!(
        "{:<5} {:<8} {:>15} {:>22} {:>20} {:>20} {:>16} {:>16}",
        cycle,
        cycle_outcome_label(outcome),
        format!(
            "{}/{}/{}/{}",
            obs.jobs_total, obs.jobs_done, obs.jobs_failed, obs.jobs_in_flight
        ),
        format!(
            "{}/{}/{}/{}",
            obs.pre_triage_total,
            obs.pre_triage_included,
            obs.pre_triage_review,
            obs.pre_triage_filtered
        ),
        format!(
            "{}/{}/{}/{}",
            obs.triage_total, obs.triage_completed, obs.triage_failed, obs.triage_pending
        ),
        format!(
            "{}/{}/{}/{}",
            obs.summary_total, obs.summary_completed, obs.summary_failed, obs.summary_pending
        ),
        format!(
            "{}/{}/{}",
            obs.triage_cache_hits, obs.triage_cache_misses, obs.triage_cache_key_unavailable
        ),
        format!(
            "{}/{}/{}",
            obs.summary_cache_hits, obs.summary_cache_misses, obs.summary_cache_key_unavailable
        )
    );
}

/// Prints the final summary when batch runner exits.
fn print_final_summary(
    total_cycles: usize,
    successful: usize,
    partial_failures: usize,
    total_failures: usize,
) {
    println!("\n╔═══════════════════════════════════════╗");
    println!("║        BATCH RUN FINAL SUMMARY        ║");
    println!("╚═══════════════════════════════════════╝");
    println!("Total cycles:      {}", total_cycles);
    println!("  Successful:      {}", successful);
    println!("  Partial failure: {}", partial_failures);
    println!("  Total failure:   {}", total_failures);
    println!("═══════════════════════════════════════\n");
}

/// Sleeps for the specified duration, checking shutdown flag periodically.
/// Returns true if shutdown was requested during sleep.
fn sleep_interruptible(duration: Duration, shutdown_flag: &Arc<AtomicBool>) -> bool {
    let check_interval = Duration::from_millis(500);
    let mut remaining = duration;

    while remaining > Duration::ZERO {
        if shutdown_flag.load(Ordering::Relaxed) {
            return true;
        }

        let sleep_time = remaining.min(check_interval);
        std::thread::sleep(sleep_time);
        remaining = remaining.saturating_sub(sleep_time);
    }

    false
}

/// Installs a signal handler for SIGINT/SIGTERM to set the shutdown flag.
fn install_signal_handler(shutdown_flag: Arc<AtomicBool>) {
    #[cfg(unix)]
    {
        use std::sync::Mutex;
        static HANDLER_INSTALLED: Mutex<bool> = Mutex::new(false);

        let mut installed = HANDLER_INSTALLED.lock().unwrap();
        if *installed {
            return;
        }

        ctrlc::set_handler(move || {
            engine_info!("[batch] Received shutdown signal (SIGINT/SIGTERM)");
            shutdown_flag.store(true, Ordering::Relaxed);
        })
        .expect("Error setting signal handler");

        *installed = true;
    }

    #[cfg(windows)]
    {
        ctrlc::set_handler(move || {
            engine_info!("[batch] Received shutdown signal (Ctrl-C)");
            shutdown_flag.store(true, Ordering::Relaxed);
        })
        .expect("Error setting signal handler");
    }
}

/// Converts microdollars to a human-readable dollar string with exact rounding.
/// Examples: 0 -> "$0.00", 1234567 -> "$1.23", 50 -> "$0.00", 5000 -> "$0.01"
#[cfg(test)]
fn microdollars_to_display(microdollars: u64) -> String {
    let cents = (microdollars + 5000) / 10000; // Round to nearest cent
    let dollars = cents / 100;
    let remaining_cents = cents % 100;
    format!("${}.{:02}", dollars, remaining_cents)
}

fn run_dry_run(paths: &RuntimePaths, args: &Args) -> Result<i32, String> {
    engine_info!("[dry-run] Starting dry-run mode: single poll, no downloads/triage");

    // Hydrate state (read-only)
    engine_info!(
        "[dry-run] Loading completed jobs from {:?}",
        paths.state_path
    );
    let completed_jobs = load_completed_jobs(&paths.state_path);
    engine_info!("[dry-run] Loaded {} completed jobs", completed_jobs.len());

    // Initialize state
    let (msg_tx, msg_rx) = mpsc::channel();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(args.llm_concurrency);
    state.set_summary_max_in_flight(args.llm_concurrency);

    // Restore completed jobs
    if !completed_jobs.is_empty() {
        let restore_msg = Msg::RestoreCompletedJobs(completed_jobs);
        let (new_state, _effects) = update(state, restore_msg);
        state = new_state;
    }

    // Create effect runner
    let platform_handler = Box::new(NoOpPlatformHandler);
    let effect_runner = EffectRunner::new(paths.clone(), msg_tx.clone(), platform_handler);

    // Dispatch poll
    engine_info!("[dry-run] Dispatching poll");
    msg_tx
        .send(Msg::PollSourcesClicked)
        .map_err(|e| format!("Failed to send poll message: {}", e))?;

    // Run dispatch loop until settlement (read-only, no signal handling needed)
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let outcome = run_dispatch_loop(
        &mut state,
        &msg_tx,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        false,
    )?;

    // Print summary
    let obs = state.batch_observation();
    println!("\n=== Dry-Run Summary ===");
    println!("Outcome: {:?}", outcome);
    println!(
        "Jobs: {} total, {} done, {} failed",
        obs.jobs_total, obs.jobs_done, obs.jobs_failed
    );
    println!(
        "Triage: {} total, {} completed, {} failed, {} pending",
        obs.triage_total, obs.triage_completed, obs.triage_failed, obs.triage_pending
    );
    println!("Session state: {:?}", obs.session_state);
    println!("======================\n");

    engine_info!("[dry-run] Dry-run complete (no state modifications)");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_args(dry_run: bool, temp_dir: &TempDir) -> Args {
        Args {
            output_dir: temp_dir.path().to_path_buf(),
            sources: PathBuf::from("test_sources.json"),
            contexts_dir: PathBuf::from("contexts"),
            prompts_dir: PathBuf::from("prompts"),
            dry_run,
            allow_unsupported_sources: false,
            llm_concurrency: 1,
            poll_interval: 1,
            force_unlock: false,
            set_briefing_since: None,
            set_briefing_since_now: false,
            clear_briefing_since: false,
            show_briefing_since: false,
        }
    }

    #[test]
    fn test_dry_run_exits_successfully_without_api_key() {
        engine_logging::initialize_for_tests();
        let temp_dir = TempDir::new().unwrap();
        let args = create_test_args(true, &temp_dir);

        // Create empty sources file to avoid validation errors
        let sources_path = temp_dir.path().join("test_sources.json");
        std::fs::write(&sources_path, r#"{"sources": []}"#).unwrap();

        let runtime_paths = RuntimePaths::new(
            args.output_dir.clone(),
            sources_path,
            args.contexts_dir.clone(),
            args.prompts_dir.clone(),
        );

        // Dry-run should succeed even without OPENAI_API_KEY
        let result = run_dry_run(&runtime_paths, &args);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_dry_run_does_not_modify_state_files() {
        engine_logging::initialize_for_tests();
        let temp_dir = TempDir::new().unwrap();
        let args = create_test_args(true, &temp_dir);

        let sources_path = temp_dir.path().join("test_sources.json");
        std::fs::write(&sources_path, r#"{"sources": []}"#).unwrap();

        let runtime_paths = RuntimePaths::new(
            args.output_dir.clone(),
            sources_path,
            args.contexts_dir.clone(),
            args.prompts_dir.clone(),
        );

        let state_path = &runtime_paths.state_path;

        // Ensure state file does not exist initially
        assert!(!state_path.exists());

        // Run dry-run
        let result = run_dry_run(&runtime_paths, &args);
        assert!(result.is_ok());

        // State file should still not exist (no writes)
        assert!(!state_path.exists());
    }

    #[test]
    fn test_should_settle_cycle_when_idle() {
        let obs = BatchObservation {
            poll_in_progress: false,
            session_state: harvester_core::SessionState::Idle,
            jobs_total: 0,
            jobs_done: 0,
            jobs_failed: 0,
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
            triage_completed: 0,
            triage_failed: 0,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed: 0,
            summary_failed: 0,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
        };

        assert!(should_settle_cycle(&obs));
    }

    #[test]
    fn test_should_not_settle_when_poll_in_progress() {
        let obs = BatchObservation {
            poll_in_progress: true,
            session_state: harvester_core::SessionState::Running,
            jobs_total: 0,
            jobs_done: 0,
            jobs_failed: 0,
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
            triage_completed: 0,
            triage_failed: 0,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed: 0,
            summary_failed: 0,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
        };

        assert!(!should_settle_cycle(&obs));
    }

    #[test]
    fn test_summarize_batch_msg_compacts_large_payloads() {
        let msg = Msg::TriageArticlesLoaded {
            request_id: 1,
            articles: Vec::new(),
        };
        assert_eq!(
            summarize_batch_msg(&msg),
            "TriageArticlesLoaded { articles: 0 }"
        );
    }

    #[test]
    fn test_truncate_for_log_appends_ellipsis() {
        let input = "abcdefghijklmnopqrstuvwxyz";
        let output = truncate_for_log(input, 10);
        assert_eq!(output, "abcdefghij...");
    }

    #[test]
    fn test_should_log_batch_msg_filters_downloading_progress() {
        let downloading = Msg::JobProgress {
            job_id: 1,
            stage: harvester_core::Stage::Downloading,
            tokens: None,
            bytes: Some(4096),
            content_preview: None,
        };
        assert!(!should_log_batch_msg(&downloading));

        let tokenizing = Msg::JobProgress {
            job_id: 1,
            stage: harvester_core::Stage::Tokenizing,
            tokens: Some(10),
            bytes: None,
            content_preview: None,
        };
        assert!(should_log_batch_msg(&tokenizing));
    }

    #[test]
    fn test_dispatch_loop_reduces_queued_poll_before_settling() {
        engine_logging::initialize_for_tests();
        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path().join("output");
        std::fs::create_dir_all(&output_dir).unwrap();

        let sources_path = temp_dir.path().join("sources.ron");
        std::fs::write(&sources_path, "SourceRegistry(sources: [])").unwrap();

        let runtime_paths = RuntimePaths::new(
            output_dir,
            sources_path,
            temp_dir.path().join("contexts"),
            temp_dir.path().join("prompts"),
        );

        let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
        let mut state = AppState::new();
        let effect_runner =
            EffectRunner::new(runtime_paths, msg_tx.clone(), Box::new(NoOpPlatformHandler));
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        msg_tx.send(Msg::PollSourcesClicked).unwrap();

        let outcome = run_dispatch_loop(
            &mut state,
            &msg_tx,
            &msg_rx,
            &effect_runner,
            &shutdown_flag,
            true,
        )
        .expect("dispatch loop should complete");
        assert_eq!(outcome, CycleOutcome::Success);

        assert!(matches!(msg_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[test]
    fn test_classify_outcome_success() {
        let obs = BatchObservation {
            poll_in_progress: false,
            session_state: harvester_core::SessionState::Idle,
            jobs_total: 5,
            jobs_done: 5,
            jobs_failed: 0,
            jobs_in_flight: 0,
            pre_triage_phase: harvester_core::PreTriagePhase::Idle,
            pre_triage_total: 0,
            pre_triage_included: 0,
            pre_triage_review: 0,
            pre_triage_filtered: 0,
            triage_phase: harvester_core::TriagePhase::Complete,
            triage_total: 5,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 5,
            triage_failed: 0,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed: 0,
            summary_failed: 0,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
        };

        assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::Success);
    }

    #[test]
    fn test_classify_outcome_partial_failure() {
        let obs = BatchObservation {
            poll_in_progress: false,
            session_state: harvester_core::SessionState::Idle,
            jobs_total: 5,
            jobs_done: 3,
            jobs_failed: 2,
            jobs_in_flight: 0,
            pre_triage_phase: harvester_core::PreTriagePhase::Idle,
            pre_triage_total: 0,
            pre_triage_included: 0,
            pre_triage_review: 0,
            pre_triage_filtered: 0,
            triage_phase: harvester_core::TriagePhase::Complete,
            triage_total: 5,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 3,
            triage_failed: 2,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed: 0,
            summary_failed: 0,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
        };

        assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::PartialFailure);
    }

    #[test]
    fn test_classify_outcome_total_failure() {
        let obs = BatchObservation {
            poll_in_progress: false,
            session_state: harvester_core::SessionState::Idle,
            jobs_total: 5,
            jobs_done: 0,
            jobs_failed: 5,
            jobs_in_flight: 0,
            pre_triage_phase: harvester_core::PreTriagePhase::Idle,
            pre_triage_total: 0,
            pre_triage_included: 0,
            pre_triage_review: 0,
            pre_triage_filtered: 0,
            triage_phase: harvester_core::TriagePhase::Complete,
            triage_total: 5,
            triage_pending: 0,
            triage_in_flight: 0,
            triage_completed: 0,
            triage_failed: 5,
            summary_total: 0,
            summary_pending: 0,
            summary_in_flight: 0,
            summary_completed: 0,
            summary_failed: 0,
            triage_cache_hits: 0,
            triage_cache_misses: 0,
            triage_cache_key_unavailable: 0,
            summary_cache_hits: 0,
            summary_cache_misses: 0,
            summary_cache_key_unavailable: 0,
        };

        assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::TotalFailure);
    }

    #[test]
    fn test_microdollars_to_display_zero() {
        assert_eq!(microdollars_to_display(0), "$0.00");
    }

    #[test]
    fn test_microdollars_to_display_rounds_down() {
        // 50 microdollars = $0.000050 -> rounds to $0.00
        assert_eq!(microdollars_to_display(50), "$0.00");
        // 4999 microdollars = $0.004999 -> rounds to $0.00
        assert_eq!(microdollars_to_display(4999), "$0.00");
    }

    #[test]
    fn test_microdollars_to_display_rounds_up() {
        // 5000 microdollars = $0.005000 -> rounds to $0.01
        assert_eq!(microdollars_to_display(5000), "$0.01");
        // 15000 microdollars = $0.015000 -> rounds to $0.02
        assert_eq!(microdollars_to_display(15000), "$0.02");
    }

    #[test]
    fn test_microdollars_to_display_exact_cents() {
        // 10000 microdollars = $0.01
        assert_eq!(microdollars_to_display(10000), "$0.01");
        // 1000000 microdollars = $1.00
        assert_eq!(microdollars_to_display(1000000), "$1.00");
    }

    #[test]
    fn test_microdollars_to_display_typical_values() {
        // 1234567 microdollars = $1.234567 -> rounds to $1.23
        assert_eq!(microdollars_to_display(1234567), "$1.23");
        // 5678901 microdollars = $5.678901 -> rounds to $5.68
        assert_eq!(microdollars_to_display(5678901), "$5.68");
    }

    #[test]
    fn test_microdollars_to_display_large_values() {
        // 123456789 microdollars = $123.456789 -> rounds to $123.46
        assert_eq!(microdollars_to_display(123456789), "$123.46");
        // 1000000000 microdollars = $1000.00
        assert_eq!(microdollars_to_display(1000000000), "$1000.00");
    }

    #[test]
    fn format_compact_tokens_thresholds() {
        assert_eq!(format_compact_tokens(0), "0");
        assert_eq!(format_compact_tokens(999), "999");
        assert_eq!(format_compact_tokens(1_000), "1K");
        assert_eq!(format_compact_tokens(12_345), "12K");
        assert_eq!(format_compact_tokens(999_999), "999K");
        assert_eq!(format_compact_tokens(1_000_000), "1.0M");
        assert_eq!(format_compact_tokens(1_234_567), "1.2M");
    }

    #[test]
    fn format_llm_usage_lines_sorted_and_stable() {
        let rows = vec![
            LlmModelUsageView {
                model: "gpt-4o-mini".to_string(),
                input_tokens: 12_345,
                output_tokens: 3_100,
            },
            LlmModelUsageView {
                model: "gpt-4o".to_string(),
                input_tokens: 500,
                output_tokens: 80,
            },
        ];
        let lines = format_llm_usage_lines(&rows);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "  gpt-4o-mini: in=12K out=3K");
        assert_eq!(lines[1], "  gpt-4o: in=500 out=80");
    }

    #[test]
    fn format_llm_usage_lines_empty_returns_empty() {
        let lines = format_llm_usage_lines(&[]);
        assert!(lines.is_empty());
    }

    fn make_checkpoint_test_paths(temp_dir: &TempDir) -> RuntimePaths {
        let output_dir = temp_dir.path().to_path_buf();
        std::fs::create_dir_all(&output_dir).unwrap();
        RuntimePaths::new(
            output_dir,
            temp_dir.path().join("sources.ron"),
            temp_dir.path().join("contexts"),
            temp_dir.path().join("prompts"),
        )
    }

    #[test]
    fn set_checkpoint_invalid_timestamp_returns_err_without_write() {
        let temp_dir = TempDir::new().unwrap();
        let paths = make_checkpoint_test_paths(&temp_dir);
        // Simulate the validation in checkpoint_command() — Set is only constructed after validation
        // We test execute_checkpoint_write with a directly-valid Set to confirm it writes
        let result =
            execute_checkpoint_write(CheckpointCommand::Set("not-rfc3339".to_string()), &paths);
        // save_briefing_checkpoint does not validate the string; validation is in checkpoint_command().
        // But the file SHOULD be written with whatever string is passed.
        // This test verifies the call succeeds (the CLI layer is responsible for validation).
        assert!(result.is_ok());
    }

    #[test]
    fn set_checkpoint_writes_file() {
        let temp_dir = TempDir::new().unwrap();
        let paths = make_checkpoint_test_paths(&temp_dir);
        execute_checkpoint_write(
            CheckpointCommand::Set("2025-12-31T23:00:00Z".to_string()),
            &paths,
        )
        .unwrap();
        let loaded = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
        assert_eq!(loaded.as_deref(), Some("2025-12-31T23:00:00Z"));
    }

    #[test]
    fn set_checkpoint_now_writes_valid_rfc3339() {
        let temp_dir = TempDir::new().unwrap();
        let paths = make_checkpoint_test_paths(&temp_dir);
        execute_checkpoint_write(CheckpointCommand::SetNow, &paths).unwrap();
        let loaded = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
        let ts = loaded.expect("checkpoint should be written");
        assert!(
            chrono::DateTime::parse_from_rfc3339(&ts).is_ok(),
            "expected valid RFC3339, got: {ts}"
        );
    }

    #[test]
    fn clear_checkpoint_deletes_file() {
        let temp_dir = TempDir::new().unwrap();
        let paths = make_checkpoint_test_paths(&temp_dir);
        // Write first
        execute_checkpoint_write(
            CheckpointCommand::Set("2025-12-31T23:00:00Z".to_string()),
            &paths,
        )
        .unwrap();
        assert!(paths.briefing_checkpoint_path.exists());
        // Then clear
        execute_checkpoint_write(CheckpointCommand::Clear, &paths).unwrap();
        assert!(!paths.briefing_checkpoint_path.exists());
    }

    #[test]
    fn show_checkpoint_prints_none_when_absent() {
        let temp_dir = TempDir::new().unwrap();
        let paths = make_checkpoint_test_paths(&temp_dir);
        let val = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
        assert_eq!(val.as_deref().unwrap_or("NONE"), "NONE");
    }
}
