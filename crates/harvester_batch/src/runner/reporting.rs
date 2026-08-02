use super::drain_control::BatchDrainSnapshot;
use super::CycleOutcome;
use harvester_core::{BatchObservation, LlmModelUsageView};
use std::io::Write;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct CycleCounts {
    pub(super) new_jobs: usize,
    pub(super) jobs_done: usize,
    pub(super) jobs_failed: usize,
    pub(super) triage_completed: usize,
    pub(super) triage_failed: usize,
    pub(super) summary_completed: usize,
    pub(super) summary_failed: usize,
    pub(super) imports_completed: usize,
    pub(super) imports_failed: usize,
}

/// Summarizes a finished drain for stdout. Batches that are still running
/// remain in the manifest and are reported so the operator knows a later drain
/// still has work to collect.
pub(super) fn format_drain_summary(
    pending_manifest_batches: &[(String, Option<String>)],
) -> String {
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

pub(super) fn write_no_progress_bailout<W: Write>(
    sink: &mut W,
    snapshot: &BatchDrainSnapshot,
) -> std::io::Result<()> {
    writeln!(
        sink,
        "[batch-wait] no-progress bailout; remaining triage={} summaries={} signal={}",
        snapshot.triage_deferred, snapshot.summary_deferred, snapshot.signal_deferred
    )
}

/// Returns the once-per-intake poll summary plus the former per-pass transcript
/// when the operator explicitly opts in. Runtime logging is unaffected.
#[allow(clippy::too_many_arguments)]
pub(super) fn format_optional_cycle_diagnostics(
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

fn cycle_outcome_label(outcome: &CycleOutcome) -> &'static str {
    match outcome {
        CycleOutcome::Success => "SUCCESS",
        CycleOutcome::PartialFailure => "PARTIAL",
        CycleOutcome::TotalFailure => "FAILED",
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
pub(super) fn format_awaiting_batch_line(
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
pub(super) fn print_poll_stats(stats: &[harvester_core::SourcePollStat]) {
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

/// Prints the final summary when batch runner exits.
#[allow(clippy::too_many_arguments)]
pub(super) fn print_final_summary(
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

/// Converts microdollars to a human-readable dollar string with exact rounding.
/// Examples: 0 -> "$0.00", 1234567 -> "$1.23", 50 -> "$0.00", 5000 -> "$0.01"
pub(crate) fn microdollars_to_display(microdollars: u64) -> String {
    let cents = (microdollars + 5000) / 10000; // Round to nearest cent
    let dollars = cents / 100;
    let remaining_cents = cents % 100;
    format!("${}.{:02}", dollars, remaining_cents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use harvester_core::{SessionState, SourcePollStat};
    use harvester_engine::{SourceId, SourceKind};

    fn observation_with_totals(
        jobs_total: usize,
        jobs_done: usize,
        jobs_failed: usize,
        triage_completed: usize,
        triage_failed: usize,
        summary_completed: usize,
        summary_failed: usize,
    ) -> BatchObservation {
        BatchObservation {
            poll_in_progress: false,
            session_state: SessionState::Idle,
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
            imports_completed: 0,
            imports_failed: 0,
            import_in_flight: false,
            source_poll_stats: vec![],
        }
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
    fn format_llm_usage_lines_formats_rows_compactly() {
        let rows = vec![
            LlmModelUsageView {
                model: "alpha".to_string(),
                input_tokens: 12_345,
                output_tokens: 3_100,
            },
            LlmModelUsageView {
                model: "beta".to_string(),
                input_tokens: 500,
                output_tokens: 80,
            },
        ];
        let lines = format_llm_usage_lines(&rows);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("alpha"));
        assert!(lines[0].contains("in=12K"));
        assert!(lines[0].contains("out=3K"));
        assert!(lines[1].contains("beta"));
        assert!(lines[1].contains("in=500"));
        assert!(lines[1].contains("out=80"));
    }

    #[test]
    fn format_llm_usage_lines_empty_returns_empty() {
        let lines = format_llm_usage_lines(&[]);
        assert!(lines.is_empty());
    }

    #[test]
    fn format_awaiting_batch_line_is_absent_when_nothing_deferred() {
        assert_eq!(format_awaiting_batch_line(0, 0, 0), None);
    }

    #[test]
    fn format_awaiting_batch_line_reports_per_stage_and_total_counts() {
        let line = format_awaiting_batch_line(3, 2, 1).unwrap();
        assert_eq!(
            line,
            "  Awaiting batch results: 3 triage, 2 summaries, 1 signal (6 total)"
        );
    }

    #[test]
    fn default_progress_output_includes_poll_summary_once_but_excludes_verbose_diagnostics() {
        let mut observation = observation_with_totals(1, 1, 0, 1, 0, 1, 0);
        observation.triage_deferred = 1;
        observation.source_poll_stats.push(SourcePollStat {
            source_id: SourceId::new("test-rss").unwrap(),
            kind: SourceKind::Rss,
            parsed: 2,
            dedup_filtered: 1,
            emitted: 1,
        });
        let usage_rows = [LlmModelUsageView {
            model: "gpt-test".to_string(),
            input_tokens: 10,
            output_tokens: 20,
        }];

        let intake_details = format_optional_cycle_diagnostics(
            false,
            true,
            true,
            1,
            &CycleOutcome::Success,
            &CycleCounts::default(),
            0,
            &observation,
            &usage_rows,
            None,
        );
        let collect_only_details = format_optional_cycle_diagnostics(
            false,
            false,
            false,
            2,
            &CycleOutcome::Success,
            &CycleCounts::default(),
            0,
            &observation,
            &usage_rows,
            None,
        );
        let details = intake_details
            .into_iter()
            .chain(collect_only_details)
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(details.matches("--- Poll summary ---").count(), 1);
        assert!(details.contains("test-rss"));
        assert!(!details.contains("Cycle"));
        assert!(!details.contains("Awaiting batch results"));
        assert!(!details.contains("gpt-test: in=10 out=20"));
    }

    #[test]
    fn verbose_progress_output_contains_cycle_source_and_model_diagnostics() {
        let mut observation = observation_with_totals(1, 1, 0, 1, 0, 1, 0);
        observation.source_poll_stats.push(SourcePollStat {
            source_id: SourceId::new("test-rss").unwrap(),
            kind: SourceKind::Rss,
            parsed: 2,
            dedup_filtered: 1,
            emitted: 1,
        });
        let details = format_optional_cycle_diagnostics(
            true,
            true,
            true,
            1,
            &CycleOutcome::Success,
            &CycleCounts::default(),
            0,
            &observation,
            &[LlmModelUsageView {
                model: "gpt-test".to_string(),
                input_tokens: 10,
                output_tokens: 20,
            }],
            None,
        )
        .join("\n");

        assert!(details.contains("Cycle"));
        assert!(details.contains("--- Poll summary ---"));
        assert!(details.contains("gpt-test: in=10 out=20"));
    }

    #[test]
    fn verbose_wait_timestamp_uses_injected_local_offset() {
        let checked_at = chrono::FixedOffset::east_opt(2 * 60 * 60)
            .unwrap()
            .with_ymd_and_hms(2026, 7, 24, 9, 48, 30)
            .single()
            .unwrap();
        let line = format_verbose_awaiting_batch_line(1, 0, 0, Some(checked_at)).unwrap();

        assert!(line.contains("2026-07-24 09:48:30 +02:00"));
        assert!(!line.contains("Z"));
        assert!(!line.contains("+00:00"));
    }

    #[test]
    fn batch_api_final_summary_distinguishes_intake_from_collection_passes_and_cost_scope() {
        let summary = format_final_summary(
            true,
            7,
            &observation_with_totals(2, 2, 0, 1, 0, 1, 0),
            2,
            1,
            1,
            Duration::from_secs(136),
            25_000,
        );

        assert!(summary.contains("intake=1 collection_passes=6"));
        assert!(summary.contains("triage_success=1 triage_failed=0"));
        assert!(summary.contains("cost_this_run=$0.03"));
        assert!(!summary.contains("7 cycles"));
    }

    #[test]
    fn ordinary_final_summary_retains_cycle_wording() {
        let summary = format_final_summary(
            false,
            7,
            &observation_with_totals(2, 2, 0, 1, 0, 1, 0),
            2,
            1,
            1,
            Duration::from_secs(136),
            0,
        );

        assert!(summary.contains("Batch complete: 7 cycles"));
        assert!(!summary.contains("collection_passes"));
    }

    #[test]
    fn drain_summary_reports_batches_left_pending_for_a_later_run() {
        assert_eq!(
            format_drain_summary(&[]),
            "[batch-drain] collected and exiting; no batches remain pending"
        );

        let summary = format_drain_summary(&[
            ("file_1".to_string(), Some("batch_1".to_string())),
            ("file_2".to_string(), Some("batch_2".to_string())),
        ]);

        assert_eq!(
            summary,
            "[batch-drain] collected and exiting; 2 batch(es) still pending: batch_1, batch_2"
        );

        // A reservation that never reached the provider has no batch id yet, so the
        // input file id has to identify it.
        let unreconciled = format_drain_summary(&[("file_3".to_string(), None)]);
        assert!(unreconciled.contains("file_3"), "got {unreconciled}");
    }

    #[test]
    fn batch_drain_bailout_prints_remaining_stage_counts_without_changing_exit_code() {
        let snapshot = BatchDrainSnapshot {
            pending_manifest_batches: vec![("file-1".to_string(), Some("batch-1".to_string()))],
            triage_deferred: 7,
            summary_deferred: 5,
            signal_deferred: 3,
        };
        let mut output = Vec::new();

        assert!(super::super::should_exit_batch_drain_after_no_progress(
            super::super::drain_control::MAX_CONSECUTIVE_BATCH_COLLECT_NO_PROGRESS
        ));
        write_no_progress_bailout(&mut output, &snapshot).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "[batch-wait] no-progress bailout; remaining triage=7 summaries=5 signal=3\n"
        );
        assert_eq!(
            super::super::exit_code_with_shutdown(super::super::determine_exit_code(0), false),
            0,
            "the bailout must retain the existing successful exit code"
        );
    }
}
