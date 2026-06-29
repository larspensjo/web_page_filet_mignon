use super::support::*;
use super::*;
use harvester_engine::llm::prompt::PromptId;
use std::collections::HashMap;

fn settled_summaries_state() -> AppState {
    let mut state = complete_triage_state_for_test(2);
    state = with_summary_metadata(state);
    seed_summaries_for_triage_hashes(&mut state, 2);
    state
}

fn settled_summaries_state_without_hydration() -> AppState {
    let mut state = complete_triage_state_for_test(2);
    seed_summaries_for_triage_hashes(&mut state, 2);
    state
}

fn prompt_contexts_loaded_msg() -> Msg {
    let mut contexts = HashMap::new();
    contexts.insert(
        PromptId::BriefingExecutiveSummary,
        vec![("policy".to_string(), "briefing context".to_string())],
    );
    contexts.insert(
        PromptId::BriefingNextItem,
        vec![("policy".to_string(), "briefing context".to_string())],
    );
    Msg::PromptContextsLoaded { contexts }
}

fn llm_metadata_loaded_msg() -> Msg {
    let mut active_versions = HashMap::new();
    active_versions.insert(PromptId::ArticleTriage, 1);
    active_versions.insert(PromptId::ArticleSummary, 1);
    active_versions.insert(PromptId::BriefingExecutiveSummary, 1);
    active_versions.insert(PromptId::BriefingNextItem, 1);
    let mut effective_models = HashMap::new();
    effective_models.insert(PromptId::ArticleTriage, "test-triage-model".to_string());
    effective_models.insert(PromptId::ArticleSummary, "test-summary-model".to_string());
    effective_models.insert(
        PromptId::BriefingExecutiveSummary,
        "test-briefing-model".to_string(),
    );
    effective_models.insert(
        PromptId::BriefingNextItem,
        "test-briefing-model".to_string(),
    );
    Msg::LlmMetadataLoaded {
        active_versions,
        effective_models,
        templates: HashMap::new(),
    }
}

fn prompt_template_files_loaded_msg() -> Msg {
    Msg::PromptTemplateFilesLoaded
}

fn success(json: &str) -> LlmResultKind {
    LlmResultKind::Success {
        output_json: json.to_string(),
        input_tokens: 1,
        output_tokens: 1,
        prompt_version: 1,
        resolved_model: "m".to_string(),
    }
}

fn first_request_id(effects: &[Effect], expected_prompt: PromptId) -> u64 {
    request_id_for_prompt(effects, expected_prompt).expect("expected request")
}

fn enter_streaming_state() -> AppState {
    let (state, effects) = update(settled_summaries_state(), Msg::GenerateBriefingClicked);
    let exec_id = first_request_id(&effects, PromptId::BriefingExecutiveSummary);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: exec_id,
            result: success(r#"{"executive_summary":"Synthesis."}"#),
            metadata: None,
        },
    );
    state
}

#[test]
fn generate_without_hydration_defers_then_dispatches_after_hydration() {
    init_logging();
    let (state, effects) = update(
        settled_summaries_state_without_hydration(),
        Msg::GenerateBriefingClicked,
    );

    assert!(state.briefing().exec_dispatch_deferred());
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::RequestLlmCompletion { .. })));
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::LoadPromptContexts)));
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::LoadPromptTemplateFiles)));
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::LoadLlmMetadata)));

    let (state, effects) = update(state, prompt_contexts_loaded_msg());
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::RequestLlmCompletion { .. })));

    let (state, effects) = update(state, prompt_template_files_loaded_msg());
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::RequestLlmCompletion { .. })));
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::LoadLlmMetadata)));

    let (state, effects) = update(state, llm_metadata_loaded_msg());
    let exec_id = first_request_id(&effects, PromptId::BriefingExecutiveSummary);
    assert!(exec_id > 0);
    assert!(!state.briefing().exec_dispatch_deferred());
    assert!(state.briefing().has_active_llm_request());
}

#[test]
fn next_item_clicked_before_exec_summary_is_noop() {
    init_logging();
    let (state, _) = update(settled_summaries_state(), Msg::GenerateBriefingClicked);
    let (state, effects) = update(state, Msg::NextBriefingItemClicked);

    assert!(state.briefing().summaries_snapshot().is_some());
    assert!(state.briefing().executive_summary().is_none());
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::RequestLlmCompletion { .. })));
}

#[test]
fn exec_completion_enters_streaming_and_writes_no_history() {
    init_logging();
    let (state, effects) = update(settled_summaries_state(), Msg::GenerateBriefingClicked);
    let exec_id = first_request_id(&effects, PromptId::BriefingExecutiveSummary);

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: exec_id,
            result: success(r#"{"executive_summary":"Synthesis."}"#),
            metadata: None,
        },
    );

    assert_eq!(state.briefing().executive_summary(), Some("Synthesis."));
    assert!(state.briefing().next_item_enabled());
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::SaveBriefingHistory { .. })));
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::PersistSummaryCache { .. })));
}

#[test]
fn next_item_emits_item_call_with_already_shown_suffix() {
    init_logging();
    let mut state = enter_streaming_state();
    state
        .briefing_mut()
        .append_stream_item(crate::BriefingItem {
            headline: "Already shown".to_string(),
            body: "x".to_string(),
        });

    let (state, effects) = update(state, Msg::NextBriefingItemClicked);
    let call = effects.iter().find_map(|effect| match effect {
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::BriefingNextItem,
            input_content,
            extra_template_vars,
            ..
        } => Some((input_content.clone(), extra_template_vars.clone())),
        _ => None,
    });
    let (input, extra) = call.expect("next-item call");

    assert_eq!(Some(input.as_str()), state.briefing().summaries_snapshot());
    assert!(extra
        .iter()
        .any(|(key, value)| key == "already_shown" && value.contains("Already shown")));
    assert!(extra.iter().any(|(key, _)| key == "briefing_time_window"));
    assert!(state.briefing().next_item_request_id().is_some());
}

#[test]
fn item_completion_appends_then_exhausts() {
    init_logging();
    let (state, effects) = update(enter_streaming_state(), Msg::NextBriefingItemClicked);
    let item_id = first_request_id(&effects, PromptId::BriefingNextItem);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: item_id,
            result: success(r#"{"status":"item","headline":"H1","body":"B1"}"#),
            metadata: None,
        },
    );
    assert_eq!(state.briefing().stream_items().len(), 1);
    assert!(state.briefing().next_item_request_id().is_none());

    let (state, effects) = update(state, Msg::NextBriefingItemClicked);
    let item_id = first_request_id(&effects, PromptId::BriefingNextItem);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: item_id,
            result: success(r#"{"status":"exhausted"}"#),
            metadata: None,
        },
    );
    assert!(state.briefing().exhausted());
    assert!(!state.briefing().next_item_enabled());
}

#[test]
fn item_failure_keeps_next_enabled_and_does_not_append() {
    init_logging();
    let (state, effects) = update(enter_streaming_state(), Msg::NextBriefingItemClicked);
    let item_id = first_request_id(&effects, PromptId::BriefingNextItem);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: item_id,
            result: LlmResultKind::Failed {
                reason: "boom".to_string(),
            },
            metadata: None,
        },
    );

    assert!(state.briefing().stream_items().is_empty());
    assert!(state.briefing().next_item_request_id().is_none());
    assert!(state.briefing().next_item_enabled());
}

#[test]
fn stale_next_item_completion_from_discarded_stream_is_ignored() {
    init_logging();
    let (state, effects) = update(enter_streaming_state(), Msg::NextBriefingItemClicked);
    let stale_item_id = first_request_id(&effects, PromptId::BriefingNextItem);

    let (state, _) = update(state, Msg::GenerateBriefingClicked);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: stale_item_id,
            result: success(r#"{"status":"item","headline":"stale","body":"b"}"#),
            metadata: None,
        },
    );

    assert!(state.briefing().stream_items().is_empty());
}

#[test]
fn streaming_with_item_in_flight_counts_as_active_work() {
    init_logging();
    let mut state = enter_streaming_state();
    state.start_session();
    let (state, _) = update(state, Msg::NextBriefingItemClicked);

    assert!(state.briefing().next_item_in_flight());
    assert!(state.view().stop_finish_button.is_enabled());
}

#[test]
fn view_exposes_next_item_enabled_and_keeps_generate_enabled_mid_stream() {
    init_logging();
    let state = enter_streaming_state();

    let view = state.view();
    assert!(view.next_item_enabled);
    assert!(view.briefing_generate_enabled);
}
