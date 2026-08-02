use super::stale_reporter::format_elapsed;
use std::io::Write;
use std::time::Instant;

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

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::{
        BatchObservation, ImportPhase, PreTriagePhase, SessionState, TriagePhase,
    };

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
}
