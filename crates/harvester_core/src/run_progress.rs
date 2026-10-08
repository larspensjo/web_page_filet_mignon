use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

/// The fixed ordered stages rendered by the desktop run timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineStage {
    ScanningSources,
    DownloadingArticles,
    LoadingArticles,
    Triaging,
    Summarizing,
    ScoringSignals,
}

impl PipelineStage {
    pub const ALL: [Self; 6] = [
        Self::ScanningSources,
        Self::DownloadingArticles,
        Self::LoadingArticles,
        Self::Triaging,
        Self::Summarizing,
        Self::ScoringSignals,
    ];

    pub const fn index(self) -> usize {
        match self {
            Self::ScanningSources => 0,
            Self::DownloadingArticles => 1,
            Self::LoadingArticles => 2,
            Self::Triaging => 3,
            Self::Summarizing => 4,
            Self::ScoringSignals => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageStatus {
    Pending,
    Active,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageRecord {
    pub status: StageStatus,
    pub completed: u32,
    /// Settled without a model request this run; always <= completed; zero for
    /// stages without model work. An item judged new work at admission stays new
    /// work even if it later settles from a result produced earlier in this run.
    pub reused: u32,
    pub failed: u32,
    pub total: u32,
    pub total_is_final: bool,
    pub started_at_utc: Option<DateTime<Utc>>,
    pub ended_at_utc: Option<DateTime<Utc>>,
}

impl Default for StageRecord {
    fn default() -> Self {
        Self {
            status: StageStatus::Pending,
            completed: 0,
            reused: 0,
            failed: 0,
            total: 0,
            total_is_final: false,
            started_at_utc: None,
            ended_at_utc: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityOutcome {
    Started,
    Succeeded,
    Failed { reason: String },
    Skipped { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub seq: u64,
    pub url: String,
    pub title: Option<String>,
    pub stage: PipelineStage,
    pub outcome: ActivityOutcome,
}

/// Measurement-derived, not a proposal: at 200 the feed added ~49 KB to every
/// envelope and held the desktop page two generations behind, failing the
/// probe's `latency_ms_p95 < 100` gate. See docs/DecisionLog.md (2026-09-08).
pub const ACTIVITY_FEED_CAPACITY: usize = 50;
pub(crate) const ACTIVITY_REASON_MAX_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunProgress {
    pub(crate) run_id: u64,
    pub(crate) started_at_utc: Option<DateTime<Utc>>,
    pub(crate) stages: [StageRecord; 6],
    pub(crate) activity: VecDeque<ActivityEntry>,
    pub(crate) next_seq: u64,
    pub(crate) signal_completed_at_reset: usize,
    pub(crate) signal_failed_at_reset: u32,
    pub(crate) signal_enqueued_at_reset: u32,
    pub(crate) download_started_job_ids: BTreeSet<crate::JobId>,
    pub(crate) download_finished_job_ids: BTreeSet<crate::JobId>,
    pub(crate) terminal: bool,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct StageCounts {
    pub completed: u32,
    pub failed: u32,
    pub total: u32,
    pub reused: u32,
}

impl RunProgress {
    pub(crate) fn new(
        run_id: u64,
        started_at_utc: Option<DateTime<Utc>>,
        signal_completed_at_reset: usize,
        signal_failed_at_reset: u32,
        signal_enqueued_at_reset: u32,
    ) -> Self {
        Self {
            run_id,
            started_at_utc,
            stages: std::array::from_fn(|_| StageRecord::default()),
            activity: VecDeque::with_capacity(ACTIVITY_FEED_CAPACITY),
            next_seq: 0,
            signal_completed_at_reset,
            signal_failed_at_reset,
            signal_enqueued_at_reset,
            download_started_job_ids: BTreeSet::new(),
            download_finished_job_ids: BTreeSet::new(),
            terminal: false,
        }
    }

    pub(crate) fn stage_mut(&mut self, stage: PipelineStage) -> &mut StageRecord {
        &mut self.stages[stage.index()]
    }
    pub(crate) fn activate(
        &mut self,
        stage: PipelineStage,
        total: u32,
        now: Option<DateTime<Utc>>,
    ) {
        let record = self.stage_mut(stage);
        if matches!(record.status, StageStatus::Done | StageStatus::Failed) {
            return;
        }
        if matches!(record.status, StageStatus::Pending) {
            record.status = StageStatus::Active;
        }
        if total > 0 && record.started_at_utc.is_none() {
            record.started_at_utc = now;
        }
        record.total = record.total.max(total);
    }
    pub(crate) fn counts(
        &mut self,
        stage: PipelineStage,
        counts: StageCounts,
        now: Option<DateTime<Utc>>,
    ) {
        self.activate(stage, counts.total, now);
        let record = self.stage_mut(stage);
        record.completed = record.completed.max(counts.completed);
        record.failed = record.failed.max(counts.failed);
        record.total = record.total.max(counts.total);
        record.reused = record.reused.max(counts.reused);
    }
    pub(crate) fn finish(&mut self, stage: PipelineStage, now: Option<DateTime<Utc>>) {
        let record = self.stage_mut(stage);
        record.total_is_final = true;
        if matches!(record.status, StageStatus::Pending) {
            return;
        }
        if !matches!(record.status, StageStatus::Done | StageStatus::Failed) {
            record.status = if record.completed == 0 && record.failed > 0 {
                StageStatus::Failed
            } else {
                StageStatus::Done
            };
            record.ended_at_utc = now;
        }
    }
    pub(crate) fn stop(&mut self, now: Option<DateTime<Utc>>) {
        for stage in PipelineStage::ALL {
            let record = self.stage_mut(stage);
            record.total_is_final = true;
            if matches!(record.status, StageStatus::Active) {
                record.status = if record.completed == 0 && record.failed > 0 {
                    StageStatus::Failed
                } else {
                    StageStatus::Done
                };
                record.ended_at_utc = now;
            }
        }
        self.terminal = true;
    }
    pub(crate) fn begin_stopping(&mut self) {
        for stage in PipelineStage::ALL {
            let record = self.stage_mut(stage);
            record.total_is_final = true;
            if record.status == StageStatus::Active
                && record.total == 0
                && record.completed == 0
                && record.failed == 0
            {
                record.status = StageStatus::Pending;
            }
        }
    }
    pub(crate) fn settle(&mut self, now: Option<DateTime<Utc>>) {
        self.stop(now);
        for record in &mut self.stages {
            if record.status == StageStatus::Pending {
                record.status = StageStatus::Done;
                record.ended_at_utc = now;
            }
        }
    }
    pub(crate) fn push_activity(
        &mut self,
        url: String,
        title: Option<String>,
        stage: PipelineStage,
        outcome: ActivityOutcome,
    ) {
        if self.activity.len() == ACTIVITY_FEED_CAPACITY {
            self.activity.pop_front();
        }
        let entry = ActivityEntry {
            seq: self.next_seq,
            url,
            title,
            stage,
            outcome,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        self.activity.push_back(entry);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RunProgressView {
    pub stages: Vec<StageProgress>,
    pub run_active: bool,
    pub activity: Vec<ActivityEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageProgress {
    pub stage: PipelineStage,
    pub status: StageStatus,
    pub completed: u32,
    /// Settled without a model request this run; always <= completed; zero for
    /// stages without model work. An item judged new work at admission stays new
    /// work even if it later settles from a result produced earlier in this run.
    pub reused: u32,
    pub failed: u32,
    pub total: u32,
    pub total_is_final: bool,
    pub started_at_utc: Option<DateTime<Utc>>,
    pub ended_at_utc: Option<DateTime<Utc>>,
}

impl RunProgress {
    pub(crate) fn view(&self) -> RunProgressView {
        RunProgressView {
            stages: PipelineStage::ALL
                .into_iter()
                .map(|stage| {
                    let r = &self.stages[stage.index()];
                    StageProgress {
                        stage,
                        status: r.status,
                        completed: r.completed,
                        reused: r.reused,
                        failed: r.failed,
                        total: r.total,
                        total_is_final: r.total_is_final,
                        started_at_utc: r.started_at_utc,
                        ended_at_utc: r.ended_at_utc,
                    }
                })
                .collect(),
            run_active: !self.terminal,
            activity: self.activity.iter().cloned().collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineRunPhase {
    Idle,
    Requested,
    Stopping,
}

/// Export-facing lifecycle state for the current pipeline run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunState {
    Idle,
    Active,
    Stopping { in_flight: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionNotice {
    pub new_result_count: usize,
    pub completed_at_utc: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PipelineActivity {
    pub poll_in_progress: usize,
    pub jobs_pending_or_in_flight: usize,
    pub pre_triage_loading: usize,
    pub intake_refresh_pending: bool,
    pub triage_pending_or_in_flight: usize,
    pub summary_pending_or_in_flight: usize,
    pub signal_pending_or_in_flight: usize,
    pub briefing_active: usize,
    pub import_in_flight: usize,
}

impl PipelineActivity {
    pub fn is_settled(&self) -> bool {
        !self.intake_refresh_pending
            && self.poll_in_progress == 0
            && self.jobs_pending_or_in_flight == 0
            && self.pre_triage_loading == 0
            && self.triage_pending_or_in_flight == 0
            && self.summary_pending_or_in_flight == 0
            && self.signal_pending_or_in_flight == 0
            && self.briefing_active == 0
            && self.import_in_flight == 0
    }
}
