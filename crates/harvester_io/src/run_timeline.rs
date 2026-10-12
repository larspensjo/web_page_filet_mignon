//! Host-boundary measurements. Instants come from callers; core stays clock-free.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use harvester_core::{AppState, LlmCapChange, LlmResultKind, Msg, PipelineStage};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::{FailureKind, JobId, JobOutcome, JobTiming};

#[derive(Default)]
pub struct RunTimeline {
    active: Option<RunMeasurements>,
    last_terminal: Option<u64>,
}

#[derive(Default)]
struct StepTotals {
    sum: Duration,
    max: Duration,
}
impl StepTotals {
    fn add(&mut self, value: Duration) {
        self.sum += value;
        self.max = self.max.max(value);
    }
}

#[derive(Default)]
struct ModelStage {
    requests: usize,
    completions: usize,
    failures: usize,
    first_dispatch: Option<Instant>,
    last_dispatch: Option<Instant>,
    last_completion: Option<Instant>,
    wall: Duration,
}

struct RunMeasurements {
    id: u64,
    host: String,
    start: Instant,
    stop: Option<Instant>,
    halted: bool,
    poll_start: Option<Instant>,
    poll_end: Option<Instant>,
    downloads: usize,
    successes: usize,
    failures: usize,
    cancelled: usize,
    download_jobs: HashSet<JobId>,
    last_download_completion: Option<Instant>,
    download_intervals: Vec<(Instant, Instant)>,
    fetch: StepTotals,
    extract: StepTotals,
    write: StepTotals,
    marker: StepTotals,
    enqueue_hand_off: Duration,
    forwarding_delay: Duration,
    model_stages: [ModelStage; 3],
    requests: HashMap<u64, (PromptId, Instant)>,
    max_model_in_flight: usize,
    rate_limited: usize,
    rate_limit_retries: u64,
    min_effective_llm_cap: usize,
    cap_changes: u64,
    remaining_requests: Option<u64>,
    remaining_tokens: Option<u64>,
    reducer: BTreeMap<String, Duration>,
    wave_numbers: HashSet<u64>,
    waves: Vec<(u64, PipelineStage, Instant, usize)>,
}

struct Observation {
    run_id: u64,
    stopped: bool,
    halted: bool,
    terminal: bool,
    model_in_flight: usize,
    effective_llm_cap: usize,
    cap_changes: u64,
    last_cap_change: Option<LlmCapChange>,
    waves: Vec<(u64, PipelineStage, usize)>,
}

impl RunTimeline {
    /// Called after every reducer step, before handing its effects to I/O.
    /// Returns each adaptive cap change once and timing lines on settlement.
    /// Repeated terminal observations are inert.
    pub fn observe_reducer(
        &mut self,
        state: &AppState,
        kind: &str,
        elapsed: Duration,
        now: Instant,
        host: &str,
    ) -> Vec<String> {
        let Some(progress) = state.run_progress() else {
            return Vec::new();
        };
        self.observe(
            Observation {
                run_id: progress.run_id(),
                stopped: state.run_stopped(),
                halted: state.provider_dispatch_halted(),
                terminal: progress.is_terminal(),
                model_in_flight: state.article_model_requests_in_flight(),
                effective_llm_cap: state.effective_llm_cap(),
                cap_changes: state.llm_cap_change_count(),
                last_cap_change: state.last_llm_cap_change(),
                waves: state.released_run_waves().collect(),
            },
            kind,
            elapsed,
            now,
            host,
        )
    }

    fn observe(
        &mut self,
        observation: Observation,
        kind: &str,
        elapsed: Duration,
        now: Instant,
        host: &str,
    ) -> Vec<String> {
        if self.last_terminal == Some(observation.run_id) {
            return Vec::new();
        }
        if self
            .active
            .as_ref()
            .is_none_or(|run| run.id != observation.run_id)
        {
            self.active = Some(RunMeasurements {
                id: observation.run_id,
                host: host.to_owned(),
                start: now,
                stop: None,
                halted: false,
                poll_start: None,
                poll_end: None,
                downloads: 0,
                successes: 0,
                failures: 0,
                cancelled: 0,
                download_jobs: HashSet::new(),
                last_download_completion: None,
                download_intervals: Vec::new(),
                fetch: StepTotals::default(),
                extract: StepTotals::default(),
                write: StepTotals::default(),
                marker: StepTotals::default(),
                enqueue_hand_off: Duration::ZERO,
                forwarding_delay: Duration::ZERO,
                model_stages: Default::default(),
                requests: HashMap::new(),
                max_model_in_flight: 0,
                rate_limited: 0,
                rate_limit_retries: 0,
                min_effective_llm_cap: observation.effective_llm_cap,
                cap_changes: 0,
                remaining_requests: None,
                remaining_tokens: None,
                reducer: BTreeMap::new(),
                wave_numbers: HashSet::new(),
                waves: Vec::new(),
            });
        }
        let run = self.active.as_mut().expect("active measurement");
        let mut lines = Vec::new();
        run.min_effective_llm_cap = run.min_effective_llm_cap.min(observation.effective_llm_cap);
        if observation.cap_changes > run.cap_changes {
            if let Some(LlmCapChange {
                from,
                to,
                reason,
                request_id,
            }) = observation.last_cap_change
            {
                lines.push(format!(
                    "[llm-adaptive-cap] run_id={} from={from} to={to} reason={reason} request_id={request_id}",
                    run.id
                ));
                run.min_effective_llm_cap = run.min_effective_llm_cap.min(from).min(to);
            }
            run.cap_changes = observation.cap_changes;
        }
        *run.reducer.entry(kind.to_owned()).or_default() += elapsed;
        run.max_model_in_flight = run.max_model_in_flight.max(observation.model_in_flight);
        run.halted |= observation.halted;
        if observation.stopped && run.stop.is_none() {
            run.stop = Some(now);
        }
        for (number, stage, members) in observation.waves {
            if run.wave_numbers.insert(number) {
                run.waves.push((number, stage, now, members));
            }
        }
        if !observation.terminal {
            return lines;
        }
        let run = self.active.take().expect("terminal measurement");
        self.last_terminal = Some(run.id);
        lines.extend(run.lines(now));
        lines
    }

    pub fn poll_started(&mut self, now: Instant) {
        if let Some(run) = &mut self.active {
            run.poll_start = Some(now);
        }
    }
    pub fn poll_ended(&mut self, now: Instant) {
        if let Some(run) = &mut self.active {
            run.poll_end = Some(now);
        }
    }
    pub fn enqueue_handed_off(&mut self, elapsed: Duration) {
        if let Some(run) = &mut self.active {
            run.enqueue_hand_off += elapsed;
        }
    }
    /// Associate a hand-off with its observed run before it can complete.
    /// Late completions from a settled run must not enter a following run's totals.
    pub fn download_queued(&mut self, job_id: JobId) {
        if let Some(run) = &mut self.active {
            run.download_jobs.insert(job_id);
        }
    }
    pub fn download_completed(
        &mut self,
        job_id: JobId,
        timing: &JobTiming,
        result: &Result<JobOutcome, FailureKind>,
        sent_at: Instant,
        now: Instant,
    ) {
        let Some(run) = &mut self.active else {
            return;
        };
        if !run.download_jobs.remove(&job_id) {
            return;
        }
        run.downloads += 1;
        run.last_download_completion = Some(
            run.last_download_completion
                .map_or(sent_at, |previous| previous.max(sent_at)),
        );
        match result {
            Ok(_) => run.successes += 1,
            Err(FailureKind::Cancelled) => run.cancelled += 1,
            Err(_) => run.failures += 1,
        }
        // The completion payload identifies actual execution, including header waits,
        // without adding a start event or altering existing progress notifications.
        if timing.fetch_attempts > 0 {
            let start = sent_at
                .checked_sub(timing.total.saturating_sub(timing.queue_wait))
                .unwrap_or(run.start);
            run.download_intervals.push((start, sent_at));
        }
        run.fetch.add(timing.fetch);
        run.extract.add(timing.extract);
        run.write.add(timing.article_write);
        run.marker.add(timing.marker_write);
        run.forwarding_delay = run
            .forwarding_delay
            .max(now.saturating_duration_since(sent_at));
    }
    pub fn llm_dispatched(&mut self, request_id: u64, prompt_id: PromptId, now: Instant) {
        let Some(run) = &mut self.active else {
            return;
        };
        if run.requests.contains_key(&request_id) {
            return;
        }
        run.requests.insert(request_id, (prompt_id, now));
        let stage = &mut run.model_stages[prompt_index(prompt_id)];
        stage.requests += 1;
        stage.first_dispatch.get_or_insert(now);
        stage.last_dispatch = Some(now);
    }
    pub fn llm_completed(&mut self, message: &Msg, now: Instant) {
        let Msg::LlmCompleted {
            request_id,
            result,
            metadata,
        } = message
        else {
            return;
        };
        let Some(run) = &mut self.active else {
            return;
        };
        let Some((prompt, dispatched)) = run.requests.remove(request_id) else {
            return;
        };
        let stage = &mut run.model_stages[prompt_index(prompt)];
        stage.completions += 1;
        stage.failures += usize::from(!matches!(result, LlmResultKind::Success { .. }));
        stage.last_completion = Some(now);
        stage.wall += now.saturating_duration_since(dispatched);
        run.rate_limited += usize::from(matches!(result, LlmResultKind::RateLimited { .. }));
        // Exhausted rate limits retain the original error contract without metadata.
        // The worker has spent its fixed rate-limit retry allowance in that case.
        run.rate_limit_retries += if matches!(result, LlmResultKind::RateLimited { .. }) {
            u64::from(harvester_engine::llm::handle::MAX_RATE_LIMIT_RETRIES)
        } else {
            metadata
                .as_ref()
                .map_or(0, |m| u64::from(m.rate_limit_retries))
        };
        if let Some(headroom) = metadata
            .as_ref()
            .and_then(|m| m.rate_limit_headroom.as_ref())
        {
            note_min(&mut run.remaining_requests, headroom.remaining_requests);
            note_min(&mut run.remaining_tokens, headroom.remaining_tokens);
        }
    }
}

fn note_min(minimum: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *minimum = Some(minimum.map_or(value, |current| current.min(value)));
    }
}
fn prompt_index(prompt: PromptId) -> usize {
    match prompt {
        PromptId::ArticleTriage => 0,
        PromptId::ArticleSummary => 1,
        PromptId::ArticleSignalCandidate => 2,
    }
}
fn optional(value: Option<u128>) -> String {
    value.map_or_else(|| "none".to_owned(), |v| v.to_string())
}

impl RunMeasurements {
    fn offset(&self, time: Option<Instant>) -> String {
        optional(time.map(|time| time.saturating_duration_since(self.start).as_millis()))
    }
    fn lines(&self, terminal: Instant) -> Vec<String> {
        let mut boundaries: Vec<_> = self
            .download_intervals
            .iter()
            .flat_map(|(start, end)| [(*start, 1_i32), (*end, -1)])
            .collect();
        boundaries.sort_unstable(); // An end at the same instant precedes a start.
        let mut in_flight = 0;
        let mut max_downloads = 0;
        for (_, change) in boundaries {
            in_flight += change;
            max_downloads = max_downloads.max(in_flight);
        }
        let mut summary = format!(
            "[run-timing] run_id={} host={} outcome={} wall_ms={} poll_start_ms={} poll_end_ms={} first_download_start_ms={} last_download_completion_ms={} downloads={} download_successes={} download_failures={} download_cancelled={} max_downloads_in_flight={} enqueue_hand_off_ms={} max_forwarding_delay_ms={} max_model_in_flight={} rate_limited={} min_remaining_requests={} min_remaining_tokens={} stop_ms={} drain_ms={}",
            self.id, self.host, if self.halted { "halted" } else if self.stop.is_some() { "stopped" } else { "completed" },
            terminal.saturating_duration_since(self.start).as_millis(), self.offset(self.poll_start), self.offset(self.poll_end),
            self.offset(self.download_intervals.iter().map(|(s, _)| *s).min()),
            self.offset(self.last_download_completion),
            self.downloads, self.successes, self.failures, self.cancelled, max_downloads,
            self.enqueue_hand_off.as_millis(), self.forwarding_delay.as_millis(), self.max_model_in_flight,
            self.rate_limited, optional(self.remaining_requests.map(u128::from)), optional(self.remaining_tokens.map(u128::from)),
            self.offset(self.stop), optional(self.stop.map(|s| terminal.saturating_duration_since(s).as_millis())),
        );
        summary.push_str(&format!(
            " min_effective_llm_cap={} llm_cap_changes={} rate_limit_retries={}",
            self.min_effective_llm_cap, self.cap_changes, self.rate_limit_retries
        ));
        for (name, step) in [
            ("fetch", &self.fetch),
            ("extract", &self.extract),
            ("write", &self.write),
            ("manifest", &self.marker),
        ] {
            summary.push_str(&format!(
                " {name}_sum_ms={} {name}_max_ms={}",
                step.sum.as_millis(),
                step.max.as_millis()
            ));
        }
        for (name, stage) in ["triage", "summary", "scoring"]
            .into_iter()
            .zip(&self.model_stages)
        {
            let average = if stage.completions == 0 {
                0
            } else {
                stage.wall.as_millis() / stage.completions as u128
            };
            summary.push_str(&format!(" {name}_requests={} {name}_first_dispatch_ms={} {name}_last_dispatch_ms={} {name}_last_completion_ms={} {name}_avg_wall_ms={} {name}_failures={}", stage.requests, self.offset(stage.first_dispatch), self.offset(stage.last_dispatch), self.offset(stage.last_completion), average, stage.failures));
        }
        for kind in ["JobDone", "FetchOutcomeClassified", "LlmCompleted"] {
            summary.push_str(&format!(
                " reducer_{kind}_ms={}",
                self.reducer
                    .get(kind)
                    .copied()
                    .unwrap_or_default()
                    .as_millis()
            ));
        }
        let mut lines = vec![summary];
        for stage in [
            PipelineStage::Triaging,
            PipelineStage::Summarizing,
            PipelineStage::ScoringSignals,
        ] {
            let releases = self
                .waves
                .iter()
                .filter(|(_, s, _, _)| *s == stage)
                .map(|(number, _, at, members)| {
                    format!("{number}:{}:{members}", self.offset(Some(*at)))
                })
                .collect::<Vec<_>>()
                .join(",");
            lines.push(format!("[run-timing-waves] run_id={} stage={stage:?} releases={releases} stop_ms={} drain_ms={}", self.id, self.offset(self.stop), optional(self.stop.map(|s| terminal.saturating_duration_since(s).as_millis()))));
        }
        lines
    }
}

#[cfg(test)]
mod tests;
