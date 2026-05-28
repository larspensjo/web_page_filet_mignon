use super::support::*;
use super::*;
use crate::LlmResultKind;
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::run_metadata::LlmRunMetadata;

#[test]
fn briefing_aggregate_not_dispatched_until_all_articles_settled() {
    init_logging();
    let mut state = AppState::new();
    state.set_summary_max_in_flight(2);
    let state = start_briefing_after_triage(state, loaded_articles().0.clone());
    let (articles, collection_text) = loaded_articles();

    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    assert_eq!(state.briefing().in_progress_count(), 2);
    assert_eq!(state.briefing().pending_count(), 0);

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 3,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "test-model".to_string(),
            },
            metadata: None,
        },
    );
    let has_aggregate = effects.iter().any(|e| {
        matches!(
            e,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::AggregateBriefing,
                ..
            }
        )
    });
    assert!(
        !has_aggregate,
        "aggregate must not dispatch while article 2 still in-flight"
    );
    assert_eq!(state.briefing().in_progress_count(), 1);

    let (_state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 4,
            result: LlmResultKind::Success {
                output_json: summary_json("Article B"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "test-model".to_string(),
            },
            metadata: None,
        },
    );
    let has_aggregate = effects.iter().any(|e| {
        matches!(
            e,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::AggregateBriefing,
                ..
            }
        )
    });
    assert!(
        has_aggregate,
        "aggregate should dispatch after all articles settled"
    );
}

#[test]
fn startup_hydration_emits_load_briefing_history() {
    let state = AppState::new();
    let (_, effects) = update(state, Msg::StartupHydrationRequested);
    assert!(
        effects.contains(&Effect::LoadBriefingHistory),
        "expected LoadBriefingHistory in startup effects, got: {:?}",
        effects
    );
}

#[test]
fn briefing_history_loaded_sets_state() {
    use crate::briefing::BriefingHistoryEntry;

    let state = AppState::new();
    let entry = BriefingHistoryEntry {
        generated_at_utc: "2026-02-21T00:00:00Z".to_string(),
        executive_summary: "Test".to_string(),
        top_stories: vec![],
        article_count: 1,
    };
    let (state, effects) = update(
        state,
        Msg::BriefingHistoryLoaded {
            entries: vec![entry],
        },
    );
    assert_eq!(state.briefing_history().len(), 1);
    assert!(effects.is_empty());
}

fn run_single_article_briefing_to_completion(state: AppState) -> (AppState, Vec<Effect>) {
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, collection_text) = loaded_single_article();
    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "test-model".to_string(),
            },
            metadata: None,
        },
    );
    let aggregate_request_id = request_id_for_prompt(&effects, PromptId::AggregateBriefing)
        .expect("aggregate briefing request");
    update(
        state,
        Msg::LlmCompleted {
            request_id: aggregate_request_id,
            result: LlmResultKind::Success {
                output_json: briefing_json(1),
                input_tokens: 20,
                output_tokens: 8,
                prompt_version: 5,
                resolved_model: "test-model".to_string(),
            },
            metadata: None,
        },
    )
}

#[test]
fn briefing_completion_appends_history_and_emits_save() {
    init_logging();
    let state = AppState::new();
    let (state, effects) = run_single_article_briefing_to_completion(state);
    assert_eq!(
        state.briefing_history().len(),
        1,
        "history should have 1 entry"
    );
    let has_save = effects
        .iter()
        .any(|e| matches!(e, Effect::SaveBriefingHistory { .. }));
    assert!(
        has_save,
        "SaveBriefingHistory effect should be emitted after briefing completion"
    );
}

#[test]
fn prompt_lab_aggregate_completion_does_not_update_history() {
    init_logging();
    use crate::briefing::BriefingHistoryEntry;
    use crate::prompt_lab::PromptLabStage;

    let mut state = AppState::new();
    state.push_briefing_history(BriefingHistoryEntry {
        generated_at_utc: "2026-02-20T10:00:00Z".to_string(),
        executive_summary: "Old summary content.".to_string(),
        top_stories: vec![],
        article_count: 2,
    });
    let history_before = state.briefing_history().to_vec();
    let (state, _) = update(state, Msg::PromptLabOpenRequested);
    let (state, _) = update(
        state,
        Msg::PromptLabStageSelected {
            stage: PromptLabStage::Briefing,
        },
    );
    let mut state = state;
    prepare_type_url_snapshot(&mut state, "article text");
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    let request_id = request_id_for_prompt(&effects, PromptId::AggregateBriefing)
        .expect("expected prompt-lab aggregate briefing request");
    let (state, completion_effects) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: LlmResultKind::Success {
                output_json: briefing_json(1),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "test-model".to_string(),
            },
            metadata: Some(LlmRunMetadata::stub()),
        },
    );
    assert!(
        completion_effects
            .iter()
            .all(|e| !matches!(e, Effect::SaveBriefingHistory { .. })),
        "Prompt Lab completion must not emit SaveBriefingHistory"
    );
    assert_eq!(
        state.briefing_history(),
        history_before.as_slice(),
        "Prompt Lab runs must not mutate briefing history"
    );
}

#[test]
fn prompt_lab_aggregate_request_includes_previous_briefings_extra_var() {
    init_logging();
    use crate::briefing::BriefingHistoryEntry;
    use crate::prompt_lab::PromptLabStage;

    let mut state = AppState::new();
    state.push_briefing_history(BriefingHistoryEntry {
        generated_at_utc: "2026-02-20T10:00:00Z".to_string(),
        executive_summary: "Old summary content.".to_string(),
        top_stories: vec![],
        article_count: 2,
    });
    let (state, _) = update(state, Msg::PromptLabOpenRequested);
    let (state, _) = update(
        state,
        Msg::PromptLabStageSelected {
            stage: PromptLabStage::Briefing,
        },
    );
    let mut state = state;
    prepare_type_url_snapshot(&mut state, "article text");
    let (_state, effects) = update(state, Msg::PromptLabRunRequested);
    match effects.into_iter().find(|effect| {
        matches!(
            effect,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::AggregateBriefing,
                ..
            }
        )
    }) {
        Some(Effect::RequestLlmCompletion {
            extra_template_vars,
            ..
        }) => {
            let previous = extra_template_vars
                .iter()
                .find(|(key, _)| key == "previous_briefings");
            assert!(
                previous.is_some(),
                "missing previous_briefings in Prompt Lab aggregate request"
            );
            let (_, value) = previous.unwrap();
            assert!(
                value.contains("Old summary content."),
                "previous_briefings should contain history snapshot: {value}"
            );

            let window = extra_template_vars
                .iter()
                .find(|(key, _)| key == "briefing_time_window");
            assert!(
                window.is_some(),
                "missing briefing_time_window in Prompt Lab aggregate request"
            );
        }
        _ => panic!("no Prompt Lab AggregateBriefing RequestLlmCompletion effect emitted"),
    }
}

#[test]
fn format_block_contains_history_content() {
    use crate::briefing::{format_previous_briefings_block, BriefingHistoryEntry};

    let mut state = AppState::new();
    state.push_briefing_history(BriefingHistoryEntry {
        generated_at_utc: "2026-02-20T10:00:00Z".to_string(),
        executive_summary: "Old summary content.".to_string(),
        top_stories: vec![],
        article_count: 2,
    });
    let block = format_previous_briefings_block(state.briefing_history());
    assert!(block.contains("Old summary content."));
    assert!(block.contains("2026-02-20T10:00:00Z"));
}

#[test]
fn aggregate_briefing_effect_includes_previous_briefings_extra_var() {
    init_logging();
    use crate::briefing::BriefingHistoryEntry;

    let mut state = AppState::new();
    state.push_briefing_history(BriefingHistoryEntry {
        generated_at_utc: "2026-02-20T10:00:00Z".to_string(),
        executive_summary: "Old summary content.".to_string(),
        top_stories: vec![],
        article_count: 2,
    });
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, collection_text) = loaded_single_article();
    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    let (_state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "test-model".to_string(),
            },
            metadata: None,
        },
    );
    let completion_effect = effects.iter().find(|e| {
        matches!(
            e,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::AggregateBriefing,
                ..
            }
        )
    });
    match completion_effect {
        Some(Effect::RequestLlmCompletion {
            extra_template_vars,
            ..
        }) => {
            let pb = extra_template_vars
                .iter()
                .find(|(k, _)| k == "previous_briefings");
            assert!(
                pb.is_some(),
                "missing previous_briefings in extra_template_vars"
            );
            let (_, value) = pb.unwrap();
            assert!(
                value.contains("Old summary content."),
                "previous_briefings value should contain history: {value}"
            );
            let window = extra_template_vars
                .iter()
                .find(|(k, _)| k == "briefing_time_window");
            assert!(
                window.is_some(),
                "missing briefing_time_window in extra_template_vars"
            );
            let (_, window_value) = window.unwrap();
            assert!(
                window_value.contains("All available articles"),
                "briefing_time_window should describe all-time coverage: {window_value}"
            );
        }
        _ => panic!("no AggregateBriefing RequestLlmCompletion effect emitted"),
    }
}

#[test]
fn aggregate_briefing_effect_includes_checkpoint_time_window_extra_var() {
    init_logging();
    let mut state = AppState::new();
    let since = chrono::DateTime::parse_from_rfc3339("2026-02-24T12:34:56Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    state.set_briefing_since_utc(Some(since));
    let state = start_briefing_after_triage(state, loaded_single_article().0.clone());
    let (articles, collection_text) = loaded_single_article();
    let (state, _) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    let (_state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "test-model".to_string(),
            },
            metadata: None,
        },
    );

    let completion_effect = effects.iter().find(|e| {
        matches!(
            e,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::AggregateBriefing,
                ..
            }
        )
    });
    match completion_effect {
        Some(Effect::RequestLlmCompletion {
            extra_template_vars,
            ..
        }) => {
            let window = extra_template_vars
                .iter()
                .find(|(k, _)| k == "briefing_time_window")
                .expect("briefing_time_window extra var");
            assert!(
                window.1.contains("2026-02-24T12:34:56Z"),
                "window label should include checkpoint timestamp: {}",
                window.1
            );
        }
        _ => panic!("no AggregateBriefing RequestLlmCompletion effect emitted"),
    }
}
