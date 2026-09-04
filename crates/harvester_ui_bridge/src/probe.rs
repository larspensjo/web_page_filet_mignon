//! Tauri-free throughput-probe construction, statistics, and thresholds.

use harvester_core::AppViewModel;
use serde::{Deserialize, Serialize};

use crate::{project, ProjectedSnapshot, SNAPSHOT_MIN_INTERVAL_MS};

/// The phase-1c probe deliberately measures job-list envelopes only; the activity feed lands in phase 2.
pub const PROBE_JOB_COUNT: usize = 300;
pub const PROBE_RATE_HZ: u64 = 20;
pub const PROBE_DURATION: std::time::Duration = std::time::Duration::from_secs(60);
pub const PROBE_CASE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
pub const PROBE_HOST_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProbePageReport {
    pub frames: u64,
    pub slow_frames: u64,
    pub csp_fetch_rejected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProbeReport {
    pub snapshot_min_interval_ms: u64,
    pub activity_feed_capacity: Option<usize>,
    pub note: String,
    pub envelope_bytes_p50: u64,
    pub envelope_bytes_p95: u64,
    pub latency_ms_p95: u64,
    pub latency_ms_max: u64,
    pub latency_measurement: String,
    pub backlog_p95: u64,
    pub backlog_max: u64,
    pub slow_frame_percent: f64,
    pub csp_fetch_rejected: bool,
    pub invoke_succeeded: bool,
    pub passed: bool,
    pub tauri_version: Option<String>,
    pub wry_version: Option<String>,
    pub webview2_com_version: Option<String>,
    pub webview2_runtime_version: Option<String>,
}

/// Make a deterministic, non-state-owned view, then project it through production IPC projection.
pub fn synthetic_snapshot(generation: u64) -> ProjectedSnapshot {
    let mut json = serde_json::to_value(AppViewModel::default()).expect("view serializes");
    let jobs = (0..PROBE_JOB_COUNT)
        .map(|index| {
            serde_json::json!({
                "job_id": index as u64 + 1,
                "url": format!("https://probe.invalid/{index}?generation={generation}"),
                "stage": "Done",
                "outcome": "Success",
                "tokens": generation as u32,
                "bytes": 1024_u64 + index as u64,
                "link_count": 0,
                "downloaded_link_count": 0,
                "links": [],
                "origin": "Direct",
                "triage_annotation": null,
                "has_summary": false,
                "summary_title": null,
                "summary_tokens": null,
                "filter_status": null,
                "has_analysis": false,
                "is_since_checkpoint": false
            })
        })
        .collect::<Vec<_>>();
    json["jobs"] = serde_json::Value::Array(jobs);
    let view: AppViewModel =
        serde_json::from_value(json).expect("probe job rows match AppViewModel");
    project(&view).0
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

pub fn evaluate(
    mut latency_ms: Vec<u64>,
    mut backlog: Vec<u64>,
    page: ProbePageReport,
    bytes: Vec<u64>,
) -> ProbeReport {
    let latency_p95 = percentile(&mut latency_ms, 95, 100);
    let backlog_p95 = percentile(&mut backlog, 95, 100);
    let bytes_p50 = percentile(&mut bytes.clone(), 50, 100);
    let bytes_p95 = percentile(&mut bytes.clone(), 95, 100);
    let latency_max = latency_ms.into_iter().max().unwrap_or(0);
    let backlog_max = backlog.into_iter().max().unwrap_or(0);
    let slow_frame_percent = if page.frames == 0 {
        100.0
    } else {
        page.slow_frames as f64 * 100.0 / page.frames as f64
    };
    let passed = latency_p95 < 100
        && latency_max < 400
        && backlog_p95 <= 2
        && backlog_max <= 10
        && slow_frame_percent <= 2.0
        && page.csp_fetch_rejected;
    ProbeReport {
        snapshot_min_interval_ms: SNAPSHOT_MIN_INTERVAL_MS,
        activity_feed_capacity: None,
        note: "Phase 1c has no activity feed; envelope byte-size measurement must be repeated in phase 2.".into(),
        envelope_bytes_p50: bytes_p50, envelope_bytes_p95: bytes_p95,
        latency_ms_p95: latency_p95, latency_ms_max: latency_max, backlog_p95, backlog_max,
        latency_measurement: "host monotonic emit timestamp to host monotonic probe_ack timestamp".into(),
        slow_frame_percent, csp_fetch_rejected: page.csp_fetch_rejected, invoke_succeeded: false, passed,
        tauri_version: None, wry_version: None,
        webview2_com_version: None, webview2_runtime_version: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_uses_the_real_projection_and_mutates_every_generation() {
        let first = synthetic_snapshot(1);
        let second = synthetic_snapshot(2);
        assert_eq!(
            first.view["jobs"].as_array().unwrap().len(),
            PROBE_JOB_COUNT
        );
        assert_ne!(first.view["jobs"][0]["url"], second.view["jobs"][0]["url"]);
    }
    #[test]
    fn thresholds_and_percentiles_are_exact() {
        let report = evaluate(
            vec![10, 99],
            vec![0, 2],
            ProbePageReport {
                frames: 100,
                slow_frames: 2,
                csp_fetch_rejected: true,
            },
            vec![100, 200],
        );
        assert!(report.passed);
        assert_eq!(report.envelope_bytes_p50, 100);
        assert_eq!(report.envelope_bytes_p95, 200);
        assert_eq!(report.tauri_version, None);
        assert_eq!(report.wry_version, None);
        assert_eq!(report.webview2_com_version, None);
        assert!(
            !evaluate(
                vec![400],
                vec![0],
                ProbePageReport {
                    frames: 100,
                    slow_frames: 0,
                    csp_fetch_rejected: true
                },
                vec![1]
            )
            .passed
        );
        assert!(
            !evaluate(
                vec![1],
                vec![11],
                ProbePageReport {
                    frames: 100,
                    slow_frames: 0,
                    csp_fetch_rejected: true
                },
                vec![1]
            )
            .passed
        );
    }
}
