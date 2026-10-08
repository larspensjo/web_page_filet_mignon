use super::*;
#[cfg(test)]
use crate::fixture_support::ManualPreTriageDecisions;
use crate::update as reduce;
use harvester_engine::{TriageArticleDelta, WindowArticle};

fn article(url: &str, hash: &str) -> LoadedArticle {
    LoadedArticle {
        url: url.into(),
        source_title: Some(url.into()),
        content_hash: hash.into(),
        prepared_text: "substantial article content ".repeat(100),
        fetched_utc: Some("2026-09-23T00:00:00Z".into()),
    }
}
fn member(a: &LoadedArticle) -> WindowArticle {
    WindowArticle {
        url: a.url.clone(),
        content_hash: a.content_hash.clone(),
        source_title: a.source_title.clone(),
        fetched_utc: a.fetched_utc.clone(),
    }
}
fn apply(mut state: AppState, delta: TriageArticleDelta) -> AppState {
    let request_id = state.alloc_triage_request_id();
    state.set_triage_in_flight(request_id);
    reduce(state, Msg::TriageArticlesLoaded { request_id, delta }).0
}
fn config(state: AppState, effects: &[Effect], budget: usize) -> (AppState, Vec<Effect>) {
    let request_id = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadProcessingConfiguration { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("configuration boundary");
    reduce(
        state,
        Msg::ProcessingConfigurationLoaded {
            request_id,
            contexts: Default::default(),
            active_versions: [(PromptId::ArticleTriage, 1), (PromptId::ArticleSummary, 1)].into(),
            effective_models: [
                (PromptId::ArticleTriage, "test-model".into()),
                (PromptId::ArticleSummary, "test-model".into()),
            ]
            .into(),
            preparation_budget: budget,
        },
    )
}
fn no_models(effects: &[Effect]) {
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
}

#[test]
fn completed_download_blocks_settlement_before_refresh_evaluation_is_served() {
    let (state, _) = reduce(
        AppState::new(),
        Msg::InputChanged("https://example.com/a".into()),
    );
    let (state, effects) = reduce(state, Msg::UrlsSubmitted);
    let id = effects
        .iter()
        .find_map(|e| match e {
            Effect::EnqueueUrl { job_id, .. } => Some(*job_id),
            _ => None,
        })
        .unwrap();
    let (state, _) = reduce(
        state,
        Msg::JobDone {
            job_id: id,
            result: crate::JobResultKind::Success,
            extracted_links: vec![],
            fetched_utc: None,
        },
    );
    assert!(state.pipeline_activity().intake_refresh_pending);
    assert!(!state.pipeline_activity().is_settled());
}

#[test]
fn delta_appends_replaces_rebudgets_preserves_verdicts_and_removes_departed_members() {
    let a = article("https://example.com/a", "a");
    let b = article("https://example.com/b", "b");
    let c = article("https://example.com/c", "c");
    let d = article("https://example.com/d", "d");
    let mut state = apply(
        AppState::new(),
        TriageArticleDelta::full_window(vec![a.clone(), b.clone(), c.clone(), d], 10_000),
    );
    let mut session = state.pre_triage().clone();
    let key = session.entries()[0].key.clone();
    session
        .set_manual_decision(&key, crate::ManualDecision::Exclude)
        .unwrap();
    state.set_pre_triage(session);
    let mut changed = b;
    changed.content_hash = "changed-b".into();
    changed.prepared_text = "short".into();
    let mut rebudgeted = c;
    rebudgeted.prepared_text = "short".into();
    let e = article("https://example.com/e", "e");
    state = apply(
        state,
        TriageArticleDelta {
            members: vec![
                member(&a),
                member(&changed),
                member(&rebudgeted),
                member(&e),
            ],
            preparation_budget: 10_000,
            articles: vec![changed.clone(), rebudgeted.clone(), e],
        },
    );
    assert_eq!(state.pre_triage().entries().len(), 4);
    assert_eq!(
        state.pre_triage().entries()[0].manual_decision,
        Some(crate::ManualDecision::Exclude)
    );
    assert_eq!(
        state.pre_triage().entries()[1].auto_verdict,
        crate::AutoVerdict::HardExclude
    );
    assert_eq!(
        state.pre_triage().entries()[2].auto_verdict,
        crate::AutoVerdict::Include
    );
    assert_eq!(
        state.pre_triage().article_content_hash(&changed.url),
        Some("changed-b")
    );
    assert!(state
        .pre_triage()
        .entry_for_url("https://example.com/d")
        .is_none());
    let mut all = vec![
        a,
        changed,
        rebudgeted,
        article("https://example.com/e", "e"),
    ];
    for a in &mut all {
        a.prepared_text.truncate(1_000);
    }
    state = apply(state, TriageArticleDelta::full_window(all, 1_000));
    assert!(state
        .pre_triage()
        .held_articles()
        .iter()
        .all(|h| h.preparation_budget == 1_000));
    assert_eq!(
        state.pre_triage().entries()[0].manual_decision,
        Some(crate::ManualDecision::Exclude)
    );
    assert_eq!(
        state.pre_triage().resolved_included_articles()[0].prepared_text,
        "short"
    );
}

#[test]
fn pending_and_in_flight_refresh_prevent_settlement_without_erasing_preparation() {
    let a = article("https://example.com/a", "a");
    let state = apply(
        AppState::new(),
        TriageArticleDelta::full_window(vec![a.clone()], 10_000),
    );
    let (state, effects) = reduce(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: vec![a.url],
            triggered_by_job_done: false,
        },
    );
    assert!(effects.is_empty());
    assert!(!state.pipeline_activity().is_settled());
    assert!(state.pipeline_activity().intake_refresh_pending);
    assert_eq!(state.pre_triage().entries().len(), 1);
    assert_ne!(
        state.pre_triage().phase(),
        crate::PreTriagePhase::LoadingArticles
    );
    let (state, request_id) = tick_until_dispatch(state);
    assert!(!state.pipeline_activity().is_settled());
    let delta = TriageArticleDelta {
        members: state
            .pre_triage()
            .held_articles()
            .iter()
            .map(|h| WindowArticle {
                url: h.url.clone(),
                content_hash: h.content_hash.clone(),
                source_title: None,
                fetched_utc: None,
            })
            .collect(),
        preparation_budget: 10_000,
        articles: vec![],
    };
    let (state, _) = reduce(state, Msg::TriageArticlesLoaded { request_id, delta });
    assert!(state.pipeline_activity().is_settled());
}

#[test]
fn triage_waits_for_matching_budget_then_summaries_reuse_snapshot_and_prepared_text() {
    let a = article("https://example.com/a", "a");
    let state = apply(
        with_summary_metadata(AppState::new()),
        TriageArticleDelta::full_window(vec![a.clone()], 10_000),
    );
    let (state, effects) = reduce(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    no_models(&effects);
    let (state, effects) = config(state, &effects, 1_000);
    no_models(&effects);
    assert!(!state.pipeline_activity().is_settled());
    let request_id = match &effects[0] {
        Effect::LoadArticlesForTriage {
            request_id, held, ..
        } => {
            assert_eq!(held[0].preparation_budget, 10_000);
            *request_id
        }
        other => panic!("expected delta load: {other:?}"),
    };
    let mut prepared = a;
    prepared.prepared_text.truncate(1_000);
    let (state, effects) = reduce(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: TriageArticleDelta::full_window(vec![prepared.clone()], 1_000),
        },
    );
    let id = effects
        .iter()
        .find_map(|e| match e {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: PromptId::ArticleTriage,
                input_content,
                ..
            } => {
                assert_eq!(input_content, &prepared.prepared_text);
                Some(*request_id)
            }
            _ => None,
        })
        .expect("triage dispatch after preparation");
    assert_eq!(state.triage().articles()[0].preparation_budget, Some(1_000));
    let (state, effects) = reduce(state, triage_success(id));
    let (state, duplicate_effects) = reduce(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert!(duplicate_effects.is_empty());
    assert!(effects.iter().all(|e| !matches!(
        e,
        Effect::LoadArticlesForTriage { .. }
            | Effect::LoadProcessingConfiguration { .. }
            | Effect::LoadPromptContexts
            | Effect::LoadPromptTemplateFiles
            | Effect::LoadLlmMetadata
    )));
    assert!(effects.iter().any(|e| matches!(e, Effect::RequestLlmCompletion { prompt_id: PromptId::ArticleSummary, input_content, .. } if input_content == &prepared.prepared_text)));
    assert_eq!(
        state.briefing().articles()[0].prepared_text,
        prepared.prepared_text
    );
}

#[test]
fn resume_reprepares_before_summary_dispatch_and_rejects_wrong_budget() {
    let url = "https://resume-budget.example/article";
    let start = || {
        let base = with_summary_metadata(support::ready_pre_triage_state(&[url]));
        let (state, effects) = reduce(
            base,
            Msg::PipelineRunRequested {
                scope: crate::PipelineRunScope::Resume,
            },
        );
        let (state, effects) = config(state, &effects, 1_000);
        let request_id = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            })
            .expect("Resume requests fresh article preparation");
        (state, request_id)
    };
    let (state, request_id) = start();
    let prepared = LoadedArticle {
        prepared_text: "within budget".into(),
        ..article(url, &format!("hash-{url}"))
    };
    let (rejected, effects) = reduce(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: TriageArticleDelta::full_window(vec![prepared.clone()], 2_000),
        },
    );
    no_models(&effects);
    assert!(matches!(
        rejected.triage().phase(),
        crate::triage::TriagePhase::Failed { .. }
    ));

    let (state, request_id) = start();
    let (state, effects) = reduce(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: TriageArticleDelta::full_window(vec![prepared], 1_000),
        },
    );
    let effects = complete_triage_if_requested(state, effects);
    assert!(effects.iter().any(|e| matches!(e, Effect::RequestLlmCompletion { prompt_id: PromptId::ArticleSummary, input_content, .. } if input_content == "within budget")), "effects={effects:?}");
}

#[test]
fn duplicate_url_delta_keeps_first_identity_and_manual_decision() {
    let first = article("https://example.com/duplicate", "first");
    let second = article("https://example.com/duplicate", "second");
    let mut state = apply(
        AppState::new(),
        TriageArticleDelta::full_window(vec![first.clone()], 10_000),
    );
    let mut session = state.pre_triage().clone();
    let key = session.entries()[0].key.clone();
    session
        .set_manual_decision(&key, crate::ManualDecision::Exclude)
        .unwrap();
    state.set_pre_triage(session);
    for _ in 0..2 {
        state = apply(
            state,
            TriageArticleDelta::full_window(vec![first.clone(), second.clone()], 10_000),
        );
        assert_eq!(state.pre_triage().entries().len(), 1);
        assert_eq!(
            state.pre_triage().article_content_hash(&first.url),
            Some("first")
        );
        assert_eq!(
            state.pre_triage().entries()[0].manual_decision,
            Some(crate::ManualDecision::Exclude)
        );
        assert_eq!(state.pre_triage().held_articles().len(), 1);
    }
}

#[test]
fn resume_run_loads_the_current_window_before_triage() {
    let a = article("https://example.com/a", "a");
    let b = article("https://example.com/b", "b");
    let state = apply(
        AppState::new(),
        TriageArticleDelta::full_window(vec![a.clone()], 10_000),
    );
    let state = support::add_completed_job_for_test(state, &b.url);
    let (state, effects) = reduce(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadProcessingConfiguration {
            require_triage_context: true,
            ..
        }]
    ));
    assert!(state.pipeline_run_armed());
    let (state, _) = reduce(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: vec![a.url.clone(), b.url.clone()],
            triggered_by_job_done: false,
        },
    );
    assert!(state.pipeline_run_armed());
    // The run's fresh load covers queued window demand before model work.
    let (state, effects) = config(state, &effects, 10_000);
    let request_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("Resume starts with a fresh window load");
    assert!(state.pipeline_activity().intake_refresh_pending);
    let (state, effects) = reduce(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: TriageArticleDelta::full_window(vec![a, b.clone()], 10_000),
        },
    );
    assert_eq!(
        triage_requests(&effects),
        1,
        "one request fits the default in-flight budget"
    );
    assert_eq!(state.triage().total(), 2);
    assert_eq!(state.triage().pending_count(), 1);
    assert_eq!(state.pipeline_waves().waves().len(), 1);
    assert!(state.triage().articles().iter().any(|t| t.url == b.url));
}

fn triage_requests(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| {
            matches!(
                e,
                Effect::RequestLlmCompletion {
                    prompt_id: PromptId::ArticleTriage,
                    ..
                }
            )
        })
        .count()
}

fn complete_triage_if_requested(state: AppState, effects: Vec<Effect>) -> Vec<Effect> {
    let Some(request_id) = effects.iter().find_map(|effect| match effect {
        Effect::RequestLlmCompletion {
            request_id,
            prompt_id: PromptId::ArticleTriage,
            ..
        } => Some(*request_id),
        _ => None,
    }) else {
        return effects;
    };
    reduce(
        state,
        Msg::LlmCompleted {
            request_id,
            result: crate::LlmResultKind::Success {
                output_json: r#"{"category":"tech","priority":3,"tags":[],"rationale":"ok"}"#
                    .into(),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "test-triage-model".into(),
            },
            metadata: None,
        },
    )
    .1
}

#[test]
fn resume_run_triages_and_summarizes_the_current_window() {
    let url = "https://resume.example/article";
    let base = with_summary_metadata(support::ready_pre_triage_state(&[url]));
    let (state, effects) = reduce(
        base,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadProcessingConfiguration {
            require_triage_context: true,
            ..
        }]
    ));
    let (state, effects) = config(state, &effects, 1_000);
    let (request_id, ordered_urls) = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadArticlesForTriage {
                request_id,
                ordered_urls,
                ..
            } => Some((*request_id, ordered_urls)),
            _ => None,
        })
        .expect("Resume refreshes the current article before model work");
    assert_eq!(ordered_urls, &[url.to_string()]);
    let (state, effects) = reduce(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: TriageArticleDelta::full_window(
                vec![LoadedArticle {
                    prepared_text: "within budget".into(),
                    ..article(url, &format!("hash-{url}"))
                }],
                1_000,
            ),
        },
    );
    let effects = complete_triage_if_requested(state, effects);
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::ArticleSummary,
                input_content,
                ..
            } if input_content == "within budget"
        )),
        "effects={effects:?}"
    );
}

#[test]
fn failed_processing_start_keeps_completed_triage_available_for_archive() {
    let state = apply(
        complete_triage_state_for_test(1),
        TriageArticleDelta::full_window(
            vec![article("https://triage-complete.com/0", "hash-tc-0")],
            10_000,
        ),
    );
    let (state, effects) = reduce(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let request_id = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadProcessingConfiguration { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    let (state, _) = reduce(
        state,
        Msg::ProcessingConfigurationFailed {
            request_id,
            reason: "missing configuration".into(),
        },
    );
    assert!(matches!(
        state.triage().phase(),
        crate::TriagePhase::Complete
    ));
    assert_eq!(state.archive_corpus().ordered_urls().len(), 1);
}
