use harvester_core::{PipelineStage, StageProgress, StageStatus};
use std::time::Duration;

const BLOCK_STAGES: [(PipelineStage, &str); 5] = [
    (PipelineStage::ScanningSources, "Scanning sources"),
    (PipelineStage::DownloadingArticles, "Downloading"),
    (PipelineStage::Triaging, "Triage"),
    (PipelineStage::Summarizing, "Summaries"),
    (PipelineStage::ScoringSignals, "Scoring"),
];

pub(crate) fn progress_status_signature(
    stages: &[StageProgress],
    stopping: bool,
) -> Vec<&'static str> {
    BLOCK_STAGES
        .iter()
        .map(|(stage, _)| stage_status(stages.iter().find(|row| row.stage == *stage), stopping))
        .collect()
}

fn stage_status(row: Option<&StageProgress>, stopping: bool) -> &'static str {
    match row {
        Some(row)
            if row.status == StageStatus::Active
                && !stopping
                && row.stage != PipelineStage::ScanningSources
                && !row.total_is_final
                && row
                    .total
                    .saturating_sub(row.reused)
                    .saturating_sub(row.completed.saturating_sub(row.reused))
                    .saturating_sub(row.failed)
                    == 0 =>
        {
            "Waiting for articles"
        }
        Some(row) => match row.status {
            StageStatus::Pending => "Pending",
            StageStatus::Active => "In progress",
            StageStatus::Done => "Done",
            StageStatus::Failed => "Failed",
        },
        None => "Pending",
    }
}

/// Presentation only: counts the same new work as the desktop stage rows.
/// Loading articles is preparation, so the command-line block omits that row.
pub(crate) fn format_progress_block(
    stages: &[StageProgress],
    stopping: bool,
    elapsed: Duration,
    cost_microdollars: u64,
) -> Vec<String> {
    let hint = if stopping {
        "Stopping safely; Ctrl+C again exits immediately"
    } else {
        "Ctrl+C stops safely"
    };
    let mut lines = vec![format!(
        "Harvester batch | {} | cost this run {} | {hint}",
        format_elapsed(elapsed),
        format_cost(cost_microdollars),
    )];
    for (stage, label) in BLOCK_STAGES {
        let row = stages.iter().find(|row| row.stage == stage);
        let (total, done, failed) = row.map_or((0, 0, 0), |row| {
            (
                row.total.saturating_sub(row.reused),
                row.completed.saturating_sub(row.reused),
                row.failed,
            )
        });
        let remaining = total.saturating_sub(done).saturating_sub(failed);
        let count = if stopping {
            format!("{done} done")
        } else if total == 0 {
            "0 to do".into()
        } else {
            format!("{remaining} of {total} to do")
        };
        let failures = if failed > 0 {
            format!(" | {failed} failed")
        } else {
            String::new()
        };
        let status = stage_status(row, stopping);
        lines.push(format!("{label}: {count}{failures} | {status}"));
    }
    lines
}

pub(crate) fn format_cost(microdollars: u64) -> String {
    let dollars = microdollars / 1_000_000;
    let cents = (microdollars % 1_000_000) / 10_000;
    format!("${dollars}.{cents:02}")
}

fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
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

#[cfg(test)]
pub(crate) fn test_stages() -> Vec<StageProgress> {
    PipelineStage::ALL
        .into_iter()
        .map(|stage| StageProgress {
            stage,
            status: StageStatus::Active,
            total: 10,
            completed: 5,
            reused: if stage.index() >= PipelineStage::Triaging.index() {
                3
            } else {
                0
            },
            failed: 1,
            total_is_final: true,
            started_at_utc: None,
            ended_at_utc: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(stages: &[StageProgress], stopping: bool) -> String {
        format_progress_block(stages, stopping, Duration::from_secs(136), 1_250_000).join("\n")
    }

    #[test]
    fn mid_run_block_excludes_reused_work_and_shows_failures() {
        assert_eq!(
            block(&test_stages(), false),
            concat!(
                "Harvester batch | 2m16s | cost this run $1.25 | Ctrl+C stops safely\n",
                "Scanning sources: 4 of 10 to do | 1 failed | In progress\n",
                "Downloading: 4 of 10 to do | 1 failed | In progress\n",
                "Triage: 4 of 7 to do | 1 failed | In progress\n",
                "Summaries: 4 of 7 to do | 1 failed | In progress\n",
                "Scoring: 4 of 7 to do | 1 failed | In progress"
            )
        );
    }

    #[test]
    fn open_article_stages_wait_for_articles() {
        let mut stages = test_stages();
        for row in &mut stages {
            row.total = 0;
            row.completed = 0;
            row.reused = 0;
            row.failed = 0;
            row.total_is_final = false;
            row.status = if row.stage == PipelineStage::ScanningSources {
                StageStatus::Pending
            } else {
                StageStatus::Active
            };
        }
        assert_eq!(
            block(&stages, false),
            concat!(
                "Harvester batch | 2m16s | cost this run $1.25 | Ctrl+C stops safely\n",
                "Scanning sources: 0 to do | Pending\n",
                "Downloading: 0 to do | Waiting for articles\n",
                "Triage: 0 to do | Waiting for articles\n",
                "Summaries: 0 to do | Waiting for articles\n",
                "Scoring: 0 to do | Waiting for articles"
            )
        );
    }

    #[test]
    fn settled_zero_new_work_has_zero_to_do() {
        let mut stages = test_stages();
        for row in &mut stages {
            row.total = row.reused;
            row.completed = row.reused;
            row.failed = 0;
            row.status = StageStatus::Done;
        }
        assert_eq!(
            block(&stages, false),
            concat!(
                "Harvester batch | 2m16s | cost this run $1.25 | Ctrl+C stops safely\n",
                "Scanning sources: 0 to do | Done\n",
                "Downloading: 0 to do | Done\n",
                "Triage: 0 to do | Done\n",
                "Summaries: 0 to do | Done\n",
                "Scoring: 0 to do | Done"
            )
        );
    }

    #[test]
    fn stopping_block_shows_new_done_counts_and_failures() {
        let mut stages = test_stages();
        for row in &mut stages {
            row.total_is_final = false;
        }
        assert_eq!(
            block(&stages, true),
            concat!(
                "Harvester batch | 2m16s | cost this run $1.25 | Stopping safely; Ctrl+C again exits immediately\n",
                "Scanning sources: 5 done | 1 failed | In progress\n",
                "Downloading: 5 done | 1 failed | In progress\n",
                "Triage: 2 done | 1 failed | In progress\n",
                "Summaries: 2 done | 1 failed | In progress\n",
                "Scoring: 2 done | 1 failed | In progress"
            )
        );
    }

    #[test]
    fn pending_and_failed_open_article_stages_keep_their_status() {
        let mut stages = test_stages();
        for row in &mut stages {
            row.total = 0;
            row.completed = 0;
            row.reused = 0;
            row.failed = 0;
            row.total_is_final = false;
            row.status = StageStatus::Pending;
        }
        stages[PipelineStage::Summarizing.index()].status = StageStatus::Failed;
        stages[PipelineStage::Summarizing.index()].failed = 1;
        assert_eq!(
            block(&stages, false),
            concat!(
                "Harvester batch | 2m16s | cost this run $1.25 | Ctrl+C stops safely\n",
                "Scanning sources: 0 to do | Pending\n",
                "Downloading: 0 to do | Pending\n",
                "Triage: 0 to do | Pending\n",
                "Summaries: 0 to do | 1 failed | Failed\n",
                "Scoring: 0 to do | Pending"
            )
        );
    }

    #[test]
    fn progress_signature_tracks_waiting_and_status_changes_without_counts() {
        let mut stages = test_stages();
        let active = progress_status_signature(&stages, false);
        stages[PipelineStage::Triaging.index()].completed += 1;
        assert_eq!(progress_status_signature(&stages, false), active);
        let row = &mut stages[PipelineStage::Triaging.index()];
        row.completed = row.total;
        row.failed = 0;
        row.total_is_final = false;
        let waiting = progress_status_signature(&stages, false);
        assert_eq!(waiting[2], "Waiting for articles");
        assert_ne!(waiting, active);
        assert_eq!(progress_status_signature(&stages, true)[2], "In progress");
        for (status, label) in [
            (StageStatus::Pending, "Pending"),
            (StageStatus::Done, "Done"),
            (StageStatus::Failed, "Failed"),
        ] {
            stages[PipelineStage::Triaging.index()].status = status;
            assert_eq!(progress_status_signature(&stages, false)[2], label);
        }
    }

    #[test]
    fn overfull_counts_saturate_and_stages_keep_independent_totals() {
        let mut stages = test_stages();
        stages[PipelineStage::Triaging.index()].reused = 100;
        stages[PipelineStage::Summarizing.index()].completed = 100;
        stages[PipelineStage::ScoringSignals.index()].total = 20;
        let text = block(&stages, false);
        assert!(text.contains("Triage: 0 to do | 1 failed"));
        assert!(text.contains("Summaries: 0 of 7 to do | 1 failed"));
        assert!(text.contains("Scoring: 14 of 17 to do | 1 failed"));
        assert!(!text.contains("reused") && !text.contains("left"));
    }
}
