//! Stdout progress reporter for the --refresh-stale-summaries-limit mode.
//!
//! Activated only when both stdout and stderr are terminals; otherwise every
//! method is a no-op.

// Phase 2 deliberately introduces the pure renderer input before Phase 4
// wires it into the runner. The legacy stdout reporter remains the sole live
// consumer until that later phase.
#![allow(dead_code)]

use chrono::{DateTime, FixedOffset, Local};
use harvester_core::{BatchObservation, StageKind};
use openai_provider_kit::BatchLifecycle;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::batch_coordinator::BatchPeek;

const FAIL_REASON_MAX_CHARS: usize = 80;

/// Cumulative reducer counts captured immediately before this process starts
/// its single source-intake pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchRunBaseline {
    pub jobs_total: usize,
    pub jobs_done: usize,
    pub jobs_failed: usize,
}

impl BatchRunBaseline {
    pub fn from_observation(observation: &BatchObservation) -> Self {
        Self {
            jobs_total: observation.jobs_total,
            jobs_done: observation.jobs_done,
            jobs_failed: observation.jobs_failed,
        }
    }
}

/// One stage's local state plus the matching provider scope, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StageProgress {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub pending_or_in_flight: usize,
    pub deferred: usize,
    pub provider_total: usize,
    pub provider_completed: usize,
    pub provisional_settled: usize,
    pub local_remaining: usize,
    pub unsubmitted: usize,
}

impl StageProgress {
    pub fn settled(self) -> usize {
        self.successful.saturating_add(self.failed)
    }
}

/// The renderer vocabulary. Runner integration will explicitly select the
/// lifecycle-specific variants in Phase 4; the projection derives pipeline
/// phases whenever no explicit lifecycle phase is supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchDisplayPhase {
    Reconciling,
    Intake,
    Triage,
    Summaries,
    Signals,
    PreparingBatch,
    CheckingProvider,
    WaitingForProvider,
    Collecting,
    Replaying,
    Persisting,
    Complete,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLifecycle {
    NoRemoteBatch,
    InProgress,
    ReadyToCollect,
    Indeterminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProviderStageProgress {
    pub submitted: usize,
    pub completed: usize,
    pub failed: usize,
    pub attached_batches: usize,
    pub terminal_batches: usize,
    pub indeterminate_batches: usize,
}

impl ProviderStageProgress {
    fn lifecycle(self) -> ProviderLifecycle {
        if self.indeterminate_batches > 0 {
            ProviderLifecycle::Indeterminate
        } else if self.attached_batches == 0 {
            ProviderLifecycle::NoRemoteBatch
        } else if self.terminal_batches > 0
            || self.completed.saturating_add(self.failed) >= self.submitted
        {
            ProviderLifecycle::ReadyToCollect
        } else {
            ProviderLifecycle::InProgress
        }
    }
}

/// Provider request counts grouped by the typed pipeline stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProviderProgress {
    pub triage: ProviderStageProgress,
    pub summaries: ProviderStageProgress,
    pub signals: ProviderStageProgress,
}

impl ProviderProgress {
    pub fn from_peeks(peeks: &[BatchPeek]) -> Self {
        let mut progress = Self::default();
        for peek in peeks {
            let stage = progress.stage_mut(peek.stage);
            stage.attached_batches = stage.attached_batches.saturating_add(1);
            if peek
                .status
                .as_ref()
                .is_some_and(is_terminal_provider_lifecycle)
            {
                stage.terminal_batches = stage.terminal_batches.saturating_add(1);
            }
            match &peek.request_counts {
                Some(counts) => {
                    stage.submitted = stage.submitted.saturating_add(counts.total as usize);
                    stage.completed = stage.completed.saturating_add(counts.completed as usize);
                    stage.failed = stage.failed.saturating_add(counts.failed as usize);
                }
                None => stage.indeterminate_batches = stage.indeterminate_batches.saturating_add(1),
            }
        }
        progress
    }

    pub fn stage(self, stage: StageKind) -> ProviderStageProgress {
        match stage {
            StageKind::Triage => self.triage,
            StageKind::Summary => self.summaries,
            StageKind::SignalCandidate => self.signals,
        }
    }

    pub fn lifecycle(self, stage: StageKind) -> ProviderLifecycle {
        self.stage(stage).lifecycle()
    }

    fn stage_mut(&mut self, stage: StageKind) -> &mut ProviderStageProgress {
        match stage {
            StageKind::Triage => &mut self.triage,
            StageKind::Summary => &mut self.summaries,
            StageKind::SignalCandidate => &mut self.signals,
        }
    }
}

fn is_terminal_provider_lifecycle(lifecycle: &BatchLifecycle) -> bool {
    matches!(
        lifecycle,
        BatchLifecycle::Completed
            | BatchLifecycle::Failed
            | BatchLifecycle::Expired
            | BatchLifecycle::Cancelled
    )
}

/// Wait timing supplied by the runner. Relative values are monotonic; local
/// values are presentation-only and retain their numeric UTC offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitProgress {
    pub last_provider_check: Option<Instant>,
    pub next_provider_check: Option<Instant>,
    pub checked_age: Option<Duration>,
    pub countdown: Option<Duration>,
    pub last_provider_check_local: Option<DateTime<FixedOffset>>,
    pub next_provider_check_local: Option<DateTime<FixedOffset>>,
    pub last_provider_check_display: Option<String>,
    pub next_provider_check_display: Option<String>,
}

/// Runner clock seam for elapsed time, wait scheduling, and local presentation.
pub trait ProgressClock {
    fn monotonic_now(&self) -> Instant;
    fn wall_now(&self) -> DateTime<FixedOffset>;
    fn sleep(&self, duration: Duration);
}

pub struct SystemProgressClock;

impl ProgressClock for SystemProgressClock {
    fn monotonic_now(&self) -> Instant {
        Instant::now()
    }

    fn wall_now(&self) -> DateTime<FixedOffset> {
        Local::now().fixed_offset()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PassCounts {
    pub intake_passes: usize,
    pub collection_passes: usize,
    pub replay_passes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IntakeProgress {
    pub discovered: usize,
    pub fetched: usize,
    pub failed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Copy)]
struct LocalStageCounts {
    total: usize,
    successful: usize,
    failed: usize,
    pending_or_in_flight: usize,
    deferred: usize,
}

/// Complete pure input for a future terminal or append-only renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchProgressSnapshot {
    pub elapsed: Duration,
    pub cost_this_run_microdollars: u64,
    pub intake: IntakeProgress,
    pub triage: StageProgress,
    pub summaries: StageProgress,
    pub signals: StageProgress,
    pub provider: ProviderProgress,
    pub phase: BatchDisplayPhase,
    pub remaining_work: usize,
    pub pass_counts: PassCounts,
    pub wait: Option<WaitProgress>,
}

/// Explicit facts supplied by the runner without reaching into reducer state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProjectionContext {
    pub phase_override: Option<BatchDisplayPhase>,
    pub pass_counts: PassCounts,
    pub cost_this_run_microdollars: u64,
    pub last_provider_check: Option<Instant>,
    pub next_provider_check: Option<Instant>,
    pub last_provider_check_local: Option<DateTime<FixedOffset>>,
    pub next_provider_check_local: Option<DateTime<FixedOffset>>,
}

/// Pure, runner-owned projection state. Its only retained data is the
/// invocation-scoped denominator/intake latches; it performs no I/O and never
/// mutates reducer state.
pub struct BatchProgressProjection {
    baseline: BatchRunBaseline,
    started_at: Instant,
    intake_total: Option<usize>,
    triage_total: usize,
    summary_total: usize,
    signal_total: usize,
}

impl BatchProgressProjection {
    pub fn new(baseline: BatchRunBaseline, started_at: Instant) -> Self {
        Self {
            baseline,
            started_at,
            intake_total: None,
            triage_total: 0,
            summary_total: 0,
            signal_total: 0,
        }
    }

    pub fn snapshot<C: ProgressClock>(
        &mut self,
        observation: &BatchObservation,
        peeks: &[BatchPeek],
        context: ProjectionContext,
        clock: &C,
    ) -> BatchProgressSnapshot {
        let now = clock.monotonic_now();
        let discovered = observation
            .jobs_total
            .saturating_sub(self.baseline.jobs_total);
        let fetched = observation
            .jobs_done
            .saturating_sub(self.baseline.jobs_done);
        let failed = observation
            .jobs_failed
            .saturating_sub(self.baseline.jobs_failed);
        if !observation.poll_in_progress {
            self.intake_total.get_or_insert(discovered);
        }
        let intake = IntakeProgress {
            discovered,
            fetched,
            failed,
            total: self.intake_total.unwrap_or(discovered),
        };
        let provider = ProviderProgress::from_peeks(peeks);
        let triage = self.stage_progress(
            StageKind::Triage,
            LocalStageCounts {
                total: observation.triage_total,
                successful: observation.triage_completed,
                failed: observation.triage_failed,
                pending_or_in_flight: observation
                    .triage_pending
                    .saturating_add(observation.triage_in_flight),
                deferred: observation.triage_deferred,
            },
            provider.stage(StageKind::Triage),
        );
        let summaries = self.stage_progress(
            StageKind::Summary,
            LocalStageCounts {
                total: observation.summary_total,
                successful: observation.summary_completed,
                failed: observation.summary_failed,
                pending_or_in_flight: observation
                    .summary_pending
                    .saturating_add(observation.summary_in_flight),
                deferred: observation.summary_deferred,
            },
            provider.stage(StageKind::Summary),
        );
        let signals = self.stage_progress(
            StageKind::SignalCandidate,
            LocalStageCounts {
                total: observation.signal_total,
                successful: observation.signal_completed,
                failed: observation.signal_failed,
                pending_or_in_flight: observation.signal_pending_or_in_flight,
                deferred: observation.signal_deferred,
            },
            provider.stage(StageKind::SignalCandidate),
        );
        let phase = classify_display_phase(observation, &provider, context.phase_override);
        let wait = wait_progress(&context, now);
        BatchProgressSnapshot {
            elapsed: now.saturating_duration_since(self.started_at),
            cost_this_run_microdollars: context.cost_this_run_microdollars,
            intake,
            triage,
            summaries,
            signals,
            provider,
            phase,
            remaining_work: triage
                .local_remaining
                .saturating_add(summaries.local_remaining)
                .saturating_add(signals.local_remaining),
            pass_counts: context.pass_counts,
            wait,
        }
    }

    fn stage_progress(
        &mut self,
        stage: StageKind,
        local: LocalStageCounts,
        provider: ProviderStageProgress,
    ) -> StageProgress {
        let latched_total = match stage {
            StageKind::Triage => {
                self.triage_total = self.triage_total.max(local.total);
                self.triage_total
            }
            StageKind::Summary => {
                self.summary_total = self.summary_total.max(local.total);
                self.summary_total
            }
            StageKind::SignalCandidate => {
                self.signal_total = self.signal_total.max(local.total);
                self.signal_total
            }
        };
        let total = latched_total.max(provider.submitted);
        let local_settled = local.successful.saturating_add(local.failed);
        StageProgress {
            total,
            successful: local.successful,
            failed: local.failed,
            pending_or_in_flight: local.pending_or_in_flight,
            deferred: local.deferred,
            provider_total: provider.submitted,
            provider_completed: provider.completed,
            provisional_settled: local_settled.saturating_add(provider.completed).min(total),
            local_remaining: local.pending_or_in_flight.saturating_add(local.deferred),
            unsubmitted: local.deferred.saturating_sub(provider.submitted),
        }
    }
}

/// Formats an operator-facing absolute timestamp without changing durable UTC
/// timestamps elsewhere in the runner.
pub fn format_local_timestamp(timestamp: DateTime<FixedOffset>) -> String {
    timestamp.format("%Y-%m-%d %H:%M:%S %:z").to_string()
}

fn wait_progress(context: &ProjectionContext, now: Instant) -> Option<WaitProgress> {
    let has_wait = context.last_provider_check.is_some()
        || context.next_provider_check.is_some()
        || context.last_provider_check_local.is_some()
        || context.next_provider_check_local.is_some();
    has_wait.then(|| WaitProgress {
        last_provider_check: context.last_provider_check,
        next_provider_check: context.next_provider_check,
        checked_age: context
            .last_provider_check
            .map(|last| now.saturating_duration_since(last)),
        countdown: context
            .next_provider_check
            .map(|next| next.saturating_duration_since(now)),
        last_provider_check_local: context.last_provider_check_local,
        next_provider_check_local: context.next_provider_check_local,
        last_provider_check_display: context
            .last_provider_check_local
            .map(format_local_timestamp),
        next_provider_check_display: context
            .next_provider_check_local
            .map(format_local_timestamp),
    })
}

/// Classifies one active phase from reducer/provider state. Signal work is
/// intentionally considered before the terminal fallback.
pub fn classify_display_phase(
    observation: &BatchObservation,
    provider: &ProviderProgress,
    phase_override: Option<BatchDisplayPhase>,
) -> BatchDisplayPhase {
    if let Some(phase) = phase_override {
        return phase;
    }
    if observation.poll_in_progress
        || (observation.jobs_total > 0
            && observation
                .jobs_done
                .saturating_add(observation.jobs_failed)
                < observation.jobs_total)
    {
        return BatchDisplayPhase::Intake;
    }
    for (stage, pending, deferred) in [
        (
            StageKind::Triage,
            observation
                .triage_pending
                .saturating_add(observation.triage_in_flight),
            observation.triage_deferred,
        ),
        (
            StageKind::Summary,
            observation
                .summary_pending
                .saturating_add(observation.summary_in_flight),
            observation.summary_deferred,
        ),
        (
            StageKind::SignalCandidate,
            observation.signal_pending_or_in_flight,
            observation.signal_deferred,
        ),
    ] {
        if pending > 0 {
            return stage_display_phase(stage);
        }
        if deferred > 0 {
            return match provider.lifecycle(stage) {
                ProviderLifecycle::NoRemoteBatch => BatchDisplayPhase::PreparingBatch,
                ProviderLifecycle::ReadyToCollect => BatchDisplayPhase::Collecting,
                ProviderLifecycle::InProgress | ProviderLifecycle::Indeterminate => {
                    BatchDisplayPhase::WaitingForProvider
                }
            };
        }
    }
    BatchDisplayPhase::Complete
}

fn stage_display_phase(stage: StageKind) -> BatchDisplayPhase {
    match stage {
        StageKind::Triage => BatchDisplayPhase::Triage,
        StageKind::Summary => BatchDisplayPhase::Summaries,
        StageKind::SignalCandidate => BatchDisplayPhase::Signals,
    }
}

pub struct ProgressReporter {
    selected: usize,
    stale_total: usize,
    limit: usize,
    concurrency: usize,
    ok: usize,
    fail: usize,
    pending: usize,
    start: Instant,
    enabled: bool,
    last_line_width: usize,
    painted_status: bool,
}

impl ProgressReporter {
    pub fn new(
        selected: usize,
        stale_total: usize,
        limit: usize,
        concurrency: usize,
        enabled: bool,
    ) -> Self {
        Self {
            selected,
            stale_total,
            limit,
            concurrency,
            ok: 0,
            fail: 0,
            pending: 0,
            start: Instant::now(),
            enabled,
            last_line_width: 0,
            painted_status: false,
        }
    }

    pub fn startup_line<W: Write>(&self, stdout: &mut W) {
        if !self.enabled {
            return;
        }
        let _ = writeln!(
            stdout,
            "[batch] refresh-stale-summaries: selected={} stale_total={} limit={} concurrency={}",
            self.selected, self.stale_total, self.limit, self.concurrency
        );
    }

    pub fn request_dispatched(&mut self) {
        if !self.enabled {
            return;
        }
        self.pending = self.pending.saturating_add(1);
    }

    pub fn completed_ok<W: Write>(&mut self, stdout: &mut W) {
        if !self.enabled {
            return;
        }
        self.ok = self.ok.saturating_add(1);
        self.pending = self.pending.saturating_sub(1);
        self.render_status(stdout);
    }

    pub fn completed_fail<O: Write, E: Write>(
        &mut self,
        url: &str,
        reason: &str,
        stdout: &mut O,
        stderr: &mut E,
    ) {
        if !self.enabled {
            return;
        }
        self.fail = self.fail.saturating_add(1);
        self.pending = self.pending.saturating_sub(1);
        self.clear_status_row(stdout);
        write_failure_line(stderr, url, reason);
        self.render_status(stdout);
    }

    pub fn unloadable_target<O: Write, E: Write>(
        &mut self,
        url: &str,
        reason: &str,
        stdout: &mut O,
        stderr: &mut E,
    ) {
        if !self.enabled {
            return;
        }
        self.fail = self.fail.saturating_add(1);
        // This target was never dispatched to the LLM, so `pending` is unchanged.
        self.clear_status_row(stdout);
        write_failure_line(stderr, url, reason);
        self.render_status(stdout);
    }

    pub fn finish<W: Write>(
        &mut self,
        ok: usize,
        fail: usize,
        cost_display: &str,
        report_path: &Path,
        stdout: &mut W,
    ) {
        if !self.enabled {
            return;
        }
        let elapsed = format_elapsed(self.start.elapsed());
        let _ = writeln!(
            stdout,
            "\n[batch] done: selected={} ok={} fail={} elapsed={} cost={} -> {}",
            self.selected,
            ok,
            fail,
            elapsed,
            cost_display,
            report_path.display()
        );
        let _ = stdout.flush();
        self.painted_status = false;
    }

    /// Test hook for Drop-time cleanup. The real `Drop` impl writes to stdout.
    pub fn drop_cleanup_into<W: Write>(&mut self, stdout: &mut W) {
        if !self.enabled || !self.painted_status {
            return;
        }
        let _ = writeln!(stdout);
        self.painted_status = false;
    }

    fn render_status<W: Write>(&mut self, stdout: &mut W) {
        let completed = self.ok + self.fail;
        let eta = format_eta(completed, self.selected, self.start.elapsed());
        let body = format!(
            "[ {}/{}  ok={}  fail={}  pending={}  ETA {} ]",
            completed, self.selected, self.ok, self.fail, self.pending, eta
        );
        let pad = self.last_line_width.saturating_sub(body.len());
        let _ = write!(stdout, "\r{}{:pad$}", body, "", pad = pad);
        let _ = stdout.flush();
        self.last_line_width = body.len();
        self.painted_status = true;
    }

    fn clear_status_row<W: Write>(&mut self, stdout: &mut W) {
        if !self.painted_status {
            return;
        }
        let _ = write!(stdout, "\r{:width$}\r", "", width = self.last_line_width);
        let _ = stdout.flush();
        self.painted_status = false;
        self.last_line_width = 0;
    }
}

impl Drop for ProgressReporter {
    fn drop(&mut self) {
        if self.enabled && self.painted_status {
            let mut out = std::io::stdout();
            self.drop_cleanup_into(&mut out);
        }
    }
}

pub fn format_eta(completed: usize, selected: usize, elapsed: Duration) -> String {
    if selected == 0 {
        return "0:00".to_string();
    }
    if completed == 0 {
        return "--:--".to_string();
    }
    let remaining = selected.saturating_sub(completed);
    if remaining == 0 {
        return "0:00".to_string();
    }
    let elapsed_secs = elapsed.as_secs() as u128;
    let eta_secs = (elapsed_secs * remaining as u128) / completed as u128;
    let eta_secs = eta_secs.min(u64::MAX as u128) as u64;
    format!("{}:{:02}", eta_secs / 60, eta_secs % 60)
}

fn format_elapsed(d: Duration) -> String {
    let secs = d.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Live progress reporter for `--import-saved-web-dir` mode.
///
/// Renders a single overwritten status line covering all three pipeline phases
/// (import → triage → summary). Disabled when stdout/stderr are not terminals.
pub struct ImportProgressReporter {
    enabled: bool,
    last_line_width: usize,
    painted_status: bool,
    start: Instant,
}

impl ImportProgressReporter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_line_width: 0,
            painted_status: false,
            start: Instant::now(),
        }
    }

    pub fn startup_line<W: Write>(&self, stdout: &mut W) {
        if !self.enabled {
            return;
        }
        let _ = writeln!(stdout, "[import] starting: import -> triage -> summary");
    }

    pub fn update_from_obs<O: Write, E: Write>(
        &mut self,
        obs: &harvester_core::BatchObservation,
        stdout: &mut O,
        _stderr: &mut E,
    ) {
        if !self.enabled {
            return;
        }
        let elapsed = format_elapsed(self.start.elapsed());
        let body = format!(
            "[import] {}  import={}/{} fail={}  triage={}/{} fail={}  summary={}/{} fail={}  t={}",
            phase_label(obs),
            obs.imports_completed,
            obs.imports_completed + obs.imports_failed,
            obs.imports_failed,
            obs.triage_completed,
            obs.triage_total,
            obs.triage_failed,
            obs.summary_completed,
            obs.summary_total,
            obs.summary_failed,
            elapsed,
        );
        let pad = self.last_line_width.saturating_sub(body.len());
        let _ = write!(stdout, "\r{}{:pad$}", body, "", pad = pad);
        let _ = stdout.flush();
        self.last_line_width = body.len();
        self.painted_status = true;
    }

    pub fn finish<W: Write>(&mut self, cost_display: &str, stdout: &mut W) {
        if !self.enabled {
            return;
        }
        let elapsed = format_elapsed(self.start.elapsed());
        if self.painted_status {
            let _ = writeln!(stdout);
        }
        let _ = writeln!(
            stdout,
            "[import] done  elapsed={}  cost={}",
            elapsed, cost_display
        );
        let _ = stdout.flush();
        self.painted_status = false;
    }
}

impl Drop for ImportProgressReporter {
    fn drop(&mut self) {
        if self.enabled && self.painted_status {
            let mut out = std::io::stdout();
            let _ = writeln!(out);
        }
    }
}

/// Live progress reporter for regular batch mode (poll → triage → summaries).
///
/// Renders a single overwritten status line each loop iteration.
/// Disabled when stdout is not a terminal.
pub struct BatchProgressReporter {
    enabled: bool,
    last_line_width: usize,
    painted_status: bool,
    start: Instant,
    frame_index: usize,
}

impl BatchProgressReporter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_line_width: 0,
            painted_status: false,
            start: Instant::now(),
            frame_index: 0,
        }
    }

    pub fn update_from_obs<W: Write>(
        &mut self,
        obs: &harvester_core::BatchObservation,
        stdout: &mut W,
    ) {
        if !self.enabled {
            return;
        }
        let elapsed = format_elapsed(self.start.elapsed());
        let phase = batch_phase_label(obs);
        let frame = ["|", "/", "-", "\\"][self.frame_index % 4];
        self.frame_index = self.frame_index.wrapping_add(1);
        let body = format!(
            "[batch] {} {}  jobs={}/{} fail={} active={}  triage={}/{} fail={}  summary={}/{} fail={}  t={}",
            phase,
            frame,
            obs.jobs_done,
            obs.jobs_total,
            obs.jobs_failed,
            obs.jobs_in_flight,
            obs.triage_completed,
            obs.triage_total,
            obs.triage_failed,
            obs.summary_completed,
            obs.summary_total,
            obs.summary_failed,
            elapsed,
        );
        let pad = self.last_line_width.saturating_sub(body.len());
        let _ = write!(stdout, "\r{}{:pad$}", body, "", pad = pad);
        let _ = stdout.flush();
        self.last_line_width = body.len();
        self.painted_status = true;
    }

    /// Renders a one-off status using the same terminal/plain-output
    /// convention as batch progress updates.
    pub fn status_line<W: Write>(&mut self, line: &str, stdout: &mut W) {
        if self.enabled {
            let pad = self.last_line_width.saturating_sub(line.len());
            let _ = write!(stdout, "\r{}{:pad$}", line, "", pad = pad);
            let _ = stdout.flush();
            self.last_line_width = line.len();
            self.painted_status = true;
        } else {
            let _ = writeln!(stdout, "{}", line);
        }
    }

    /// Call before printing the per-cycle table so the status line is cleared.
    pub fn finish_cycle<W: Write>(&mut self, stdout: &mut W) {
        if self.enabled && self.painted_status {
            let _ = writeln!(stdout);
            self.painted_status = false;
            self.last_line_width = 0;
        }
    }
}

impl Drop for BatchProgressReporter {
    fn drop(&mut self) {
        if self.enabled && self.painted_status {
            let _ = writeln!(std::io::stdout());
        }
    }
}

fn batch_phase_label(obs: &harvester_core::BatchObservation) -> &'static str {
    if obs.poll_in_progress
        || (obs.jobs_total > 0 && obs.jobs_done + obs.jobs_failed < obs.jobs_total)
    {
        return "FETCHING ";
    }
    if obs.triage_in_flight > 0 || obs.triage_pending > 0 {
        return "TRIAGING ";
    }
    if obs.summary_in_flight > 0 || obs.summary_pending > 0 {
        return "SUMMARIZE";
    }
    "SETTLING "
}

fn phase_label(obs: &harvester_core::BatchObservation) -> &'static str {
    use harvester_core::ImportPhase;
    if obs.import_in_flight || matches!(obs.import_phase, ImportPhase::Importing) {
        return "IMPORTING";
    }
    if obs.triage_in_flight > 0 || obs.triage_pending > 0 {
        return "TRIAGING ";
    }
    if obs.summary_in_flight > 0 || obs.summary_pending > 0 {
        return "SUMMARIZE";
    }
    "SETTLING "
}

fn write_failure_line<W: Write>(stderr: &mut W, url: &str, reason: &str) {
    let normalized: String = reason
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let truncated = if normalized.chars().count() > FAIL_REASON_MAX_CHARS {
        let head: String = normalized.chars().take(FAIL_REASON_MAX_CHARS).collect();
        format!("{head}...")
    } else {
        normalized
    };
    let _ = writeln!(stderr, "FAIL {url} - {truncated}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use harvester_core::{
        BatchObservation, ImportPhase, PreTriagePhase, SessionState, TriagePhase,
    };
    use openai_provider_kit::{BatchLifecycle, BatchRequestCounts};
    use std::cell::{Cell, RefCell};
    use std::path::Path;
    use std::time::Duration;

    fn import_obs_idle() -> BatchObservation {
        BatchObservation {
            poll_in_progress: false,
            session_state: SessionState::Idle,
            jobs_total: 0,
            jobs_done: 0,
            jobs_failed: 0,
            jobs_in_flight: 0,
            pre_triage_phase: PreTriagePhase::Idle,
            pre_triage_total: 0,
            pre_triage_included: 0,
            pre_triage_review: 0,
            pre_triage_filtered: 0,
            triage_phase: TriagePhase::Idle,
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
            import_phase: ImportPhase::Idle,
            imports_completed: 0,
            imports_failed: 0,
            import_in_flight: false,
            source_poll_stats: vec![],
        }
    }

    #[test]
    fn import_progress_startup_line_contains_key_fields() {
        let reporter = ImportProgressReporter::new(true);
        let mut out = Vec::<u8>::new();
        reporter.startup_line(&mut out);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("[import]"), "startup line missing prefix: {s:?}");
        assert!(
            s.ends_with('\n'),
            "startup line must end with newline: {s:?}"
        );
    }

    #[test]
    fn import_progress_disabled_startup_writes_nothing() {
        let reporter = ImportProgressReporter::new(false);
        let mut out = Vec::<u8>::new();
        reporter.startup_line(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn import_progress_update_writes_status_line() {
        let mut reporter = ImportProgressReporter::new(true);
        let mut obs = import_obs_idle();
        obs.imports_completed = 3;
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.update_from_obs(&obs, &mut out, &mut err);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.starts_with('\r'), "status line must start with CR: {s:?}");
        assert!(s.contains("3"), "should show import count: {s:?}");
    }

    #[test]
    fn import_progress_disabled_update_writes_nothing() {
        let mut reporter = ImportProgressReporter::new(false);
        let obs = import_obs_idle();
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.update_from_obs(&obs, &mut out, &mut err);
        assert!(out.is_empty() && err.is_empty());
    }

    #[test]
    fn import_progress_finish_prints_summary_line() {
        let mut reporter = ImportProgressReporter::new(true);
        let obs = import_obs_idle();
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.update_from_obs(&obs, &mut out, &mut err);
        out.clear();
        reporter.finish("$0.01", &mut out);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("$0.01"), "finish must show cost: {s:?}");
        assert!(s.contains("[import]"), "finish must show prefix: {s:?}");
        assert!(s.ends_with('\n'), "finish must end with newline: {s:?}");
        assert!(!s.contains('\r'), "finish line must not contain CR: {s:?}");
    }

    #[test]
    fn import_progress_update_shows_failure_counts() {
        let mut reporter = ImportProgressReporter::new(true);
        let mut obs = import_obs_idle();
        obs.imports_completed = 5;
        obs.imports_failed = 2;
        obs.triage_completed = 3;
        obs.triage_total = 5;
        obs.triage_failed = 1;
        obs.summary_completed = 1;
        obs.summary_total = 3;
        obs.summary_failed = 1;
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.update_from_obs(&obs, &mut out, &mut err);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.contains("import=5/7 fail=2"),
            "must show import failures: {s:?}"
        );
        assert!(
            s.contains("triage=3/5 fail=1"),
            "must show triage failures: {s:?}"
        );
        assert!(
            s.contains("summary=1/3 fail=1"),
            "must show summary failures: {s:?}"
        );
    }

    #[test]
    fn format_eta_zero_completed_returns_dashes() {
        assert_eq!(format_eta(0, 10, Duration::from_secs(5)), "--:--");
    }

    #[test]
    fn format_eta_partial_progress_returns_minutes_seconds() {
        assert_eq!(format_eta(1, 10, Duration::from_secs(10)), "1:30");
    }

    #[test]
    fn format_eta_all_completed_returns_zero() {
        assert_eq!(format_eta(10, 10, Duration::from_secs(10)), "0:00");
    }

    #[test]
    fn format_eta_clamps_to_zero_when_completed_exceeds_selected() {
        assert_eq!(format_eta(11, 10, Duration::from_secs(10)), "0:00");
    }

    #[test]
    fn format_eta_zero_selected_returns_zero() {
        assert_eq!(format_eta(0, 0, Duration::from_secs(5)), "0:00");
    }

    #[test]
    fn startup_line_emits_expected_fields() {
        let reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut out = Vec::<u8>::new();
        reporter.startup_line(&mut out);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.contains("selected=50")
                && s.contains("stale_total=137")
                && s.contains("limit=50")
                && s.contains("concurrency=6"),
            "unexpected startup line: {s:?}"
        );
        assert!(s.starts_with("[batch] refresh-stale-summaries:"));
        assert!(
            s.ends_with('\n'),
            "startup line must end with newline: {s:?}"
        );
    }

    #[test]
    fn startup_line_disabled_writes_nothing() {
        let reporter = ProgressReporter::new(50, 137, 50, 6, false);
        let mut out = Vec::<u8>::new();
        reporter.startup_line(&mut out);
        assert!(out.is_empty(), "disabled reporter must not write");
    }

    #[test]
    fn completed_ok_increments_counts_and_redraws() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut out = Vec::<u8>::new();
        reporter.request_dispatched();
        reporter.request_dispatched();
        reporter.completed_ok(&mut out);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("1/50"), "completed count: {s:?}");
        assert!(s.contains("ok=1"), "ok count: {s:?}");
        assert!(s.contains("fail=0"), "fail count: {s:?}");
        assert!(s.contains("pending=1"), "pending count: {s:?}");
        assert!(s.contains("ETA "), "ETA field present: {s:?}");
        assert!(s.starts_with('\r'), "status line begins with CR: {s:?}");
    }

    #[test]
    fn request_dispatched_alone_does_not_paint_status() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let out = Vec::<u8>::new();
        reporter.request_dispatched();
        assert!(out.is_empty(), "dispatch must not paint a status line");
    }

    #[test]
    fn completed_ok_disabled_writes_nothing() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, false);
        let mut out = Vec::<u8>::new();
        reporter.request_dispatched();
        reporter.completed_ok(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn completed_fail_writes_sticky_stderr_and_redraws_status() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.request_dispatched();
        reporter.completed_fail("https://example.com/a", "rate-limit", &mut out, &mut err);
        let stderr_s = std::str::from_utf8(&err).unwrap();
        assert!(
            stderr_s.contains("FAIL https://example.com/a"),
            "{stderr_s:?}"
        );
        assert!(stderr_s.contains("rate-limit"), "{stderr_s:?}");
        assert!(stderr_s.ends_with('\n'), "{stderr_s:?}");
        let stdout_s = std::str::from_utf8(&out).unwrap();
        assert!(
            stdout_s.contains("fail=1") && stdout_s.contains("pending=0"),
            "{stdout_s:?}"
        );
    }

    #[test]
    fn completed_fail_balances_pending_with_request_dispatched() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        for _ in 0..3 {
            reporter.request_dispatched();
        }
        reporter.completed_fail("u", "r", &mut out, &mut err);
        let stdout_s = std::str::from_utf8(&out).unwrap();
        assert!(stdout_s.contains("pending=2"), "{stdout_s:?}");
    }

    #[test]
    fn completed_fail_truncates_long_reason() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.request_dispatched();
        let long_reason = "x".repeat(200);
        reporter.completed_fail("u", &long_reason, &mut out, &mut err);
        let stderr_s = std::str::from_utf8(&err).unwrap();
        assert!(
            stderr_s.lines().next().unwrap().len() < 160,
            "stderr line not truncated: {stderr_s:?}"
        );
    }

    #[test]
    fn completed_fail_normalizes_control_chars_in_reason() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.request_dispatched();
        reporter.completed_fail("u", "line1\nline2\rline3\tend", &mut out, &mut err);
        let stderr_s = std::str::from_utf8(&err).unwrap();
        assert_eq!(stderr_s.matches('\n').count(), 1, "{stderr_s:?}");
        assert_eq!(stderr_s.matches('\r').count(), 0, "{stderr_s:?}");
        assert_eq!(stderr_s.matches('\t').count(), 0, "{stderr_s:?}");
        assert!(stderr_s.contains("line1"));
        assert!(stderr_s.contains("line2"));
        assert!(stderr_s.contains("line3"));
        assert!(stderr_s.contains("end"));
    }

    #[test]
    fn completed_fail_clears_active_status_row_before_failure() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.request_dispatched();
        reporter.completed_ok(&mut out);
        out.clear();
        reporter.request_dispatched();
        reporter.completed_fail("u", "r", &mut out, &mut err);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.starts_with('\r'), "must start with CR: {s:?}");
        let first_status_idx = s.find('[').expect("new status line in output");
        let prefix = &s[..first_status_idx];
        let cr_count = prefix.matches('\r').count();
        assert!(
            cr_count >= 2,
            "expected at least 2 CRs before new status, got {cr_count}: {prefix:?}"
        );
        let inner = prefix.trim_start_matches('\r').trim_end_matches('\r');
        assert!(
            !inner.is_empty() && inner.chars().all(|c| c == ' '),
            "expected only spaces between CRs in clear sequence, got: {inner:?}"
        );
    }

    #[test]
    fn completed_fail_disabled_writes_nothing() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, false);
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.request_dispatched();
        reporter.completed_fail("u", "r", &mut out, &mut err);
        assert!(out.is_empty() && err.is_empty());
    }

    #[test]
    fn unloadable_target_increments_fail_without_touching_pending() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        for _ in 0..3 {
            reporter.unloadable_target(
                "https://example.com/missing",
                "selected article could not be loaded from archive",
                &mut out,
                &mut err,
            );
        }
        let stdout_s = std::str::from_utf8(&out).unwrap();
        assert!(stdout_s.contains("fail=3"), "{stdout_s:?}");
        assert!(stdout_s.contains("pending=0"), "{stdout_s:?}");
        let stderr_s = std::str::from_utf8(&err).unwrap();
        assert_eq!(stderr_s.lines().count(), 3, "{stderr_s:?}");
        assert!(stderr_s.contains("could not be loaded"));
    }

    #[test]
    fn unloadable_target_does_not_underflow_when_pending_already_zero() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.unloadable_target("u", "r", &mut out, &mut err);
        let stdout_s = std::str::from_utf8(&out).unwrap();
        assert!(stdout_s.contains("pending=0"));
    }

    #[test]
    fn unloadable_target_clears_active_status_row_before_failure() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.unloadable_target("u1", "r1", &mut out, &mut err);
        out.clear();
        reporter.unloadable_target("u2", "r2", &mut out, &mut err);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.starts_with('\r'), "must start with CR: {s:?}");
        let first_status_idx = s.find('[').expect("new status line");
        let prefix = &s[..first_status_idx];
        assert!(
            prefix.matches('\r').count() >= 2,
            "expected at least 2 CRs before new status: {prefix:?}"
        );
    }

    #[test]
    fn unloadable_target_disabled_writes_nothing() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, false);
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.unloadable_target("u", "r", &mut out, &mut err);
        assert!(out.is_empty() && err.is_empty());
    }

    #[test]
    fn finish_writes_done_line_with_path_and_no_cr() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut out = Vec::<u8>::new();
        for _ in 0..50 {
            reporter.request_dispatched();
        }
        for _ in 0..48 {
            reporter.completed_ok(&mut out);
        }
        out.clear();

        let path = Path::new("output/summary_refresh_reports/summary-refresh-20260510.json");
        reporter.finish(48, 2, "$0.018", path, &mut out);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.starts_with('\n'),
            "finish moves to a new row first: {s:?}"
        );
        assert!(s.contains("selected=50"));
        assert!(s.contains("ok=48"));
        assert!(s.contains("fail=2"));
        assert!(s.contains("$0.018"));
        assert!(s.contains("summary-refresh-20260510.json"));
        assert!(!s.contains('\r'));
        assert!(s.ends_with('\n'));
    }

    #[test]
    fn finish_disabled_writes_nothing() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, false);
        let mut out = Vec::<u8>::new();
        let path = Path::new("p.json");
        reporter.finish(0, 0, "$0.00", path, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn drop_cleanup_emits_newline_when_status_was_painted_and_finish_not_called() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut sink = Vec::<u8>::new();
        reporter.request_dispatched();
        reporter.completed_ok(&mut sink);
        let mut cleanup = Vec::<u8>::new();
        reporter.drop_cleanup_into(&mut cleanup);
        assert_eq!(cleanup, b"\n");
    }

    #[test]
    fn drop_cleanup_is_silent_when_no_status_was_painted() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut cleanup = Vec::<u8>::new();
        reporter.drop_cleanup_into(&mut cleanup);
        assert!(cleanup.is_empty());
    }

    #[test]
    fn drop_cleanup_is_silent_when_disabled() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, false);
        let mut cleanup = Vec::<u8>::new();
        reporter.drop_cleanup_into(&mut cleanup);
        assert!(cleanup.is_empty());
    }

    #[test]
    fn finish_marks_cleanup_done_so_drop_is_silent() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, true);
        let mut sink = Vec::<u8>::new();
        reporter.request_dispatched();
        reporter.completed_ok(&mut sink);
        let path = Path::new("p.json");
        reporter.finish(1, 0, "$0.00", path, &mut sink);
        let mut cleanup = Vec::<u8>::new();
        reporter.drop_cleanup_into(&mut cleanup);
        assert!(cleanup.is_empty());
    }

    #[test]
    fn full_disabled_walkthrough_writes_nothing() {
        let mut reporter = ProgressReporter::new(50, 137, 50, 6, false);
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.startup_line(&mut out);
        for _ in 0..3 {
            reporter.request_dispatched();
        }
        reporter.completed_ok(&mut out);
        reporter.completed_fail("u", "r", &mut out, &mut err);
        reporter.unloadable_target("u2", "r2", &mut out, &mut err);
        reporter.finish(1, 1, "$0.00", Path::new("p.json"), &mut out);
        let mut cleanup = Vec::<u8>::new();
        reporter.drop_cleanup_into(&mut cleanup);
        assert!(out.is_empty() && err.is_empty() && cleanup.is_empty());
    }

    #[test]
    fn batch_progress_update_writes_status_line_with_counts() {
        let mut reporter = BatchProgressReporter::new(true);
        let mut obs = import_obs_idle();
        obs.jobs_total = 10;
        obs.jobs_done = 7;
        obs.jobs_failed = 1;
        obs.jobs_in_flight = 2;
        obs.triage_completed = 5;
        obs.triage_total = 8;
        obs.triage_failed = 2;
        obs.summary_completed = 1;
        obs.summary_total = 3;
        obs.summary_failed = 1;
        let mut out = Vec::<u8>::new();
        reporter.update_from_obs(&obs, &mut out);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.starts_with('\r'), "must start with CR: {s:?}");
        assert!(s.contains("[batch]"), "must show prefix: {s:?}");
        assert!(
            s.contains("jobs=7/10 fail=1 active=2"),
            "must show job counts: {s:?}"
        );
        assert!(
            s.contains("triage=5/8 fail=2"),
            "must show triage counts: {s:?}"
        );
        assert!(
            s.contains("summary=1/3 fail=1"),
            "must show summary counts: {s:?}"
        );
    }

    #[test]
    fn batch_progress_heartbeat_changes_between_updates() {
        let mut reporter = BatchProgressReporter::new(true);
        let obs = import_obs_idle();
        let mut out = Vec::<u8>::new();
        reporter.update_from_obs(&obs, &mut out);
        reporter.update_from_obs(&obs, &mut out);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.contains("SETTLING  |") && s.contains("SETTLING  /"),
            "must show a changing heartbeat: {s:?}"
        );
    }

    #[test]
    fn batch_progress_disabled_writes_nothing() {
        let mut reporter = BatchProgressReporter::new(false);
        let obs = import_obs_idle();
        let mut out = Vec::<u8>::new();
        reporter.update_from_obs(&obs, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn batch_progress_finish_cycle_clears_status_line() {
        let mut reporter = BatchProgressReporter::new(true);
        let obs = import_obs_idle();
        let mut out = Vec::<u8>::new();
        reporter.update_from_obs(&obs, &mut out);
        out.clear();
        reporter.finish_cycle(&mut out);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(
            s.ends_with('\n'),
            "finish_cycle must end with newline: {s:?}"
        );
    }

    struct ManualProgressClock {
        start: Instant,
        elapsed: Cell<Duration>,
        wall: RefCell<DateTime<FixedOffset>>,
    }

    impl ManualProgressClock {
        fn new(wall: DateTime<FixedOffset>) -> Self {
            Self {
                start: Instant::now(),
                elapsed: Cell::new(Duration::ZERO),
                wall: RefCell::new(wall),
            }
        }

        fn advance(&self, duration: Duration) {
            self.elapsed
                .set(self.elapsed.get().saturating_add(duration));
            let wall = *self.wall.borrow();
            *self.wall.borrow_mut() = wall + chrono::Duration::from_std(duration).unwrap();
        }
    }

    impl ProgressClock for ManualProgressClock {
        fn monotonic_now(&self) -> Instant {
            self.start + self.elapsed.get()
        }

        fn wall_now(&self) -> DateTime<FixedOffset> {
            *self.wall.borrow()
        }

        fn sleep(&self, duration: Duration) {
            self.advance(duration);
        }
    }

    fn projection(
        clock: &ManualProgressClock,
        observation: &BatchObservation,
    ) -> BatchProgressProjection {
        BatchProgressProjection::new(
            BatchRunBaseline::from_observation(observation),
            clock.monotonic_now(),
        )
    }

    fn peek(
        stage: StageKind,
        batch_id: &str,
        status: Option<BatchLifecycle>,
        completed: u32,
        total: u32,
    ) -> BatchPeek {
        BatchPeek {
            batch_id: batch_id.into(),
            stage,
            status: status.clone(),
            request_counts: status.map(|_| BatchRequestCounts {
                total,
                completed,
                failed: 0,
            }),
        }
    }

    fn snapshot(
        projection: &mut BatchProgressProjection,
        observation: &BatchObservation,
        peeks: &[BatchPeek],
        clock: &ManualProgressClock,
    ) -> BatchProgressSnapshot {
        projection.snapshot(observation, peeks, ProjectionContext::default(), clock)
    }

    #[test]
    fn projection_uses_current_run_intake_deltas_and_freezes_total() {
        let mut initial = import_obs_idle();
        initial.jobs_total = 7_225;
        initial.jobs_done = 7_218;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(2 * 3600)
                .unwrap()
                .with_ymd_and_hms(2026, 7, 23, 9, 48, 30)
                .unwrap(),
        );
        let mut progress = projection(&clock, &initial);
        let mut observed = initial.clone();
        observed.jobs_total += 76;
        observed.jobs_done += 69;
        observed.jobs_failed += 7;
        let first = snapshot(&mut progress, &observed, &[], &clock);
        assert_eq!(first.intake.discovered, 76);
        assert_eq!(first.intake.fetched, 69);
        assert_eq!(first.intake.failed, 7);
        assert_eq!(first.intake.total, 76);

        observed.jobs_total += 2;
        let after_settlement = snapshot(&mut progress, &observed, &[], &clock);
        assert_eq!(after_settlement.intake.discovered, 78);
        assert_eq!(after_settlement.intake.total, 76);
    }

    #[test]
    fn signal_work_selects_signals_before_terminal_fallback() {
        let mut observation = import_obs_idle();
        observation.signal_total = 7;
        observation.signal_pending_or_in_flight = 2;
        observation.signal_deferred = 5;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        assert_eq!(
            snapshot(&mut progress, &observation, &[], &clock).phase,
            BatchDisplayPhase::Signals
        );
    }

    #[test]
    fn provider_progress_is_provisional_without_replacing_local_denominator() {
        let mut observation = import_obs_idle();
        observation.signal_total = 32;
        observation.signal_deferred = 32;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(
            &mut progress,
            &observation,
            &[peek(
                StageKind::SignalCandidate,
                "batch-signal",
                Some(BatchLifecycle::InProgress),
                25,
                32,
            )],
            &clock,
        );
        assert_eq!(snapshot.signals.total, 32);
        assert_eq!(snapshot.signals.settled(), 0);
        assert_eq!(snapshot.signals.provisional_settled, 25);
    }

    #[test]
    fn budget_capped_provider_scope_keeps_unsubmitted_work_visible() {
        let mut observation = import_obs_idle();
        observation.signal_total = 50;
        observation.signal_deferred = 50;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(
            &mut progress,
            &observation,
            &[peek(
                StageKind::SignalCandidate,
                "batch-signal",
                Some(BatchLifecycle::InProgress),
                25,
                32,
            )],
            &clock,
        );
        assert_eq!(snapshot.signals.total, 50);
        assert_eq!(snapshot.signals.provider_total, 32);
        assert_eq!(snapshot.signals.unsubmitted, 18);
    }

    #[test]
    fn same_stage_peeks_aggregate_by_typed_stage_after_chunking() {
        let peeks = [
            peek(
                StageKind::Triage,
                "batch-one",
                Some(BatchLifecycle::InProgress),
                10,
                16,
            ),
            peek(
                StageKind::Triage,
                "batch-two",
                Some(BatchLifecycle::Finalizing),
                7,
                9,
            ),
            peek(
                StageKind::Summary,
                "batch-summary",
                Some(BatchLifecycle::InProgress),
                3,
                5,
            ),
        ];
        let provider = ProviderProgress::from_peeks(&peeks);
        assert_eq!(provider.triage.submitted, 25);
        assert_eq!(provider.triage.completed, 17);
        assert_eq!(provider.summaries.submitted, 5);

        let terminal = ProviderProgress::from_peeks(&[peek(
            StageKind::Triage,
            "batch-terminal",
            Some(BatchLifecycle::Completed),
            8,
            10,
        )]);
        assert_eq!(
            terminal.lifecycle(StageKind::Triage),
            ProviderLifecycle::ReadyToCollect
        );
    }

    #[test]
    fn custom_id_text_cannot_change_typed_provider_grouping() {
        let provider = ProviderProgress::from_peeks(&[peek(
            StageKind::SignalCandidate,
            "signal-triage-looking-custom-id",
            Some(BatchLifecycle::InProgress),
            1,
            2,
        )]);
        assert_eq!(provider.signals.submitted, 2);
        assert_eq!(provider.triage.submitted, 0);
    }

    #[test]
    fn provider_lookup_failure_is_indeterminate() {
        let mut observation = import_obs_idle();
        observation.triage_total = 8;
        observation.triage_deferred = 8;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(
            &mut progress,
            &observation,
            &[peek(StageKind::Triage, "batch-failed-lookup", None, 0, 0)],
            &clock,
        );
        assert_eq!(
            snapshot.provider.lifecycle(StageKind::Triage),
            ProviderLifecycle::Indeterminate
        );
        assert_eq!(snapshot.phase, BatchDisplayPhase::WaitingForProvider);
    }

    #[test]
    fn deferred_work_without_peeks_is_preparing_not_zero_request_wait() {
        let mut observation = import_obs_idle();
        observation.summary_total = 4;
        observation.summary_deferred = 4;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(&mut progress, &observation, &[], &clock);
        assert_eq!(snapshot.phase, BatchDisplayPhase::PreparingBatch);
        assert_eq!(snapshot.summaries.provider_total, 0);
        assert_eq!(snapshot.summaries.total, 4);
    }

    #[test]
    fn rearm_transition_cannot_shrink_latched_stage_total() {
        let mut observation = import_obs_idle();
        observation.signal_total = 50;
        observation.signal_pending_or_in_flight = 50;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        assert_eq!(
            snapshot(&mut progress, &observation, &[], &clock)
                .signals
                .total,
            50
        );
        observation.signal_total = 0;
        observation.signal_pending_or_in_flight = 0;
        observation.signal_deferred = 50;
        assert_eq!(
            snapshot(&mut progress, &observation, &[], &clock)
                .signals
                .total,
            50
        );
    }

    #[test]
    fn rearm_can_replace_pending_urls_without_shrinking_latched_total() {
        let mut observation = import_obs_idle();
        observation.summary_total = 12;
        observation.summary_pending = 12;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let _ = snapshot(&mut progress, &observation, &[], &clock);
        observation.summary_total = 5;
        observation.summary_pending = 5;
        let rearmed = snapshot(&mut progress, &observation, &[], &clock);
        assert_eq!(rearmed.summaries.total, 12);
        assert_eq!(rearmed.summaries.pending_or_in_flight, 5);
    }

    #[test]
    fn failures_settle_stage_progress_without_hiding_failure_count() {
        let mut observation = import_obs_idle();
        observation.triage_total = 10;
        observation.triage_completed = 7;
        observation.triage_failed = 3;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(&mut progress, &observation, &[], &clock);
        assert_eq!(snapshot.triage.settled(), 10);
        assert_eq!(snapshot.triage.failed, 3);
    }

    #[test]
    fn dynamic_stage_totals_remain_per_stage_without_an_overall_percentage() {
        let mut observation = import_obs_idle();
        observation.triage_total = 10;
        observation.summary_total = 4;
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = snapshot(&mut progress, &observation, &[], &clock);
        assert_eq!(snapshot.triage.total, 10);
        assert_eq!(snapshot.summaries.total, 4);
    }

    #[test]
    fn replay_cost_is_explicitly_scoped_to_this_run() {
        let observation = import_obs_idle();
        let clock = ManualProgressClock::new(
            FixedOffset::east_opt(0)
                .unwrap()
                .timestamp_opt(0, 0)
                .unwrap(),
        );
        let mut progress = projection(&clock, &observation);
        let snapshot = progress.snapshot(
            &observation,
            &[],
            ProjectionContext {
                cost_this_run_microdollars: 250_000,
                ..ProjectionContext::default()
            },
            &clock,
        );
        assert_eq!(snapshot.cost_this_run_microdollars, 250_000);
    }

    #[test]
    fn local_wait_timestamps_keep_fixed_offsets() {
        let observation = import_obs_idle();
        let summer = FixedOffset::east_opt(2 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 48, 30)
            .unwrap();
        let winter = FixedOffset::east_opt(3600)
            .unwrap()
            .with_ymd_and_hms(2026, 12, 23, 9, 48, 30)
            .unwrap();
        let clock = ManualProgressClock::new(summer);
        let mut progress = projection(&clock, &observation);
        let snapshot = progress.snapshot(
            &observation,
            &[],
            ProjectionContext {
                last_provider_check_local: Some(summer),
                next_provider_check_local: Some(winter),
                ..ProjectionContext::default()
            },
            &clock,
        );
        let wait = snapshot.wait.unwrap();
        assert_eq!(
            wait.last_provider_check_display.as_deref(),
            Some("2026-07-23 09:48:30 +02:00")
        );
        assert_eq!(
            wait.next_provider_check_display.as_deref(),
            Some("2026-12-23 09:48:30 +01:00")
        );
        assert_eq!(summer.to_utc().to_rfc3339(), "2026-07-23T07:48:30+00:00");
    }

    #[test]
    fn clock_contract_supports_manual_advancement_and_all_display_phases() {
        let wall = FixedOffset::east_opt(2 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 48, 30)
            .unwrap();
        let clock = ManualProgressClock::new(wall);
        clock.sleep(Duration::from_secs(5));
        assert_eq!(clock.wall_now(), wall + chrono::Duration::seconds(5));
        let _system_clock = SystemProgressClock;
        let phases = [
            BatchDisplayPhase::Reconciling,
            BatchDisplayPhase::Intake,
            BatchDisplayPhase::Triage,
            BatchDisplayPhase::Summaries,
            BatchDisplayPhase::Signals,
            BatchDisplayPhase::PreparingBatch,
            BatchDisplayPhase::CheckingProvider,
            BatchDisplayPhase::WaitingForProvider,
            BatchDisplayPhase::Collecting,
            BatchDisplayPhase::Replaying,
            BatchDisplayPhase::Persisting,
            BatchDisplayPhase::Complete,
            BatchDisplayPhase::Interrupted,
        ];
        assert_eq!(phases.len(), 13);
    }
}
