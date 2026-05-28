use super::*;
use std::collections::{HashMap, HashSet};

use crate::PromptLabStage;
use harvester_engine::llm::OPENAI_MODEL_GPT_4O;

#[test]
fn prompt_lab_open_sets_visible_and_dirty() {
    init_logging();
    let state = AppState::new();
    let (state, effects) = update(state, Msg::PromptLabOpenRequested);
    assert!(state.prompt_lab().is_visible());
    assert!(state.view().dirty);
    assert!(effects.is_empty());
}

#[test]
fn prompt_lab_close_clears_visible() {
    init_logging();
    let mut state = AppState::new();
    state.open_prompt_lab();
    let (state, effects) = update(state, Msg::PromptLabCloseRequested);
    assert!(!state.prompt_lab().is_visible());
    assert!(state.view().dirty);
    assert!(effects.is_empty());
}

#[test]
fn prompt_lab_stage_selected_updates_stage() {
    init_logging();
    let state = AppState::new();
    let (state, _) = update(
        state,
        Msg::PromptLabStageSelected {
            stage: crate::prompt_lab::PromptLabStage::Summary,
        },
    );
    assert_eq!(
        state.prompt_lab().selected_stage(),
        crate::prompt_lab::PromptLabStage::Summary
    );
}

#[test]
fn prompt_lab_input_changed_updates_input() {
    init_logging();
    let state = AppState::new();
    let (state, _) = update(
        state,
        Msg::PromptLabInputChanged {
            text: "hello world".to_string(),
        },
    );
    assert_eq!(state.prompt_lab().input(), "hello world");
}

#[test]
fn prompt_lab_input_changed_sets_dirty() {
    init_logging();
    let state = AppState::new();
    let (state, _) = update(
        state,
        Msg::PromptLabInputChanged {
            text: "dirty text".to_string(),
        },
    );
    assert!(state.view().dirty);
}

#[test]
fn prompt_lab_run_requested_with_nonempty_input_emits_effect_and_creates_pending_run() {
    init_logging();
    let mut state = AppState::new();
    prepare_type_url_snapshot(&mut state, "some article text");
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0], Effect::RequestLlmCompletion { .. }));
    assert_eq!(state.prompt_lab().run_count(), 1);
    assert!(state.prompt_lab().has_in_flight_run());
    use crate::prompt_lab::PromptLabRunStatus;
    assert!(matches!(
        state.prompt_lab().latest_run().unwrap().status,
        PromptLabRunStatus::Pending { .. }
    ));
}

#[test]
fn prompt_lab_run_requested_with_empty_input_emits_no_effects() {
    init_logging();
    let state = AppState::new();
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    assert!(effects.is_empty());
    assert_eq!(state.prompt_lab().run_count(), 0);
}

#[test]
fn prompt_lab_run_requested_while_in_flight_emits_no_effects() {
    init_logging();
    let mut state = AppState::new();
    prepare_type_url_snapshot(&mut state, "text");
    let (mut state, _) = update(state, Msg::PromptLabRunRequested);
    assert!(state.prompt_lab().has_in_flight_run());
    prepare_type_url_snapshot(&mut state, "different text");
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    assert!(effects.is_empty());
    assert_eq!(state.prompt_lab().run_count(), 1);
}

#[test]
fn input_source_selection_updates_state_and_dirty() {
    init_logging();
    let state = AppState::new();
    let (state, _) = update(
        state,
        Msg::PromptLabInputSourceSelected {
            source: crate::prompt_lab::PromptLabInputSource::TypeUrl,
        },
    );
    assert_eq!(
        state.prompt_lab().selected_input_source(),
        crate::prompt_lab::PromptLabInputSource::TypeUrl
    );
    assert!(state.view().dirty);
}

#[test]
fn url_input_change_marks_dirty() {
    init_logging();
    let (state, _) = update(
        AppState::new(),
        Msg::PromptLabUrlInputChanged {
            url: "https://example.com".to_string(),
        },
    );
    assert_eq!(state.prompt_lab().url_input(), "https://example.com");
    assert!(state.view().dirty);
}

#[test]
fn resolve_requested_emits_effect() {
    init_logging();
    let mut state = AppState::new();
    state
        .prompt_lab_mut()
        .set_url_input("https://example.com".to_string());
    let (state, effects) = update(state, Msg::PromptLabResolveRequested);
    assert_eq!(effects.len(), 1);
    assert!(matches!(
        &effects[0],
        Effect::ResolvePromptLabInputFromUrl { .. }
    ));
    assert!(state.prompt_lab().pending_resolve_id().is_some());
}

#[test]
fn resolve_requested_no_op_when_url_empty() {
    init_logging();
    let (state, effects) = update(AppState::new(), Msg::PromptLabResolveRequested);
    assert!(effects.is_empty());
    assert!(state.prompt_lab().pending_resolve_id().is_none());
}

#[test]
fn input_resolved_stores_snapshot_and_marks_dirty() {
    init_logging();
    let mut state = AppState::new();
    state.prompt_lab_mut().begin_url_resolution(1);
    let (state, _) = update(
        state,
        Msg::PromptLabInputResolved {
            resolve_id: 1,
            result: Ok("snapshot".to_string()),
        },
    );
    assert_eq!(state.prompt_lab().resolved_url_snapshot(), Some("snapshot"));
    assert!(state.view().dirty);
}

#[test]
fn stale_input_resolved_ignored() {
    init_logging();
    let mut state = AppState::new();
    state.prompt_lab_mut().begin_url_resolution(7);
    let (state, _) = update(
        state,
        Msg::PromptLabInputResolved {
            resolve_id: 8,
            result: Ok("ignored".to_string()),
        },
    );
    assert_eq!(state.prompt_lab().pending_resolve_id(), Some(7));
    assert!(state.prompt_lab().resolved_url_snapshot().is_none());
    assert!(!state.view().dirty);
}

#[test]
fn run_requested_fromtriage_no_op_when_no_triage_articles() {
    init_logging();
    let (_state, effects) = update(AppState::new(), Msg::PromptLabRunRequested);
    assert!(effects.is_empty());
}

#[test]
fn run_requested_uses_resolved_snapshot_after_job_selection() {
    init_logging();
    let (state, _) = update(
        AppState::new(),
        Msg::InputChanged("https://example.com/article/".to_string()),
    );
    let (mut state, _) = update(state, Msg::UrlsSubmitted);

    state.triage_mut().set_articles(vec![LoadedArticle {
        url: "https://example.com/article".to_string(),
        source_title: Some("Example".to_string()),
        prepared_text: "selected article prepared text".to_string(),
        content_hash: "hash-selected".to_string(),
        fetched_utc: None,
    }]);
    state.triage_mut().transition_to_triaging();
    state.triage_mut().complete_article(
        0,
        crate::triage::ArticleTriageResult {
            category: "news".to_string(),
            priority: 3,
            tags: vec!["tag".to_string()],
            rationale: "ok".to_string(),
            input_tokens: 1,
            output_tokens: 1,
        },
    );

    let (state, effects) = update(state, Msg::JobSelected { job_id: 1 });
    let resolve_id = match effects.first() {
        Some(Effect::ResolvePromptLabInputFromUrl { resolve_id, .. }) => *resolve_id,
        other => panic!("expected resolve effect, got {other:?}"),
    };
    let (state, _) = update(
        state,
        Msg::PromptLabInputResolved {
            resolve_id,
            result: Ok("selected article prepared text".to_string()),
        },
    );
    let (_state, effects) = update(state, Msg::PromptLabRunRequested);
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::RequestLlmCompletion { input_content, .. } => {
            assert_eq!(input_content, "selected article prepared text");
        }
        other => panic!("expected RequestLlmCompletion, got {other:?}"),
    }
}

#[test]
fn job_selected_without_summary_selects_triage_tab_and_requests_resolve() {
    init_logging();
    let (state, _) = update(
        AppState::new(),
        Msg::InputChanged("https://example.com/article".to_string()),
    );
    let (state, _) = update(state, Msg::UrlsSubmitted);
    let (state, _) = update(
        state,
        Msg::LeftTabSelected {
            tab: crate::tabs::LeftTab::PromptLab,
        },
    );
    let (state, effects) = update(state, Msg::JobSelected { job_id: 1 });
    assert_eq!(state.active_tab(), crate::tabs::AppTab::Triage);
    assert_eq!(
        state.prompt_lab().url_input(),
        "https://example.com/article"
    );
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::ResolvePromptLabInputFromUrl { .. })));
}

#[test]
fn job_selected_with_summary_selects_summary_tab() {
    init_logging();
    let mut state = make_state_with_summarized_job_for_update();
    let job_id = state.view().jobs.first().map(|job| job.job_id).unwrap_or(1);
    state.select_tab(crate::tabs::AppTab::Triage);

    let (state, _effects) = update(state, Msg::JobSelected { job_id });
    assert_eq!(state.active_tab(), crate::tabs::AppTab::Summary);
}

#[test]
fn run_requested_typeurl_no_op_when_snapshot_not_resolved() {
    init_logging();
    let (state, _effects) = update(
        AppState::new(),
        Msg::PromptLabInputSourceSelected {
            source: crate::prompt_lab::PromptLabInputSource::TypeUrl,
        },
    );
    let (state, _) = update(
        state,
        Msg::PromptLabUrlInputChanged {
            url: "https://example.com".to_string(),
        },
    );
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    assert!(effects.is_empty());
    assert!(state.prompt_lab().resolved_url_snapshot().is_none());
}

#[test]
fn rerun_dispatches_same_parameters_as_original_run() {
    init_logging();
    let state = AppState::new();
    let (state, request_id) = dispatch_lab_run(state);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Success {
                output_json: "{}".to_string(),
                input_tokens: 5,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "model".to_string(),
            },
            metadata: Some(LlmRunMetadata::stub()),
        },
    );
    let (_state, effects) = update(state, Msg::PromptLabRerunRequested);
    assert_eq!(effects.len(), 1);
    if let Effect::RequestLlmCompletion { input_content, .. } = &effects[0] {
        assert_eq!(input_content, "article content");
    } else {
        panic!("expected RequestLlmCompletion");
    }
}

#[test]
fn rerun_blocked_when_in_flight() {
    init_logging();
    let state = AppState::new();
    let (state, _) = dispatch_lab_run(state);
    let (_state, effects) = update(state, Msg::PromptLabRerunRequested);
    assert!(effects.is_empty());
}

#[test]
fn prompt_lab_lifecycle_leaves_triage_session_unchanged() {
    init_logging();
    let mut state = AppState::new();
    let triage_phase = state.triage().phase().clone();
    prepare_type_url_snapshot(&mut state, "content");
    let (state, _) = update(state, Msg::PromptLabRunRequested);
    assert_eq!(state.triage().phase(), &triage_phase);
}

#[test]
fn prompt_lab_lifecycle_leaves_briefing_session_unchanged() {
    init_logging();
    let mut state = AppState::new();
    let briefing_phase = state.briefing().phase().clone();
    prepare_type_url_snapshot(&mut state, "content");
    let (state, _) = update(state, Msg::PromptLabRunRequested);
    assert_eq!(state.briefing().phase(), &briefing_phase);
}

#[test]
fn llm_completed_success_routes_to_lab_run() {
    init_logging();
    let state = AppState::new();
    let (state, request_id) = dispatch_lab_run(state);
    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Success {
                output_json: r#"{"priority":3}"#.to_string(),
                input_tokens: 10,
                output_tokens: 20,
                prompt_version: 1,
                resolved_model: "model-x".to_string(),
            },
            metadata: Some(LlmRunMetadata::stub()),
        },
    );
    assert!(effects.is_empty());
    use crate::prompt_lab::PromptLabRunStatus;
    assert!(matches!(
        state.prompt_lab().latest_run().unwrap().status,
        PromptLabRunStatus::Completed { .. }
    ));
    assert!(!state.prompt_lab().has_in_flight_run());
}

#[test]
fn llm_completed_validation_failed_routes_to_lab_run_as_failed() {
    init_logging();
    let state = AppState::new();
    let (state, request_id) = dispatch_lab_run(state);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::ValidationFailed {
                reason: "bad json".to_string(),
                raw_response: "garbage".to_string(),
            },
            metadata: None,
        },
    );
    use crate::prompt_lab::PromptLabRunStatus;
    assert!(matches!(
        state.prompt_lab().latest_run().unwrap().status,
        PromptLabRunStatus::Failed { .. }
    ));
    assert!(!state.prompt_lab().has_in_flight_run());
}

#[test]
fn llm_completed_quota_exhausted_routes_to_lab_run_as_failed() {
    init_logging();
    let state = AppState::new();
    let (state, request_id) = dispatch_lab_run(state);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::QuotaExhausted {
                reason: "over limit".to_string(),
            },
            metadata: None,
        },
    );
    use crate::prompt_lab::PromptLabRunStatus;
    assert!(matches!(
        state.prompt_lab().latest_run().unwrap().status,
        PromptLabRunStatus::Failed { .. }
    ));
}

#[test]
fn prompt_lab_history_cleared_removes_completed_and_failed() {
    init_logging();
    let mut state = AppState::new();
    let rid = state.allocate_next_llm_request_id();
    let run = state.allocate_next_prompt_lab_run_id();
    state.add_prompt_lab_pending_run(crate::state::PromptLabPendingRunRegistration {
        run_id: run,
        stage: PromptLabStage::Triage,
        prompt_id: PromptId::ArticleTriage,
        input_snapshot: "x".to_string(),
        request_id: rid,
        overrides: crate::prompt_lab::PromptLabRunOverrides::default(),
        compare_batch_id: None,
        compare_candidate_id: None,
    });
    state.complete_prompt_lab_run(run, "{}".to_string(), LlmRunMetadata::stub());
    state.consume_prompt_lab_ownership(rid);
    assert_eq!(state.prompt_lab().run_count(), 1);
    let (state, effects) = update(state, Msg::PromptLabHistoryCleared);
    assert_eq!(state.prompt_lab().run_count(), 0);
    assert!(effects.is_empty());
    assert!(state.prompt_lab().latest_run().is_none());
}

#[test]
fn prompt_lab_lifecycle_leaves_briefing_default() {
    init_logging();
    let state = AppState::new();
    let briefing_before = state.briefing().clone();

    let (state, _) = update(state, Msg::PromptLabOpenRequested);
    let (state, _) = update(
        state,
        Msg::PromptLabStageSelected {
            stage: crate::prompt_lab::PromptLabStage::Summary,
        },
    );
    let (mut state, _) = update(
        state,
        Msg::PromptLabInputChanged {
            text: "article text".to_string(),
        },
    );
    prepare_type_url_snapshot(&mut state, "article text");
    let (state, effects) = {
        let (s, e) = update(state, Msg::PromptLabRunRequested);
        (s, e)
    };
    let request_id = effects
        .iter()
        .find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        })
        .unwrap();

    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Success {
                output_json: r#"{"priority":3,"category":"news","tags":[],"rationale":"ok"}"#
                    .to_string(),
                input_tokens: 5,
                output_tokens: 10,
                prompt_version: 1,
                resolved_model: "m".to_string(),
            },
            metadata: None,
        },
    );

    assert_eq!(
        state.briefing().clone(),
        briefing_before,
        "briefing must be unchanged"
    );
}

#[test]
fn prompt_lab_lifecycle_leaves_triage_default() {
    init_logging();
    let state = AppState::new();
    let triage_before = state.triage().clone();

    let (mut state, _) = update(state, Msg::PromptLabOpenRequested);
    prepare_type_url_snapshot(&mut state, "article text");
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    let request_id = effects
        .iter()
        .find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        })
        .unwrap();

    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Failed {
                reason: "timeout".to_string(),
            },
            metadata: None,
        },
    );

    assert_eq!(
        state.triage().clone(),
        triage_before,
        "triage must be unchanged"
    );
}

#[test]
fn triage_and_lab_coexistence_no_bleed() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(1);
    let (mut state, triage_effects) = start_triage_for_test(state, loaded_triage_articles(1));
    let triage_req_id = triage_effects
        .iter()
        .find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        })
        .expect("triage request");

    prepare_type_url_snapshot(&mut state, "article text");
    let (state, lab_effects) = update(state, Msg::PromptLabRunRequested);
    let lab_req_id = lab_effects
        .iter()
        .find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        })
        .expect("lab request");

    assert_ne!(triage_req_id, lab_req_id, "request IDs must be distinct");

    let (state, _) = update(state, triage_success(triage_req_id));
    use crate::prompt_lab::PromptLabRunStatus;
    assert!(matches!(
        state.prompt_lab().latest_run().unwrap().status,
        PromptLabRunStatus::Pending { .. }
    ));

    let triage_completed_before = state.triage().completed_count();
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: lab_req_id,
            result: LlmResultKind::Success {
                output_json: r#"{"priority":3,"category":"news","tags":[],"rationale":"ok"}"#
                    .to_string(),
                input_tokens: 5,
                output_tokens: 10,
                prompt_version: 1,
                resolved_model: "m".to_string(),
            },
            metadata: Some(LlmRunMetadata::stub()),
        },
    );
    assert_eq!(state.triage().completed_count(), triage_completed_before);
    assert!(matches!(
        state.prompt_lab().latest_run().unwrap().status,
        PromptLabRunStatus::Completed { .. }
    ));
}

#[test]
fn id_namespace_all_request_ids_distinct() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(3);
    let (mut state, triage_effects) = start_triage_for_test(state, loaded_triage_articles(3));
    let triage_ids: Vec<u64> = triage_effects
        .iter()
        .filter_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(triage_ids.len(), 3);

    prepare_type_url_snapshot(&mut state, "text1");
    let (state, e1) = update(state, Msg::PromptLabRunRequested);
    let lab_id1 = e1
        .iter()
        .find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        })
        .unwrap();

    let (mut state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: lab_id1,
            result: LlmResultKind::Failed {
                reason: "done".to_string(),
            },
            metadata: None,
        },
    );
    prepare_type_url_snapshot(&mut state, "text2");
    let (_, e2) = update(state, Msg::PromptLabRunRequested);
    let lab_id2 = e2
        .iter()
        .find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        })
        .unwrap();

    let all_ids = [triage_ids.as_slice(), &[lab_id1, lab_id2]].concat();
    let unique: HashSet<u64> = all_ids.iter().copied().collect();
    assert_eq!(
        unique.len(),
        all_ids.len(),
        "all request_ids must be distinct"
    );
}

#[test]
fn prompt_lab_run_with_model_override_emits_effect_containing_it() {
    init_logging();
    use harvester_engine::llm::{ModelId, ProviderKind};
    let mut state = AppState::new();
    prepare_type_url_snapshot(&mut state, "some text");
    let override_model = ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_4O);
    state
        .prompt_lab_mut()
        .set_model_override(Some(override_model.clone()));
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    assert_eq!(effects.len(), 1);
    if let Effect::RequestLlmCompletion { model_override, .. } = &effects[0] {
        assert_eq!(model_override.as_ref(), Some(&override_model));
    } else {
        panic!("expected RequestLlmCompletion effect");
    }
    let run = state.prompt_lab().latest_run().unwrap();
    assert_eq!(run.model_override.as_ref(), Some(&override_model));
}

#[test]
fn prompt_lab_run_with_prompt_version_override_emits_effect_containing_it() {
    init_logging();
    let mut state = AppState::new();
    prepare_type_url_snapshot(&mut state, "some text");
    state.prompt_lab_mut().set_prompt_version_override(Some(42));
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    assert_eq!(effects.len(), 1);
    if let Effect::RequestLlmCompletion { prompt_version, .. } = &effects[0] {
        assert_eq!(*prompt_version, Some(42));
    } else {
        panic!("expected RequestLlmCompletion effect");
    }
    let run = state.prompt_lab().latest_run().unwrap();
    assert_eq!(run.prompt_version_used, Some(42));
}

#[test]
fn prompt_lab_run_record_stores_dispatched_override_values() {
    init_logging();
    use harvester_engine::llm::{ModelId, ProviderKind};
    let mut state = AppState::new();
    prepare_type_url_snapshot(&mut state, "some text");
    let override_model = ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_4O);
    state
        .prompt_lab_mut()
        .set_model_override(Some(override_model.clone()));
    state.prompt_lab_mut().set_prompt_version_override(Some(7));
    let (state, _) = update(state, Msg::PromptLabRunRequested);
    let run = state.prompt_lab().latest_run().unwrap();
    assert_eq!(run.model_override.as_ref(), Some(&override_model));
    assert_eq!(run.prompt_version_used, Some(7));
}

#[test]
fn prompt_lab_run_none_overrides_behaves_as_before() {
    init_logging();
    let mut state = AppState::new();
    prepare_type_url_snapshot(&mut state, "some text");
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    assert_eq!(effects.len(), 1);
    if let Effect::RequestLlmCompletion { model_override, .. } = &effects[0] {
        assert!(model_override.is_none());
    } else {
        panic!("expected RequestLlmCompletion effect");
    }
    let run = state.prompt_lab().latest_run().unwrap();
    assert!(run.model_override.is_none());
    assert!(run.prompt_version_used.is_none());
}

#[test]
fn production_dispatch_paths_emit_none_model_override() {
    init_logging();
    let mut triage_state = AppState::new();
    let triage_request_id = triage_state.alloc_triage_request_id();
    triage_state.set_triage_in_flight(triage_request_id);
    let (_, effects) = update(
        triage_state,
        Msg::TriageArticlesLoaded {
            request_id: triage_request_id,
            articles: loaded_triage_articles(1),
        },
    );
    let llm_effect = effects
        .iter()
        .find(|e| matches!(e, Effect::RequestLlmCompletion { .. }));
    if let Some(Effect::RequestLlmCompletion { model_override, .. }) = llm_effect {
        assert!(model_override.is_none());
    }
}

#[test]
fn dispatch_prompt_lab_run_uses_applied_context_overlay() {
    init_logging();
    let mut state = AppState::new();
    let prompt_id = PromptId::ArticleTriage;
    let mut contexts = HashMap::new();
    contexts.insert(prompt_id, vec![("prod".into(), "value".into())]);
    state.set_prompt_contexts(contexts);
    let context_snapshot = state.context_for(prompt_id).to_vec();
    state
        .prompt_lab_mut()
        .initialize_context_draft(prompt_id, &context_snapshot);
    state
        .prompt_lab_mut()
        .update_context_draft_text(prompt_id, "override=two".to_string());
    assert!(state.prompt_lab_mut().apply_context_draft(prompt_id));
    let effects = super::prompt_lab::dispatch_prompt_lab_run(
        &mut state,
        super::prompt_lab::PromptLabDispatchRequest {
            stage: PromptLabStage::Triage,
            prompt_id,
            input_snapshot: "input".to_string(),
            prompt_version: None,
            model_override: None,
            compare_batch_id: None,
            compare_candidate_id: None,
        },
    );
    assert_eq!(effects.len(), 1);
    if let Effect::RequestLlmCompletion { context, .. } = &effects[0] {
        assert_eq!(context, &vec![("override".into(), "two".into())]);
    } else {
        panic!("expected RequestLlmCompletion effect");
    }
}

#[test]
fn dispatch_prompt_lab_run_uses_production_context_without_overlay() {
    init_logging();
    let mut state = AppState::new();
    let prompt_id = PromptId::ArticleTriage;
    let mut contexts = HashMap::new();
    contexts.insert(prompt_id, vec![("prod".into(), "value".into())]);
    state.set_prompt_contexts(contexts);
    let effects = super::prompt_lab::dispatch_prompt_lab_run(
        &mut state,
        super::prompt_lab::PromptLabDispatchRequest {
            stage: PromptLabStage::Triage,
            prompt_id,
            input_snapshot: "input".to_string(),
            prompt_version: None,
            model_override: None,
            compare_batch_id: None,
            compare_candidate_id: None,
        },
    );
    assert_eq!(effects.len(), 1);
    if let Effect::RequestLlmCompletion { context, .. } = &effects[0] {
        assert_eq!(context, &vec![("prod".into(), "value".into())]);
    } else {
        panic!("expected RequestLlmCompletion effect");
    }
}

#[test]
fn prompt_lab_save_requested_without_applied_context_emits_no_effect() {
    init_logging();
    let state = AppState::new();
    let (_state, effects) = update(state, Msg::PromptLabContextSaveRequested);
    assert!(effects.is_empty());
}

#[test]
fn prompt_lab_save_requested_emits_save_effect() {
    init_logging();
    let mut state = AppState::new();
    let prompt_id = PromptId::ArticleTriage;
    let mut contexts = HashMap::new();
    contexts.insert(prompt_id, vec![("prod".into(), "value".into())]);
    state.set_prompt_contexts(contexts);
    let context_snapshot = state.context_for(prompt_id).to_vec();
    state
        .prompt_lab_mut()
        .initialize_context_draft(prompt_id, &context_snapshot);
    state
        .prompt_lab_mut()
        .update_context_draft_text(prompt_id, "override=two".to_string());
    assert!(state.prompt_lab_mut().apply_context_draft(prompt_id));
    let (_state, effects) = update(state, Msg::PromptLabContextSaveRequested);
    assert_eq!(effects.len(), 1);
    if let Effect::SavePromptContextFile { context_pairs, .. } = &effects[0] {
        assert_eq!(context_pairs, &vec![("override".into(), "two".into())]);
    } else {
        panic!("expected SavePromptContextFile effect");
    }
}

#[test]
fn prompt_lab_compare_start_dispatches_first_candidate() {
    init_logging();
    let mut state = AppState::new();
    prepare_type_url_snapshot(&mut state, "compare input");
    let (state, _) = update(state, Msg::PromptLabCompareCurrentSettingsCaptured);
    let (state, _) = update(state, Msg::PromptLabCompareBaselineCaptured);
    let (_state, effects) = update(state, Msg::PromptLabCompareBatchStartRequested);
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0], Effect::RequestLlmCompletion { .. }));
}

#[test]
fn prompt_lab_compare_completion_advances_to_next_candidate() {
    init_logging();
    let mut state = AppState::new();
    prepare_type_url_snapshot(&mut state, "compare input");
    let (state, _) = update(state, Msg::PromptLabCompareCurrentSettingsCaptured);
    let (state, _) = update(state, Msg::PromptLabCompareBaselineCaptured);
    let (state, effects) = update(state, Msg::PromptLabCompareBatchStartRequested);
    let first_request_id = effects
        .iter()
        .find_map(|effect| {
            if let Effect::RequestLlmCompletion { request_id, .. } = effect {
                Some(*request_id)
            } else {
                None
            }
        })
        .expect("first request id");
    let (_state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: first_request_id,
            result: LlmResultKind::Success {
                output_json: triage_json(),
                input_tokens: 5,
                output_tokens: 3,
                prompt_version: 1,
                resolved_model: "m".to_string(),
            },
            metadata: Some(LlmRunMetadata::stub()),
        },
    );
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0], Effect::RequestLlmCompletion { .. }));
}
