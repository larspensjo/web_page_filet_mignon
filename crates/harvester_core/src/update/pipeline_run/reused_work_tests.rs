use super::*;
use crate::triage::ArticleTriageResult;
use crate::UnfinishedStageVerdict as Verdict;

fn cached_triage() -> ArticleTriageResult {
    ArticleTriageResult {
        category: "news".into(),
        priority: 3,
        tags: vec!["tag".into()],
        rationale: "ok".into(),
        input_tokens: 10,
        output_tokens: 5,
    }
}

fn seed(state: &mut AppState, article: &LoadedArticle, triage: bool, summary: bool, scoring: bool) {
    let result = cached_triage();
    if triage {
        state.store_triage_result(&article.content_hash, result.clone());
    }
    if summary {
        seed_cached_summary(state, article);
    }
    if scoring {
        let summary_key = state
            .current_summary_cache_key(&article.content_hash)
            .unwrap();
        let summary = state.try_reuse_summary(&summary_key).unwrap();
        let key = crate::update::signal_candidate::input_key_for_current_results(
            state,
            article,
            &result,
            &summary_key,
            summary,
        )
        .unwrap();
        let Msg::LlmCompleted {
            result: LlmResultKind::Success { output_json, .. },
            ..
        } = signal_success(0)
        else {
            unreachable!()
        };
        let result =
            harvester_engine::llm::validation::validate_signal_candidate(&output_json).unwrap();
        state.store_signal_candidate_result(key, result, "2026-09-07T12:00:00Z".into());
    }
}

fn start(
    state: AppState,
    articles: Vec<LoadedArticle>,
    scope: crate::PipelineRunScope,
) -> (AppState, Vec<Effect>) {
    let state = if scope == crate::PipelineRunScope::Full {
        crate::update(
            state,
            Msg::RestoreCompletedJobs(
                articles
                    .iter()
                    .map(|article| crate::CompletedJobSnapshot {
                        url: article.url.clone(),
                        tokens: Some(1000),
                        bytes: Some(1024),
                        links: Vec::new(),
                        fetched_utc: article.fetched_utc.clone(),
                    })
                    .collect(),
            ),
        )
        .0
    } else {
        state
    };
    let (state, effects) = crate::update(state, Msg::PipelineRunRequested { scope });
    let (state, effects) =
        crate::fixture_support::complete_processing_configuration(state, effects, 100_000);
    let id = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    crate::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    )
}

fn mixed() -> (AppState, Vec<LoadedArticle>, Vec<Effect>) {
    let articles: Vec<_> = (0..6).map(loaded_article).collect();
    let mut state = add_metadata(AppState::new());
    state.set_llm_max_in_flight(1);
    for article in &articles[2..5] {
        seed(&mut state, article, true, true, true);
    }
    seed(&mut state, &articles[5], true, false, false);
    let (state, effects) = start(state, articles.clone(), crate::PipelineRunScope::Resume);
    (state, articles, effects)
}

fn counts(state: &AppState, stage: PipelineStage) -> (u32, u32, u32, u32) {
    let s = &state.run_progress().unwrap().stages[stage.index()];
    (s.total, s.completed, s.failed, s.reused)
}

fn check_admissions(
    before: &[std::collections::HashSet<crate::pipeline_waves::Identity>; 3],
    state: &AppState,
) {
    let run = state.pipeline_admission.as_ref().unwrap();
    for (stage, prior) in before.iter().enumerate() {
        for (url, hash) in run.admitted[stage].difference(prior) {
            let v = state.unfinished_stage_verdicts(url, hash).unwrap();
            let verdict = [v.triage, v.summary, v.scoring][stage];
            assert_eq!(
                run.reused[stage].contains(&(url.clone(), hash.clone())),
                verdict == Verdict::Complete,
                "stage={stage} url={url} verdict={verdict:?}"
            );
        }
    }
}

fn complete_one(state: AppState, previous: &mut ProgressSnapshot, agreement: bool) -> AppState {
    complete_one_with_effects(state, previous, agreement).0
}

fn complete_one_with_effects(
    state: AppState,
    previous: &mut ProgressSnapshot,
    agreement: bool,
) -> (AppState, Vec<Effect>) {
    let before = state.pipeline_admission.as_ref().unwrap().admitted.clone();
    let msg = if let Some(id) = scoring_request(&state) {
        signal_success(id)
    } else if let Some(id) = summary_request(&state) {
        summary_result(id, true)
    } else {
        triage_success(triage_request(&state).unwrap())
    };
    let (state, effects) = crate::update(state, msg);
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::LoadProcessingConfiguration { .. })));
    assert_progress_does_not_regress(previous, &state);
    if agreement {
        check_admissions(&before, &state);
    }
    (state, effects)
}

fn drain(mut state: AppState, previous: &mut ProgressSnapshot) -> AppState {
    for _ in 0..100 {
        if state.article_model_requests_in_flight() == 0 {
            return state;
        }
        state = complete_one(state, previous, false);
    }
    panic!("requests did not drain");
}

#[test]
fn mixed_reuse_settles_at_admission_with_one_slot_and_preserves_waves() {
    let (mut state, articles, effects) = mixed();
    assert_eq!(counts(&state, PipelineStage::Triaging), (6, 4, 0, 4));
    assert_eq!(state.article_model_requests_in_flight(), 1);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::RequestLlmCompletion { .. }))
            .count(),
        1
    );
    assert!(
        request_id(&effects, PromptId::ArticleSummary).is_some(),
        "released summary outranks new triage"
    );
    assert_eq!(counts(&state, PipelineStage::Summarizing), (2, 1, 0, 1));
    // The first wave's cached summaries wait for its two new triage members.
    assert!(!state.pipeline_admission.as_ref().unwrap().admitted[1]
        .contains(&(articles[2].url.clone(), articles[2].content_hash.clone())));
    assert_eq!(
        state.triage_cache_metrics().hits(),
        4,
        "admission hits survive the metrics reset"
    );
    let mut previous = progress_snapshot(&state);
    let mut requests = [0; 3];
    while state.article_model_requests_in_flight() > 0 {
        let stage = if scoring_request(&state).is_some() {
            2
        } else if summary_request(&state).is_some() {
            1
        } else {
            0
        };
        requests[stage] += 1;
        state = complete_one(state, &mut previous, false);
    }
    assert_eq!(requests, [2, 3, 3]);
    for (stage, reused) in [
        (PipelineStage::Triaging, 4),
        (PipelineStage::Summarizing, 3),
        (PipelineStage::ScoringSignals, 3),
    ] {
        assert_eq!(counts(&state, stage), (6, 6, 0, reused));
    }
    assert!(state.run_progress().unwrap().terminal);
    assert_eq!(
        state.view().run_completion_notice.unwrap().new_result_count,
        3
    );
}

#[test]
fn second_run_reuses_current_sessions_without_readmitting_scores() {
    let (state, articles, _) = mixed();
    let state = run_to_completion(state, &articles, &[true; 3]);
    let (state, effects) = crate::update::test_support::update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    assert_eq!(counts(&state, PipelineStage::Triaging), (6, 6, 0, 6));
    assert_eq!(counts(&state, PipelineStage::Summarizing), (6, 6, 0, 6));
    assert_eq!(counts(&state, PipelineStage::ScoringSignals), (0, 0, 0, 0));
    assert!(state.run_progress().unwrap().terminal);
    assert_eq!(
        state.view().run_completion_notice.unwrap().new_result_count,
        0
    );
}

fn reload(
    mut state: AppState,
    articles: Vec<LoadedArticle>,
    previous: &mut ProgressSnapshot,
) -> AppState {
    let id = state.pre_triage_coordinator.begin_preparation_load();
    state.set_triage_in_flight(id);
    let state = crate::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    )
    .0;
    assert_progress_does_not_regress(previous, &state);
    state
}

#[test]
fn settlements_survive_prune_and_later_admissions_including_summaries() {
    let articles: Vec<_> = (0..5).map(loaded_article).collect();
    let mut state = add_metadata(AppState::new());
    state.set_llm_max_in_flight(1);
    for i in [0, 1, 3] {
        seed(&mut state, &articles[i], true, true, true);
    }
    let (state, _) = start(state, articles[..3].to_vec(), crate::PipelineRunScope::Full);
    let mut previous = progress_snapshot(&state);
    let state = drain(state, &mut previous);
    for stage in [PipelineStage::Triaging, PipelineStage::Summarizing] {
        assert_eq!(counts(&state, stage), (3, 3, 0, 2));
    }
    let state = reload(state, vec![articles[1].clone()], &mut previous);
    for stage in [PipelineStage::Triaging, PipelineStage::Summarizing] {
        assert_eq!(counts(&state, stage), (3, 3, 0, 2));
    }
    let state = reload(
        state,
        articles[1..2]
            .iter()
            .chain(&articles[3..5])
            .cloned()
            .collect(),
        &mut previous,
    );
    assert_eq!(counts(&state, PipelineStage::Triaging), (5, 4, 0, 3));
    let state = complete_one(state, &mut previous, false);
    assert_eq!(counts(&state, PipelineStage::Triaging), (5, 5, 0, 3));
    assert_eq!(counts(&state, PipelineStage::Summarizing), (5, 4, 0, 3));
    let state = drain(state, &mut previous);
    assert_eq!(counts(&state, PipelineStage::Summarizing), (5, 5, 0, 3));
    assert_eq!(counts(&state, PipelineStage::ScoringSignals), (5, 5, 0, 3));
}

#[test]
fn failures_survive_prune_and_later_completions() {
    let articles: Vec<_> = (0..3).map(loaded_article).collect();
    let (state, _) = start(
        add_metadata(AppState::new()),
        articles[..2].to_vec(),
        crate::PipelineRunScope::Full,
    );
    let mut previous = progress_snapshot(&state);
    let id = triage_request(&state).unwrap();
    let state = crate::update(
        state,
        Msg::LlmCompleted {
            request_id: id,
            result: LlmResultKind::Failed {
                reason: "test failure".into(),
            },
            metadata: None,
        },
    )
    .0;
    assert_progress_does_not_regress(&mut previous, &state);
    let state = complete_one(state, &mut previous, false);
    let state = crate::update(
        state.clone(),
        summary_result(summary_request(&state).unwrap(), false),
    )
    .0;
    assert_progress_does_not_regress(&mut previous, &state);
    let state = reload(state, vec![articles[2].clone()], &mut previous);
    let state = drain(state, &mut previous);
    assert_eq!(counts(&state, PipelineStage::Triaging), (3, 2, 1, 0));
    assert_eq!(counts(&state, PipelineStage::Summarizing), (2, 1, 1, 0));
}

#[test]
fn stop_withdraws_new_work_and_preserves_reuse_through_drain() {
    let (state, _, _) = mixed();
    let reused: Vec<_> = state
        .run_progress()
        .unwrap()
        .stages
        .iter()
        .map(|s| s.reused)
        .collect();
    let mut previous = progress_snapshot(&state);
    let (state, effects) = crate::update(state, Msg::StopFinishClicked);
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    assert_progress_does_not_regress(&mut previous, &state);
    assert_eq!(state.pipeline_run_phase(), PipelineRunPhase::Stopping);
    assert_eq!(state.triage().pending_count(), 0);
    assert_eq!(state.briefing().pending_count(), 0);
    let id = summary_request(&state).unwrap();
    let (state, effects) = crate::update(state, summary_result(id, true));
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    assert_progress_does_not_regress(&mut previous, &state);
    assert_eq!(
        state
            .run_progress()
            .unwrap()
            .stages
            .iter()
            .map(|s| s.reused)
            .collect::<Vec<_>>(),
        reused
    );
    assert!(state.run_progress().unwrap().terminal);
}

#[test]
fn classifier_agrees_at_each_admission_including_partial_cache() {
    let articles: Vec<_> = (0..6).map(loaded_article).collect();
    let mut state = add_metadata(AppState::new());
    state.set_llm_max_in_flight(1);
    for article in &articles[2..5] {
        seed(&mut state, article, true, true, true);
    }
    seed(&mut state, &articles[5], false, true, true);
    // Load the window before starting, so the run-start downstream verdict is observable.
    let id = state.pre_triage_coordinator.begin_preparation_load();
    state.set_triage_in_flight(id);
    let state = crate::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles.clone(), 100_000),
        },
    )
    .0;
    assert_eq!(
        state
            .unfinished_stage_verdicts(&articles[5].url, &articles[5].content_hash)
            .unwrap()
            .summary,
        Verdict::Unknown
    );
    let (mut state, _) = start(state, articles.clone(), crate::PipelineRunScope::Resume);
    check_admissions(&Default::default(), &state);
    let mut previous = progress_snapshot(&state);
    while state.article_model_requests_in_flight() > 0 {
        state = complete_one(state, &mut previous, true);
    }
    let member = (articles[5].url.clone(), articles[5].content_hash.clone());
    let run = state.pipeline_admission.as_ref().unwrap();
    assert!(!run.reused[0].contains(&member));
    assert!(run.reused[1].contains(&member));
    assert!(run.reused[2].contains(&member));
}

#[test]
fn result_appearing_after_admission_is_free_but_stays_new_work() {
    let triage_requests = |effects: &[Effect]| {
        effects
            .iter()
            .filter(|effect| {
                matches!(
                    effect,
                    Effect::RequestLlmCompletion {
                        prompt_id: PromptId::ArticleTriage,
                        ..
                    }
                )
            })
            .count()
    };
    let first = loaded_article(0);
    let mut second = first.clone();
    second.url = loaded_article(1).url;
    let (state, effects) = start(
        add_metadata(AppState::new()),
        vec![first, second],
        crate::PipelineRunScope::Resume,
    );
    assert_eq!(counts(&state, PipelineStage::Triaging), (2, 0, 0, 0));
    assert!(request_id(&effects, PromptId::ArticleTriage).is_some());
    let mut triage_request_count = triage_requests(&effects);
    let mut previous = progress_snapshot(&state);
    let (mut state, effects) = complete_one_with_effects(state, &mut previous, false);
    triage_request_count += triage_requests(&effects);
    assert_eq!(counts(&state, PipelineStage::Triaging), (2, 2, 0, 0));
    assert!(triage_request(&state).is_none());
    for _ in 0..100 {
        if state.article_model_requests_in_flight() == 0 {
            break;
        }
        let (next_state, effects) = complete_one_with_effects(state, &mut previous, false);
        triage_request_count += triage_requests(&effects);
        state = next_state;
    }
    assert!(state.run_progress().unwrap().terminal);
    assert_eq!(
        triage_request_count, 1,
        "duplicate content needs one triage request"
    );
    assert_eq!(counts(&state, PipelineStage::Triaging), (2, 2, 0, 0));
}
