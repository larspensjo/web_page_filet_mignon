//! Stdout progress reporter for the --refresh-stale-summaries-limit mode.
//!
//! Activated only when both stdout and stderr are terminals; otherwise every
//! method is a no-op.

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

const FAIL_REASON_MAX_CHARS: usize = 80;

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
}

impl BatchProgressReporter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_line_width: 0,
            painted_status: false,
            start: Instant::now(),
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
        let body = format!(
            "[batch] {}  jobs={}/{} fail={}  triage={}/{} fail={}  summary={}/{} fail={}  t={}",
            phase,
            obs.jobs_done,
            obs.jobs_total,
            obs.jobs_failed,
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
    use harvester_core::{
        BatchObservation, ImportPhase, PreTriagePhase, SessionState, TriagePhase,
    };
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
            s.contains("jobs=7/10 fail=1"),
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
}
