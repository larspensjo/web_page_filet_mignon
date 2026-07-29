use crate::cli::{Args, CheckpointCommand};
use crate::lock;
use crate::progress::{
    BatchDisplayPhase, BatchProgressProjection, BatchProgressSnapshot, BatchRunBaseline,
    PassCounts, PlainProgressReporter, ProgressClock, ProgressGlyphs, ProgressReporter,
    ProjectionContext, SystemProgressClock, TerminalProgressSurface,
};
use crate::{
    batch_coordinator::{BatchCoordinator, BatchPeek, BufferedRequest, SubmissionBudget},
    batch_manifest::{BatchManifestStore, PendingEntry},
};
use chrono::Utc;
use crossterm::{cursor::Show, QueueableCommand};
use engine_logging::{engine_debug, engine_info, engine_warn};
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
use harvester_core::{
    update, AppState, ArticleSummaryResult, BatchObservation, CompletedJobSnapshot, FrozenBatchKey,
    ImportPhase, LlmModelUsageView, Msg, SignalCandidateCacheKey, StageKind, SummaryCache,
    SummaryCacheEntry, SummaryCacheKey, TriageCacheKey,
};
use harvester_engine::llm::prompt::{PromptId, PromptTemplateOwned, PROMPT_VERSION_DRAFT};
use harvester_engine::llm::prompts::register_defaults;
use harvester_engine::llm::{
    load_context_file, validate_summary, LlmCommand, LlmCompletionCommand, LlmCompletionError,
    LlmConfig, LlmEvent, LlmHandle, LlmQuotas, ModelId, OpenAiProvider, PricingRegistry,
    PromptRegistry, ProviderKind, ReplayRecord, TokenUsage, DEFAULT_BRIEFING_MODEL,
    DEFAULT_SUMMARY_MODEL, DEFAULT_TRIAGE_MODEL, OPENAI_MODEL_GPT_4O_MINI,
};
use harvester_engine::{
    ensure_output_dir, load_and_prepare_articles_filtered, scan_archive_article_metadata,
    AtomicFileWriter,
};
use harvester_io::{
    load_blacklist, load_briefing_checkpoint, load_completed_jobs, load_entity_index,
    load_prompt_templates, load_signal_candidate_cache, load_signal_candidate_overrides,
    load_sources, load_summary_cache, load_triage_cache, persist_completed_jobs,
    persist_summary_cache, save_blacklist, save_briefing_checkpoint, save_entity_index,
    upsert_entry, EffectRunner, EntityIndexPatch, NoOpPlatformHandler, RuntimePaths,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

struct BatchRuntime {
    coordinator: BatchCoordinator<OpenAiProvider>,
    config: LlmConfig,
    runtime: tokio::runtime::Runtime,
    reconciled: bool,
    realized_cost_microdollars: u64,
    recorded_replay_lines: HashSet<String>,
}

impl BatchRuntime {
    fn new(
        provider: OpenAiProvider,
        config: LlmConfig,
        paths: &RuntimePaths,
    ) -> Result<Self, String> {
        let manifest =
            BatchManifestStore::load(paths.output_dir.clone()).map_err(|err| err.to_string())?;
        let budget = SubmissionBudget::from_quotas(&config.quotas);
        let recorded_replay_lines = load_recorded_batch_replay_lines(&config.replay_output_dir());
        Ok(Self {
            coordinator: BatchCoordinator::new(provider, manifest, budget),
            config,
            runtime: tokio::runtime::Runtime::new().map_err(|err| err.to_string())?,
            reconciled: false,
            realized_cost_microdollars: 0,
            recorded_replay_lines,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CycleOutcome {
    Success,
    PartialFailure,
    TotalFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CycleCounts {
    new_jobs: usize,
    jobs_done: usize,
    jobs_failed: usize,
    triage_completed: usize,
    triage_failed: usize,
    summary_completed: usize,
    summary_failed: usize,
    imports_completed: usize,
    imports_failed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CycleCounterBaseline {
    jobs_total: usize,
    jobs_done: usize,
    jobs_failed: usize,
    triage_completed: usize,
    triage_failed: usize,
    summary_completed: usize,
    summary_failed: usize,
    imports_completed: usize,
    imports_failed: usize,
}

#[derive(Debug, Clone, Copy)]
struct DispatchLoopOptions {
    enable_ai_orchestration: bool,
    require_new_jobs_since: Option<usize>,
    tick_interval: Duration,
}

const MAX_DISPATCH_INBOX_BATCH: usize = 32;
const BATCH_WAIT_INTERVAL: Duration = Duration::from_secs(5 * 60);
const MAX_CONSECUTIVE_BATCH_COLLECT_NO_PROGRESS: usize = 2;
const PROGRESS_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const PLAIN_PROGRESS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const LOCAL_WAIT_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(500);

enum BatchProgressSurface<W: Write> {
    Terminal(TerminalProgressSurface<W>),
    Plain(PlainProgressReporter<W>),
}

impl BatchProgressSurface<std::io::Stdout> {
    fn new(interactive: bool, ascii_progress: bool) -> Self {
        if interactive {
            Self::Terminal(TerminalProgressSurface::new(
                std::io::stdout(),
                progress_glyphs(ascii_progress),
            ))
        } else {
            Self::Plain(PlainProgressReporter::new(std::io::stdout()))
        }
    }
}

fn progress_glyphs(ascii_progress: bool) -> ProgressGlyphs {
    if ascii_progress {
        ProgressGlyphs::Ascii
    } else {
        ProgressGlyphs::Unicode
    }
}

fn batch_mode_label(batch_api_enabled: bool, drain: bool) -> &'static str {
    match (batch_api_enabled, drain) {
        (_, true) => "drain",
        (true, false) => "batch-api",
        (false, false) => "recurring",
    }
}

impl<W: Write> BatchProgressSurface<W> {
    fn paint(&mut self, snapshot: &BatchProgressSnapshot) {
        let result = match self {
            Self::Terminal(surface) => surface.repaint(snapshot),
            Self::Plain(reporter) => reporter.report(snapshot),
        };
        if let Err(err) = result {
            engine_warn!(
                "[batch-progress] stdout repaint failed; continuing safely: {}",
                err
            );
        }
    }

    fn suspend_for_output(&mut self) {
        if let Self::Terminal(surface) = self {
            if let Err(err) = surface.suspend_for_output() {
                engine_warn!("[batch-progress] failed to suspend dashboard: {}", err);
            }
        }
    }

    fn resume(&mut self, snapshot: &BatchProgressSnapshot) {
        if let Self::Terminal(surface) = self {
            if let Err(err) = surface.resume(snapshot) {
                engine_warn!("[batch-progress] failed to resume dashboard: {}", err);
            }
        } else {
            self.paint(snapshot);
        }
    }

    fn finish(&mut self) {
        if let Self::Terminal(surface) = self {
            if let Err(err) = surface.finish() {
                engine_warn!("[batch-progress] failed to finish dashboard: {}", err);
            }
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, Self::Terminal(_))
    }
}

struct LiveBatchProgress<C: ProgressClock, W: Write> {
    clock: C,
    projection: BatchProgressProjection,
    surface: BatchProgressSurface<W>,
    phase_override: Option<BatchDisplayPhase>,
    peeks: Vec<BatchPeek>,
    last_provider_check: Option<Instant>,
    next_provider_check: Option<Instant>,
    last_provider_check_local: Option<chrono::DateTime<chrono::FixedOffset>>,
    next_provider_check_local: Option<chrono::DateTime<chrono::FixedOffset>>,
    pass_counts: PassCounts,
    last_render: Instant,
    last_plain_phase: Option<BatchDisplayPhase>,
}

type LiveSystemBatchProgress = LiveBatchProgress<SystemProgressClock, std::io::Stdout>;

impl LiveBatchProgress<SystemProgressClock, std::io::Stdout> {
    fn new(baseline: BatchRunBaseline, interactive: bool, ascii_progress: bool) -> Self {
        let clock = SystemProgressClock;
        let surface = BatchProgressSurface::new(interactive, ascii_progress);
        Self::with_parts(baseline, clock, surface)
    }
}

impl<C: ProgressClock, W: Write> LiveBatchProgress<C, W> {
    fn with_parts(baseline: BatchRunBaseline, clock: C, surface: BatchProgressSurface<W>) -> Self {
        let started_at = clock.monotonic_now();
        Self {
            clock,
            projection: BatchProgressProjection::new(baseline, started_at),
            surface,
            phase_override: None,
            peeks: Vec::new(),
            last_provider_check: None,
            next_provider_check: None,
            last_provider_check_local: None,
            next_provider_check_local: None,
            pass_counts: PassCounts::default(),
            last_render: started_at,
            last_plain_phase: None,
        }
    }

    fn set_phase(&mut self, phase: BatchDisplayPhase) {
        self.phase_override = Some(phase);
    }

    fn clear_phase_override(&mut self) {
        self.phase_override = None;
    }

    fn set_provider_check(&mut self, peeks: Vec<BatchPeek>) {
        self.peeks = retain_last_successful_provider_counts(&self.peeks, peeks);
        let now = self.clock.monotonic_now();
        let local_now = self.clock.wall_now();
        let wait_interval = chrono::Duration::from_std(BATCH_WAIT_INTERVAL)
            .expect("batch wait interval fits chrono duration");
        self.last_provider_check = Some(now);
        self.next_provider_check = Some(now + BATCH_WAIT_INTERVAL);
        self.last_provider_check_local = Some(local_now);
        self.next_provider_check_local = Some(local_now + wait_interval);
    }

    fn set_provider_wait_render(&mut self, render: &ProviderWaitRender) {
        self.set_phase(render.phase);
        self.peeks = retain_last_successful_provider_counts(&self.peeks, render.peeks.clone());
        self.last_provider_check = render.last_provider_check;
        self.next_provider_check = render.next_provider_check;
        self.last_provider_check_local = render.last_provider_check_local;
        self.next_provider_check_local = render.next_provider_check_local;
    }

    fn snapshot(
        &mut self,
        state: &AppState,
        cost_this_run_microdollars: u64,
    ) -> BatchProgressSnapshot {
        self.projection.snapshot(
            &state.batch_observation(),
            &self.peeks,
            ProjectionContext {
                phase_override: self.phase_override,
                pass_counts: self.pass_counts,
                cost_this_run_microdollars,
                last_provider_check: self.last_provider_check,
                next_provider_check: self.next_provider_check,
                last_provider_check_local: self.last_provider_check_local,
                next_provider_check_local: self.next_provider_check_local,
            },
            &self.clock,
        )
    }

    fn paint(&mut self, state: &AppState, cost: u64, force: bool) {
        let now = self.clock.monotonic_now();
        let due = now.saturating_duration_since(self.last_render) >= PROGRESS_REFRESH_INTERVAL;
        let plain_due =
            now.saturating_duration_since(self.last_render) >= PLAIN_PROGRESS_HEARTBEAT_INTERVAL;
        if !force && !due {
            return;
        }
        let snapshot = self.snapshot(state, cost);
        let phase_changed = self.last_plain_phase != Some(snapshot.phase);
        if !self.surface.is_terminal() && !force && !phase_changed && !plain_due {
            return;
        }
        self.surface.paint(&snapshot);
        self.last_render = now;
        self.last_plain_phase = Some(snapshot.phase);
    }

    fn suspend_for_output(&mut self) {
        self.surface.suspend_for_output();
    }

    fn resume(&mut self, state: &AppState, cost: u64) {
        let snapshot = self.snapshot(state, cost);
        self.surface.resume(&snapshot);
        self.last_render = self.clock.monotonic_now();
        self.last_plain_phase = Some(snapshot.phase);
    }

    fn finish(&mut self) {
        self.surface.finish();
    }

    fn record_pass(&mut self, collect_only: bool) {
        self.pass_counts.intake_passes = 1;
        if collect_only {
            self.pass_counts.collection_passes =
                self.pass_counts.collection_passes.saturating_add(1);
        }
    }
}

/// Retains the last successful provider counts when a status lookup fails.
/// The current failed lookup remains in the result, so the pure formatter keeps
/// showing its retry/indeterminate state rather than pretending the old result
/// was fresh.
fn retain_last_successful_provider_counts(
    previous: &[BatchPeek],
    current: Vec<BatchPeek>,
) -> Vec<BatchPeek> {
    let mut retained: Vec<_> = previous
        .iter()
        .filter(|peek| peek.status.is_some() && peek.request_counts.is_some())
        .cloned()
        .collect();
    retained.retain(|old| {
        !current
            .iter()
            .any(|new| new.batch_id == old.batch_id && new.status.is_some())
    });
    retained.extend(current);
    retained
}

#[derive(Debug, Clone)]
struct ProviderWaitRender {
    phase: BatchDisplayPhase,
    peeks: Vec<BatchPeek>,
    last_provider_check: Option<Instant>,
    next_provider_check: Option<Instant>,
    last_provider_check_local: Option<chrono::DateTime<chrono::FixedOffset>>,
    next_provider_check_local: Option<chrono::DateTime<chrono::FixedOffset>>,
}

enum ProviderWaitOutcome {
    Collect(Vec<BatchPeek>),
    Shutdown,
}

/// Waits locally between existing provider peeks. The injected clock makes the
/// heartbeat cadence and shutdown polling deterministic without changing
/// coordinator transport or provider-check frequency.
fn run_provider_wait_loop<C, P, R>(
    clock: &C,
    shutdown_flag: &AtomicBool,
    mut latest_peeks: Vec<BatchPeek>,
    mut peek: P,
    mut render: R,
) -> ProviderWaitOutcome
where
    C: ProgressClock,
    P: FnMut() -> Vec<BatchPeek>,
    R: FnMut(ProviderWaitRender),
{
    let mut last_provider_check = None;
    let mut next_provider_check = None;
    let mut last_provider_check_local = None;
    let mut next_provider_check_local = None;
    loop {
        render(ProviderWaitRender {
            phase: BatchDisplayPhase::CheckingProvider,
            peeks: latest_peeks.clone(),
            last_provider_check,
            next_provider_check,
            last_provider_check_local,
            next_provider_check_local,
        });
        latest_peeks = peek();
        // A signal cannot be observed during the synchronous provider call,
        // but it must win as soon as that atomic call returns.
        if shutdown_flag.load(Ordering::Relaxed) {
            return ProviderWaitOutcome::Shutdown;
        }
        if decide_batch_wait(&latest_peeks) == BatchWaitDecision::RunCollectCycle {
            return ProviderWaitOutcome::Collect(latest_peeks);
        }

        let checked_at = clock.monotonic_now();
        let next_check = checked_at + BATCH_WAIT_INTERVAL;
        let checked_at_local = clock.wall_now();
        let next_check_local = checked_at_local
            + chrono::Duration::from_std(BATCH_WAIT_INTERVAL)
                .expect("batch wait interval fits chrono duration");
        last_provider_check = Some(checked_at);
        next_provider_check = Some(next_check);
        last_provider_check_local = Some(checked_at_local);
        next_provider_check_local = Some(next_check_local);
        let mut next_refresh = checked_at;
        loop {
            if shutdown_flag.load(Ordering::Relaxed) {
                return ProviderWaitOutcome::Shutdown;
            }
            let now = clock.monotonic_now();
            if now >= next_check {
                break;
            }
            if now >= next_refresh {
                render(ProviderWaitRender {
                    phase: BatchDisplayPhase::WaitingForProvider,
                    peeks: latest_peeks.clone(),
                    last_provider_check,
                    next_provider_check,
                    last_provider_check_local,
                    next_provider_check_local,
                });
                next_refresh = now + LOCAL_WAIT_REFRESH_INTERVAL;
            }
            let until_refresh = next_refresh.saturating_duration_since(now);
            let until_check = next_check.saturating_duration_since(now);
            let sleep_for = SHUTDOWN_POLL_INTERVAL.min(until_refresh).min(until_check);
            if !sleep_for.is_zero() {
                clock.sleep(sleep_for);
            }
        }
    }
}

/// Local-only retry delay used by the no-progress safeguard. It deliberately
/// shares the same 500 ms shutdown polling and one-second presentation cadence
/// as the provider wait, while making no provider calls.
fn wait_with_local_heartbeat<C, R>(
    clock: &C,
    shutdown_flag: &AtomicBool,
    duration: Duration,
    mut render: R,
) -> bool
where
    C: ProgressClock,
    R: FnMut(),
{
    let deadline = clock.monotonic_now() + duration;
    let mut next_refresh = clock.monotonic_now();
    loop {
        if shutdown_flag.load(Ordering::Relaxed) {
            return true;
        }
        let now = clock.monotonic_now();
        if now >= deadline {
            return false;
        }
        if now >= next_refresh {
            render();
            next_refresh = now + LOCAL_WAIT_REFRESH_INTERVAL;
        }
        let sleep_for = SHUTDOWN_POLL_INTERVAL
            .min(next_refresh.saturating_duration_since(now))
            .min(deadline.saturating_duration_since(now));
        if !sleep_for.is_zero() {
            clock.sleep(sleep_for);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchWaitDecision {
    KeepWaiting,
    RunCollectCycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BatchDrainSnapshot {
    pending_manifest_batches: Vec<(String, Option<String>)>,
    triage_deferred: usize,
    summary_deferred: usize,
    signal_deferred: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryRefreshTarget {
    primary_url: String,
    content_hash: String,
    cache_key: SummaryCacheKey,
    related_urls: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct SummaryRefreshSelection {
    targets: Vec<SummaryRefreshTarget>,
    total_stale: usize,
}

type SummaryRefreshRuntime = (
    LlmHandle,
    Arc<RwLock<PromptRegistry>>,
    u32,
    String,
    Vec<(String, String)>,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SummaryRefreshFailure {
    request_id: Option<u64>,
    url: String,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SummaryRefreshReport {
    started_at_utc: String,
    finished_at_utc: String,
    status: String,
    output_dir: String,
    prompt_version: u32,
    configured_model: String,
    limit: usize,
    stale_total_before: usize,
    selected: usize,
    attempted: usize,
    succeeded: usize,
    failed: usize,
    skipped_unloadable: usize,
    remaining_stale_estimate: usize,
    summary_cache_entries_before: usize,
    summary_cache_entries_after: usize,
    usage_calls: u64,
    input_tokens: u64,
    output_tokens: u64,
    estimated_cost_microdollars: u64,
    estimated_cost_display: String,
    failures: Vec<SummaryRefreshFailure>,
}

impl CycleCounterBaseline {
    fn from_observation(obs: &BatchObservation) -> Self {
        Self {
            jobs_total: obs.jobs_total,
            jobs_done: obs.jobs_done,
            jobs_failed: obs.jobs_failed,
            triage_completed: obs.triage_completed,
            triage_failed: obs.triage_failed,
            summary_completed: obs.summary_completed,
            summary_failed: obs.summary_failed,
            imports_completed: obs.imports_completed,
            imports_failed: obs.imports_failed,
        }
    }

    fn measure_cycle_and_advance(&mut self, obs: &BatchObservation) -> CycleCounts {
        let counts = CycleCounts {
            new_jobs: obs.jobs_total.saturating_sub(self.jobs_total),
            jobs_done: obs.jobs_done.saturating_sub(self.jobs_done),
            jobs_failed: obs.jobs_failed.saturating_sub(self.jobs_failed),
            triage_completed: obs.triage_completed.saturating_sub(self.triage_completed),
            triage_failed: obs.triage_failed.saturating_sub(self.triage_failed),
            summary_completed: obs.summary_completed.saturating_sub(self.summary_completed),
            summary_failed: obs.summary_failed.saturating_sub(self.summary_failed),
            imports_completed: obs.imports_completed.saturating_sub(self.imports_completed),
            imports_failed: obs.imports_failed.saturating_sub(self.imports_failed),
        };
        *self = Self::from_observation(obs);
        counts
    }
}

/// Determines if the batch cycle should settle (all reducer-owned work quiesced).
fn should_settle_cycle(status: harvester_core::BatchStatus) -> bool {
    matches!(status, harvester_core::BatchStatus::Settled)
}

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

fn should_check_settlement_this_iteration(orchestrated: bool) -> bool {
    !orchestrated
}

fn batch_buffer_is_quiescent(state: &AppState, buffered_ids: &HashSet<u64>) -> bool {
    !buffered_ids.is_empty()
        && state
            .pending_llm_request_ids()
            .all(|request_id| buffered_ids.contains(&request_id))
}

fn should_stop_after_cycle(single_shot: bool, shutdown_requested: bool) -> bool {
    shutdown_requested || single_shot
}

/// A collect-only cycle skips source polling and only advances work the batch
/// manifest already owns. Batch API mode polls once and then collects; drain
/// mode never polls, so its very first cycle is already collect-only.
fn is_collect_only_cycle(batch_api_enabled: bool, drain: bool, cycle_count: usize) -> bool {
    batch_api_enabled && (drain || cycle_count > 1)
}

/// Summarizes a finished drain for stdout. Batches that are still running
/// remain in the manifest and are reported so the operator knows a later drain
/// still has work to collect.
fn format_drain_summary(pending_manifest_batches: &[(String, Option<String>)]) -> String {
    if pending_manifest_batches.is_empty() {
        return "[batch-drain] collected and exiting; no batches remain pending".to_string();
    }
    let ids: Vec<_> = pending_manifest_batches
        .iter()
        .map(|(input_file_id, batch_id)| batch_id.clone().unwrap_or_else(|| input_file_id.clone()))
        .collect();
    format!(
        "[batch-drain] collected and exiting; {} batch(es) still pending: {}",
        ids.len(),
        ids.join(", ")
    )
}

fn require_new_jobs_since(
    single_shot: bool,
    batch_api: bool,
    cycle_jobs_total_baseline: usize,
) -> Option<usize> {
    (single_shot && !batch_api).then_some(cycle_jobs_total_baseline)
}

fn is_terminal_batch_status(status: &openai_provider_kit::BatchLifecycle) -> bool {
    matches!(
        status,
        openai_provider_kit::BatchLifecycle::Completed
            | openai_provider_kit::BatchLifecycle::Failed
            | openai_provider_kit::BatchLifecycle::Expired
            | openai_provider_kit::BatchLifecycle::Cancelled
    )
}

fn decide_batch_wait(peeks: &[BatchPeek]) -> BatchWaitDecision {
    if peeks.is_empty()
        || peeks
            .iter()
            .filter_map(|peek| peek.status.as_ref())
            .any(is_terminal_batch_status)
    {
        BatchWaitDecision::RunCollectCycle
    } else {
        BatchWaitDecision::KeepWaiting
    }
}

fn batch_drain_made_progress(before: &BatchDrainSnapshot, after: &BatchDrainSnapshot) -> bool {
    before != after
}

fn should_exit_batch_drain_after_no_progress(consecutive_cycles: usize) -> bool {
    consecutive_cycles >= MAX_CONSECUTIVE_BATCH_COLLECT_NO_PROGRESS
}

fn write_no_progress_bailout<W: Write>(
    sink: &mut W,
    snapshot: &BatchDrainSnapshot,
) -> std::io::Result<()> {
    writeln!(
        sink,
        "[batch-wait] no-progress bailout; remaining triage={} summaries={} signal={}",
        snapshot.triage_deferred, snapshot.summary_deferred, snapshot.signal_deferred
    )
}

fn exit_code_with_shutdown(default_exit_code: i32, shutdown_requested: bool) -> i32 {
    if shutdown_requested {
        130
    } else {
        default_exit_code
    }
}

fn determine_exit_code(total_failure_cycles: usize) -> i32 {
    if total_failure_cycles > 0 {
        1
    } else {
        0
    }
}

fn should_run_ai_orchestration(
    enable_ai_orchestration: bool,
    require_new_jobs_since: Option<usize>,
    obs: &BatchObservation,
) -> bool {
    if !enable_ai_orchestration {
        return false;
    }
    match require_new_jobs_since {
        Some(baseline_jobs_total) => obs.jobs_total > baseline_jobs_total,
        None => true,
    }
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

    let signal_candidate_model = config
        .signal_candidate_model
        .as_ref()
        .or(config.summary_model.as_ref())
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleSignalCandidate, signal_candidate_model);

    let briefing_model = config
        .briefing_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::AggregateBriefing, briefing_model.clone());
    map.insert(PromptId::BriefingExecutiveSummary, briefing_model.clone());
    map.insert(PromptId::BriefingNextItem, briefing_model);

    map
}

fn apply_signal_candidate_selection_settings(state: &mut AppState, args: &Args) {
    state.set_signal_candidate_threshold(
        args.signal_candidate_threshold
            .unwrap_or(DEFAULT_SELECTION_THRESHOLD),
    );
}

fn build_effect_runner(
    paths: &RuntimePaths,
    msg_tx: mpsc::Sender<Msg>,
    llm_concurrency: usize,
    platform_handler: Box<NoOpPlatformHandler>,
    batch_api: bool,
) -> Result<(EffectRunner, Option<BatchRuntime>), String> {
    if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        if api_key.trim().is_empty() {
            engine_warn!("[batch] OPENAI_API_KEY is empty; AI triage/summary features disabled");
            return Ok((
                EffectRunner::new(paths.clone(), msg_tx, platform_handler),
                None,
            ));
        }
        let batch_provider = OpenAiProvider::new(api_key);
        let provider: Arc<dyn harvester_engine::llm::provider::LlmProvider> =
            Arc::new(batch_provider.clone());
        let provider_clone = Arc::clone(&provider);
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        let registry = Arc::new(RwLock::new(registry));
        let config = LlmConfig {
            provider,
            default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_4O_MINI),
            triage_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_TRIAGE_MODEL)),
            summary_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_SUMMARY_MODEL)),
            signal_candidate_model: None,
            briefing_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_BRIEFING_MODEL)),
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
        let batch_runtime = if batch_api {
            Some(BatchRuntime::new(batch_provider, config.clone(), paths)?)
        } else {
            None
        };
        let handle = LlmHandle::new(config);
        Ok((
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
            ),
            batch_runtime,
        ))
    } else {
        engine_warn!("[batch] OPENAI_API_KEY not set; AI triage/summary features disabled");
        Ok((
            EffectRunner::new(paths.clone(), msg_tx, platform_handler),
            None,
        ))
    }
}

fn build_prompt_registry_with_saved_overlays(paths: &RuntimePaths) -> PromptRegistry {
    let mut registry = PromptRegistry::new();
    register_defaults(&mut registry);

    for entry in load_prompt_templates(&paths.prompts_dir) {
        let loaded_template = match entry {
            Ok(template) => template,
            Err(reason) => {
                engine_warn!(
                    "[summary-refresh] failed to load saved prompt template: {}",
                    reason
                );
                continue;
            }
        };

        if loaded_template.template_file.version == PROMPT_VERSION_DRAFT {
            engine_warn!(
                "[summary-refresh] skipping draft saved template prompt_id={:?} path={}",
                loaded_template.prompt_id,
                loaded_template.path.display()
            );
            continue;
        }

        registry.register_overlay(PromptTemplateOwned {
            id: loaded_template.prompt_id,
            version: loaded_template.template_file.version,
            system_template: loaded_template.template_file.system_template,
            user_template: loaded_template.template_file.user_template,
            description: loaded_template.template_file.description,
            expected_format: loaded_template.template_file.expected_format,
        });
    }

    registry
}

fn load_summary_context_pairs(paths: &RuntimePaths) -> Vec<(String, String)> {
    let path = paths.contexts_dir.join("article_summary.toml");
    if !path.exists() {
        return Vec::new();
    }

    match load_context_file(&path) {
        Ok(file) => file.variables.into_iter().collect(),
        Err(err) => {
            engine_warn!(
                "[summary-refresh] failed to load summary context from {}: {}",
                path.display(),
                err
            );
            Vec::new()
        }
    }
}

fn build_summary_refresh_runtime(
    paths: &RuntimePaths,
    llm_concurrency: usize,
) -> Result<SummaryRefreshRuntime, String> {
    let api_key =
        std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY not set".to_string())?;
    if api_key.trim().is_empty() {
        return Err("OPENAI_API_KEY is empty".to_string());
    }

    let provider: Arc<dyn harvester_engine::llm::provider::LlmProvider> =
        Arc::new(OpenAiProvider::new(api_key));
    let registry = Arc::new(RwLock::new(build_prompt_registry_with_saved_overlays(
        paths,
    )));
    let summary_context = load_summary_context_pairs(paths);
    let summary_prompt_version = registry
        .read()
        .unwrap()
        .active(PromptId::ArticleSummary)
        .map(|template| template.version)
        .ok_or_else(|| "summary prompt not registered".to_string())?;

    let config = LlmConfig {
        provider,
        default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_4O_MINI),
        triage_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_TRIAGE_MODEL)),
        summary_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_SUMMARY_MODEL)),
        signal_candidate_model: None,
        briefing_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_BRIEFING_MODEL)),
        registry: Arc::clone(&registry),
        quotas: LlmQuotas::default(),
        output_dir: paths.output_dir.clone(),
        pricing: PricingRegistry::with_defaults(),
        max_input_bytes: 100_000,
        #[allow(deprecated)]
        max_input_chars: 0,
        timestamp_utc: Arc::new(|| Utc::now().to_rfc3339()),
        session_id: format!(
            "batch-summary-refresh-{}",
            Utc::now().format("%Y%m%d-%H%M%S")
        ),
        replay_cache: None,
        max_concurrent_requests: llm_concurrency,
    };
    let summary_model = config
        .summary_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();

    Ok((
        LlmHandle::new(config),
        registry,
        summary_prompt_version,
        summary_model,
        summary_context,
    ))
}

fn select_stale_summary_targets(
    metas: &[harvester_engine::ArchiveArticleMeta],
    summary_cache: &SummaryCache,
    prompt_version: u32,
    model_id: &str,
    context: &[(String, String)],
    limit: usize,
) -> SummaryRefreshSelection {
    let mut grouped_targets: Vec<SummaryRefreshTarget> = Vec::new();
    let mut grouped_by_hash = HashMap::<String, usize>::new();

    for meta in metas {
        let Some(content_hash) = meta.content_hash.as_ref().filter(|hash| !hash.is_empty()) else {
            continue;
        };

        if let Some(index) = grouped_by_hash.get(content_hash).copied() {
            grouped_targets[index]
                .related_urls
                .push((meta.url.clone(), meta.fetched_utc.clone()));
            continue;
        }

        let Ok(cache_key) = SummaryCacheKey::try_new(
            content_hash,
            PromptId::ArticleSummary,
            Some(prompt_version),
            Some(model_id),
            context,
        ) else {
            continue;
        };

        grouped_by_hash.insert(content_hash.clone(), grouped_targets.len());
        grouped_targets.push(SummaryRefreshTarget {
            primary_url: meta.url.clone(),
            content_hash: content_hash.clone(),
            cache_key,
            related_urls: vec![(meta.url.clone(), meta.fetched_utc.clone())],
        });
    }

    let mut selection = SummaryRefreshSelection::default();
    for target in grouped_targets {
        if summary_cache.lookup(&target.cache_key).is_some() {
            continue;
        }
        selection.total_stale += 1;
        if selection.targets.len() < limit {
            selection.targets.push(target);
        }
    }

    selection
}

fn format_llm_completion_error(error: &LlmCompletionError) -> String {
    match error {
        LlmCompletionError::ProviderError(err) => format!("provider error: {err}"),
        LlmCompletionError::ValidationFailed { reason, .. } => {
            format!("validation failed: {reason}")
        }
        LlmCompletionError::QuotaExhausted { description, .. } => {
            format!("quota exhausted: {description}")
        }
        LlmCompletionError::PromptNotFound { prompt_id } => {
            format!("prompt not found: {prompt_id:?}")
        }
        LlmCompletionError::PersistenceFailed { detail, .. } => {
            format!("persistence failed: {detail}")
        }
        LlmCompletionError::InputTooLarge { size, limit } => {
            format!("input too large: {size} > {limit}")
        }
        LlmCompletionError::TemplateRenderFailed { detail } => {
            format!("template render failed: {detail}")
        }
        LlmCompletionError::UnsupportedModel { model, reason } => {
            format!("unsupported model {}: {reason}", model.model_name())
        }
    }
}

fn summary_refresh_status_label(successes: usize, failures: usize) -> &'static str {
    match (successes, failures) {
        (0, 0) => "noop",
        (_, 0) => "success",
        (0, _) => "failed",
        _ => "partial_success",
    }
}

fn summary_refresh_exit_code(successes: usize, failures: usize) -> i32 {
    if failures > 0 && successes == 0 {
        1
    } else {
        0
    }
}

fn persist_summary_refresh_report(
    paths: &RuntimePaths,
    started_at_utc: &str,
    report: &SummaryRefreshReport,
) -> Result<PathBuf, String> {
    let reports_dir = paths.output_dir.join("summary_refresh_reports");
    ensure_output_dir(&reports_dir)
        .map_err(|err| format!("failed to create summary refresh report directory: {err}"))?;

    let serialized = serde_json::to_string_pretty(report)
        .map_err(|err| format!("failed to serialize summary refresh report: {err}"))?;

    let compact_timestamp: String = started_at_utc
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .take(14)
        .collect();
    let timestamp = if compact_timestamp.is_empty() {
        "latest".to_string()
    } else {
        compact_timestamp
    };

    let report_filename = format!("summary-refresh-{timestamp}.json");
    let reports_writer = AtomicFileWriter::new(reports_dir.clone());
    let report_path = reports_writer
        .write(&report_filename, &serialized)
        .map_err(|err| format!("failed to write summary refresh report: {err}"))?;

    let latest_writer = AtomicFileWriter::new(paths.output_dir.clone());
    latest_writer
        .write(".summary_refresh_last.json", &serialized)
        .map_err(|err| format!("failed to write latest summary refresh report: {err}"))?;

    Ok(report_path)
}

fn run_refresh_stale_summaries_mode(
    paths: &RuntimePaths,
    args: &Args,
    shutdown_flag: &Arc<AtomicBool>,
) -> Result<i32, String> {
    let limit = args
        .refresh_stale_summaries_limit
        .ok_or_else(|| "missing --refresh-stale-summaries-limit".to_string())?;
    if limit == 0 {
        return Err("--refresh-stale-summaries-limit must be greater than zero".to_string());
    }
    let started_at_utc = Utc::now().to_rfc3339();
    let progress_enabled = std::io::stdout().is_terminal() && std::io::stderr().is_terminal();
    let mut progress: Option<ProgressReporter> = None;

    let (llm_handle, registry, prompt_version, summary_model, summary_context) =
        build_summary_refresh_runtime(paths, args.llm_concurrency)?;

    let result = (|| -> Result<(SummaryRefreshReport, i32), String> {
        let mut summary_cache = load_summary_cache(&paths.summary_cache_path);
        let summary_cache_entries_before = summary_cache.len();
        let article_metas = scan_archive_article_metadata(&paths.output_dir)?;
        let selection = select_stale_summary_targets(
            &article_metas,
            &summary_cache,
            prompt_version,
            &summary_model,
            &summary_context,
            limit,
        );

        engine_info!(
            "[summary-refresh] stale_total={} selected={} limit={} prompt_version={} model_id={}",
            selection.total_stale,
            selection.targets.len(),
            limit,
            prompt_version,
            summary_model
        );
        progress = Some(ProgressReporter::new(
            selection.targets.len(),
            selection.total_stale,
            limit,
            args.llm_concurrency,
            progress_enabled,
        ));
        if let Some(p) = progress.as_ref() {
            p.startup_line(&mut std::io::stdout());
        }

        let mut report = SummaryRefreshReport {
            started_at_utc: started_at_utc.clone(),
            finished_at_utc: started_at_utc.clone(),
            status: "noop".to_string(),
            output_dir: paths.output_dir.display().to_string(),
            prompt_version,
            configured_model: summary_model.clone(),
            limit,
            stale_total_before: selection.total_stale,
            selected: selection.targets.len(),
            attempted: 0,
            succeeded: 0,
            failed: 0,
            skipped_unloadable: 0,
            remaining_stale_estimate: selection.total_stale,
            summary_cache_entries_before,
            summary_cache_entries_after: summary_cache_entries_before,
            usage_calls: 0,
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_microdollars: 0,
            estimated_cost_display: microdollars_to_display(0),
            failures: Vec::new(),
        };

        if selection.total_stale == 0 {
            engine_info!("[summary-refresh] all summaries already match current cache key");
            report.finished_at_utc = Utc::now().to_rfc3339();
            return Ok((report, 0));
        }

        let selected_urls: Vec<String> = selection
            .targets
            .iter()
            .map(|target| target.primary_url.clone())
            .collect();
        let (articles, _) = {
            let guard = registry.read().unwrap();
            load_and_prepare_articles_filtered(
                &paths.output_dir,
                100_000,
                &guard,
                &selected_urls,
                None,
            )?
        };

        let mut targets_by_url: HashMap<String, SummaryRefreshTarget> = selection
            .targets
            .into_iter()
            .map(|target| (target.primary_url.clone(), target))
            .collect();
        let mut pending = HashMap::<u64, SummaryRefreshTarget>::new();
        let mut request_id = 0u64;

        for article in articles {
            if shutdown_flag.load(Ordering::Relaxed) {
                engine_info!(
                    "[summary-refresh] Shutdown requested; stopping new LLM request dispatch"
                );
                break;
            }
            let Some(target) = targets_by_url.remove(&article.url) else {
                continue;
            };

            request_id += 1;
            llm_handle
                .send(LlmCommand::Complete(Box::new(LlmCompletionCommand {
                    request_id,
                    prompt_id: PromptId::ArticleSummary,
                    prompt_version: Some(prompt_version),
                    model_override: None,
                    input_content: article.prepared_text,
                    context: summary_context.clone(),
                    template_override: None,
                    extra_template_vars: vec![],
                })))
                .map_err(|err| format!("failed to dispatch summary refresh request: {err}"))?;
            pending.insert(request_id, target);
            if let Some(p) = progress.as_mut() {
                p.request_dispatched();
            }
        }

        report.attempted = pending.len();
        let interrupted_during_dispatch = shutdown_flag.load(Ordering::Relaxed);
        if !interrupted_during_dispatch && !targets_by_url.is_empty() {
            report.failed += targets_by_url.len();
            report.skipped_unloadable = targets_by_url.len();
            engine_warn!(
                "[summary-refresh] {} selected article(s) could not be loaded from archive",
                targets_by_url.len()
            );
            for (_, target) in targets_by_url {
                report.failures.push(SummaryRefreshFailure {
                    request_id: None,
                    url: target.primary_url,
                    reason: "selected article could not be loaded from archive".to_string(),
                });
                if let Some(p) = progress.as_mut() {
                    let failure = report.failures.last().expect("failure was just pushed");
                    p.unloadable_target(
                        &failure.url,
                        "selected article could not be loaded from archive",
                        &mut std::io::stdout(),
                        &mut std::io::stderr(),
                    );
                }
            }
        }
        if pending.is_empty() && interrupted_during_dispatch {
            report.status = "interrupted".to_string();
            report.finished_at_utc = Utc::now().to_rfc3339();
            return Ok((report, 130));
        }
        if pending.is_empty() {
            return Err("no stale summaries could be dispatched".to_string());
        }

        let event_rx = llm_handle.event_receiver();
        let mut entity_index = load_entity_index(&paths.entity_index_path);

        while !pending.is_empty() && !shutdown_flag.load(Ordering::Relaxed) {
            let event = {
                let receiver = event_rx.lock().unwrap();
                receiver.recv_timeout(Duration::from_millis(100))
            };
            let event = match event {
                Ok(event) => event,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("summary refresh worker stopped unexpectedly".to_string())
                }
            };

            let LlmEvent::Completed { request_id, result } = event else {
                continue;
            };
            let Some(target) = pending.remove(&request_id) else {
                engine_warn!(
                    "[summary-refresh] received completion for unknown request_id={}",
                    request_id
                );
                continue;
            };

            match result {
                Ok(completion) => match validate_summary(&completion.output_json) {
                    Ok(summary) => {
                        let summary_result = ArticleSummaryResult {
                            title: summary.title,
                            summary: summary.summary,
                            key_points: summary.key_points,
                            input_tokens: completion.metadata.input_tokens,
                            output_tokens: completion.metadata.output_tokens,
                            entities: summary.entities,
                        };
                        summary_cache.insert(
                            target.cache_key,
                            SummaryCacheEntry {
                                result: summary_result.clone(),
                                created_at_utc: completion.metadata.timestamp_utc.clone(),
                            },
                        );

                        let mut seen_urls = HashSet::new();
                        for (url, fetched_utc) in target.related_urls {
                            if !seen_urls.insert(url.clone()) {
                                continue;
                            }
                            upsert_entry(
                                &mut entity_index,
                                &url,
                                EntityIndexPatch {
                                    fetched_utc,
                                    content_hash: Some(target.content_hash.clone()),
                                    summary_entities: Some(summary_result.entities.clone()),
                                    themes: None,
                                },
                            );
                        }

                        report.succeeded += 1;
                        if let Some(p) = progress.as_mut() {
                            p.completed_ok(&mut std::io::stdout());
                        }
                        engine_info!(
                            "[summary-refresh] refreshed request_id={} url={} content_hash={}",
                            request_id,
                            target.primary_url,
                            &target.content_hash[..target.content_hash.len().min(8)]
                        );
                    }
                    Err(err) => {
                        engine_warn!(
                            "[summary-refresh] validation failed after success request_id={} url={} reason={}",
                            request_id,
                            target.primary_url,
                            err
                        );
                        report.failed += 1;
                        report.failures.push(SummaryRefreshFailure {
                            request_id: Some(request_id),
                            url: target.primary_url,
                            reason: format!("validation failed after success: {err}"),
                        });
                        if let Some(p) = progress.as_mut() {
                            let failure = report.failures.last().expect("failure was just pushed");
                            p.completed_fail(
                                &failure.url,
                                &failure.reason,
                                &mut std::io::stdout(),
                                &mut std::io::stderr(),
                            );
                        }
                    }
                },
                Err(err) => {
                    report.failed += 1;
                    let reason = format_llm_completion_error(&err);
                    engine_warn!(
                        "[summary-refresh] request failed request_id={} url={} reason={}",
                        request_id,
                        target.primary_url,
                        reason
                    );
                    report.failures.push(SummaryRefreshFailure {
                        request_id: Some(request_id),
                        url: target.primary_url,
                        reason,
                    });
                    if let Some(p) = progress.as_mut() {
                        let failure = report.failures.last().expect("failure was just pushed");
                        p.completed_fail(
                            &failure.url,
                            &failure.reason,
                            &mut std::io::stdout(),
                            &mut std::io::stderr(),
                        );
                    }
                }
            }
        }

        if report.succeeded > 0 {
            persist_summary_cache(&summary_cache, &paths.summary_cache_path)
                .map_err(|err| format!("failed to persist summary cache: {err}"))?;
            save_entity_index(&paths.entity_index_path, &entity_index)
                .map_err(|err| format!("failed to persist entity index: {err}"))?;
        }
        report.summary_cache_entries_after = summary_cache.len();
        report.remaining_stale_estimate =
            report.stale_total_before.saturating_sub(report.succeeded);
        let interrupted = shutdown_flag.load(Ordering::Relaxed);
        report.status = if interrupted {
            "interrupted".to_string()
        } else {
            summary_refresh_status_label(report.succeeded, report.failed).to_string()
        };
        report.finished_at_utc = Utc::now().to_rfc3339();

        engine_info!(
            "[summary-refresh] completed successes={} failures={}",
            report.succeeded,
            report.failed
        );
        let exit_code = if interrupted {
            130
        } else {
            summary_refresh_exit_code(report.succeeded, report.failed)
        };
        Ok((report, exit_code))
    })();

    let usage_totals = if shutdown_flag.load(Ordering::Relaxed) {
        None
    } else {
        llm_handle.usage_totals()
    };
    llm_handle.drain_and_stop();
    match result {
        Ok((mut report, exit_code)) => {
            let totals = usage_totals.unwrap_or(harvester_engine::llm::LlmUsageTotals {
                calls: 0,
                input_tokens: 0,
                output_tokens: 0,
                cost_microdollars: 0,
            });
            report.usage_calls = totals.calls;
            report.input_tokens = totals.input_tokens;
            report.output_tokens = totals.output_tokens;
            report.estimated_cost_microdollars = totals.cost_microdollars;
            report.estimated_cost_display = microdollars_to_display(totals.cost_microdollars);

            engine_info!(
                "[summary-refresh] usage calls={} input_tokens={} output_tokens={} cost={}",
                totals.calls,
                totals.input_tokens,
                totals.output_tokens,
                report.estimated_cost_display
            );

            let report_path = persist_summary_refresh_report(paths, &started_at_utc, &report)?;
            engine_info!(
                "[summary-refresh] report written path={} status={} remaining_stale_estimate={}",
                report_path.display(),
                report.status,
                report.remaining_stale_estimate
            );
            if let Some(p) = progress.as_mut() {
                p.finish(
                    report.succeeded,
                    report.failed,
                    &report.estimated_cost_display,
                    &report_path,
                    &mut std::io::stdout(),
                );
            }
            Ok(exit_code_with_shutdown(
                exit_code,
                shutdown_flag.load(Ordering::Relaxed),
            ))
        }
        Err(err) => Err(err),
    }
}

fn is_ai_orchestration_enabled() -> bool {
    std::env::var("OPENAI_API_KEY")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

/// Drain must never orchestrate. Restored completed jobs feed the pre-triage
/// session, so orchestration would dispatch triage over the whole corpus and
/// submit fresh batches — exactly the new work drain exists to avoid.
fn should_enable_ai_orchestration_for_mode(api_key_available: bool, drain: bool) -> bool {
    api_key_available && !drain
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
        Msg::PollStarted { total } => format!("PollStarted(total={total})"),
        Msg::AllSourcesPollEnded => "AllSourcesPollEnded".to_string(),
        Msg::SourcePollCompleted {
            source_id, urls, ..
        } => {
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
        Msg::TriageArticlesLoadProgress {
            files_scanned,
            files_total,
            ..
        } => format!(
            "TriageArticlesLoadProgress {{ files_scanned: {}, files_total: {} }}",
            files_scanned, files_total
        ),
        Msg::ArticlesLoaded { articles, .. } => {
            format!("ArticlesLoaded {{ articles: {} }}", articles.len())
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

    let sources_path = args.sources_path();
    let paths = RuntimePaths::new(
        args.output_dir.clone(),
        sources_path,
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

    // Install signal handler immediately after lock acquisition so Ctrl-C always
    // reaches the shared graceful-shutdown path for every execution mode.
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let interactive = std::io::stdout().is_terminal() && std::io::stderr().is_terminal();
    install_signal_handler(Arc::clone(&shutdown_flag), interactive);

    if args.dry_run {
        engine_info!("[batch] Dry-run mode: single poll only");
        return run_dry_run(&paths, &args, &shutdown_flag);
    }

    // Import mode: branch before source loading
    if let Some(import_dir) = &args.import_saved_web_dir {
        engine_info!("[batch] Import mode: dir={}", import_dir.display());
        return run_import_mode(
            &paths,
            &args,
            import_dir.clone(),
            Arc::clone(&shutdown_flag),
        );
    }

    if args.refresh_stale_summaries_limit.is_some() {
        engine_info!("[batch] Summary refresh mode enabled");
        return run_refresh_stale_summaries_mode(&paths, &args, &shutdown_flag);
    }

    // Validate source configuration
    engine_info!(
        "[batch] Loading source registry from {:?}",
        paths.sources_path
    );
    let source_registry = load_sources(&paths.sources_path);

    // Drain never polls, so an unsupported source must not be able to abort a
    // collection of work that has already been paid for.
    if !args.allow_unsupported_sources && !args.drain {
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
    if args.batch_api_enabled() {
        let session_limit = LlmQuotas::default()
            .max_calls_per_session
            .map(|limit| limit as usize)
            .unwrap_or(crate::batch_coordinator::MAX_BATCH_LINES);
        state.set_deferred_batch_max_in_flight(session_limit);
    } else {
        state.set_triage_max_in_flight(args.llm_concurrency);
        state.set_summary_max_in_flight(args.llm_concurrency);
    }
    apply_signal_candidate_selection_settings(&mut state, &args);

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

    // Hydrate domain blacklist.
    let blacklist = load_blacklist(&paths.blacklist_path);
    if !blacklist.is_empty() {
        let (new_state, _) = update(state, Msg::BlacklistHydrated { state: blacklist });
        state = new_state;
    }

    // Build EffectRunner (with optional LLM support based on OPENAI_API_KEY)
    engine_info!("[batch] Building EffectRunner");
    let enable_ai_orchestration =
        should_enable_ai_orchestration_for_mode(is_ai_orchestration_enabled(), args.drain);
    let platform_handler = Box::new(NoOpPlatformHandler);
    let (effect_runner, mut batch_runtime) = build_effect_runner(
        &paths,
        msg_tx.clone(),
        args.llm_concurrency,
        platform_handler,
        args.batch_api_enabled(),
    )?;
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

    let run_baseline = BatchRunBaseline::from_observation(&state.batch_observation());
    let run_started_at = Instant::now();
    let mut progress = LiveBatchProgress::new(run_baseline, interactive, args.ascii_progress);
    if !interactive {
        println!(
            "[batch] started mode={}",
            batch_mode_label(args.batch_api_enabled(), args.drain)
        );
    }

    // Ordinary mode polls repeatedly. Batch API mode runs one intake cycle,
    // then drains exactly that cycle's deferred work without polling again.
    let poll_interval = Duration::from_secs((args.poll_interval * 60) as u64);
    let mut cycle_count = 0;
    let mut total_cycles = 0;
    let mut total_failure_cycles = 0;
    let mut cycle_baseline = CycleCounterBaseline::from_observation(&state.batch_observation());
    let mut total_new_articles = 0usize;
    let mut total_triaged = 0usize;
    let mut total_summarized = 0usize;
    let mut collect_cycle_baseline: Option<BatchDrainSnapshot> = None;
    let mut consecutive_no_progress_collect_cycles = 0usize;

    'cycles: loop {
        cycle_count += 1;
        total_cycles += 1;
        let collect_only_cycle =
            is_collect_only_cycle(args.batch_api_enabled(), args.drain, cycle_count);
        progress.record_pass(collect_only_cycle);
        if collect_only_cycle {
            engine_info!(
                "[batch] === Starting collect-only cycle {} ===",
                cycle_count
            );
        } else {
            engine_info!("[batch] === Starting cycle {} ===", cycle_count);
        }
        let cycle_jobs_total_baseline = state.batch_observation().jobs_total;

        // Batch results are collected at the cycle boundary before re-arming.
        // A collected manifest snapshot is durable before this reducer message,
        // so a crash after the snapshot is replayed safely on the next run.
        if let Some(batch) = batch_runtime.as_mut() {
            if !batch.reconciled {
                progress.set_phase(BatchDisplayPhase::Reconciling);
                progress.paint(&state, batch.realized_cost_microdollars, true);
                match batch.runtime.block_on(batch.coordinator.reconcile_once()) {
                    Ok(()) => batch.reconciled = true,
                    Err(err) => {
                        engine_warn!("[batch-reconcile] failed; retrying next cycle: {}", err)
                    }
                }
            }
            if let Err(err) = remove_collected_with_persisted_cache_confirmation(batch, &paths) {
                engine_warn!(
                    "[batch-collect] persisted cache confirmation failed; retaining snapshots: {}",
                    err
                );
            }
            progress.set_phase(BatchDisplayPhase::Collecting);
            progress.paint(&state, batch.realized_cost_microdollars, true);
            let collected = match batch
                .runtime
                .block_on(batch.coordinator.collect_completed())
            {
                Ok(collected) => collected,
                Err(err) => {
                    engine_warn!("[batch-collect] failed; retrying next cycle: {}", err);
                    Vec::new()
                }
            };
            if !collected.is_empty() {
                let invalid = invalid_collected_custom_ids(&collected);
                if !invalid.is_empty() {
                    if let Err(err) = batch.coordinator.release_invalid_collected(&invalid) {
                        engine_warn!(
                            "[batch-collect] invalid-line release failed; snapshots retained for retry: {}",
                            err
                        );
                    }
                }
                persist_batch_replay_records(&collected, batch);
                let (new_state, collection_effects) =
                    update(state, Msg::BatchResultsCollected { entries: collected });
                state = new_state;
                if !collection_effects.is_empty() {
                    let collection_effects =
                        divert_batch_effects(&state, collection_effects, batch, &msg_tx);
                    if !collection_effects.is_empty() {
                        effect_runner.enqueue(collection_effects);
                    }
                }
            }

            // Only the runner advances deferred work into a new dispatch
            // epoch. Pre-loop effects use the same diversion path as effects
            // reduced inside the dispatch loop.
            progress.set_phase(BatchDisplayPhase::Replaying);
            progress.paint(&state, batch.realized_cost_microdollars, true);
            let (new_state, rearm_effects) = update(state, Msg::RearmDeferredBatchStages);
            state = new_state;
            if !rearm_effects.is_empty() {
                let rearm_effects = divert_batch_effects(&state, rearm_effects, batch, &msg_tx);
                if !rearm_effects.is_empty() {
                    effect_runner.enqueue(rearm_effects);
                }
            }
        }

        if !collect_only_cycle {
            progress.clear_phase_override();
            progress.paint(
                &state,
                batch_runtime
                    .as_ref()
                    .map_or(0, |batch| batch.realized_cost_microdollars),
                true,
            );
            engine_info!("[batch] Dispatching poll sources");
            msg_tx
                .send(Msg::PollSourcesClicked)
                .map_err(|e| format!("Failed to dispatch poll: {}", e))?;
        }

        // Run dispatch loop until settled
        let outcome = run_dispatch_loop_with_tick_interval(
            &mut state,
            &msg_tx,
            &msg_rx,
            &effect_runner,
            &shutdown_flag,
            DispatchLoopOptions {
                enable_ai_orchestration,
                require_new_jobs_since: require_new_jobs_since(
                    args.single_shot,
                    args.batch_api_enabled(),
                    cycle_jobs_total_baseline,
                ),
                tick_interval: Duration::from_millis(75),
            },
            Some(&mut progress),
            batch_runtime.as_mut(),
        )?;

        // Track outcome statistics
        match outcome {
            CycleOutcome::Success => {}
            CycleOutcome::PartialFailure => {}
            CycleOutcome::TotalFailure => total_failure_cycles += 1,
        }

        // Print cycle summary
        let obs = state.batch_observation();
        let cycle_counts = cycle_baseline.measure_cycle_and_advance(&obs);
        total_new_articles += cycle_counts.new_jobs;
        total_triaged += cycle_counts.triage_completed;
        total_summarized += cycle_counts.summary_completed;
        let current_cost = batch_runtime
            .as_ref()
            .map_or(0, |batch| batch.realized_cost_microdollars);
        let diagnostics = format_optional_cycle_diagnostics(
            args.verbose_progress,
            cycle_count == 1,
            !collect_only_cycle,
            cycle_count,
            &outcome,
            &cycle_counts,
            current_cost,
            &obs,
            &state.llm_usage_rows(),
            progress.last_provider_check_local,
        );
        if !diagnostics.is_empty() {
            progress.suspend_for_output();
            for line in diagnostics {
                println!("{line}");
            }
            progress.resume(&state, current_cost);
        }

        // Persist state
        engine_info!("[batch] Persisting state");
        progress.set_phase(BatchDisplayPhase::Persisting);
        progress.paint(&state, current_cost, true);
        let completed_jobs = state.completed_jobs_snapshot();
        persist_completed_jobs(&paths.state_path, &completed_jobs);
        if let Err(err) = save_blacklist(&paths.blacklist_path, state.blacklist()) {
            engine_warn!("[batch] failed to save blacklist: {}", err);
        }

        let shutdown_requested = shutdown_flag.load(Ordering::Relaxed);

        // Drain collects whatever the provider has already finished and exits.
        // It deliberately does not consult the reducer's deferred counters: a
        // fresh drain process has no deferred work to begin with, because that
        // state lives in memory rather than on disk. The manifest is the only
        // durable record of what is still outstanding.
        if args.drain {
            // A drain has no next cycle to run the confirmation, so snapshots
            // whose results already reached the caches are pruned here. Failure
            // only retains them for the next run, so it is not fatal.
            if let Some(batch) = batch_runtime.as_mut() {
                if let Err(err) = remove_collected_with_persisted_cache_confirmation(batch, &paths)
                {
                    engine_warn!(
                        "[batch-collect] persisted cache confirmation failed; retaining snapshots: {}",
                        err
                    );
                }
            }
            let summary = format_drain_summary(
                &batch_runtime
                    .as_ref()
                    .map(|batch| batch.coordinator.pending_manifest_batches())
                    .unwrap_or_default(),
            );
            engine_info!("{}", summary);
            progress.suspend_for_output();
            println!("{summary}");
            progress.resume(&state, current_cost);
            break;
        }

        if args.batch_api_enabled() {
            let deferred_total = obs.triage_deferred + obs.summary_deferred + obs.signal_deferred;
            if shutdown_requested {
                engine_info!("[batch] Shutdown signal received, exiting");
                break;
            }
            if deferred_total == 0 {
                engine_info!("[batch] Batch API drain settled; exiting");
                break;
            }

            let Some(batch) = batch_runtime.as_mut() else {
                engine_warn!(
                    "[batch-wait] {} deferred requests cannot be drained because Batch API runtime is unavailable",
                    deferred_total
                );
                break;
            };
            let drain_snapshot = BatchDrainSnapshot {
                pending_manifest_batches: batch.coordinator.pending_manifest_batches(),
                triage_deferred: obs.triage_deferred,
                summary_deferred: obs.summary_deferred,
                signal_deferred: obs.signal_deferred,
            };
            let delay_before_peek = if let Some(before) = collect_cycle_baseline.take() {
                if batch_drain_made_progress(&before, &drain_snapshot) {
                    consecutive_no_progress_collect_cycles = 0;
                    false
                } else {
                    consecutive_no_progress_collect_cycles += 1;
                    true
                }
            } else {
                false
            };
            if should_exit_batch_drain_after_no_progress(consecutive_no_progress_collect_cycles) {
                engine_warn!(
                    "[batch-wait] collect-only operation made no progress for {} consecutive cycles; exiting with deferred triage={} summaries={} signal={} pending_manifest_batches={:?}",
                    consecutive_no_progress_collect_cycles,
                    obs.triage_deferred,
                    obs.summary_deferred,
                    obs.signal_deferred,
                    drain_snapshot.pending_manifest_batches
                );
                progress.suspend_for_output();
                if let Err(err) = write_no_progress_bailout(&mut std::io::stdout(), &drain_snapshot)
                {
                    engine_warn!("[batch-progress] failed to print bailout summary: {}", err);
                }
                progress.resume(
                    &state,
                    batch_runtime
                        .as_ref()
                        .map_or(0, |runtime| runtime.realized_cost_microdollars),
                );
                break;
            }
            if delay_before_peek {
                engine_warn!(
                    "[batch-wait] collect-only operation made no progress; waiting {} minutes before retrying pending_manifest_batches={:?}",
                    BATCH_WAIT_INTERVAL.as_secs() / 60,
                    drain_snapshot.pending_manifest_batches
                );
                progress.clear_phase_override();
                let delay_cost = batch.realized_cost_microdollars;
                let delay_clock = SystemProgressClock;
                if wait_with_local_heartbeat(
                    &delay_clock,
                    shutdown_flag.as_ref(),
                    BATCH_WAIT_INTERVAL,
                    || progress.paint(&state, delay_cost, false),
                ) {
                    progress.set_phase(BatchDisplayPhase::Interrupted);
                    progress.paint(&state, delay_cost, true);
                    engine_info!("[batch] Shutdown during batch retry wait, exiting");
                    break 'cycles;
                }
            }
            let wait_cost = batch.realized_cost_microdollars;
            let clock = SystemProgressClock;
            match run_provider_wait_loop(
                &clock,
                shutdown_flag.as_ref(),
                progress.peeks.clone(),
                || {
                    batch
                        .runtime
                        .block_on(batch.coordinator.peek_pending_batches())
                },
                |render| {
                    let force = render.phase == BatchDisplayPhase::CheckingProvider;
                    progress.set_provider_wait_render(&render);
                    progress.paint(&state, wait_cost, force);
                },
            ) {
                ProviderWaitOutcome::Collect(peeks) => {
                    progress.set_provider_check(peeks);
                    progress.set_phase(BatchDisplayPhase::Collecting);
                    progress.paint(&state, wait_cost, true);
                    collect_cycle_baseline = Some(drain_snapshot.clone());
                    engine_info!(
                        "[batch-wait] collection or reconciliation is ready; starting collect-only cycle"
                    );
                    continue 'cycles;
                }
                ProviderWaitOutcome::Shutdown => {
                    progress.set_phase(BatchDisplayPhase::Interrupted);
                    progress.paint(&state, wait_cost, true);
                    engine_info!("[batch] Shutdown during batch wait, exiting");
                    break 'cycles;
                }
            }
        }

        // Check for shutdown signal or single-shot completion.
        if should_stop_after_cycle(args.single_shot, shutdown_requested) {
            if args.single_shot {
                engine_info!("[batch] Single-shot mode completed one cycle; exiting");
            }
            if shutdown_requested {
                engine_info!("[batch] Shutdown signal received, exiting");
            }
            break;
        }

        // Sleep interruptibly before the next ordinary polling cycle.
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
    if let Err(err) = save_blacklist(&paths.blacklist_path, state.blacklist()) {
        engine_warn!("[batch] failed to save blacklist on shutdown: {}", err);
    }

    let final_cost = batch_runtime
        .as_ref()
        .map_or(0, |batch| batch.realized_cost_microdollars);
    progress.set_phase(if shutdown_flag.load(Ordering::Relaxed) {
        BatchDisplayPhase::Interrupted
    } else {
        BatchDisplayPhase::Complete
    });
    progress.paint(&state, final_cost, true);
    progress.suspend_for_output();

    // Print final summary
    print_final_summary(
        args.batch_api_enabled(),
        total_cycles,
        &state.batch_observation(),
        total_new_articles,
        total_triaged,
        total_summarized,
        run_started_at.elapsed(),
        final_cost,
    );
    let final_obs = state.batch_observation();
    print_poll_stats(&final_obs.source_poll_stats);
    if args.verbose_progress {
        if let Some(line) = format_awaiting_batch_line(
            final_obs.triage_deferred,
            final_obs.summary_deferred,
            final_obs.signal_deferred,
        ) {
            println!("{line}");
            println!("  Run again after the batches complete to collect results.");
        }
    }
    progress.finish();

    engine_info!("[batch] Shutdown complete");

    Ok(exit_code_with_shutdown(
        determine_exit_code(total_failure_cycles),
        shutdown_flag.load(Ordering::Relaxed),
    ))
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
    options: DispatchLoopOptions,
) -> Result<CycleOutcome, String> {
    run_dispatch_loop_with_tick_interval(
        state,
        msg_tx,
        msg_rx,
        effect_runner,
        shutdown_flag,
        options,
        None,
        None,
    )
}

fn is_batch_eligible_prompt(prompt_id: PromptId) -> bool {
    matches!(
        prompt_id,
        PromptId::ArticleTriage | PromptId::ArticleSummary | PromptId::ArticleSignalCandidate
    )
}

fn batch_custom_id(key: &FrozenBatchKey) -> String {
    let custom_stage = match key.stage {
        StageKind::Triage => "triage",
        StageKind::Summary => "summary",
        StageKind::SignalCandidate => "signal",
    };
    let model_hash = harvester_engine::llm::content_hash(&key.model_id);
    format!(
        "{}-{}-v{}-{}-{}",
        custom_stage,
        &key.content_hash[..key.content_hash.len().min(16)],
        key.prompt_version,
        &key.context_hash[..key.context_hash.len().min(8)],
        &model_hash[..8]
    )
}

/// Diverts only cache-keyed, non-interactive article stages. Every other
/// effect remains on the normal EffectRunner path.
fn divert_batch_effects(
    state: &AppState,
    effects: Vec<harvester_core::Effect>,
    batch: &mut BatchRuntime,
    msg_tx: &mpsc::Sender<Msg>,
) -> Vec<harvester_core::Effect> {
    let mut passthrough = Vec::new();
    for effect in effects {
        let harvester_core::Effect::RequestLlmCompletion {
            request_id,
            prompt_id,
            prompt_version,
            model_override,
            input_content,
            context,
            template_override,
            extra_template_vars,
        } = effect
        else {
            passthrough.push(effect);
            continue;
        };
        if !is_batch_eligible_prompt(prompt_id) {
            passthrough.push(harvester_core::Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                prompt_version,
                model_override,
                input_content,
                context,
                template_override,
                extra_template_vars,
            });
            continue;
        }
        let Some(mut key) = state.frozen_batch_key_for_request(request_id) else {
            engine_logging::engine_warn!(
                "[batch-submit] request_id={} prompt_id={:?} missing frozen cache key; dispatching synchronously",
                request_id, prompt_id
            );
            passthrough.push(harvester_core::Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                prompt_version,
                model_override,
                input_content,
                context,
                template_override,
                extra_template_vars,
            });
            continue;
        };
        let command = LlmCompletionCommand {
            request_id,
            prompt_id,
            prompt_version,
            model_override,
            input_content,
            context,
            template_override,
            extra_template_vars,
        };
        match harvester_engine::llm::prepare_completion(&command, &batch.config) {
            Ok(prepared) => {
                let stage = key.stage;
                key.rendered_system = prepared.system_message;
                key.rendered_user = prepared.user_message;
                let custom_id = batch_custom_id(&key);
                if batch.coordinator.failed_attempts_for(&custom_id) >= 2 {
                    engine_logging::engine_warn!(
                        "[batch-submit] custom_id={} cache_key={} reached two batch attempts; falling back to synchronous dispatch",
                        custom_id, key.content_hash
                    );
                    passthrough.push(harvester_core::Effect::RequestLlmCompletion {
                        request_id: command.request_id,
                        prompt_id: command.prompt_id,
                        prompt_version: command.prompt_version,
                        model_override: command.model_override,
                        input_content: command.input_content,
                        context: command.context,
                        template_override: command.template_override,
                        extra_template_vars: command.extra_template_vars,
                    });
                    continue;
                }
                let estimated_input_tokens = prepared
                    .request
                    .messages()
                    .iter()
                    .map(|message| message.content().chars().count() as u64)
                    .sum::<u64>()
                    .div_ceil(4);
                let estimated_usage =
                    TokenUsage::new(estimated_input_tokens.min(u64::from(u32::MAX)) as u32, 0);
                let estimated_cost_microdollars = batch
                    .config
                    .pricing
                    .batch_cost_microdollars(prepared.model.model_name(), &estimated_usage);
                batch.coordinator.buffer(BufferedRequest {
                    request_id,
                    stage,
                    line: openai_provider_kit::BatchInputLine {
                        custom_id: custom_id.clone(),
                        method: "POST".to_string(),
                        url: "/v1/chat/completions".to_string(),
                        body: openai_provider_kit::openai_chat_completion_body(&prepared.request),
                    },
                    entry: PendingEntry {
                        custom_id,
                        key,
                        stage,
                        attempts: 0,
                        collected: None,
                    },
                    estimated_input_tokens,
                    estimated_cost_microdollars,
                });
            }
            Err(err) => {
                send_batch_preparation_failure(msg_tx, request_id, &err);
            }
        }
    }
    passthrough
}

fn send_batch_preparation_failure(
    msg_tx: &mpsc::Sender<Msg>,
    request_id: u64,
    err: &LlmCompletionError,
) {
    engine_warn!(
        "[batch-submit] request_id={} render/preparation failed: {:?}",
        request_id,
        err
    );
    let _ = msg_tx.send(Msg::LlmCompleted {
        request_id,
        result: harvester_core::LlmResultKind::Failed {
            reason: format!("batch request preparation failed: {err:?}"),
        },
        metadata: None,
    });
}

fn persist_batch_replay_records(
    entries: &[harvester_core::CollectedEntry],
    batch: &mut BatchRuntime,
) {
    for entry in entries {
        let replay_line_id = format!("batch-{}-{}", entry.batch_id, entry.custom_id);
        if batch.recorded_replay_lines.contains(&replay_line_id) {
            continue;
        }
        let (raw_response, usage, resolved_model, validated_output, validation_error) = match &entry
            .outcome
        {
            harvester_core::CollectedOutcome::Success {
                raw_output_json,
                usage,
                resolved_model,
            } => {
                let validation_error = match entry.stage {
                    StageKind::Triage => harvester_engine::llm::validate_triage(raw_output_json)
                        .err()
                        .map(|err| err.to_string()),
                    StageKind::Summary => harvester_engine::llm::validate_summary(raw_output_json)
                        .err()
                        .map(|err| err.to_string()),
                    StageKind::SignalCandidate => {
                        harvester_engine::llm::validate_signal_candidate(raw_output_json)
                            .err()
                            .map(|err| err.to_string())
                    }
                };
                match validation_error {
                    None => (
                        raw_output_json.clone(),
                        *usage,
                        resolved_model.clone(),
                        serde_json::from_str(raw_output_json).ok(),
                        None,
                    ),
                    Some(err) => (
                        raw_output_json.clone(),
                        *usage,
                        resolved_model.clone(),
                        None,
                        Some(err),
                    ),
                }
            }
            harvester_core::CollectedOutcome::LineError { detail } => (
                detail.clone(),
                TokenUsage::new(0, 0),
                entry.key.model_id.clone(),
                None,
                Some(detail.clone()),
            ),
        };
        let priced_model = if resolved_model.trim().is_empty() {
            &entry.key.model_id
        } else {
            &resolved_model
        };
        let cost_microdollars = batch
            .config
            .pricing
            .batch_cost_microdollars(priced_model, &usage);
        let record = ReplayRecord {
            request_id: replay_line_id.clone(),
            input_content_hash: entry.key.content_hash.clone(),
            prompt_id: entry.key.prompt_id,
            prompt_version: entry.key.prompt_version,
            model_id: priced_model.to_string(),
            timestamp_utc: entry.created_at_utc.clone(),
            rendered_system_message: entry.key.rendered_system.clone(),
            rendered_user_message: entry.key.rendered_user.clone(),
            raw_response,
            usage,
            validated_output,
            validation_error,
            cost_microdollars,
            wall_ms: 0,
            cache_status: "batch_collected".to_string(),
        };
        match harvester_engine::llm::persist_replay_record(
            &batch.config.replay_output_dir(),
            &record,
        ) {
            Ok(_) => {
                batch.recorded_replay_lines.insert(replay_line_id);
                batch.realized_cost_microdollars = batch
                    .realized_cost_microdollars
                    .saturating_add(cost_microdollars);
            }
            Err(err) => {
                engine_logging::engine_warn!(
                    "[batch-replay] batch_id={} custom_id={} cache_key={} persist failed: {}",
                    entry.batch_id,
                    entry.custom_id,
                    entry.key.content_hash,
                    err
                );
            }
        }
    }
}

fn load_recorded_batch_replay_lines(dir: &std::path::Path) -> HashSet<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return HashSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| harvester_engine::llm::load_replay_record(&entry.path()).ok())
        .filter(|record| record.request_id.starts_with("batch-"))
        .map(|record| record.request_id)
        .collect()
}

fn remove_collected_with_persisted_cache_confirmation(
    batch: &mut BatchRuntime,
    paths: &RuntimePaths,
) -> Result<(), String> {
    let triage_cache = load_triage_cache(&paths.triage_cache_path);
    let summary_cache = load_summary_cache(&paths.summary_cache_path);
    let signal_cache =
        load_signal_candidate_cache(&paths.signal_candidate_cache_path).map_err(|err| {
            format!(
                "signal cache {}: {err}",
                paths.signal_candidate_cache_path.display()
            )
        })?;
    batch
        .coordinator
        .remove_collected_if(|entry| match entry.stage {
            StageKind::Triage => TriageCacheKey::try_new_with_context_hash(
                &entry.key.content_hash,
                entry.key.prompt_id,
                Some(entry.key.prompt_version),
                Some(&entry.key.model_id),
                &entry.key.context_hash,
            )
            .ok()
            .is_some_and(|key| triage_cache.lookup(&key).is_some()),
            StageKind::Summary => summary_cache
                .lookup(&SummaryCacheKey {
                    content_hash: entry.key.content_hash.clone(),
                    prompt_id: entry.key.prompt_id,
                    prompt_version: entry.key.prompt_version,
                    model_id: entry.key.model_id.clone(),
                    context_hash: entry.key.context_hash.clone(),
                })
                .is_some(),
            StageKind::SignalCandidate => signal_cache
                .get(&SignalCandidateCacheKey {
                    signal_input_hash: entry.key.content_hash.clone(),
                    prompt_id: entry.key.prompt_id,
                    prompt_version: entry.key.prompt_version,
                    model_id: entry.key.model_id.clone(),
                    context_hash: entry.key.context_hash.clone(),
                })
                .is_some(),
        })
}

fn invalid_collected_custom_ids(entries: &[harvester_core::CollectedEntry]) -> HashSet<String> {
    entries
        .iter()
        .filter_map(|entry| match &entry.outcome {
            harvester_core::CollectedOutcome::Success {
                raw_output_json, ..
            } => {
                let valid = match entry.stage {
                    StageKind::Triage => {
                        harvester_engine::llm::validate_triage(raw_output_json).is_ok()
                    }
                    StageKind::Summary => {
                        harvester_engine::llm::validate_summary(raw_output_json).is_ok()
                    }
                    StageKind::SignalCandidate => {
                        harvester_engine::llm::validate_signal_candidate(raw_output_json).is_ok()
                    }
                };
                (!valid).then(|| entry.custom_id.clone())
            }
            harvester_core::CollectedOutcome::LineError { .. } => None,
        })
        .collect()
}

#[allow(clippy::too_many_arguments)] // Batch runtime is optional at this runner boundary.
fn run_dispatch_loop_with_tick_interval(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
    mut progress: Option<&mut LiveSystemBatchProgress>,
    mut batch_runtime: Option<&mut BatchRuntime>,
) -> Result<CycleOutcome, String> {
    let timeout = Duration::from_millis(100);
    let mut iterations = 0;
    let mut last_tick = Instant::now();
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

        // Receive at least one message with timeout, then drain a bounded batch.
        // Large restored states make reducer clones expensive; bounding the batch
        // keeps progress and tick-driven orchestration responsive under bursts.
        let mut recv_idle = false;
        let mut enqueued_effects = false;
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
                        engine_debug!("[batch] Processing message: {}", summarize_batch_msg(&msg));
                    }
                    let (new_state, effects) = update(state.clone(), msg);
                    *state = new_state;
                    queued_effects.extend(effects);
                    if let Some(p) = progress.as_deref_mut() {
                        let cost = batch_runtime
                            .as_ref()
                            .map_or(0, |batch| batch.realized_cost_microdollars);
                        p.clear_phase_override();
                        p.paint(state, cost, false);
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
                    engine_debug!("[batch] Enqueuing {} effects", queued_effects.len());
                    let queued_effects = if let Some(batch) = batch_runtime.as_deref_mut() {
                        divert_batch_effects(state, queued_effects, batch, msg_tx)
                    } else {
                        queued_effects
                    };
                    if !queued_effects.is_empty() {
                        enqueued_effects = true;
                        effect_runner.enqueue(queued_effects);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                recv_idle = true;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Message channel disconnected unexpectedly".to_string());
            }
        }

        if options.enable_ai_orchestration && last_tick.elapsed() >= options.tick_interval {
            let (new_state, tick_effects) = update(state.clone(), Msg::Tick);
            *state = new_state;
            if !tick_effects.is_empty() {
                let tick_effects = if let Some(batch) = batch_runtime.as_deref_mut() {
                    divert_batch_effects(state, tick_effects, batch, msg_tx)
                } else {
                    tick_effects
                };
                if !tick_effects.is_empty() {
                    enqueued_effects = true;
                    effect_runner.enqueue(tick_effects);
                }
            }
            last_tick = Instant::now();
        }

        // Check for settlement after processing available work.
        let mut orchestrated = false;
        if let Some(p) = progress.as_deref_mut() {
            let cost = batch_runtime
                .as_ref()
                .map_or(0, |batch| batch.realized_cost_microdollars);
            p.clear_phase_override();
            p.paint(state, cost, false);
        }
        let obs = state.batch_observation();
        if should_run_ai_orchestration(
            options.enable_ai_orchestration,
            options.require_new_jobs_since,
            &obs,
        ) {
            if let Some(next_msg) = maybe_dispatch_batch_ai_orchestration(state) {
                msg_tx.send(next_msg.clone()).map_err(|e| {
                    format!(
                        "Failed to dispatch orchestration message {:?}: {}",
                        next_msg, e
                    )
                })?;
                orchestrated = true;
            }
        }

        // This prevents an immediate idle-state exit before queued actions
        // (like PollSourcesClicked) have been reduced.
        if !orchestrated && recv_idle && !enqueued_effects {
            if let Some(batch) = batch_runtime.as_deref_mut() {
                let buffered_ids = batch.coordinator.buffered_request_ids();
                if batch_buffer_is_quiescent(state, &buffered_ids) {
                    if let Err(err) = batch
                        .runtime
                        .block_on(batch.coordinator.flush(msg_tx, Utc::now().to_rfc3339()))
                    {
                        engine_warn!(
                            "[batch-submit] flush failed; manifest/buffer state retained where possible: {}",
                            err
                        );
                    }
                    continue;
                }
            }
        }

        if should_check_settlement_this_iteration(orchestrated)
            && should_settle_cycle(state.batch_status())
        {
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
    match state.batch_next_action() {
        harvester_core::BatchNextAction::DispatchTriage => Some(Msg::TriageClicked),
        harvester_core::BatchNextAction::DispatchSummaries => Some(Msg::PrepareSummariesClicked),
        harvester_core::BatchNextAction::None => None,
    }
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

/// Formats the awaiting-batch-results summary line, or `None` when no work is
/// deferred to a pending Batch API job.
fn format_awaiting_batch_line(
    triage_deferred: usize,
    summary_deferred: usize,
    signal_deferred: usize,
) -> Option<String> {
    let total = triage_deferred + summary_deferred + signal_deferred;
    (total > 0).then(|| {
        format!(
            "  Awaiting batch results: {} triage, {} summaries, {} signal ({} total)",
            triage_deferred, summary_deferred, signal_deferred, total
        )
    })
}

/// Formats the verbose Batch API wait detail with a presentation-only local
/// wall-clock timestamp. Durable batch timestamps remain UTC elsewhere.
fn format_verbose_awaiting_batch_line(
    triage_deferred: usize,
    summary_deferred: usize,
    signal_deferred: usize,
    checked_at_local: Option<chrono::DateTime<chrono::FixedOffset>>,
) -> Option<String> {
    format_awaiting_batch_line(triage_deferred, summary_deferred, signal_deferred).map(|line| {
        match checked_at_local {
            Some(checked_at) => format!(
                "{line} · checked_at={}",
                checked_at.format("%Y-%m-%d %H:%M:%S %:z")
            ),
            None => line,
        }
    })
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

/// Prints a grouped poll-stats summary (RSS / Brave / other source types).
fn print_poll_stats(stats: &[harvester_core::SourcePollStat]) {
    if let Some(summary) = format_poll_summary(stats) {
        println!("{summary}");
    }
}

fn format_poll_summary(stats: &[harvester_core::SourcePollStat]) -> Option<String> {
    (!stats.is_empty()).then(|| {
        format!(
            "\n--- Poll summary ---\n{}\n--------------------",
            harvester_core::format_poll_stats(stats)
        )
    })
}

/// Returns the once-per-intake poll summary plus the former per-pass transcript
/// when the operator explicitly opts in. Runtime logging is unaffected.
#[allow(clippy::too_many_arguments)]
fn format_optional_cycle_diagnostics(
    verbose_progress: bool,
    include_header: bool,
    include_poll_summary: bool,
    cycle: usize,
    outcome: &CycleOutcome,
    counts: &CycleCounts,
    batch_cost_microdollars: u64,
    observation: &BatchObservation,
    usage_rows: &[LlmModelUsageView],
    checked_at_local: Option<chrono::DateTime<chrono::FixedOffset>>,
) -> Vec<String> {
    let mut lines = Vec::new();
    if verbose_progress {
        if include_header {
            lines.push(format!(
                "{:<6} {:<9} {:>20} {:>18} {:>21}",
                "Cycle", "Outcome", "Jobs(new/done/fail)", "Triage(ok/fail)", "Summaries(ok/fail)"
            ));
            lines.push("-".repeat(78));
        }
        lines.push(format!(
            "{:<6} {:<9} {:>20} {:>18} {:>21}",
            cycle,
            cycle_outcome_label(outcome),
            format!(
                "{}/{}/{}",
                counts.new_jobs, counts.jobs_done, counts.jobs_failed
            ),
            format!("{}/{}", counts.triage_completed, counts.triage_failed),
            format!("{}/{}", counts.summary_completed, counts.summary_failed),
        ));
        if batch_cost_microdollars > 0 {
            lines.push(format!(
                "  Batch API realized tokens/cost this run: discounted {} ({} microdollars)",
                microdollars_to_display(batch_cost_microdollars),
                batch_cost_microdollars
            ));
        }
        if let Some(line) = format_verbose_awaiting_batch_line(
            observation.triage_deferred,
            observation.summary_deferred,
            observation.signal_deferred,
            checked_at_local,
        ) {
            lines.push(line);
        }
    }
    if include_poll_summary {
        lines.extend(format_poll_summary(&observation.source_poll_stats));
    }
    if verbose_progress {
        lines.extend(format_llm_usage_lines(usage_rows));
    }
    lines
}

/// Prints the final summary when batch runner exits.
#[allow(clippy::too_many_arguments)]
fn print_final_summary(
    batch_api: bool,
    total_cycles: usize,
    observation: &BatchObservation,
    total_new_articles: usize,
    total_triaged: usize,
    total_summarized: usize,
    elapsed: Duration,
    batch_cost_microdollars: u64,
) {
    println!(
        "{}",
        format_final_summary(
            batch_api,
            total_cycles,
            observation,
            total_new_articles,
            total_triaged,
            total_summarized,
            elapsed,
            batch_cost_microdollars,
        )
    );
}

#[allow(clippy::too_many_arguments)]
fn format_final_summary(
    batch_api: bool,
    total_cycles: usize,
    observation: &BatchObservation,
    total_new_articles: usize,
    total_triaged: usize,
    total_summarized: usize,
    elapsed: Duration,
    batch_cost_microdollars: u64,
) -> String {
    let elapsed = format_summary_elapsed(elapsed);
    let deferred =
        observation.triage_deferred + observation.summary_deferred + observation.signal_deferred;
    let stages = format!(
        "intake_success={} intake_failed={} triage_success={} triage_failed={} summaries_success={} summaries_failed={} signals_success={} signals_failed={} deferred={} elapsed={} cost_this_run={}",
        observation.jobs_done,
        observation.jobs_failed,
        observation.triage_completed,
        observation.triage_failed,
        observation.summary_completed,
        observation.summary_failed,
        observation.signal_completed,
        observation.signal_failed,
        deferred,
        elapsed,
        microdollars_to_display(batch_cost_microdollars),
    );
    if batch_api {
        format!(
            "[batch] complete intake=1 collection_passes={} {}",
            total_cycles.saturating_sub(1),
            stages
        )
    } else {
        format!(
            "\n-- Batch complete: {} cycles, {} new articles, {} triaged, {} summarized --\n{}",
            total_cycles, total_new_articles, total_triaged, total_summarized, stages
        )
    }
}

fn format_summary_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if hours > 0 {
        format!("{hours}h{minutes:02}m")
    } else {
        format!("{minutes}m")
    }
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

/// Installs a signal handler for SIGINT/SIGTERM.
///
/// The first interrupt requests the runner's graceful shutdown path. The lock
/// remains held until `LockGuard` drops after the run returns. A second signal
/// hard-exits so a stuck network call cannot make the process unkillable.
fn install_signal_handler(shutdown_flag: Arc<AtomicBool>, interactive: bool) {
    let handler = move || {
        if shutdown_flag.swap(true, Ordering::Relaxed) {
            eprintln!("harvester_batch: interrupted again — exiting immediately");
            // The process exits without unwinding on the second interrupt, so
            // Drop cannot restore a cursor hidden by the dashboard.
            let mut stdout = std::io::stdout();
            restore_cursor_before_immediate_exit(&mut stdout, interactive);
            std::process::exit(130);
        }
        eprintln!("harvester_batch: interrupted — shutting down; lock remains held");
    };

    ctrlc::set_handler(handler).expect("Error setting signal handler");
}

fn restore_cursor_before_immediate_exit<W: Write>(stdout: &mut W, interactive: bool) {
    if interactive {
        let _ = stdout.queue(Show);
        let _ = stdout.flush();
    }
}

/// Converts microdollars to a human-readable dollar string with exact rounding.
/// Examples: 0 -> "$0.00", 1234567 -> "$1.23", 50 -> "$0.00", 5000 -> "$0.01"
fn microdollars_to_display(microdollars: u64) -> String {
    let cents = (microdollars + 5000) / 10000; // Round to nearest cent
    let dollars = cents / 100;
    let remaining_cents = cents % 100;
    format!("${}.{:02}", dollars, remaining_cents)
}

/// Runs the import-mode workflow for browser-saved webpage imports.
///
/// Branches before source loading and drives only the import pipeline.
/// Exits after the import and any requested downstream work (summaries/briefing) settles.
fn run_import_mode(
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

fn run_dry_run(
    paths: &RuntimePaths,
    args: &Args,
    shutdown_flag: &Arc<AtomicBool>,
) -> Result<i32, String> {
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
    apply_signal_candidate_selection_settings(&mut state, args);

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

    // Run dispatch loop until settlement or graceful shutdown.
    let outcome = run_dispatch_loop(
        &mut state,
        &msg_tx,
        &msg_rx,
        &effect_runner,
        shutdown_flag,
        DispatchLoopOptions {
            enable_ai_orchestration: false,
            require_new_jobs_since: None,
            tick_interval: Duration::from_millis(75),
        },
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
    print_poll_stats(&obs.source_poll_stats);
    println!("======================\n");

    engine_info!("[dry-run] Dry-run complete (no state modifications)");
    Ok(exit_code_with_shutdown(
        0,
        shutdown_flag.load(Ordering::Relaxed),
    ))
}

#[cfg(test)]
mod tests;
