use std::time::Duration;

use super::projection::ProgressStage;
use unicode_width::UnicodeWidthChar;
#[cfg(test)]
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
use super::IntakeProgress;
use super::{BatchDisplayPhase, BatchProgressSnapshot, StageProgress};

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
        ProgressStage::Triage,
        snapshot.triage,
        active,
        glyphs,
    ));
    lines.push(format_stage_row(
        "Summaries",
        ProgressStage::Summary,
        snapshot.summaries,
        active,
        glyphs,
    ));
    lines.push(format_stage_row(
        "Signals",
        ProgressStage::SignalCandidate,
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
            display_settled(progress),
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
    let separator = glyphs.separator();
    format!(
        "[batch] {phase} {settled}/{total} {separator} {} left {separator} t={} {separator} run={}",
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
    stage: ProgressStage,
    progress: StageProgress,
    active: Option<ProgressStage>,
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
        format_active_stage(progress, glyphs)
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

fn format_active_stage(progress: StageProgress, glyphs: ProgressGlyphs) -> String {
    format_progress_body(progress, progress.settled(), glyphs, None)
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
    active: Option<ProgressStage>,
    glyphs: ProgressGlyphs,
) -> String {
    let separator = glyphs.separator();
    let mut parts = Vec::new();
    if let Some(stage) = active {
        let progress = stage_progress(snapshot, stage);
        if progress.local_remaining > 0 {
            parts.push(format!(
                "{} awaiting local settlement",
                progress.local_remaining
            ));
        }
    }
    if parts.is_empty() {
        parts.push(match snapshot.phase {
            BatchDisplayPhase::Complete => "complete".to_string(),
            BatchDisplayPhase::Interrupted => "interrupted; safe to resume".to_string(),
            _ => format!("{} left", snapshot.remaining_work),
        });
    }

    parts.push("Ctrl+C is safe".to_string());
    parts.join(&format!(" {separator} "))
}

fn active_stage(snapshot: &BatchProgressSnapshot) -> Option<ProgressStage> {
    match snapshot.phase {
        BatchDisplayPhase::Triage => Some(ProgressStage::Triage),
        BatchDisplayPhase::Summaries => Some(ProgressStage::Summary),
        BatchDisplayPhase::Signals => Some(ProgressStage::SignalCandidate),
        BatchDisplayPhase::Intake
        | BatchDisplayPhase::Complete
        | BatchDisplayPhase::Interrupted => None,
        _ => [
            ProgressStage::Triage,
            ProgressStage::Summary,
            ProgressStage::SignalCandidate,
        ]
        .into_iter()
        .find(|stage| {
            let progress = stage_progress(snapshot, *stage);
            progress.local_remaining > 0 || progress.settled() < progress.total
        }),
    }
}

fn stage_progress(snapshot: &BatchProgressSnapshot, stage: ProgressStage) -> StageProgress {
    match stage {
        ProgressStage::Triage => snapshot.triage,
        ProgressStage::Summary => snapshot.summaries,
        ProgressStage::SignalCandidate => snapshot.signals,
    }
}

fn display_settled(progress: StageProgress) -> usize {
    progress.settled().min(progress.total)
}

fn stage_label_upper(stage: ProgressStage) -> &'static str {
    match stage {
        ProgressStage::Triage => "TRIAGE",
        ProgressStage::Summary => "SUMMARIES",
        ProgressStage::SignalCandidate => "SIGNALS",
    }
}

fn phase_label_upper(phase: BatchDisplayPhase) -> &'static str {
    match phase {
        BatchDisplayPhase::Intake => "INTAKE",
        BatchDisplayPhase::Triage => "TRIAGE",
        BatchDisplayPhase::Summaries => "SUMMARIES",
        BatchDisplayPhase::Signals => "SIGNALS",
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
        phase,
        remaining_work: 7,
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
    fn formatter_exact_wide_dashboard_for_complete_and_interrupted() {
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
    fn formatter_zero_totals_and_overfull_counts_never_make_fake_or_overfull_bars() {
        let mut zero = renderer_snapshot(BatchDisplayPhase::Signals);
        zero.signals = StageProgress::default();
        zero.remaining_work = 0;
        let zero_lines = format_dashboard(&zero, 140, ProgressGlyphs::Unicode);
        assert!(zero_lines[4].contains("0/0"));
        assert!(!zero_lines[4].contains('%'));

        let mut stale = renderer_snapshot(BatchDisplayPhase::Signals);
        stale.signals.total = 32;
        stale.signals.successful = 1_000;
        let row = &format_dashboard(&stale, 140, ProgressGlyphs::Unicode)[4];
        assert!(row.contains("32/32"));
        assert!(row.contains("100%"));
        assert!(row.contains("[████████████████████]"));
    }

    #[test]
    fn formatter_clips_by_display_columns_at_requested_widths() {
        let snapshot = renderer_snapshot(BatchDisplayPhase::Signals);
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
        let snapshot = renderer_snapshot(BatchDisplayPhase::Signals);
        let narrow = format_dashboard(&snapshot, 71, ProgressGlyphs::Unicode);
        assert_eq!(narrow.len(), 1);
        for required in ["SIGNALS", "25/32", "7 left", "t=2h15m37s", "run=$0.25"] {
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
