use super::*;
use crate::PipelineRunScope;

fn start_resume(state: AppState, articles: Vec<LoadedArticle>) -> (AppState, Vec<Effect>) {
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Resume,
        },
    );
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::LoadProcessingConfiguration { .. }))
            .count(),
        1
    );
    assert!(request_id(&effects, PromptId::ArticleTriage).is_none());
    let (state, effects) =
        crate::fixture_support::complete_processing_configuration(state, effects, 100_000);
    let id = effects
        .iter()
        .find_map(|e| {
            if let Effect::LoadArticlesForTriage { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        })
        .expect("fresh incremental load");
    assert!(!state.run_progress().unwrap().terminal);
    crate::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    )
}

#[test]
fn in_flight_triage_member_blocks_wave_summaries_until_last_completion() {
    let mut state = add_metadata(AppState::new());
    state.set_llm_max_in_flight(3);
    let (state, effects) = start_resume(state, vec![loaded_article(0), loaded_article(1)]);
    let ids: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: PromptId::ArticleTriage,
                ..
            } => Some(*request_id),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(state.pipeline_waves().waves()[0].members.len(), 2);
    let (state, effects) = crate::update(state, triage_success(ids[0]));
    assert!(request_id(&effects, PromptId::ArticleSummary).is_none());
    assert_eq!(state.triage().in_progress_count(), 1);
    let (state, effects) = crate::update(state, triage_success(ids[1]));
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(
                effect,
                Effect::RequestLlmCompletion {
                    prompt_id: PromptId::ArticleSummary,
                    ..
                }
            ))
            .count(),
        2
    );
    assert_eq!(state.briefing().in_progress_count(), 2);
}

#[test]
fn thirty_article_resume_splits_in_window_order_and_loads_configuration_once() {
    let mut state = add_metadata(AppState::new());
    state.set_llm_max_in_flight(3);
    let articles: Vec<_> = (0..30).map(loaded_article).collect();
    let (state, effects) = start_resume(state, articles.clone());
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::RequestLlmCompletion { .. }))
            .count(),
        3
    );
    let waves: Vec<_> = state
        .pipeline_waves()
        .waves()
        .iter()
        .filter(|w| w.stage == PipelineStage::Triaging)
        .collect();
    assert_eq!(
        waves.iter().map(|w| w.members.len()).collect::<Vec<_>>(),
        [12, 12, 6]
    );
    assert_eq!(
        waves
            .iter()
            .flat_map(|w| w.members.iter().map(|m| m.0.clone()))
            .collect::<Vec<_>>(),
        articles.iter().map(|a| a.url.clone()).collect::<Vec<_>>()
    );
    assert_eq!(state.triage().pending_count(), 27);
    let (state, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Resume,
        },
    );
    assert!(effects.is_empty());
    let (state, effects) = crate::update(
        state,
        Msg::PromptContextsLoaded {
            contexts: HashMap::from([(
                PromptId::ArticleTriage,
                vec![("policy".into(), "changed mid-run".into())],
            )]),
        },
    );
    assert!(effects.is_empty());
    assert!(state.context_for(PromptId::ArticleTriage).is_empty());
    let stage = &state.run_progress().unwrap().stages[PipelineStage::Triaging.index()];
    assert_eq!(
        (stage.total, stage.total_is_final, stage.status),
        (30, true, StageStatus::Active)
    );
    let state = run_to_completion(state, &articles, &[true; 30]);
    assert_eq!(state.briefing().completed_summary_count(), 30);
    assert_eq!(state.signal_candidate().observation_counts().completed, 30);
    assert_eq!(
        state.run_progress().unwrap().activity.len(),
        ACTIVITY_FEED_CAPACITY
    );
    assert!(state
        .run_progress()
        .unwrap()
        .stages
        .iter()
        .all(|s| s.total_is_final && matches!(s.status, StageStatus::Done | StageStatus::Failed)));
}

#[test]
fn dispatches_triage_in_wave_then_member_order_with_one_slot() {
    let articles: Vec<_> = (0..8).map(loaded_article).collect();
    let mut state = add_metadata(AppState::new());
    state.set_llm_max_in_flight(1);
    let (mut state, mut effects) = start_resume(state, articles.clone());
    let triage_waves: Vec<_> = state
        .pipeline_waves()
        .waves()
        .iter()
        .filter(|w| w.stage == PipelineStage::Triaging)
        .map(|w| w.members.len())
        .collect();
    assert_eq!(triage_waves, [4, 4]);
    for article in articles {
        let id = request_id(&effects, PromptId::ArticleTriage).expect("next member dispatched");
        let index = state.triage().find_article_by_request_id(id).unwrap();
        assert_eq!(state.triage().articles()[index].url, article.url);
        (state, effects) = crate::update(
            state,
            Msg::LlmCompleted {
                request_id: id,
                result: LlmResultKind::Failed {
                    reason: "order probe".into(),
                },
                metadata: None,
            },
        );
    }
    assert!(request_id(&effects, PromptId::ArticleTriage).is_none());
}

#[test]
fn failed_triage_is_requested_in_the_next_run_without_an_advance_message() {
    let article = loaded_article(0);
    let (state, effects) = start_resume(add_metadata(AppState::new()), vec![article.clone()]);
    let id = request_id(&effects, PromptId::ArticleTriage).unwrap();
    let (state, effects) = crate::update(
        state,
        Msg::LlmCompleted {
            request_id: id,
            result: LlmResultKind::Failed {
                reason: "transient".into(),
            },
            metadata: None,
        },
    );
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    assert!(state.run_progress().unwrap().terminal);
    let previous_id = state.run_progress().unwrap().run_id;
    let notice = state.run_completion_notice().cloned();
    let (state, _) = crate::update(state, Msg::PipelineRunAdvance);
    assert_eq!(state.run_completion_notice(), notice.as_ref());
    let (state, effects) = start_resume(state, vec![article]);
    assert_ne!(state.run_progress().unwrap().run_id, previous_id);
    assert!(request_id(&effects, PromptId::ArticleTriage).is_some());
    assert_eq!(state.triage().total(), 1);
    assert_eq!(state.triage().failed_count(), 0);
}

#[test]
fn stale_summary_requeues_while_current_triage_and_other_results_survive() {
    let articles = vec![loaded_article(0), loaded_article(1)];
    let mut initial = add_metadata(AppState::new());
    initial.set_llm_max_in_flight(3);
    let (state, _) = start_resume(initial, articles.clone());
    let mut state = run_to_completion(state, &articles, &[true, true]);
    let original_triage = state.triage().articles().to_vec();
    // The cache for article 1 is already current under the next summary prompt.
    let result = state.briefing().articles()[1].summary_state.clone();
    let crate::briefing::ArticleSummaryState::Completed { result } = result else {
        panic!("summary");
    };
    let current_key = SummaryCacheKey::try_new(
        &articles[1].content_hash,
        PromptId::ArticleSummary,
        Some(2),
        Some("test-summary-model"),
        &[],
    )
    .unwrap();
    state.store_summary_result(current_key, result, "now".into());
    let mut versions = HashMap::new();
    let mut models = HashMap::new();
    for id in [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ] {
        versions.insert(id, if id == PromptId::ArticleSummary { 2 } else { 1 });
        models.insert(id, state.effective_model_for(id).unwrap().to_string());
    }
    let (state, _) = crate::update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: versions,
            effective_models: models,
        },
    );
    let (state, effects) = start_resume(state, articles.clone());
    assert_eq!(state.triage().articles(), original_triage);
    assert!(request_id(&effects, PromptId::ArticleTriage).is_none());
    assert_eq!(
        state.briefing().articles()[1]
            .cache_key_snapshot
            .as_ref()
            .unwrap()
            .prompt_version,
        2
    );
    assert!(matches!(
        state.briefing().articles()[1].summary_state,
        crate::briefing::ArticleSummaryState::Completed { .. }
    ));
    assert!(
        state.summary_cache().len() >= 3,
        "old results stay in the cache"
    );
    assert_eq!(
        state.briefing().articles()[0]
            .cache_key_snapshot
            .as_ref()
            .unwrap()
            .prompt_version,
        2
    );
    assert!(matches!(
        state.briefing().articles()[0].summary_state,
        crate::briefing::ArticleSummaryState::Pending
            | crate::briefing::ArticleSummaryState::InProgress { .. }
    ));
}

#[test]
fn stop_withdraws_pending_triage_and_keeps_the_in_flight_result() {
    let articles: Vec<_> = (0..3).map(loaded_article).collect();
    let mut state = add_metadata(AppState::new());
    state.set_llm_max_in_flight(1);
    let (mut state, effects) = start_resume(state, articles);
    let id = request_id(&effects, PromptId::ArticleTriage).unwrap();
    assert_eq!(state.triage().pending_count(), 2);
    state.start_session();
    let (state, effects) = crate::update(state, Msg::StopFinishClicked);
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::StopFinish { .. })));
    assert_eq!(state.triage().pending_count(), 0);
    assert_eq!(state.triage().in_progress_count(), 1);
    assert_eq!(
        state.pipeline_run_phase(),
        crate::PipelineRunPhase::Stopping
    );
    assert_eq!(
        state.run_state(),
        crate::RunState::Stopping { in_flight: 1 }
    );
    assert!(!state.run_progress().unwrap().terminal);
    assert!(!state.view().archive_enabled);
    let (state, effects) = crate::update(state, triage_success(id));
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    assert!(
        state.pipeline_activity().is_settled(),
        "{:?} {:?} pending={} in_flight={}",
        state.pipeline_activity(),
        state.triage().phase(),
        state.triage().pending_count(),
        state.triage().in_progress_count()
    );
    assert_eq!(state.triage().phase(), &crate::TriagePhase::Complete);
    assert!(state.run_progress().unwrap().terminal);
    assert_eq!(state.pipeline_run_phase(), crate::PipelineRunPhase::Idle);
    assert_eq!(
        state.batch_observation().session_state,
        crate::SessionState::Idle
    );
    assert_eq!(state.run_state(), crate::RunState::Idle);
    assert!(state.view().archive_enabled);
    assert_eq!(state.triage_cache().len(), 1);
    assert!(state.triage().can_start());
    assert!(
        matches!(state.unfinished_work(), crate::UnfinishedWork::Known(work)
        if work.needs_triage == 2)
    );
    let (_, effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Resume,
        },
    );
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::LoadProcessingConfiguration { .. })));
}

#[test]
fn stop_drains_overlapping_triage_and_summary_without_downstream_dispatch() {
    let first = loaded_article(0);
    let mut state = add_metadata(AppState::new());
    state.set_llm_max_in_flight(2);
    let (state, effects) = start_resume(state, vec![first.clone()]);
    let triage_id = request_id(&effects, PromptId::ArticleTriage).expect("first triage");
    let (mut state, effects) = crate::update(state, triage_success(triage_id));
    let summary_id = request_id(&effects, PromptId::ArticleSummary).expect("summary starts");

    let later: Vec<_> = (1..=5).map(loaded_article).collect();
    let all_articles = std::iter::once(first)
        .chain(later.iter().cloned())
        .collect::<Vec<_>>();
    let load_id = state.pre_triage_coordinator.begin_preparation_load();
    state.set_triage_in_flight(load_id);
    let (state, effects) = crate::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_id,
            delta: harvester_engine::TriageArticleDelta::full_window(all_articles, 100_000),
        },
    );
    let triage_drain_id = request_id(&effects, PromptId::ArticleTriage)
        .expect("a later triage wave overlaps the summary request");
    assert_eq!(state.triage().pending_count(), 4);
    assert_eq!(state.article_model_requests_in_flight(), 2);

    let (state, effects) = crate::update(state, Msg::StopFinishClicked);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::StopFinish { .. })));
    assert_eq!(state.triage().pending_count(), 0);
    assert_eq!(state.triage().failed_count(), 0);
    assert_eq!(
        state.pipeline_run_phase(),
        crate::PipelineRunPhase::Stopping
    );
    assert_eq!(
        state.run_state(),
        crate::RunState::Stopping { in_flight: 2 }
    );
    assert!(!state.run_progress().unwrap().terminal);

    let (state, effects) = crate::update(state, summary_result(summary_id, true));
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::RequestLlmCompletion { .. })));
    assert_eq!(state.summary_cache().len(), 1);
    assert_eq!(
        state.pipeline_run_phase(),
        crate::PipelineRunPhase::Stopping
    );
    assert_eq!(
        state.run_state(),
        crate::RunState::Stopping { in_flight: 1 }
    );
    assert!(!state.run_progress().unwrap().terminal);

    let (state, effects) = crate::update(state, triage_success(triage_drain_id));
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::RequestLlmCompletion { .. })));
    assert!(state.run_progress().unwrap().terminal);
    assert_eq!(state.pipeline_run_phase(), crate::PipelineRunPhase::Idle);
    assert_eq!(
        state.batch_observation().session_state,
        crate::SessionState::Idle
    );
    assert_eq!(state.run_state(), crate::RunState::Idle);
    assert!(state.view().archive_enabled);
    assert_eq!(state.triage_cache().len(), 2);
    assert!(
        matches!(state.unfinished_work(), crate::UnfinishedWork::Known(work)
        if work.needs_triage == 4 && work.needs_summary == 1)
    );
}

#[test]
fn hydration_with_eligible_scoring_is_settled_and_unadmitted() {
    let article = loaded_article(0);
    let mut state = add_metadata(AppState::new());
    state.store_triage_result(
        &article.content_hash,
        crate::ArticleTriageResult {
            category: "news".into(),
            priority: 3,
            tags: vec![],
            rationale: "ok".into(),
            input_tokens: 0,
            output_tokens: 0,
        },
    );
    seed_cached_summary(&mut state, &article);
    (state, _) = crate::update(state, Msg::PipelineRunAdvance); // Persist the fixture seeds before hydration.
    let id = state.alloc_triage_request_id();
    state.set_triage_in_flight(id);
    let (state, effects) = crate::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: id,
            delta: harvester_engine::TriageArticleDelta::full_window(vec![article], 100_000),
        },
    );
    assert!(effects.is_empty());
    assert!(state.pipeline_activity().is_settled());
    assert_eq!(state.signal_candidate().observation_counts().total, 0);
    assert!(
        matches!(state.unfinished_work(), crate::UnfinishedWork::Known(work) if work.needs_scoring == 1)
    );
}

#[test]
fn run_requested_during_continuous_downloads_starts_triage_before_downloads_end() {
    let mut state = add_metadata(AppState::new());
    state.set_llm_max_in_flight(3);
    let (state, configuration_effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    let (state, _) = crate::update(state, Msg::PollStarted { total: 1 });
    let articles: Vec<_> = (0..6).map(loaded_article).collect();
    let (mut state, effects) = crate::update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("continuous").unwrap(),
            urls: articles.iter().map(|a| a.url.clone()).collect(),
            kind: SourceKind::Rss,
            parsed: 6,
            dedup_filtered: 0,
        },
    );
    let jobs: Vec<_> = effects
        .iter()
        .filter_map(|e| {
            if let Effect::EnqueueUrl { job_id, .. } = e {
                Some(*job_id)
            } else {
                None
            }
        })
        .collect();
    // Each finished download records fresh refresh demand, so demand is never idle
    // while downloads keep arriving.
    let finish_download = |state: AppState, index: usize| {
        let (state, _) = crate::update(
            state,
            Msg::JobDone {
                job_id: jobs[index],
                result: JobResultKind::Success,
                extracted_links: vec![],
                fetched_utc: articles[index].fetched_utc.clone(),
            },
        );
        crate::update(
            state,
            Msg::EvaluatePreTriageRefresh {
                ordered_urls: articles[..=index].iter().map(|a| a.url.clone()).collect(),
                triggered_by_job_done: true,
            },
        )
        .0
    };
    state = finish_download(state, 0);
    let (next, _) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Resume,
        },
    );
    let (next, mut effects) = crate::fixture_support::complete_processing_configuration(
        next,
        configuration_effects,
        100_000,
    );
    state = next;
    let mut triage_requested = request_id(&effects, PromptId::ArticleTriage).is_some();
    let mut tick = 0;
    state = finish_download(state, 1);
    for index in 1..articles.len() - 1 {
        let mut load = effects.iter().find_map(|e| {
            if let Effect::LoadArticlesForTriage { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        });
        while load.is_none() && tick < 200 {
            tick += 1;
            let (next, tick_effects) = crate::update(
                state,
                Msg::tick_at(DateTime::from_timestamp(BASE_TIME + tick, 0).unwrap()),
            );
            state = next;
            load = tick_effects.iter().find_map(|e| {
                if let Effect::LoadArticlesForTriage { request_id, .. } = e {
                    Some(*request_id)
                } else {
                    None
                }
            });
        }
        let id = load.expect("the refresh coordinator dispatches a load while downloads arrive");
        // The next download lands while this load is in flight.
        state = finish_download(state, index + 1);
        let (next, load_effects) = crate::update(
            state,
            Msg::TriageArticlesLoaded {
                request_id: id,
                delta: harvester_engine::TriageArticleDelta::full_window(
                    articles[..=index].to_vec(),
                    100_000,
                ),
            },
        );
        state = next;
        triage_requested |= request_id(&load_effects, PromptId::ArticleTriage).is_some();
        effects = load_effects;
    }
    assert!(
        triage_requested,
        "a run requested mid-download must start triage while downloads are still arriving"
    );
    assert!(state.is_poll_in_progress());
    // Downloads that land after the start keep joining as later triage waves.
    assert!(
        state
            .pipeline_waves()
            .waves()
            .iter()
            .filter(|w| w.stage == PipelineStage::Triaging)
            .count()
            >= 2
    );
    assert!(state.triage().total() >= 4);
}

#[test]
fn three_download_bursts_release_overlapping_waves_and_monotonic_totals() {
    let mut state = add_metadata(AppState::new());
    state.set_llm_max_in_flight(3);
    let (state, configuration_effects) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    let (state, _) = crate::update(state, Msg::PollStarted { total: 1 });
    let articles: Vec<_> = (0..3).map(loaded_article).collect();
    let (state, effects) = crate::update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("waves").unwrap(),
            urls: articles.iter().map(|a| a.url.clone()).collect(),
            kind: SourceKind::Rss,
            parsed: 3,
            dedup_filtered: 0,
        },
    );
    let jobs: Vec<_> = effects
        .iter()
        .filter_map(|e| {
            if let Effect::EnqueueUrl { job_id, .. } = e {
                Some(*job_id)
            } else {
                None
            }
        })
        .collect();
    let (state, _) = crate::update(state, Msg::AllSourcesPollEnded);
    let (state, _) = crate::update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Full,
        },
    );
    let (mut state, _effects) = crate::fixture_support::complete_processing_configuration(
        state,
        configuration_effects,
        100_000,
    );
    let mut progress = progress_snapshot(&state);
    for burst in 0..3 {
        let (next, _) = crate::update(
            state,
            Msg::JobDone {
                job_id: jobs[burst],
                result: JobResultKind::Success,
                extracted_links: vec![],
                fetched_utc: articles[burst].fetched_utc.clone(),
            },
        );
        state = next;
        assert!(!state.run_progress().unwrap().terminal);
        let (next, _) = crate::update(
            state,
            Msg::EvaluatePreTriageRefresh {
                ordered_urls: articles[..=burst].iter().map(|a| a.url.clone()).collect(),
                triggered_by_job_done: true,
            },
        );
        state = next;
        assert!(state.pipeline_activity().intake_refresh_pending);
        let mut load = None;
        if burst == 0 {
            let (next, effects) = crate::update(state, Msg::PipelineRunAdvance);
            state = next;
            load = effects.iter().find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            });
        } else {
            for tick_index in 1..=crate::pre_triage_coordinator::QUIET_TICKS_AFTER_POLL {
                let (next, effects) = crate::update(
                    state,
                    Msg::tick_at(
                        DateTime::from_timestamp(
                            BASE_TIME + 100 * burst as i64 + tick_index as i64,
                            0,
                        )
                        .unwrap(),
                    ),
                );
                let (next, advance_effects) = crate::update(next, Msg::PipelineRunAdvance);
                state = next;
                load = effects
                    .iter()
                    .chain(&advance_effects)
                    .find_map(|effect| match effect {
                        Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                        _ => None,
                    });
                assert!(effects
                    .iter()
                    .chain(&advance_effects)
                    .all(|e| !matches!(e, Effect::LoadProcessingConfiguration { .. })));
                if load.is_some() {
                    break;
                }
            }
        }
        let id = load.expect("quiet window releases a load while downloads remain");
        assert!(!state.run_progress().unwrap().terminal);
        let (next, effects) = crate::update(
            state,
            Msg::TriageArticlesLoaded {
                request_id: id,
                delta: harvester_engine::TriageArticleDelta::full_window(
                    articles[..=burst].to_vec(),
                    100_000,
                ),
            },
        );
        state = next;
        let triage = request_id(&effects, PromptId::ArticleTriage).unwrap();
        assert_eq!(
            state
                .pipeline_waves()
                .waves()
                .iter()
                .filter(|w| w.stage == PipelineStage::Triaging)
                .count(),
            burst + 1
        );
        let stage = &state.run_progress().unwrap().stages[PipelineStage::Triaging.index()];
        assert_eq!(stage.total, (burst + 1) as u32);
        assert_eq!(stage.total_is_final, burst == 2);
        assert_progress_does_not_regress(&mut progress, &state);
        let (next, effects) = crate::update(state, triage_success(triage));
        state = next;
        let summary = request_id(&effects, PromptId::ArticleSummary)
            .expect("summary released in the same step");
        if burst == 0 {
            assert_eq!(state.batch_observation().jobs_in_flight, 2);
            assert_eq!(
                state.triage().total(),
                1,
                "wave 1 summary dispatches before wave 3 admission"
            );
            assert_eq!(
                state.run_progress().unwrap().stages[PipelineStage::Triaging.index()].status,
                StageStatus::Active
            );
        }
        let (next, effects) = crate::update(state, summary_result(summary, true));
        state = next;
        let score = request_id(&effects, PromptId::ArticleSignalCandidate).unwrap();
        let (next, _) = crate::update(state, signal_success(score));
        state = next;
        assert_progress_does_not_regress(&mut progress, &state);
        assert_eq!(state.run_progress().unwrap().terminal, burst == 2);
    }
    let notice = state.run_completion_notice().cloned().unwrap();
    let (state, _) = crate::update(
        state,
        Msg::tick_at(DateTime::from_timestamp(BASE_TIME + 999, 0).unwrap()),
    );
    assert_eq!(state.run_completion_notice(), Some(&notice));
    assert_eq!(notice.new_result_count, 3);
    assert!(state
        .run_progress()
        .unwrap()
        .stages
        .iter()
        .all(|s| s.total_is_final && s.status == StageStatus::Done));
}

#[test]
fn changed_prompt_keys_requeue_only_articles_without_current_results() {
    let articles = vec![loaded_article(0), loaded_article(1)];
    let mut initial = add_metadata(AppState::new());
    initial.set_llm_max_in_flight(3);
    let (state, _) = start_resume(initial, articles.clone());
    let state = run_to_completion(state, &articles, &[true, true]);
    let crate::triage::ArticleTriageState::Completed { result: triage } =
        state.triage().articles()[1].triage_state.clone()
    else {
        panic!("triage result")
    };
    let crate::briefing::ArticleSummaryState::Completed { result: summary } =
        state.briefing().articles()[1].summary_state.clone()
    else {
        panic!("summary result")
    };
    let models = [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
    ]
    .into_iter()
    .map(|id| (id, state.effective_model_for(id).unwrap().to_owned()))
    .collect();
    let (mut state, _) = crate::update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: HashMap::from([
                (PromptId::ArticleTriage, 2),
                (PromptId::ArticleSummary, 2),
                (PromptId::ArticleSignalCandidate, 1),
            ]),
            effective_models: models,
        },
    );
    state.store_triage_result(&articles[1].content_hash, triage);
    let key = state
        .current_summary_cache_key(&articles[1].content_hash)
        .unwrap();
    state.store_summary_result(key, summary, "now".into());
    let (state, effects) = start_resume(state, articles.clone());
    let requests: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                ..
            } if *prompt_id != PromptId::ArticleSignalCandidate => Some((*request_id, *prompt_id)),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].1, PromptId::ArticleTriage);
    assert!(matches!(
        state.triage().articles()[1].triage_state,
        crate::triage::ArticleTriageState::Completed { .. }
    ));
    let (state, effects) = crate::update(state, triage_success(requests[0].0));
    assert!(request_id(&effects, PromptId::ArticleSummary).is_some());
    assert!(request_id(&effects, PromptId::ArticleTriage).is_none());
    assert!(matches!(
        state.briefing().articles()[1].summary_state,
        crate::briefing::ArticleSummaryState::Completed { .. }
    ));
    assert!(matches!(
        state.briefing().articles()[0].summary_state,
        crate::briefing::ArticleSummaryState::InProgress { .. }
    ));
    assert!(state.triage_cache().len() >= 4);
    assert!(state.summary_cache().len() >= 3);
}
