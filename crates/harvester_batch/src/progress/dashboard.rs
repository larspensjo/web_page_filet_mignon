use std::time::Duration;

use harvester_core::StageKind;
use unicode_width::UnicodeWidthChar;
#[cfg(test)]
use unicode_width::UnicodeWidthStr;

use super::{BatchDisplayPhase, BatchProgressSnapshot, ProviderLifecycle, StageProgress};
#[cfg(test)]
use super::{IntakeProgress, PassCounts, ProviderProgress, ProviderStageProgress, WaitProgress};

pub(crate) const MIN_DASHBOARD_WIDTH: usize = 72;
const PROGRESS_BAR_WIDTH: usize = 20;

/// The restrained glyph families supported by the batch progress renderer.
/// Interactive callers select their preferred family; redirected output always
/// uses [`ProgressGlyphs::Ascii`] through [`PlainProgressReporter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressGlyphs {
    Unicode,
    Ascii,
}

impl ProgressGlyphs {
    fn done_marker(self) -> &'static str {
        match self {
            Self::Unicode => "✓",
            Self::Ascii => "[DONE]",
        }
    }

    fn running_marker(self) -> &'static str {
        match self {
            Self::Unicode => "↻",
            Self::Ascii => "[RUN]",
        }
    }

    fn inactive_marker(self) -> &'static str {
        match self {
            Self::Unicode => "·",
            Self::Ascii => "[   ]",
        }
    }

    fn bar_complete(self) -> char {
        match self {
            Self::Unicode => '█',
            Self::Ascii => '#',
        }
    }

    fn bar_remaining(self) -> char {
        match self {
            Self::Unicode => '─',
            Self::Ascii => '-',
        }
    }

    fn separator(self) -> &'static str {
        match self {
            Self::Unicode => "·",
            Self::Ascii => "|",
        }
    }
}

/// Purely formats the current run snapshot into terminal rows. It deliberately
/// performs neither terminal queries nor I/O; [`TerminalProgressSurface`]
/// handles cursor movement and painting separately.
pub fn format_dashboard(
    snapshot: &BatchProgressSnapshot,
    width: usize,
    glyphs: ProgressGlyphs,
) -> Vec<String> {
    if width < MIN_DASHBOARD_WIDTH {
        return vec![clip_to_display_width(
            &format_compact_dashboard(snapshot, glyphs),
            width,
        )];
    }

    let active = active_stage(snapshot);
    let separator = glyphs.separator();
    let mut lines = Vec::with_capacity(6);
    lines.push(format!(
        "Harvester batch {separator} {} {separator} cost this run {}",
        format_dashboard_elapsed(snapshot.elapsed),
        format_cost(snapshot.cost_this_run_microdollars),
    ));
    lines.push(format_intake_row(snapshot, glyphs));
    lines.push(format_stage_row(
        "Triage",
        StageKind::Triage,
        snapshot,
        snapshot.triage,
        active,
        glyphs,
    ));
    lines.push(format_stage_row(
        "Summaries",
        StageKind::Summary,
        snapshot,
        snapshot.summaries,
        active,
        glyphs,
    ));
    lines.push(format_stage_row(
        "Signals",
        StageKind::SignalCandidate,
        snapshot,
        snapshot.signals,
        active,
        glyphs,
    ));
    lines.push(format_dashboard_footer(snapshot, active, glyphs));

    lines
        .into_iter()
        .map(|line| clip_to_display_width(&line, width))
        .collect()
}

fn format_compact_dashboard(snapshot: &BatchProgressSnapshot, glyphs: ProgressGlyphs) -> String {
    let (phase, settled, total) = if let Some(stage) = active_stage(snapshot) {
        let progress = stage_progress(snapshot, stage);
        (
            stage_label_upper(stage),
            display_settled(snapshot, stage, progress),
            progress.total,
        )
    } else if matches!(snapshot.phase, BatchDisplayPhase::Intake) {
        (
            "INTAKE",
            snapshot
                .intake
                .fetched
                .saturating_add(snapshot.intake.failed),
            snapshot.intake.total,
        )
    } else {
        (phase_label_upper(snapshot.phase), 0, 0)
    };
    let next = snapshot
        .wait
        .as_ref()
        .and_then(|wait| wait.countdown)
        .map(format_countdown)
        .unwrap_or_else(|| "--:--".to_string());
    let separator = glyphs.separator();
    format!(
        "[batch] {phase} {settled}/{total} {separator} {} left {separator} t={} {separator} next={next} {separator} run={}",
        snapshot.remaining_work,
        format_dashboard_elapsed(snapshot.elapsed),
        format_cost(snapshot.cost_this_run_microdollars),
    )
}

fn format_intake_row(snapshot: &BatchProgressSnapshot, glyphs: ProgressGlyphs) -> String {
    let settled = snapshot
        .intake
        .fetched
        .saturating_add(snapshot.intake.failed);
    let marker = if matches!(snapshot.phase, BatchDisplayPhase::Intake) {
        glyphs.running_marker()
    } else if snapshot.intake.total > 0 && settled >= snapshot.intake.total {
        glyphs.done_marker()
    } else {
        glyphs.inactive_marker()
    };
    format!(
        "{marker} {:<10} {} discovered {} {} fetched {} {} failed",
        "Intake",
        snapshot.intake.discovered,
        glyphs.separator(),
        snapshot.intake.fetched,
        glyphs.separator(),
        snapshot.intake.failed,
    )
}

fn format_stage_row(
    label: &str,
    stage: StageKind,
    snapshot: &BatchProgressSnapshot,
    progress: StageProgress,
    active: Option<StageKind>,
    glyphs: ProgressGlyphs,
) -> String {
    let marker = if active == Some(stage) {
        glyphs.running_marker()
    } else if progress.total > 0 && progress.settled() >= progress.total {
        glyphs.done_marker()
    } else {
        glyphs.inactive_marker()
    };
    let body = if active == Some(stage) {
        format_active_stage(progress, stage, snapshot, glyphs)
    } else {
        format!(
            "{}/{} {} {} failed",
            progress.settled(),
            progress.total,
            glyphs.separator(),
            progress.failed
        )
    };
    format!("{marker} {label:<10} {body}")
}

fn format_active_stage(
    progress: StageProgress,
    stage: StageKind,
    snapshot: &BatchProgressSnapshot,
    glyphs: ProgressGlyphs,
) -> String {
    match snapshot.phase {
        BatchDisplayPhase::PreparingBatch => format!(
            "preparing next batch {} {} queued",
            glyphs.separator(),
            progress.deferred
        ),
        BatchDisplayPhase::CheckingProvider => "checking provider...".to_string(),
        BatchDisplayPhase::WaitingForProvider
            if snapshot.provider.lifecycle(stage) == ProviderLifecycle::Indeterminate =>
        {
            format!("waiting provider status {} retrying...", glyphs.separator())
        }
        BatchDisplayPhase::WaitingForProvider => format_progress_body(
            progress,
            display_settled(snapshot, stage, progress),
            glyphs,
            None,
        ),
        BatchDisplayPhase::Collecting => {
            format_progress_body(progress, progress.settled(), glyphs, Some("collecting"))
        }
        BatchDisplayPhase::Replaying => {
            format_progress_body(progress, progress.settled(), glyphs, Some("replaying"))
        }
        _ => format_progress_body(progress, progress.settled(), glyphs, None),
    }
}

fn format_progress_body(
    progress: StageProgress,
    settled: usize,
    glyphs: ProgressGlyphs,
    prefix: Option<&str>,
) -> String {
    let counts = format!("{}/{}", settled.min(progress.total), progress.total);
    let prefix = prefix.map(|value| format!("{value} {} ", glyphs.separator()));
    if progress.total == 0 {
        return format!(
            "{}{} {} {} failed",
            prefix.unwrap_or_default(),
            counts,
            glyphs.separator(),
            progress.failed
        );
    }
    let settled = settled.min(progress.total);
    let filled = ((settled as u128 * PROGRESS_BAR_WIDTH as u128) / progress.total as u128)
        .min(PROGRESS_BAR_WIDTH as u128) as usize;
    let percent = ((settled as u128 * 100) / progress.total as u128).min(100);
    let bar = format!(
        "{}{}",
        glyphs.bar_complete().to_string().repeat(filled),
        glyphs
            .bar_remaining()
            .to_string()
            .repeat(PROGRESS_BAR_WIDTH.saturating_sub(filled))
    );
    format!(
        "{}{}  [{bar}] {percent}%",
        prefix.unwrap_or_default(),
        counts
    )
}

fn format_dashboard_footer(
    snapshot: &BatchProgressSnapshot,
    active: Option<StageKind>,
    glyphs: ProgressGlyphs,
) -> String {
    let separator = glyphs.separator();
    let mut parts = Vec::new();
    if let Some(stage) = active {
        let progress = stage_progress(snapshot, stage);
        if matches!(snapshot.phase, BatchDisplayPhase::WaitingForProvider)
            && progress.provider_total > 0
            && progress.provider_total < progress.total
        {
            parts.push(format!(
                "provider {}/{} submitted",
                progress.provider_completed, progress.provider_total
            ));
        }
        if progress.local_remaining > 0 {
            parts.push(format!(
                "{} awaiting local settlement",
                progress.local_remaining
            ));
        }
        if progress.unsubmitted > 0 {
            parts.push(format!("{} not submitted", progress.unsubmitted));
        }
    }
    if parts.is_empty() {
        parts.push(match snapshot.phase {
            BatchDisplayPhase::Complete => "complete".to_string(),
            BatchDisplayPhase::Interrupted => "interrupted; safe to resume".to_string(),
            _ => format!("{} left", snapshot.remaining_work),
        });
    }
    if let Some(wait) = &snapshot.wait {
        if let Some(checked) = wait.last_provider_check_local {
            parts.push(format!("checked {}", checked.format("%H:%M:%S %:z")));
        }
        if let Some(next) = wait.next_provider_check_local {
            let countdown = wait
                .countdown
                .map(format_countdown)
                .unwrap_or_else(|| "--:--".to_string());
            parts.push(format!(
                "next {} ({countdown})",
                next.format("%H:%M:%S %:z")
            ));
        }
    }
    parts.push("Ctrl+C is safe".to_string());
    parts.join(&format!(" {separator} "))
}

fn active_stage(snapshot: &BatchProgressSnapshot) -> Option<StageKind> {
    match snapshot.phase {
        BatchDisplayPhase::Triage => Some(StageKind::Triage),
        BatchDisplayPhase::Summaries => Some(StageKind::Summary),
        BatchDisplayPhase::Signals => Some(StageKind::SignalCandidate),
        BatchDisplayPhase::Intake
        | BatchDisplayPhase::Complete
        | BatchDisplayPhase::Interrupted => None,
        _ => [
            StageKind::Triage,
            StageKind::Summary,
            StageKind::SignalCandidate,
        ]
        .into_iter()
        .find(|stage| {
            let progress = stage_progress(snapshot, *stage);
            progress.provider_total > 0
                || progress.local_remaining > 0
                || progress.settled() < progress.total
        }),
    }
}

fn stage_progress(snapshot: &BatchProgressSnapshot, stage: StageKind) -> StageProgress {
    match stage {
        StageKind::Triage => snapshot.triage,
        StageKind::Summary => snapshot.summaries,
        StageKind::SignalCandidate => snapshot.signals,
    }
}

fn display_settled(
    snapshot: &BatchProgressSnapshot,
    stage: StageKind,
    progress: StageProgress,
) -> usize {
    if matches!(snapshot.phase, BatchDisplayPhase::WaitingForProvider)
        && snapshot.provider.lifecycle(stage) != ProviderLifecycle::Indeterminate
    {
        progress.provisional_settled.min(progress.total)
    } else {
        progress.settled().min(progress.total)
    }
}

fn stage_label_upper(stage: StageKind) -> &'static str {
    match stage {
        StageKind::Triage => "TRIAGE",
        StageKind::Summary => "SUMMARIES",
        StageKind::SignalCandidate => "SIGNALS",
    }
}

fn phase_label_upper(phase: BatchDisplayPhase) -> &'static str {
    match phase {
        BatchDisplayPhase::Reconciling => "RECONCILING",
        BatchDisplayPhase::Intake => "INTAKE",
        BatchDisplayPhase::Triage => "TRIAGE",
        BatchDisplayPhase::Summaries => "SUMMARIES",
        BatchDisplayPhase::Signals => "SIGNALS",
        BatchDisplayPhase::PreparingBatch => "PREPARING",
        BatchDisplayPhase::CheckingProvider => "CHECKING",
        BatchDisplayPhase::WaitingForProvider => "WAITING",
        BatchDisplayPhase::Collecting => "COLLECTING",
        BatchDisplayPhase::Replaying => "REPLAYING",
        BatchDisplayPhase::Persisting => "PERSISTING",
        BatchDisplayPhase::Complete => "COMPLETE",
        BatchDisplayPhase::Interrupted => "INTERRUPTED",
    }
}

fn format_dashboard_elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h{minutes}m{seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m{seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

fn format_countdown(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let minutes = seconds / 60;
    format!("{minutes:02}:{:02}", seconds % 60)
}

fn format_cost(microdollars: u64) -> String {
    let dollars = microdollars / 1_000_000;
    let cents = (microdollars % 1_000_000) / 10_000;
    format!("${dollars}.{cents:02}")
}

fn clip_to_display_width(input: &str, width: usize) -> String {
    let mut clipped = String::new();
    let mut used: usize = 0;
    for character in input.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if used.saturating_add(character_width) > width {
            break;
        }
        clipped.push(character);
        used = used.saturating_add(character_width);
    }
    clipped
}

#[cfg(test)]
fn display_width(input: &str) -> usize {
    UnicodeWidthStr::width(input)
}

#[cfg(test)]
pub(crate) fn renderer_stage(
    total: usize,
    successful: usize,
    pending_or_in_flight: usize,
) -> StageProgress {
    StageProgress {
        total,
        successful,
        pending_or_in_flight,
        local_remaining: pending_or_in_flight,
        provisional_settled: successful,
        ..StageProgress::default()
    }
}

#[cfg(test)]
pub(crate) fn renderer_snapshot(phase: BatchDisplayPhase) -> BatchProgressSnapshot {
    BatchProgressSnapshot {
        elapsed: Duration::from_secs(8_137),
        cost_this_run_microdollars: 250_000,
        intake: IntakeProgress {
            discovered: 76,
            fetched: 69,
            failed: 7,
            total: 76,
        },
        triage: renderer_stage(419, 419, 0),
        summaries: renderer_stage(397, 397, 0),
        signals: renderer_stage(32, 25, 7),
        provider: ProviderProgress::default(),
        phase,
        remaining_work: 7,
        pass_counts: PassCounts::default(),
        wait: None,
    }
}

#[cfg(test)]
fn unicode_expected(
    intake: &str,
    triage: &str,
    summaries: &str,
    signals: &str,
    footer: &str,
) -> Vec<String> {
    vec![
        "Harvester batch · 2h15m37s · cost this run $0.25".to_string(),
        intake.to_string(),
        triage.to_string(),
        summaries.to_string(),
        signals.to_string(),
        footer.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};

    #[test]
    fn formatter_exact_wide_dashboard_for_intake_and_each_llm_stage() {
        let mut intake = renderer_snapshot(BatchDisplayPhase::Intake);
        intake.intake = IntakeProgress {
            discovered: 5,
            fetched: 2,
            failed: 1,
            total: 5,
        };
        intake.triage = StageProgress::default();
        intake.summaries = StageProgress::default();
        intake.signals = StageProgress::default();
        intake.remaining_work = 0;
        assert_eq!(
            format_dashboard(&intake, 140, ProgressGlyphs::Unicode),
            unicode_expected(
                "↻ Intake     5 discovered · 2 fetched · 1 failed",
                "· Triage     0/0 · 0 failed",
                "· Summaries  0/0 · 0 failed",
                "· Signals    0/0 · 0 failed",
                "0 left · Ctrl+C is safe",
            )
        );

        let triage = renderer_snapshot(BatchDisplayPhase::Triage);
        assert_eq!(
            format_dashboard(&triage, 140, ProgressGlyphs::Unicode),
            unicode_expected(
                "✓ Intake     76 discovered · 69 fetched · 7 failed",
                "↻ Triage     419/419  [████████████████████] 100%",
                "✓ Summaries  397/397 · 0 failed",
                "· Signals    25/32 · 0 failed",
                "7 left · Ctrl+C is safe",
            )
        );

        let summaries = renderer_snapshot(BatchDisplayPhase::Summaries);
        assert_eq!(
            format_dashboard(&summaries, 140, ProgressGlyphs::Unicode),
            unicode_expected(
                "✓ Intake     76 discovered · 69 fetched · 7 failed",
                "✓ Triage     419/419 · 0 failed",
                "↻ Summaries  397/397  [████████████████████] 100%",
                "· Signals    25/32 · 0 failed",
                "7 left · Ctrl+C is safe",
            )
        );

        let signals = renderer_snapshot(BatchDisplayPhase::Signals);
        assert_eq!(
            format_dashboard(&signals, 140, ProgressGlyphs::Unicode),
            unicode_expected(
                "✓ Intake     76 discovered · 69 fetched · 7 failed",
                "✓ Triage     419/419 · 0 failed",
                "✓ Summaries  397/397 · 0 failed",
                "↻ Signals    25/32  [███████████████─────] 78%",
                "7 awaiting local settlement · Ctrl+C is safe",
            )
        );
    }

    #[test]
    fn formatter_exact_wide_dashboard_for_provider_wait_replay_complete_and_interrupted() {
        let checked = FixedOffset::east_opt(2 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 43, 30)
            .unwrap();
        let next = FixedOffset::east_opt(2 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 23, 9, 48, 30)
            .unwrap();
        let mut waiting = renderer_snapshot(BatchDisplayPhase::WaitingForProvider);
        waiting.signals.provider_total = 32;
        waiting.signals.provider_completed = 25;
        waiting.signals.provisional_settled = 25;
        waiting.provider.signals = ProviderStageProgress {
            submitted: 32,
            completed: 25,
            attached_batches: 1,
            ..ProviderStageProgress::default()
        };
        waiting.wait = Some(WaitProgress {
            last_provider_check: None,
            next_provider_check: None,
            checked_age: None,
            countdown: Some(Duration::from_secs(293)),
            last_provider_check_local: Some(checked),
            next_provider_check_local: Some(next),
            last_provider_check_display: None,
            next_provider_check_display: None,
        });
        assert_eq!(
            format_dashboard(&waiting, 140, ProgressGlyphs::Unicode),
            unicode_expected(
                "✓ Intake     76 discovered · 69 fetched · 7 failed",
                "✓ Triage     419/419 · 0 failed",
                "✓ Summaries  397/397 · 0 failed",
                "↻ Signals    25/32  [███████████████─────] 78%",
                "7 awaiting local settlement · checked 09:43:30 +02:00 · next 09:48:30 +02:00 (04:53) · Ctrl+C is safe",
            )
        );

        let replaying = renderer_snapshot(BatchDisplayPhase::Replaying);
        assert_eq!(
            format_dashboard(&replaying, 140, ProgressGlyphs::Unicode),
            unicode_expected(
                "✓ Intake     76 discovered · 69 fetched · 7 failed",
                "✓ Triage     419/419 · 0 failed",
                "✓ Summaries  397/397 · 0 failed",
                "↻ Signals    replaying · 25/32  [███████████████─────] 78%",
                "7 awaiting local settlement · Ctrl+C is safe",
            )
        );

        let collecting = renderer_snapshot(BatchDisplayPhase::Collecting);
        assert_eq!(
            format_dashboard(&collecting, 140, ProgressGlyphs::Unicode),
            unicode_expected(
                "✓ Intake     76 discovered · 69 fetched · 7 failed",
                "✓ Triage     419/419 · 0 failed",
                "✓ Summaries  397/397 · 0 failed",
                "↻ Signals    collecting · 25/32  [███████████████─────] 78%",
                "7 awaiting local settlement · Ctrl+C is safe",
            )
        );

        let mut complete = renderer_snapshot(BatchDisplayPhase::Complete);
        complete.signals = renderer_stage(32, 32, 0);
        complete.remaining_work = 0;
        assert_eq!(
            format_dashboard(&complete, 140, ProgressGlyphs::Unicode),
            unicode_expected(
                "✓ Intake     76 discovered · 69 fetched · 7 failed",
                "✓ Triage     419/419 · 0 failed",
                "✓ Summaries  397/397 · 0 failed",
                "✓ Signals    32/32 · 0 failed",
                "complete · Ctrl+C is safe",
            )
        );

        let interrupted = renderer_snapshot(BatchDisplayPhase::Interrupted);
        assert_eq!(
            format_dashboard(&interrupted, 140, ProgressGlyphs::Unicode),
            unicode_expected(
                "✓ Intake     76 discovered · 69 fetched · 7 failed",
                "✓ Triage     419/419 · 0 failed",
                "✓ Summaries  397/397 · 0 failed",
                "· Signals    25/32 · 0 failed",
                "interrupted; safe to resume · Ctrl+C is safe",
            )
        );
    }

    #[test]
    fn formatter_zero_totals_and_stale_provider_counts_never_make_fake_or_overfull_bars() {
        let mut zero = renderer_snapshot(BatchDisplayPhase::Signals);
        zero.signals = StageProgress::default();
        zero.remaining_work = 0;
        let zero_lines = format_dashboard(&zero, 140, ProgressGlyphs::Unicode);
        assert!(zero_lines[4].contains("0/0"));
        assert!(!zero_lines[4].contains('%'));

        let mut stale = renderer_snapshot(BatchDisplayPhase::WaitingForProvider);
        stale.signals.total = 32;
        stale.signals.provisional_settled = 1_000;
        stale.signals.provider_total = 32;
        stale.provider.signals = ProviderStageProgress {
            submitted: 32,
            completed: 1_000,
            attached_batches: 1,
            ..ProviderStageProgress::default()
        };
        let row = &format_dashboard(&stale, 140, ProgressGlyphs::Unicode)[4];
        assert!(row.contains("32/32"));
        assert!(row.contains("100%"));
        assert!(row.contains("[████████████████████]"));
    }

    #[test]
    fn formatter_shows_preparing_and_partially_submitted_provider_scopes() {
        let mut preparing = renderer_snapshot(BatchDisplayPhase::PreparingBatch);
        preparing.signals.deferred = 7;
        let preparing_lines = format_dashboard(&preparing, 140, ProgressGlyphs::Unicode);
        assert_eq!(
            preparing_lines[4],
            "↻ Signals    preparing next batch · 7 queued"
        );
        assert!(!preparing_lines.iter().any(|line| line.contains("0/0")));

        let mut subset = renderer_snapshot(BatchDisplayPhase::WaitingForProvider);
        subset.signals = StageProgress {
            total: 50,
            deferred: 50,
            provider_total: 32,
            provider_completed: 25,
            provisional_settled: 25,
            local_remaining: 50,
            unsubmitted: 18,
            ..StageProgress::default()
        };
        subset.remaining_work = 50;
        subset.provider.signals = ProviderStageProgress {
            submitted: 32,
            completed: 25,
            attached_batches: 1,
            ..ProviderStageProgress::default()
        };
        let subset_lines = format_dashboard(&subset, 140, ProgressGlyphs::Unicode);
        assert_eq!(
            subset_lines[4],
            "↻ Signals    25/50  [██████████──────────] 50%"
        );
        assert_eq!(
            subset_lines[5],
            "provider 25/32 submitted · 50 awaiting local settlement · 18 not submitted · Ctrl+C is safe"
        );
    }

    #[test]
    fn formatter_clips_by_display_columns_at_requested_widths() {
        let snapshot = renderer_snapshot(BatchDisplayPhase::WaitingForProvider);
        for width in [72, 100, 140] {
            for line in format_dashboard(&snapshot, width, ProgressGlyphs::Unicode) {
                assert!(
                    display_width(&line) <= width,
                    "{line:?} is {} columns at width {width}",
                    display_width(&line)
                );
            }
        }

        let cjk_row = clip_to_display_width("Signals 進捗 ████████████████████", 18);
        assert!(display_width(&cjk_row) <= 18);
        assert_eq!(cjk_row, "Signals 進捗 █████");
    }

    #[test]
    fn formatter_narrow_fallback_and_ascii_mode_preserve_required_information() {
        let mut snapshot = renderer_snapshot(BatchDisplayPhase::WaitingForProvider);
        snapshot.signals.provider_total = 32;
        snapshot.signals.provisional_settled = 25;
        snapshot.provider.signals = ProviderStageProgress {
            submitted: 32,
            completed: 25,
            attached_batches: 1,
            ..ProviderStageProgress::default()
        };
        snapshot.wait = Some(WaitProgress {
            last_provider_check: None,
            next_provider_check: None,
            checked_age: None,
            countdown: Some(Duration::from_secs(293)),
            last_provider_check_local: None,
            next_provider_check_local: None,
            last_provider_check_display: None,
            next_provider_check_display: None,
        });
        let narrow = format_dashboard(&snapshot, 71, ProgressGlyphs::Unicode);
        assert_eq!(narrow.len(), 1);
        for required in [
            "SIGNALS",
            "25/32",
            "7 left",
            "t=2h15m37s",
            "next=04:53",
            "run=$0.25",
        ] {
            assert!(
                narrow[0].contains(required),
                "missing {required}: {narrow:?}"
            );
        }

        let ascii = format_dashboard(&snapshot, 140, ProgressGlyphs::Ascii);
        assert!(ascii.iter().all(|line| line.is_ascii()));
        assert!(ascii[4].contains("[RUN]"));
        assert!(ascii[4].contains('#'));
        assert!(ascii[4].contains('-'));
    }
}
