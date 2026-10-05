use harvester_core::{BatchObservation, UnfinishedWork};
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

#[derive(Default)]
pub(crate) struct CycleStartWorkReporter {
    count_printed: bool,
    notice_printed: bool,
}

impl CycleStartWorkReporter {
    pub(crate) fn pending_count_line(
        &mut self,
        state: &harvester_core::AppState,
    ) -> Option<String> {
        if self.count_printed {
            return None;
        }
        let UnfinishedWork::Known(work) = state.unfinished_work() else {
            return None;
        };
        self.count_printed = true;
        Some(format!(
            "[batch] cycle-start unfinished_articles={} estimated_calls={}",
            work.articles_with_work, work.estimated_calls
        ))
    }

    pub(crate) fn pending_lines(&mut self, state: &harvester_core::AppState) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(line) = self.pending_count_line(state) {
            lines.push(line);
        }
        if !self.notice_printed {
            if let Some((articles, calls)) = state.reprocess_notice() {
                lines.push(format!(
                    "[batch] reprocess notice: {} unfinished articles; up to {} model calls",
                    articles, calls
                ));
                self.notice_printed = true;
            }
        }
        lines
    }
}

/// Prints a grouped poll-stats summary (RSS / Brave / other source types).
pub(super) fn print_poll_stats(stats: &[harvester_core::SourcePollStat]) {
    if let Some(summary) = format_poll_summary(stats) {
        println!("{summary}");
    }
}

#[derive(Default)]
pub(super) struct PollSummaryReporter {
    printed: bool,
}

impl PollSummaryReporter {
    pub(super) fn take(&mut self, stats: &[harvester_core::SourcePollStat]) -> Option<String> {
        if self.printed {
            return None;
        }
        let summary = format_poll_summary(stats)?;
        self.printed = true;
        Some(summary)
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
    total_cycles: usize,
    observation: &BatchObservation,
    total_new_articles: usize,
    total_triaged: usize,
    total_summarized: usize,
    elapsed: Duration,
) {
    println!(
        "{}",
        format_final_summary(
            total_cycles,
            observation,
            total_new_articles,
            total_triaged,
            total_summarized,
            elapsed,
        )
    );
}

#[allow(clippy::too_many_arguments)]
fn format_final_summary(
    total_cycles: usize,
    observation: &BatchObservation,
    total_new_articles: usize,
    total_triaged: usize,
    total_summarized: usize,
    elapsed: Duration,
) -> String {
    let elapsed = format_summary_elapsed(elapsed);
    let stages = format!(
        "intake_success={} intake_failed={} triage_success={} triage_failed={} summaries_success={} summaries_failed={} signals_success={} signals_failed={} elapsed={}",
        observation.jobs_done,
        observation.jobs_failed,
        observation.triage_completed,
        observation.triage_failed,
        observation.summary_completed,
        observation.summary_failed,
        observation.signal_completed,
        observation.signal_failed,
        elapsed,
    );
    format!(
        "\n-- Batch complete: {} cycles, {} new articles, {} triaged, {} summarized --\n{}",
        total_cycles, total_new_articles, total_triaged, total_summarized, stages
    )
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

/// One-line notice printed before state hydration so an interactive launch is
/// never silent between the password prompt and the live progress block.
pub(super) fn format_startup_notice(mode_label: &str) -> String {
    format!("Harvester batch · starting ({mode_label}) · loading state and caches")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::format_cost as microdollars_to_display;
    use harvester_core::{CompletedJobSnapshot, SessionState, SourcePollStat};
    use harvester_engine::{llm::PromptId, SourceId, SourceKind};
    use std::collections::HashMap;

    fn state_with_unfinished_article_and_limited_quota() -> harvester_core::AppState {
        let url = "https://cycle-start.example/article";
        let article = harvester_core::LoadedArticle {
            url: url.into(),
            source_title: Some("Cycle-start article".into()),
            prepared_text: "articleword ".repeat(220),
            content_hash: "cycle-start-content".into(),
            fetched_utc: Some("2026-09-27T00:00:00Z".into()),
        };
        let state = harvester_core::AppState::new();
        let (state, _) = harvester_core::update(
            state,
            harvester_core::Msg::LlmMetadataLoaded {
                active_versions: HashMap::from([
                    (PromptId::ArticleTriage, 1),
                    (PromptId::ArticleSummary, 1),
                    (PromptId::ArticleSignalCandidate, 1),
                ]),
                effective_models: HashMap::from([
                    (PromptId::ArticleTriage, "triage-test-model".into()),
                    (PromptId::ArticleSummary, "summary-test-model".into()),
                    (PromptId::ArticleSignalCandidate, "signal-test-model".into()),
                ]),
            },
        );
        let (state, _) =
            harvester_core::update(state, harvester_core::Msg::PromptTemplateFilesLoaded);
        let (state, _) = harvester_core::update(
            state,
            harvester_core::Msg::PromptContextsLoaded {
                contexts: HashMap::new(),
            },
        );
        let (state, _) = harvester_core::update(
            state,
            harvester_core::Msg::RestoreCompletedJobs(vec![CompletedJobSnapshot {
                url: url.into(),
                tokens: Some(100),
                bytes: Some(4_000),
                links: Vec::new(),
                fetched_utc: article.fetched_utc.clone(),
            }]),
        );
        let (mut state, _) = harvester_core::update(
            state,
            harvester_core::Msg::EvaluatePreTriageRefresh {
                ordered_urls: vec![url.into()],
                triggered_by_job_done: false,
            },
        );
        let mut load_request_id = None;
        for tick in 1..=20 {
            let (next, effects) = harvester_core::update(
                state,
                harvester_core::Msg::tick_at(
                    chrono::DateTime::from_timestamp(1_790_000_000 + tick, 0)
                        .expect("valid fixture time"),
                ),
            );
            state = next;
            load_request_id = load_request_id.or_else(|| {
                effects.iter().find_map(|effect| match effect {
                    harvester_core::Effect::LoadArticlesForTriage { request_id, .. } => {
                        Some(*request_id)
                    }
                    _ => None,
                })
            });
            if load_request_id.is_some() {
                break;
            }
        }
        let load_request_id = load_request_id.expect("pre-triage load request");
        let (state, _) = harvester_core::update(
            state,
            harvester_core::Msg::TriageArticlesLoaded {
                request_id: load_request_id,
                delta: harvester_engine::TriageArticleDelta::full_window(vec![article], 100_000),
            },
        );
        let (state, _) = harvester_core::update(
            state,
            harvester_core::Msg::LlmQuotaConfigured {
                limits: harvester_core::LlmQuotaLimits {
                    max_calls_per_session: Some(1),
                    max_input_tokens_per_session: None,
                    max_output_tokens_per_session: None,
                    max_cost_microdollars_per_session: None,
                },
            },
        );
        state
    }

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
            imports_completed: 0,
            imports_failed: 0,
            import_in_flight: false,
            source_poll_stats: vec![],
        }
    }

    #[test]
    fn startup_notice_names_mode_and_hydration_work() {
        assert_eq!(
            format_startup_notice("one cycle"),
            "Harvester batch · starting (one cycle) · loading state and caches"
        );
    }

    #[test]
    fn cycle_start_reports_unfinished_count_and_reprocess_notice() {
        let state = state_with_unfinished_article_and_limited_quota();
        let mut reporter = CycleStartWorkReporter::default();
        let lines = reporter.pending_lines(&state);
        assert!(lines[0].starts_with("[batch] cycle-start unfinished_articles=1 estimated_calls="));
        assert_eq!(
            lines.len(),
            1,
            "the host must not infer a notice from quota"
        );
        assert!(reporter.pending_lines(&state).is_empty());

        let (state, effects) = harvester_core::update(
            state,
            harvester_core::Msg::PipelineRunRequested {
                scope: harvester_core::PipelineRunScope::Resume,
            },
        );
        let (state, effects) = harvester_core::fixture_support::complete_processing_configuration(
            state, effects, 100_000,
        );
        let request_id = effects
            .iter()
            .find_map(|effect| match effect {
                harvester_core::Effect::LoadArticlesForTriage { request_id, .. } => {
                    Some(*request_id)
                }
                _ => None,
            })
            .expect("run loads its previous window");
        let (state, _) = harvester_core::update(
            state,
            harvester_core::Msg::TriageArticlesLoaded {
                request_id,
                delta: harvester_engine::TriageArticleDelta::full_window(
                    vec![harvester_core::LoadedArticle {
                        url: "https://cycle-start.example/article".into(),
                        source_title: Some("Cycle-start article".into()),
                        prepared_text: "articleword ".repeat(220),
                        content_hash: "cycle-start-content".into(),
                        fetched_utc: Some("2026-09-27T00:00:00Z".into()),
                    }],
                    100_000,
                ),
            },
        );
        assert!(reporter
            .pending_lines(&state)
            .iter()
            .any(|line| line.starts_with("[batch] reprocess notice:")));
        assert!(reporter.pending_lines(&state).is_empty());
    }

    #[test]
    fn test_microdollars_to_display_zero() {
        assert_eq!(microdollars_to_display(0), "$0.00");
    }

    #[test]
    fn test_microdollars_to_display_rounds_down() {
        // 50 microdollars = $0.000050 -> displays $0.00
        assert_eq!(microdollars_to_display(50), "$0.00");
        // 4999 microdollars = $0.004999 -> displays $0.00
        assert_eq!(microdollars_to_display(4999), "$0.00");
    }

    #[test]
    fn test_microdollars_to_display_truncates_fractional_cents() {
        // Preserve the dashboard header's precision: truncate fractional cents.
        assert_eq!(microdollars_to_display(5000), "$0.00");
        assert_eq!(microdollars_to_display(15000), "$0.01");
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
        // 1234567 microdollars = $1.234567 -> displays $1.23
        assert_eq!(microdollars_to_display(1234567), "$1.23");
        // 5678901 microdollars = $5.678901 -> displays $5.67
        assert_eq!(microdollars_to_display(5678901), "$5.67");
    }

    #[test]
    fn test_microdollars_to_display_large_values() {
        // 123456789 microdollars = $123.456789 -> displays $123.45
        assert_eq!(microdollars_to_display(123456789), "$123.45");
        // 1000000000 microdollars = $1000.00
        assert_eq!(microdollars_to_display(1000000000), "$1000.00");
    }

    #[test]
    fn default_progress_output_includes_poll_summary_once_but_excludes_verbose_diagnostics() {
        let mut observation = observation_with_totals(1, 1, 0, 1, 0, 1, 0);
        observation.source_poll_stats.push(SourcePollStat {
            source_id: SourceId::new("test-rss").unwrap(),
            kind: SourceKind::Rss,
            parsed: 2,
            dedup_filtered: 1,
            emitted: 1,
        });
        let mut reporter = PollSummaryReporter::default();
        assert!(reporter.take(&[]).is_none());
        let details = reporter.take(&observation.source_poll_stats).unwrap();
        let block = crate::progress::format_progress_block(
            &crate::progress::test_stages(),
            false,
            Duration::ZERO,
            0,
        );
        assert!(!block.join("\n").contains("Poll summary"));
        assert!(reporter.take(&observation.source_poll_stats).is_none());
        assert_eq!(details.matches("--- Poll summary ---").count(), 1);
        assert!(details.contains("test-rss"));
        assert!(!details.contains("Cycle"));
        assert!(!details.contains("gpt-test: in=10 out=20"));
    }

    #[test]
    fn ordinary_final_summary_retains_cycle_wording() {
        let summary = format_final_summary(
            7,
            &observation_with_totals(2, 2, 0, 1, 0, 1, 0),
            2,
            1,
            1,
            Duration::from_secs(136),
        );

        assert!(summary.contains("Batch complete: 7 cycles"));
    }
}
