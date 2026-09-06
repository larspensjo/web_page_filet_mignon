//! Tauri-free throughput-probe construction, statistics, and thresholds.

use std::time::Duration;

use chrono::{DateTime, Utc};
use harvester_core::{
    AppViewModel, DesktopJobListView, JobFilterStatus, JobListMode, JobListRowView, JobOrigin,
    JobResultKind, LinkDownloadState, LinkRowView, ScoreBand, SelectedJobView,
    SelectedJobVisibility, SignalCandidateOutcome, SignalCandidateRow, SignalCandidateRowState,
    Stage, TriageAnnotationView, DESKTOP_JOB_LIST_MAX_ROWS, MAX_EXTRACTED_LINKS,
};
use harvester_engine::{llm::dto::SourceTier, LinkKind};
use serde::{Deserialize, Serialize};

use crate::{project, ProjectedSnapshot, SNAPSHOT_MIN_INTERVAL_MS};

pub const PROBE_CORPUS_JOBS: usize = 9_475;
pub const PROBE_TYPICAL_LIST_ROWS: usize = 110;
pub const PROBE_GATED_CASE_DURATION: Duration = Duration::from_secs(45);
pub const PROBE_SCENARIO_DURATION: Duration = Duration::from_secs(10);
pub const PROBE_ENVELOPE_BYTES_P95_BUDGET: u64 = 400 * 1024;
pub const PROBE_RATE_HZ: u64 = 20;
pub const PROBE_HOST_WATCHDOG: Duration = Duration::from_secs(600);
/// How long the host waits for one page-side step - a freshly created page window requesting its
/// first snapshot, or a finished case sending its report - before giving up on that case. Far
/// shorter than `PROBE_HOST_WATCHDOG`, which bounds the whole run, so a page that never responds
/// surfaces in seconds instead of stalling the run for the full watchdog.
pub const PROBE_PAGE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// The two gated payload shapes, then the two measured-only worst shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeCase {
    TypicalScope,
    CappedNoCheckpoint,
    PopulatedResults,
    SelectedJobMaxLinks,
}

impl ProbeCase {
    pub const ALL: [Self; 4] = [
        Self::TypicalScope,
        Self::CappedNoCheckpoint,
        Self::PopulatedResults,
        Self::SelectedJobMaxLinks,
    ];

    pub fn is_gated(self) -> bool {
        matches!(self, Self::TypicalScope | Self::CappedNoCheckpoint)
    }

    pub fn duration(self) -> Duration {
        if self.is_gated() {
            PROBE_GATED_CASE_DURATION
        } else {
            PROBE_SCENARIO_DURATION
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::TypicalScope => "typical-scope",
            Self::CappedNoCheckpoint => "capped-no-checkpoint",
            Self::PopulatedResults => "populated-results",
            Self::SelectedJobMaxLinks => "selected-job-max-links",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProbePageReport {
    pub frames: u64,
    pub slow_frames: u64,
    pub csp_fetch_rejected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeCaseSamples {
    pub case: ProbeCase,
    pub latency_ms: Vec<u64>,
    pub backlog: Vec<u64>,
    pub envelope_bytes: Vec<u64>,
    pub page: ProbePageReport,
    pub invoke_succeeded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProbeCaseReport {
    pub case: String,
    pub gated: bool,
    pub list_rows: usize,
    pub corpus_jobs: usize,
    pub candidate_rows: usize,
    pub selected_link_count: usize,
    pub envelope_bytes_p50: u64,
    pub envelope_bytes_p95: u64,
    pub latency_ms_p95: u64,
    pub latency_ms_max: u64,
    pub backlog_p95: u64,
    pub backlog_max: u64,
    pub frames: u64,
    pub slow_frames: u64,
    pub slow_frame_percent: f64,
    pub csp_fetch_rejected: bool,
    pub invoke_succeeded: bool,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    pub snapshot_min_interval_ms: u64,
    pub activity_feed_capacity: Option<usize>,
    pub cases: Vec<ProbeCaseReport>,
    pub passed: bool,
    pub tauri_version: Option<String>,
    pub wry_version: Option<String>,
    pub webview2_com_version: Option<String>,
    pub webview2_runtime_version: Option<String>,
}

/// Make a deterministic, non-state-owned view at the desktop projection shape.
pub fn synthetic_view(case: ProbeCase, generation: u64) -> AppViewModel {
    let list_rows = match case {
        ProbeCase::CappedNoCheckpoint => DESKTOP_JOB_LIST_MAX_ROWS,
        _ => PROBE_TYPICAL_LIST_ROWS,
    };
    let rows = (0..list_rows)
        .map(|index| synthetic_job_list_row(index, generation))
        .collect::<Vec<_>>();
    let selected_link_count = match case {
        ProbeCase::TypicalScope => 20,
        ProbeCase::SelectedJobMaxLinks => MAX_EXTRACTED_LINKS,
        ProbeCase::CappedNoCheckpoint | ProbeCase::PopulatedResults => 0,
    };
    let selected_job =
        (selected_link_count > 0).then(|| synthetic_selected_job(generation, selected_link_count));
    let signal_candidate_rows = match case {
        ProbeCase::TypicalScope | ProbeCase::SelectedJobMaxLinks => {
            synthetic_signal_candidate_rows(generation, 24)
        }
        ProbeCase::PopulatedResults => synthetic_signal_candidate_rows(generation, 750),
        ProbeCase::CappedNoCheckpoint => Vec::new(),
    };
    let results = matches!(case, ProbeCase::PopulatedResults);
    AppViewModel {
        job_count: PROBE_CORPUS_JOBS,
        job_list_mode: if results {
            JobListMode::Results
        } else {
            JobListMode::SinceCheckpoint
        },
        desktop_job_list: DesktopJobListView {
            mode: if results {
                JobListMode::Results
            } else {
                JobListMode::SinceCheckpoint
            },
            rows: if results { Vec::new() } else { rows },
            selected_job,
            scoped_count: match case {
                ProbeCase::TypicalScope | ProbeCase::SelectedJobMaxLinks => PROBE_TYPICAL_LIST_ROWS,
                ProbeCase::CappedNoCheckpoint => PROBE_CORPUS_JOBS,
                ProbeCase::PopulatedResults => 0,
            },
            visible_count: if results { 0 } else { list_rows },
            truncated: matches!(case, ProbeCase::CappedNoCheckpoint),
            hidden_without_fetch_time: 0,
            ..Default::default()
        },
        signal_candidate_rows,
        ..Default::default()
    }
}

/// Project the deterministic probe view through production IPC projection.
pub fn synthetic_snapshot(case: ProbeCase, generation: u64) -> ProjectedSnapshot {
    project(&synthetic_view(case, generation)).0
}

fn synthetic_job_list_row(index: usize, generation: u64) -> JobListRowView {
    JobListRowView {
        job_id: index as u64 + 1,
        url: format!("https://probe.invalid/articles/{index}?generation={generation}"),
        stage: Stage::Done,
        outcome: Some(JobResultKind::Success),
        tokens: Some(1_024 + index as u32),
        bytes: Some(32_768 + index as u64),
        link_count: 3,
        downloaded_link_count: 1,
        origin: JobOrigin::Direct,
        triage_annotation: Some(TriageAnnotationView {
            priority: 3,
            category: "Enterprise technology".into(),
            tags: vec!["cloud".into(), "security".into(), "markets".into()],
        }),
        has_summary: true,
        summary_title: Some(format!(
            "Probe article {index} maps reliable enterprise platform changes {generation}"
        )),
        summary_tokens: Some(256),
        filter_status: Some(JobFilterStatus::AutoIncluded),
        has_analysis: true,
        is_since_checkpoint: true,
        fetched_utc: Some(probe_time(index as i64)),
    }
}

fn synthetic_selected_job(generation: u64, link_count: usize) -> SelectedJobView {
    SelectedJobView {
        job_id: 1,
        url: format!("https://probe.invalid/selected?generation={generation}"),
        summary_title: Some(format!(
            "Selected probe article represents the complete reading-pane payload {generation}"
        )),
        stage: Stage::Done,
        outcome: Some(JobResultKind::Success),
        tokens: Some(2_048),
        bytes: Some(98_304),
        origin: JobOrigin::Direct,
        triage_annotation: Some(TriageAnnotationView {
            priority: 2,
            category: "Strategic intelligence".into(),
            tags: vec!["ai".into(), "policy".into(), "infrastructure".into()],
        }),
        has_summary: true,
        summary_tokens: Some(384),
        filter_status: Some(JobFilterStatus::AutoIncluded),
        fetched_utc: Some(probe_time(0)),
        list_visibility: SelectedJobVisibility::Visible,
        links: (0..link_count)
            .map(|index| LinkRowView {
                index: index as u32,
                url: format!(
                    "https://links.probe.invalid/reports/{index}/detail?generation={generation}"
                ),
                label: format!("Linked evidence {index} for the selected probe article"),
                kind: LinkKind::Hyperlink,
                download_state: LinkDownloadState::NotDownloaded,
                age_suspect: false,
            })
            .collect(),
    }
}

fn synthetic_signal_candidate_rows(generation: u64, count: usize) -> Vec<SignalCandidateRow> {
    (0..count)
        .map(|index| SignalCandidateRow {
            job_id: index as u64 + 1,
            url: format!("https://signals.probe.invalid/{index}?generation={generation}"),
            score: 40 + (index % 55) as u8,
            score_band: match index % 3 {
                0 => ScoreBand::High,
                1 => ScoreBand::Mid,
                _ => ScoreBand::Low,
            },
            source_tier: match index % 3 {
                0 => SourceTier::Tier1,
                1 => SourceTier::Tier2,
                _ => SourceTier::Tier3,
            },
            themes: vec![
                "enterprise software".into(),
                "market structure".into(),
                format!("theme-{}", index % 17),
            ],
            gist_truncated: format!(
                "Signal candidate {index} has a realistic concise gist for the probe generation {generation}."
            ),
            dupes_count: index % 8,
            state_label: match index % 3 {
                0 => SignalCandidateRowState::Scoring,
                1 => SignalCandidateRowState::Failed {
                    reason: "source metadata did not validate".into(),
                },
                _ => SignalCandidateRowState::Scored,
            },
            signal_key: format!("probe-signal-{}-{}", index % 97, generation),
            outcome: (index % 3 == 2).then_some(SignalCandidateOutcome::Selected),
        })
        .collect()
}

fn probe_time(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_725_000_000 + seconds, 0).expect("valid probe timestamp")
}

pub fn percentile(sorted: &mut [u64], numerator: usize, denominator: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted.sort_unstable();
    let index = (sorted.len() * numerator)
        .div_ceil(denominator)
        .saturating_sub(1);
    sorted[index]
}

pub fn evaluate(cases: Vec<ProbeCaseSamples>) -> ProbeReport {
    let cases = cases.into_iter().map(evaluate_case).collect::<Vec<_>>();
    let passed = ProbeCase::ALL
        .iter()
        .copied()
        .filter(|case| case.is_gated())
        .all(|required| {
            cases
                .iter()
                .any(|report| report.case == required.slug() && report.passed)
        });
    ProbeReport {
        snapshot_min_interval_ms: SNAPSHOT_MIN_INTERVAL_MS,
        activity_feed_capacity: None,
        cases,
        passed,
        tauri_version: None,
        wry_version: None,
        webview2_com_version: None,
        webview2_runtime_version: None,
    }
}

fn evaluate_case(samples: ProbeCaseSamples) -> ProbeCaseReport {
    let ProbeCaseSamples {
        case,
        mut latency_ms,
        mut backlog,
        mut envelope_bytes,
        page,
        invoke_succeeded,
    } = samples;
    let view = synthetic_view(case, 0);
    let envelope_bytes_p50 = percentile(&mut envelope_bytes, 50, 100);
    let envelope_bytes_p95 = percentile(&mut envelope_bytes, 95, 100);
    let latency_ms_p95 = percentile(&mut latency_ms, 95, 100);
    let backlog_p95 = percentile(&mut backlog, 95, 100);
    let latency_ms_max = latency_ms.into_iter().max().unwrap_or(0);
    let backlog_max = backlog.into_iter().max().unwrap_or(0);
    let slow_frame_percent = if page.frames == 0 {
        100.0
    } else {
        page.slow_frames as f64 * 100.0 / page.frames as f64
    };
    let gated = case.is_gated();
    let passed = !gated
        || (latency_ms_p95 < 100
            && latency_ms_max < 400
            && backlog_p95 <= 2
            && backlog_max <= 10
            && slow_frame_percent <= 2.0
            && page.csp_fetch_rejected
            && invoke_succeeded
            && envelope_bytes_p95 < PROBE_ENVELOPE_BYTES_P95_BUDGET);
    ProbeCaseReport {
        case: case.slug().into(),
        gated,
        list_rows: view.desktop_job_list.rows.len(),
        corpus_jobs: view.job_count,
        candidate_rows: view.signal_candidate_rows.len(),
        selected_link_count: view
            .desktop_job_list
            .selected_job
            .as_ref()
            .map_or(0, |selected| selected.links.len()),
        envelope_bytes_p50,
        envelope_bytes_p95,
        latency_ms_p95,
        latency_ms_max,
        backlog_p95,
        backlog_max,
        frames: page.frames,
        slow_frames: page.slow_frames,
        slow_frame_percent,
        csp_fetch_rejected: page.csp_fetch_rejected,
        invoke_succeeded,
        passed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_samples(case: ProbeCase) -> ProbeCaseSamples {
        ProbeCaseSamples {
            case,
            latency_ms: vec![10, 99],
            backlog: vec![0, 2],
            envelope_bytes: vec![100, 200],
            page: ProbePageReport {
                frames: 100,
                slow_frames: 2,
                csp_fetch_rejected: true,
            },
            invoke_succeeded: true,
        }
    }

    fn passing_gated_samples() -> Vec<ProbeCaseSamples> {
        vec![
            passing_samples(ProbeCase::TypicalScope),
            passing_samples(ProbeCase::CappedNoCheckpoint),
        ]
    }

    #[test]
    fn probe_uses_the_real_projection_and_mutates_every_generation() {
        let first = synthetic_snapshot(ProbeCase::TypicalScope, 1);
        let second = synthetic_snapshot(ProbeCase::TypicalScope, 2);
        assert_eq!(
            first.view["desktop_job_list"]["rows"]
                .as_array()
                .unwrap()
                .len(),
            PROBE_TYPICAL_LIST_ROWS
        );
        assert_ne!(
            first.view["desktop_job_list"]["rows"][0]["url"],
            second.view["desktop_job_list"]["rows"][0]["url"]
        );
        assert_eq!(
            synthetic_view(ProbeCase::SelectedJobMaxLinks, 1)
                .desktop_job_list
                .selected_job
                .unwrap()
                .links
                .len(),
            MAX_EXTRACTED_LINKS
        );
        for case in [ProbeCase::TypicalScope, ProbeCase::SelectedJobMaxLinks] {
            let view = synthetic_view(case, 1);
            assert_eq!(
                view.desktop_job_list.scoped_count,
                view.desktop_job_list.visible_count
            );
            assert!(!view.desktop_job_list.truncated);
        }
        let capped = synthetic_view(ProbeCase::CappedNoCheckpoint, 1);
        assert_eq!(capped.desktop_job_list.scoped_count, PROBE_CORPUS_JOBS);
        assert!(capped.desktop_job_list.truncated);
    }

    #[test]
    fn thresholds_and_percentiles_are_exact() {
        let report = evaluate(passing_gated_samples());
        let case = &report.cases[0];
        assert!(report.passed);
        assert_eq!(case.envelope_bytes_p50, 100);
        assert_eq!(case.envelope_bytes_p95, 200);
        assert_eq!(report.tauri_version, None);
        assert_eq!(report.wry_version, None);
        assert_eq!(report.webview2_com_version, None);
        assert!(
            !evaluate(vec![
                ProbeCaseSamples {
                    latency_ms: vec![400],
                    ..passing_samples(ProbeCase::TypicalScope)
                },
                passing_samples(ProbeCase::CappedNoCheckpoint),
            ])
            .passed
        );
        assert!(
            !evaluate(vec![
                ProbeCaseSamples {
                    backlog: vec![11],
                    ..passing_samples(ProbeCase::TypicalScope)
                },
                passing_samples(ProbeCase::CappedNoCheckpoint),
            ])
            .passed
        );
    }

    #[test]
    fn missing_gated_case_fails_the_run() {
        let report = evaluate(vec![passing_samples(ProbeCase::TypicalScope)]);
        assert!(!report.passed);
        assert_eq!(report.cases.len(), 1);
        assert_eq!(report.cases[0].case, ProbeCase::TypicalScope.slug());
    }

    #[test]
    fn a_failing_case_fails_the_run_even_when_the_aggregate_passes() {
        let typical = ProbeCaseSamples {
            page: ProbePageReport {
                frames: 100,
                slow_frames: 0,
                csp_fetch_rejected: true,
            },
            ..passing_samples(ProbeCase::TypicalScope)
        };
        let capped = ProbeCaseSamples {
            page: ProbePageReport {
                frames: 100,
                slow_frames: 3,
                csp_fetch_rejected: true,
            },
            ..passing_samples(ProbeCase::CappedNoCheckpoint)
        };
        let report = evaluate(vec![typical, capped]);
        assert!(!report.passed);
        assert!(report.cases[0].passed);
        assert!(!report.cases[1].passed);
    }

    #[test]
    fn a_non_gated_scenario_never_fails_the_run() {
        let oversized = ProbeCaseSamples {
            latency_ms: vec![500],
            envelope_bytes: vec![PROBE_ENVELOPE_BYTES_P95_BUDGET + 1],
            ..passing_samples(ProbeCase::SelectedJobMaxLinks)
        };
        let mut samples = passing_gated_samples();
        samples.push(oversized);
        let report = evaluate(samples);
        assert!(report.passed);
        assert!(report.cases[2].passed);
        assert!(report.cases[2].envelope_bytes_p95 > PROBE_ENVELOPE_BYTES_P95_BUDGET);
        assert_eq!(report.cases[2].latency_ms_max, 500);
    }
}
