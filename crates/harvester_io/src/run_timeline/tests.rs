use super::*;
use harvester_engine::llm::LlmRunMetadata;

fn observation(id: u64, terminal: bool) -> Observation {
    Observation {
        run_id: id,
        stopped: false,
        halted: false,
        terminal,
        model_in_flight: 0,
        effective_llm_cap: 3,
        cap_changes: 0,
        last_cap_change: None,
        waves: Vec::new(),
    }
}
fn at(start: Instant, ms: u64) -> Instant {
    start + Duration::from_millis(ms)
}
fn observe(t: &mut RunTimeline, o: Observation, kind: &str, ms: u64, now: Instant) -> Vec<String> {
    t.observe(o, kind, Duration::from_millis(ms), now, "desktop")
}
fn has(line: &str, field: &str) {
    assert!(
        line.split_whitespace().any(|v| v == field),
        "missing {field} in {line}"
    );
}

#[test]
fn summary_fields_and_offsets_cover_io_and_reducer_boundaries() {
    let start = Instant::now();
    let mut t = RunTimeline::default();
    observe(
        &mut t,
        observation(7, false),
        "PipelineRunRequested",
        1,
        start,
    );
    t.poll_started(at(start, 10));
    t.poll_ended(at(start, 100));
    t.enqueue_handed_off(Duration::from_millis(8));
    let timing = JobTiming {
        queue_wait: Duration::from_millis(20),
        total: Duration::from_millis(120),
        fetch_attempts: 2,
        fetch: Duration::from_millis(60),
        extract: Duration::from_millis(15),
        article_write: Duration::from_millis(6),
        marker_write: Duration::from_millis(4),
        ..Default::default()
    };
    let outcome = Ok(JobOutcome {
        final_url: "https://example.com/".into(),
        tokens: None,
        bytes_written: None,
        extracted_links: vec![],
        fetched_utc: None,
    });
    t.download_queued(1);
    t.download_completed(1, &timing, &outcome, at(start, 150), at(start, 170));
    t.download_queued(2);
    t.download_completed(
        2,
        &JobTiming {
            fetch: Duration::from_millis(30),
            ..timing.clone()
        },
        &Err(FailureKind::Timeout),
        at(start, 220),
        at(start, 230),
    );
    t.llm_dispatched(1, PromptId::ArticleTriage, at(start, 180));
    let mut o = observation(7, false);
    o.model_in_flight = 3;
    observe(&mut t, o, "JobDone", 4, at(start, 200));
    let mut metadata = LlmRunMetadata::stub();
    // Deserialize the public additive metadata contract, without a kit dependency here.
    metadata.rate_limit_headroom = serde_json::from_str(r#"{"remaining_requests":9,"remaining_tokens":800,"limit_requests":null,"limit_tokens":null,"reset_requests":null,"reset_tokens":null}"#).unwrap();
    t.llm_completed(
        &Msg::LlmCompleted {
            request_id: 1,
            result: LlmResultKind::Success {
                output_json: "{}".into(),
                input_tokens: 1,
                output_tokens: 1,
                prompt_version: 1,
                resolved_model: "test".into(),
            },
            metadata: Some(metadata),
        },
        at(start, 280),
    );
    t.llm_dispatched(2, PromptId::ArticleSummary, at(start, 290));
    t.llm_completed(
        &Msg::LlmCompleted {
            request_id: 2,
            result: LlmResultKind::RateLimited {
                reason: "limited".into(),
            },
            metadata: None,
        },
        at(start, 310),
    );
    observe(
        &mut t,
        observation(7, false),
        "FetchOutcomeClassified",
        2,
        at(start, 310),
    );
    let lines = observe(
        &mut t,
        observation(7, true),
        "LlmCompleted",
        3,
        at(start, 400),
    );
    for field in [
        "run_id=7",
        "host=desktop",
        "outcome=completed",
        "wall_ms=400",
        "poll_start_ms=10",
        "poll_end_ms=100",
        "first_download_start_ms=50",
        "last_download_completion_ms=220",
        "downloads=2",
        "download_successes=1",
        "download_failures=1",
        "fetch_sum_ms=90",
        "fetch_max_ms=60",
        "extract_sum_ms=30",
        "write_sum_ms=12",
        "manifest_sum_ms=8",
        "max_downloads_in_flight=2",
        "max_model_in_flight=3",
        "enqueue_hand_off_ms=8",
        "max_forwarding_delay_ms=20",
        "triage_requests=1",
        "triage_first_dispatch_ms=180",
        "triage_last_completion_ms=280",
        "triage_avg_wall_ms=100",
        "summary_failures=1",
        "rate_limited=1",
        "min_remaining_requests=9",
        "min_remaining_tokens=800",
        "reducer_JobDone_ms=4",
        "reducer_FetchOutcomeClassified_ms=2",
        "reducer_LlmCompleted_ms=3",
    ] {
        has(&lines[0], field);
    }
}

#[test]
fn multiple_ledger_releases_in_one_observation_remain_individual() {
    let start = Instant::now();
    let mut t = RunTimeline::default();
    observe(&mut t, observation(1, false), "start", 0, start);
    let mut o = observation(1, false);
    o.waves = vec![
        (1, PipelineStage::Triaging, 12),
        (2, PipelineStage::Triaging, 5),
        (3, PipelineStage::Summarizing, 2),
    ];
    observe(&mut t, o, "load", 0, at(start, 50));
    let mut o = observation(1, true);
    o.waves = vec![
        (1, PipelineStage::Triaging, 12),
        (2, PipelineStage::Triaging, 5),
        (3, PipelineStage::Summarizing, 2),
    ];
    let lines = observe(&mut t, o, "end", 0, at(start, 100));
    has(&lines[1], "releases=1:50:12,2:50:5");
    has(&lines[2], "releases=3:50:2");
}

#[test]
fn stop_offset_and_drain_are_reported_in_summary_and_wave_lines() {
    let start = Instant::now();
    let mut t = RunTimeline::default();
    observe(&mut t, observation(2, false), "start", 0, start);
    let mut o = observation(2, false);
    o.stopped = true;
    observe(&mut t, o, "StopFinishClicked", 0, at(start, 40));
    let lines = observe(&mut t, observation(2, true), "JobDone", 0, at(start, 110));
    has(&lines[0], "outcome=stopped");
    for line in &lines {
        has(line, "stop_ms=40");
        has(line, "drain_ms=70");
    }
}

#[test]
fn stop_that_settles_in_one_step_reports_zero_drain() {
    let start = Instant::now();
    let mut timeline = RunTimeline::default();
    observe(&mut timeline, observation(1, false), "start", 0, start);
    let mut terminal = observation(1, true);
    terminal.stopped = true;
    let lines = observe(
        &mut timeline,
        terminal,
        "StopFinishClicked",
        0,
        at(start, 30),
    );
    for line in lines {
        has(&line, "stop_ms=30");
        has(&line, "drain_ms=0");
    }
}

#[test]
fn ignored_stop_click_does_not_change_outcome_or_start_drain() {
    use harvester_core::{update, PipelineRunScope};
    let start = Instant::now();
    let state = update(
        AppState::new(),
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Resume,
        },
    )
    .0;
    let (state, effects) = update(state, Msg::StopFinishClicked);
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, harvester_core::Effect::StopFinish { .. })));
    assert!(!state.run_stopped());
    let mut real_timeline = RunTimeline::default();
    assert!(real_timeline
        .observe_reducer(
            &state,
            "StopFinishClicked",
            Duration::ZERO,
            start,
            "desktop"
        )
        .is_empty());
    assert!(real_timeline.active.as_ref().unwrap().stop.is_none());
    let mut timeline = RunTimeline::default();
    observe(&mut timeline, observation(1, false), "start", 0, start);
    observe(
        &mut timeline,
        observation(1, false),
        "StopFinishClicked",
        0,
        at(start, 10),
    );
    let lines = observe(&mut timeline, observation(1, true), "end", 0, at(start, 20));
    has(&lines[0], "outcome=completed");
    has(&lines[0], "stop_ms=none");
    has(&lines[0], "drain_ms=none");
}

#[test]
fn stopped_settlement_is_read_from_real_state_after_phase_returns_idle() {
    use harvester_core::{update, AiAvailability, AiUnavailableReason, PipelineRunScope};
    use harvester_engine::{SourceId, SourceKind};
    let start = Instant::now();
    let state = update(
        AppState::new(),
        Msg::AiAvailabilityDetected {
            availability: AiAvailability::Unavailable {
                reason: AiUnavailableReason::MissingApiKey,
            },
        },
    )
    .0;
    let state = update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Full,
        },
    )
    .0;
    let state = update(state, Msg::PollStarted { total: 1 }).0;
    let state = update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("same-step-stop").unwrap(),
            urls: vec!["https://example.com/article".into()],
            kind: SourceKind::Curated,
            parsed: 1,
            dedup_filtered: 0,
        },
    )
    .0;
    let state = update(state, Msg::AllSourcesPollEnded).0;
    let mut timeline = RunTimeline::default();
    assert!(timeline
        .observe_reducer(&state, "UrlsSubmitted", Duration::ZERO, start, "desktop")
        .is_empty());
    let (state, effects) = update(state, Msg::StopFinishClicked);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, harvester_core::Effect::StopFinish { .. })));
    let state = update(
        state,
        Msg::JobDone {
            job_id: 1,
            result: harvester_core::JobResultKind::Failed {
                reason: "cancelled".into(),
            },
            extracted_links: vec![],
            fetched_utc: None,
        },
    )
    .0;
    assert!(state.run_progress().unwrap().is_terminal());
    assert_eq!(
        state.pipeline_run_phase(),
        harvester_core::PipelineRunPhase::Idle
    );
    assert!(state.run_stopped());
    let lines =
        timeline.observe_reducer(&state, "JobDone", Duration::ZERO, at(start, 30), "desktop");
    has(&lines[0], "outcome=stopped");
    has(&lines[0], "stop_ms=30");
    has(&lines[0], "drain_ms=0");
}

#[test]
fn provider_halt_reports_halted_even_when_followed_by_stop() {
    let start = Instant::now();
    let mut timeline = RunTimeline::default();
    observe(&mut timeline, observation(1, false), "start", 0, start);
    let mut halted = observation(1, false);
    halted.halted = true;
    observe(&mut timeline, halted, "LlmCompleted", 0, at(start, 10));
    let mut terminal = observation(1, true);
    terminal.stopped = true;
    let lines = observe(
        &mut timeline,
        terminal,
        "StopFinishClicked",
        0,
        at(start, 20),
    );
    has(&lines[0], "outcome=halted");
}

#[test]
fn duplicate_model_dispatch_preserves_original_prompt_and_instant() {
    let start = Instant::now();
    let mut timeline = RunTimeline::default();
    observe(&mut timeline, observation(1, false), "start", 0, start);
    timeline.llm_dispatched(1, PromptId::ArticleTriage, at(start, 10));
    timeline.llm_dispatched(1, PromptId::ArticleSummary, at(start, 25));
    timeline.llm_completed(
        &Msg::LlmCompleted {
            request_id: 1,
            result: LlmResultKind::RateLimited {
                reason: "limited".into(),
            },
            metadata: None,
        },
        at(start, 50),
    );
    let lines = observe(&mut timeline, observation(1, true), "end", 0, at(start, 60));
    has(&lines[0], "triage_requests=1");
    has(&lines[0], "triage_avg_wall_ms=40");
    has(&lines[0], "triage_failures=1");
    has(&lines[0], "summary_requests=0");
}

#[test]
fn terminal_is_logged_once_and_measurements_reset_between_runs() {
    let start = Instant::now();
    let mut t = RunTimeline::default();
    observe(&mut t, observation(1, false), "start", 0, start);
    t.poll_ended(at(start, 10));
    t.enqueue_handed_off(Duration::from_millis(8));
    observe(&mut t, observation(1, true), "end", 0, at(start, 20));
    assert!(observe(&mut t, observation(1, true), "tick", 0, at(start, 30)).is_empty());
    observe(&mut t, observation(2, false), "start", 0, at(start, 40));
    let lines = observe(&mut t, observation(2, true), "end", 0, at(start, 70));
    for field in [
        "run_id=2",
        "wall_ms=30",
        "poll_end_ms=none",
        "enqueue_hand_off_ms=0",
        "min_remaining_tokens=none",
    ] {
        has(&lines[0], field);
    }
}

#[test]
fn late_and_duplicate_download_completions_do_not_contaminate_runs() {
    let start = Instant::now();
    let mut timeline = RunTimeline::default();
    observe(&mut timeline, observation(1, false), "start", 0, start);
    timeline.download_queued(1);
    observe(
        &mut timeline,
        observation(1, true),
        "StopFinishClicked",
        0,
        at(start, 10),
    );
    observe(
        &mut timeline,
        observation(2, false),
        "start",
        0,
        at(start, 20),
    );
    timeline.download_queued(2);
    let timing = JobTiming {
        fetch_attempts: 1,
        fetch: Duration::from_millis(5),
        total: Duration::from_millis(5),
        ..Default::default()
    };
    timeline.download_completed(
        1,
        &timing,
        &Err(FailureKind::Cancelled),
        at(start, 30),
        at(start, 32),
    );
    timeline.download_completed(
        2,
        &timing,
        &Err(FailureKind::HttpStatus(404)),
        at(start, 40),
        at(start, 42),
    );
    timeline.download_completed(
        2,
        &timing,
        &Err(FailureKind::HttpStatus(404)),
        at(start, 40),
        at(start, 50),
    );
    let lines = observe(&mut timeline, observation(2, true), "end", 0, at(start, 60));
    for field in [
        "run_id=2",
        "downloads=1",
        "download_failures=1",
        "download_cancelled=0",
        "fetch_sum_ms=5",
        "last_download_completion_ms=20",
        "max_forwarding_delay_ms=2",
    ] {
        has(&lines[0], field);
    }
}

#[test]
fn run_without_downloads_reports_model_work() {
    let start = Instant::now();
    let mut t = RunTimeline::default();
    observe(&mut t, observation(1, false), "start", 0, start);
    t.llm_dispatched(1, PromptId::ArticleSignalCandidate, at(start, 10));
    t.llm_completed(
        &Msg::LlmCompleted {
            request_id: 1,
            result: LlmResultKind::Failed {
                reason: "failed".into(),
            },
            metadata: None,
        },
        at(start, 30),
    );
    let lines = observe(&mut t, observation(1, true), "end", 0, at(start, 40));
    for field in [
        "downloads=0",
        "first_download_start_ms=none",
        "scoring_requests=1",
        "scoring_failures=1",
        "scoring_avg_wall_ms=20",
    ] {
        has(&lines[0], field);
    }
}

#[test]
fn run_without_model_work_reports_cancelled_downloads() {
    let start = Instant::now();
    let mut t = RunTimeline::default();
    observe(&mut t, observation(1, false), "start", 0, start);
    t.download_queued(1);
    t.download_completed(
        1,
        &JobTiming::default(),
        &Err(FailureKind::Cancelled),
        at(start, 10),
        at(start, 12),
    );
    let lines = observe(&mut t, observation(1, true), "end", 0, at(start, 20));
    for field in [
        "downloads=1",
        "download_cancelled=1",
        "max_downloads_in_flight=0",
        "triage_requests=0",
        "triage_avg_wall_ms=0",
        "summary_requests=0",
        "scoring_requests=0",
    ] {
        has(&lines[0], field);
    }
}

#[test]
fn minimum_headroom_tracks_partial_reports_and_ignores_unowned_completions() {
    let start = Instant::now();
    let mut t = RunTimeline::default();
    observe(&mut t, observation(1, false), "start", 0, start);
    for (id, requests, tokens) in [
        (1, Some(8), Some(900)),
        (2, Some(3), None),
        (3, None, Some(600)),
    ] {
        t.llm_dispatched(id, PromptId::ArticleTriage, start);
        let mut metadata = LlmRunMetadata::stub();
        metadata.rate_limit_headroom = serde_json::from_value(serde_json::json!({"remaining_requests":requests,"remaining_tokens":tokens,"limit_requests":null,"limit_tokens":null,"reset_requests":null,"reset_tokens":null})).unwrap();
        let message = Msg::LlmCompleted {
            request_id: id,
            result: LlmResultKind::Failed {
                reason: "failure".into(),
            },
            metadata: Some(metadata),
        };
        t.llm_completed(&message, at(start, 10));
        t.llm_completed(&message, at(start, 20));
    }
    let lines = observe(&mut t, observation(1, true), "end", 0, at(start, 30));
    for field in [
        "min_remaining_requests=3",
        "min_remaining_tokens=600",
        "triage_requests=3",
        "triage_failures=3",
        "triage_avg_wall_ms=10",
    ] {
        has(&lines[0], field);
    }
}

#[test]
fn reducer_observation_reads_two_new_waves_from_the_real_ledger() {
    use harvester_core::{update, Effect, LoadedArticle, PipelineRunScope};
    let start = Instant::now();
    let mut state = AppState::new();
    state.set_llm_max_in_flight(1);
    let prompts = [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ];
    state = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: prompts.into_iter().map(|id| (id, 1)).collect(),
            effective_models: prompts
                .into_iter()
                .map(|id| (id, "test-model".into()))
                .collect(),
        },
    )
    .0;
    state = update(state, Msg::PromptTemplateFilesLoaded).0;
    state = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: Default::default(),
        },
    )
    .0;
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Resume,
        },
    );
    let mut t = RunTimeline::default();
    assert!(t
        .observe_reducer(
            &state,
            "PipelineRunRequested",
            Duration::ZERO,
            start,
            "batch"
        )
        .is_empty());
    let (state, effects) =
        harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000);
    let request_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    let articles = (0..7)
        .map(|index| LoadedArticle {
            url: format!("https://example.com/{index}"),
            source_title: Some(format!("Article {index}")),
            prepared_text: "Detailed report. ".repeat(40),
            content_hash: format!("hash-{index}"),
            fetched_utc: Some("2026-10-12T00:00:00Z".into()),
        })
        .collect();
    let (state, effects) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    );
    assert_eq!(
        state.released_run_waves().collect::<Vec<_>>(),
        vec![
            (1, PipelineStage::Triaging, 4),
            (2, PipelineStage::Triaging, 3)
        ]
    );
    assert!(t
        .observe_reducer(
            &state,
            "TriageArticlesLoaded",
            Duration::ZERO,
            at(start, 30),
            "batch"
        )
        .is_empty());
    let request_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RequestLlmCompletion { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    // Drive the provider halt through real reducer state as well as synthetic observations.
    let mut halted_state = state.clone();
    let mut halted_timeline = RunTimeline::default();
    halted_timeline.observe_reducer(&halted_state, "start", Duration::ZERO, start, "batch");
    let mut next_request = request_id;
    let mut halted_lines = Vec::new();
    for index in 0..3 {
        let (next, effects) = update(
            halted_state,
            Msg::LlmCompleted {
                request_id: next_request,
                result: LlmResultKind::RateLimited {
                    reason: "fixture provider limit".into(),
                },
                metadata: None,
            },
        );
        halted_state = next;
        halted_lines.extend(halted_timeline.observe_reducer(
            &halted_state,
            "LlmCompleted",
            Duration::ZERO,
            at(start, 40 + index * 10),
            "batch",
        ));
        if index < 2 {
            next_request = effects
                .iter()
                .find_map(|effect| match effect {
                    Effect::RequestLlmCompletion { request_id, .. } => Some(*request_id),
                    _ => None,
                })
                .expect("next request before provider threshold");
        }
    }
    assert!(halted_state.provider_dispatch_halted());
    for _ in 0..5 {
        if !halted_lines.is_empty() {
            break;
        }
        halted_state = update(halted_state, Msg::PipelineRunAdvance).0;
        halted_lines.extend(halted_timeline.observe_reducer(
            &halted_state,
            "PipelineRunAdvance",
            Duration::ZERO,
            at(start, 70),
            "batch",
        ));
    }
    assert!(!halted_lines.is_empty(), "provider halted run must settle");
    has(&halted_lines[0], "outcome=halted");
    let (state, _) = update(state, Msg::StopFinishClicked);
    assert!(t
        .observe_reducer(
            &state,
            "StopFinishClicked",
            Duration::ZERO,
            at(start, 40),
            "batch"
        )
        .is_empty());
    let (mut state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Failed {
                reason: "fixture failure".into(),
            },
            metadata: None,
        },
    );
    let mut lines = t.observe_reducer(
        &state,
        "LlmCompleted",
        Duration::ZERO,
        at(start, 60),
        "batch",
    );
    for _ in 0..5 {
        if !lines.is_empty() {
            break;
        }
        state = update(state, Msg::PipelineRunAdvance).0;
        lines = t.observe_reducer(
            &state,
            "PipelineRunAdvance",
            Duration::ZERO,
            at(start, 60),
            "batch",
        );
    }
    assert!(!lines.is_empty(), "stopped reducer run must settle");
    has(&lines[0], "host=batch");
    has(&lines[1], "releases=1:30:4,2:30:3");
}

#[test]
fn cap_changes_are_detected_once_and_summarized_through_terminal() {
    let start = Instant::now();
    let mut timeline = RunTimeline::default();
    observe(&mut timeline, observation(1, false), "start", 0, start);
    let mut reduced = observation(1, false);
    reduced.effective_llm_cap = 1;
    reduced.cap_changes = 1;
    reduced.last_cap_change = Some(LlmCapChange {
        from: 3,
        to: 1,
        reason: "rate_limited",
        request_id: 7,
    });
    let lines = observe(&mut timeline, reduced, "LlmCompleted", 0, at(start, 10));
    assert_eq!(
        lines,
        vec!["[llm-adaptive-cap] run_id=1 from=3 to=1 reason=rate_limited request_id=7"]
    );
    let mut unchanged = observation(1, false);
    unchanged.effective_llm_cap = 1;
    unchanged.cap_changes = 1;
    unchanged.last_cap_change = Some(LlmCapChange {
        from: 3,
        to: 1,
        reason: "rate_limited",
        request_id: 7,
    });
    assert!(observe(&mut timeline, unchanged, "Tick", 0, at(start, 20)).is_empty());
    let mut terminal = observation(1, true);
    terminal.effective_llm_cap = 2;
    terminal.cap_changes = 2;
    terminal.last_cap_change = Some(LlmCapChange {
        from: 1,
        to: 2,
        reason: "recovered",
        request_id: 20,
    });
    let lines = observe(&mut timeline, terminal, "LlmCompleted", 0, at(start, 30));
    assert_eq!(
        lines[0],
        "[llm-adaptive-cap] run_id=1 from=1 to=2 reason=recovered request_id=20"
    );
    has(&lines[1], "min_effective_llm_cap=1");
    has(&lines[1], "llm_cap_changes=2");
    has(&lines[1], "rate_limit_retries=0");
    assert!(observe(
        &mut timeline,
        observation(1, true),
        "Tick",
        0,
        at(start, 40)
    )
    .is_empty());
    let lines = observe(
        &mut timeline,
        observation(2, true),
        "start",
        0,
        at(start, 50),
    );
    has(&lines[0], "min_effective_llm_cap=3");
    has(&lines[0], "llm_cap_changes=0");
}

#[test]
fn timing_summary_counts_retries_only_for_owned_requests() {
    let start = Instant::now();
    let mut timeline = RunTimeline::default();
    observe(&mut timeline, observation(1, false), "start", 0, start);
    timeline.llm_dispatched(7, PromptId::ArticleTriage, at(start, 5));
    let mut metadata = LlmRunMetadata::stub();
    metadata.rate_limit_retries = 2;
    let mut message = Msg::LlmCompleted {
        request_id: 999,
        result: LlmResultKind::Success {
            output_json: "{}".into(),
            input_tokens: 0,
            output_tokens: 0,
            prompt_version: 1,
            resolved_model: "model".into(),
        },
        metadata: Some(metadata),
    };
    timeline.llm_completed(&message, at(start, 10));
    if let Msg::LlmCompleted { request_id, .. } = &mut message {
        *request_id = 7;
    }
    timeline.llm_completed(&message, at(start, 15));
    timeline.llm_completed(&message, at(start, 20));
    timeline.llm_dispatched(8, PromptId::ArticleTriage, at(start, 20));
    timeline.llm_completed(
        &Msg::LlmCompleted {
            request_id: 8,
            result: LlmResultKind::RateLimited {
                reason: "exhausted".into(),
            },
            metadata: None,
        },
        at(start, 21),
    );
    let lines = observe(
        &mut timeline,
        observation(1, true),
        "finish",
        0,
        at(start, 25),
    );
    has(&lines[0], "rate_limit_retries=5");
    has(&lines[0], "rate_limited=1");
}

#[test]
fn timing_summary_counts_retries_on_failed_provider_completion() {
    let start = Instant::now();
    let mut timeline = RunTimeline::default();
    observe(&mut timeline, observation(1, false), "start", 0, start);
    timeline.llm_dispatched(7, PromptId::ArticleTriage, at(start, 5));
    let message =
        super::super::effect_helpers::map_llm_event(harvester_engine::llm::LlmEvent::Completed {
            request_id: 7,
            result: Err(harvester_engine::llm::LlmCompletionError::ProviderError {
                error: harvester_engine::llm::LlmError::Timeout,
                failure_metadata: Some(harvester_engine::llm::LlmFailureMetadata {
                    rate_limit_retries: 1,
                    prompt_id: PromptId::ArticleTriage,
                    prompt_version: 1,
                    resolved_model: Some("model".into()),
                    input_bytes: 100,
                    wall_ms: Some(50),
                    timestamp_utc: "2026-10-12T00:00:00Z".into(),
                }),
            }),
        });
    timeline.llm_completed(&message, at(start, 15));
    let lines = observe(
        &mut timeline,
        observation(1, true),
        "finish",
        0,
        at(start, 25),
    );
    has(&lines[0], "rate_limit_retries=1");
    has(&lines[0], "rate_limited=0");
}

#[test]
fn reducer_observer_reads_real_adaptive_cap_changes() {
    use harvester_core::{update, Effect, LoadedArticle, PipelineRunScope};
    let start = Instant::now();
    let mut state = AppState::new();
    state.set_llm_max_in_flight(3);
    let prompts = [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ];
    state = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: prompts.into_iter().map(|id| (id, 1)).collect(),
            effective_models: prompts.into_iter().map(|id| (id, "model".into())).collect(),
        },
    )
    .0;
    state = update(state, Msg::PromptTemplateFilesLoaded).0;
    state = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: Default::default(),
        },
    )
    .0;
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Resume,
        },
    );
    let mut timeline = RunTimeline::default();
    assert!(timeline
        .observe_reducer(&state, "start", Duration::ZERO, start, "batch")
        .is_empty());
    let (state, effects) =
        harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000);
    let request_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    let articles = vec![LoadedArticle {
        url: "https://example.com/one".into(),
        source_title: None,
        prepared_text: "Detailed report. ".repeat(40),
        content_hash: "hash".into(),
        fetched_utc: None,
    }];
    let (state, effects) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    );
    let id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RequestLlmCompletion { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: id,
            result: LlmResultKind::RateLimited {
                reason: "exhausted".into(),
            },
            metadata: None,
        },
    );
    let lines = timeline.observe_reducer(
        &state,
        "LlmCompleted",
        Duration::ZERO,
        at(start, 10),
        "batch",
    );
    assert!(lines.iter().any(|line| line
        == &format!(
            "[llm-adaptive-cap] run_id=1 from=3 to=1 reason=rate_limited request_id={id}"
        )));
    assert_eq!(state.effective_llm_cap(), 1);
    assert_eq!(state.llm_cap_change_count(), 1);
    assert!(timeline
        .observe_reducer(&state, "Tick", Duration::ZERO, at(start, 20), "batch")
        .is_empty());
}
