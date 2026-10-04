use std::sync::Once;

use harvester_core::{
    update, AppState, Effect, LlmRequestState, LlmResultKind, Msg, SessionState, StopPolicy,
};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::{ExtractedLink, LinkKind};

fn init_logging() {
    static INIT: Once = Once::new();
    INIT.call_once(engine_logging::initialize_for_tests);
}

fn submit_urls(state: AppState, input: &str) -> (AppState, Vec<Effect>) {
    let (state, _) = update(state, Msg::InputChanged(input.to_string()));
    update(state, Msg::UrlsSubmitted)
}

#[test]
fn urls_pasted_trims_and_ignores_empty() {
    init_logging();
    let state = AppState::new();
    let input = "https://a.example.com \n\n  https://b.example.com\n   \n";

    let (next, effects) = submit_urls(state, input);
    let view = next.view();

    assert_eq!(
        next.batch_observation().session_state,
        SessionState::Running
    );
    assert_eq!(view.job_count, 2);
    assert!({
        let mut state = next.clone();
        state.consume_dirty()
    });
    assert_eq!(
        effects,
        vec![
            Effect::StartSession,
            Effect::EnqueueUrl {
                job_id: 1,
                url: "https://a.example.com".to_string(),
            },
            Effect::EnqueueUrl {
                job_id: 2,
                url: "https://b.example.com".to_string(),
            },
        ]
    );

    let (next, effects) = submit_urls(next, "   \n\n");
    assert_eq!(next.view().job_count, 2);
    assert!(effects.is_empty());
}

#[test]
fn stop_finish_moves_running_to_finishing() {
    init_logging();
    let state = AppState::new();
    let (state, _effects) = submit_urls(state, "https://example.com\n");
    let (state, _effects) = update(state, Msg::StopFinishClicked);

    assert_eq!(
        state.batch_observation().session_state,
        SessionState::Finishing
    );
    assert_eq!(
        state.view().run_state,
        harvester_core::RunState::Stopping { in_flight: 0 }
    );
    assert!(!state.view().archive_enabled);
    assert!({
        let mut snapshot = state.clone();
        snapshot.consume_dirty()
    });
}

#[test]
fn stop_finish_emits_effect() {
    init_logging();
    let state = AppState::new();
    let (state, _effects) = submit_urls(state, "https://example.com\n");
    let (_state, effects) = update(state, Msg::StopFinishClicked);

    assert_eq!(
        effects,
        vec![
            Effect::StopFinish {
                policy: StopPolicy::Finish
            },
            Effect::FlushResults,
            Effect::PersistRuntimeState {
                snapshot: harvester_core::PersistenceSnapshot {
                    fetch_time_recovery_done: false,
                    job_list_mode: Some(Default::default()),
                    selected_article_url: None,
                    completed: Vec::new(),
                    pending_intake: vec!["https://example.com".to_string()],
                    blacklist: Default::default(),
                }
            }
        ]
    );
}

#[test]
fn stop_finish_click_is_ignored_after_work_has_already_settled() {
    init_logging();
    let state = AppState::new();
    let (state, _effects) = submit_urls(state, "https://example.com\n");
    let (state, _effects) = update(state, Msg::StopFinishClicked);

    let (next, effects) = update(state, Msg::StopFinishClicked);

    assert_eq!(
        next.batch_observation().session_state,
        SessionState::Finishing
    );
    assert!(effects.is_empty());
}

#[test]
fn urls_pasted_again_after_stop_drain() {
    init_logging();
    let state = AppState::new();
    let (state, _effects) = submit_urls(state, "https://example.com\n");
    let (state, _effects) = update(state, Msg::StopFinishClicked);
    let job_count = state.view().job_count;
    let (state, during_drain) = submit_urls(state, "https://a.example.com\n");
    assert!(during_drain.is_empty());
    assert_eq!(state.view().job_count, job_count);
    let (mut state, _) = update(
        state,
        Msg::JobDone {
            job_id: 1,
            result: harvester_core::JobResultKind::Failed {
                reason: "cancelled from Stop queue drain".into(),
            },

            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );
    assert_eq!(state.batch_observation().session_state, SessionState::Idle);
    assert_eq!(state.view().run_state, harvester_core::RunState::Idle);
    assert!(state.consume_dirty());

    let (next, effects) = submit_urls(state, "https://a.example.com\n");

    assert_eq!(
        next.batch_observation().session_state,
        SessionState::Running
    );
    assert_eq!(next.view().job_count, 2);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::EnqueueUrl {
            job_id: 2,
            url
        } if url == "https://a.example.com"
    )));
}

#[test]
fn urls_pasted_while_running_stays_running() {
    init_logging();
    let state = AppState::new();
    // First paste: Idle -> Running
    let (state, effects) = submit_urls(state, "https://first.example.com\n");
    assert_eq!(
        state.batch_observation().session_state,
        SessionState::Running
    );
    assert_eq!(effects.len(), 2); // StartSession + EnqueueUrl

    // Second paste while Running: should stay Running, no StartSession
    let (state, effects) = submit_urls(state, "https://second.example.com\n");
    assert_eq!(
        state.batch_observation().session_state,
        SessionState::Running
    );
    assert_eq!(state.view().job_count, 2);
    assert_eq!(
        effects,
        vec![Effect::EnqueueUrl {
            job_id: 2,
            url: "https://second.example.com".to_string(),
        }]
    );
}

#[test]
fn duplicate_paste_skipped() {
    init_logging();
    let state = AppState::new();
    // First paste
    let (state, effects) = submit_urls(state, "https://example.com\n");
    assert_eq!(state.view().job_count, 1);
    assert_eq!(effects.len(), 2); // StartSession + EnqueueUrl
    let view = state.view();
    assert_eq!(view.last_paste_stats.as_ref().unwrap().enqueued, 1);
    assert_eq!(view.last_paste_stats.as_ref().unwrap().skipped, 0);

    // Second paste with same URL - should be skipped
    let (state, effects) = submit_urls(state, "https://example.com\n");
    assert_eq!(state.view().job_count, 1); // No new job
    assert_eq!(effects.len(), 0); // No effects
    let view = state.view();
    assert_eq!(view.last_paste_stats.as_ref().unwrap().enqueued, 0);
    assert_eq!(view.last_paste_stats.as_ref().unwrap().skipped, 1);
}

#[test]
fn url_normalization_catches_variants() {
    init_logging();
    let state = AppState::new();
    // First paste with trailing slash
    let (state, effects) = submit_urls(state, "https://example.com/\n");
    assert_eq!(state.view().job_count, 1);
    assert_eq!(effects.len(), 2);

    // Second paste without trailing slash - should be recognized as duplicate
    let (state, effects) = submit_urls(state, "https://example.com\n");
    assert_eq!(state.view().job_count, 1);
    assert_eq!(effects.len(), 0);
    assert_eq!(state.view().last_paste_stats.as_ref().unwrap().skipped, 1);

    // Third paste with different case - should be recognized as duplicate
    let (state, effects) = submit_urls(state, "HTTPS://EXAMPLE.COM\n");
    assert_eq!(state.view().job_count, 1);
    assert_eq!(effects.len(), 0);
    assert_eq!(state.view().last_paste_stats.as_ref().unwrap().skipped, 1);

    // Fourth paste with extra whitespace - should be recognized as duplicate
    let (state, effects) = submit_urls(state, "  https://example.com/  \n");
    assert_eq!(state.view().job_count, 1);
    assert_eq!(effects.len(), 0);
    assert_eq!(state.view().last_paste_stats.as_ref().unwrap().skipped, 1);
}

#[test]
fn paste_with_mixed_new_and_duplicate_urls() {
    init_logging();
    let state = AppState::new();
    // First paste with two URLs
    let (state, effects) = submit_urls(state, "https://a.example.com\nhttps://b.example.com\n");
    assert_eq!(state.view().job_count, 2);
    assert_eq!(effects.len(), 3); // StartSession + 2x EnqueueUrl
    let view = state.view();
    assert_eq!(view.last_paste_stats.as_ref().unwrap().enqueued, 2);
    assert_eq!(view.last_paste_stats.as_ref().unwrap().skipped, 0);

    // Second paste with one duplicate and one new URL
    let (state, effects) = submit_urls(state, "https://a.example.com\nhttps://c.example.com\n");
    assert_eq!(state.view().job_count, 3);
    assert_eq!(effects.len(), 1); // Only 1 EnqueueUrl (c.example.com)
    let view = state.view();
    assert_eq!(view.last_paste_stats.as_ref().unwrap().enqueued, 1);
    assert_eq!(view.last_paste_stats.as_ref().unwrap().skipped, 1);
}

#[test]
fn archive_click_emits_effect_without_state_change() {
    init_logging();
    let state = AppState::new();
    let before = state.view();

    let (next, effects) = update(state, Msg::ArchiveClicked);

    assert_eq!(next.view(), before);
    assert_eq!(effects.len(), 1);
    let Effect::OpenArchiveDialog {
        request_id,
        article_count,
        since_utc,
        default_basename,
        pending_pre_triage_count,
        token_estimates,
        ..
    } = &effects[0]
    else {
        panic!("expected OpenArchiveDialog effect, got {:?}", effects[0]);
    };
    assert!(*request_id > 0);
    assert_eq!(*article_count, 0);
    assert!(since_utc.is_none());
    assert_eq!(default_basename, "archive.md");
    assert_eq!(*pending_pre_triage_count, 0);
    assert_eq!(
        *token_estimates,
        harvester_core::ArchiveTokenEstimates::default()
    );
}

fn send_llm_request_with_context(state: AppState) -> (AppState, Vec<Effect>) {
    let (state, _) = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: [(PromptId::ArticleTriage, 1)].into(),
            effective_models: [(PromptId::ArticleTriage, "test-model".into())].into(),
        },
    );
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Resume,
        },
    );
    let (state, effects) =
        harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000);
    let request_id = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("preparation request");
    let articles = (0..2)
        .map(|index| harvester_core::LoadedArticle {
            url: format!("https://example.com/{index}"),
            source_title: None,
            prepared_text: std::iter::repeat_n("content", 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("hash-{index}"),
            fetched_utc: None,
        })
        .collect();
    let (state, effects) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    );
    (
        state,
        effects
            .into_iter()
            .filter(|e| matches!(e, Effect::RequestLlmCompletion { .. }))
            .collect(),
    )
}

fn extract_request_id(effect: &Effect) -> u64 {
    if let Effect::RequestLlmCompletion { request_id, .. } = effect {
        *request_id
    } else {
        panic!("expected RequestLlmCompletion effect")
    }
}

#[test]
fn request_llm_completion_emits_effect_and_tracks_pending() {
    init_logging();
    let (state, effects) = send_llm_request_with_context(AppState::new());
    assert_eq!(effects.len(), 1);
    let request_id = extract_request_id(&effects[0]);
    assert_eq!(
        state.llm_request_state(request_id),
        Some(&LlmRequestState::Pending {
            prompt_id: PromptId::ArticleTriage
        })
    );
}

#[test]
fn llm_completed_success_updates_state() {
    init_logging();
    let (state, effects) = send_llm_request_with_context(AppState::new());
    let request_id = extract_request_id(&effects[0]);
    let json = r#"{"category":"news","priority":1,"tags":[],"rationale":"ok"}"#.to_string();
    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Success {
                output_json: json.clone(),
                input_tokens: 5,
                output_tokens: 10,
                prompt_version: 1,
                resolved_model: "test-model".to_string(),
            },
            metadata: None,
        },
    );
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SaveResults { .. })));
    assert_eq!(
        state.llm_request_state(request_id),
        Some(&LlmRequestState::Completed {
            output_json: json,
            input_tokens: 5,
            output_tokens: 10,
        })
    );
}

#[test]
fn llm_completed_unknown_request_is_ignored() {
    init_logging();
    let (state, effects) = update(
        AppState::new(),
        Msg::LlmCompleted {
            request_id: 42,
            result: LlmResultKind::Failed {
                reason: "not found".to_string(),
            },
            metadata: None,
        },
    );
    assert!(effects.is_empty());
    assert!(state.llm_request_state(42).is_none());
}

#[test]
fn request_ids_monotonically_increase() {
    init_logging();
    let (state, effects_a) = send_llm_request_with_context(AppState::new());
    let request_id_a = extract_request_id(&effects_a[0]);
    let (_state, effects_b) = update(
        state,
        Msg::LlmCompleted {
            request_id: request_id_a,
            result: LlmResultKind::Failed {
                reason: "fixture".into(),
            },
            metadata: None,
        },
    );
    let request_id_b = extract_request_id(&effects_b[0]);
    assert!(request_id_b > request_id_a);
}

#[test]
fn extracted_links_do_not_bypass_stop_intake_drain() {
    init_logging();
    let (state, _) = submit_urls(
        AppState::new(),
        "https://source.example/article\nhttps://pending.example/article\n",
    );
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id: 1,
            result: harvester_core::JobResultKind::Success,
            extracted_links: vec![ExtractedLink {
                url: "https://linked.example/article/one".into(),
                text: None,
                kind: LinkKind::Hyperlink,
            }],
            fetched_utc: None,
        },
    );
    let (state, _) = update(state, Msg::StopFinishClicked);
    assert!(matches!(
        state.view().run_state,
        harvester_core::RunState::Stopping { .. }
    ));
    let job_count = state.view().job_count;
    let (_, open_effects) = update(
        state.clone(),
        Msg::ExtractedLinkOpenRequested {
            job_id: 1,
            link_index: 0,
        },
    );
    assert_eq!(
        open_effects,
        vec![Effect::OpenUrlInBrowser {
            url: "https://linked.example/article/one".into()
        }]
    );
    let (state, effects) = submit_urls(state, "https://linked.example/article/one");
    assert!(effects.is_empty());
    assert_eq!(state.view().job_count, job_count);
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id: 2,
            result: harvester_core::JobResultKind::Failed {
                reason: "cancelled from Stop queue drain".into(),
            },
            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );
    let (state, effects) = submit_urls(state, "https://linked.example/article/one");
    assert_eq!(state.view().job_count, job_count + 1);
    assert!(matches!(effects.first(), Some(Effect::StartSession)));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::EnqueueUrl { url, .. } if url == "https://linked.example/article/one"
    )));
}
