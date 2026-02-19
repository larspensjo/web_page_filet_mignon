use std::sync::Once;

use harvester_core::{update, AppState, LlmResultKind, Msg};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::run_metadata::{CacheStatus, LlmRunMetadata, LlmRunMetadataInit};

fn init_logging() {
    static INIT: Once = Once::new();
    INIT.call_once(engine_logging::initialize_for_tests);
}

fn make_metadata(
    model: &str,
    input: u32,
    output: u32,
    cache_status: CacheStatus,
) -> LlmRunMetadata {
    LlmRunMetadata::new(LlmRunMetadataInit {
        prompt_id: PromptId::ArticleTriage,
        prompt_version: 1,
        resolved_model: model.to_string(),
        input_bytes: 100,
        input_tokens: input,
        output_tokens: output,
        cost_microdollars: 0,
        wall_ms: 10,
        parse_ok: true,
        validation_error: None,
        cache_status,
        timestamp_utc: "2026-01-01T00:00:00Z".to_string(),
    })
}

/// Send a LlmCompleted message (with a Failed result so no routing side-effects occur).
fn send_llm_completed(
    state: AppState,
    model: &str,
    input: u32,
    output: u32,
    cache_status: CacheStatus,
) -> AppState {
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 999,
            result: LlmResultKind::Failed {
                reason: "test".to_string(),
            },
            metadata: Some(make_metadata(model, input, output, cache_status)),
        },
    );
    state
}

#[test]
fn llm_usage_records_miss_by_model() {
    init_logging();
    let state = AppState::new();
    let state = send_llm_completed(state, "gpt-4o-mini", 100, 50, CacheStatus::Miss);
    let rows = state.llm_usage_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].model, "gpt-4o-mini");
    assert_eq!(rows[0].input_tokens, 100);
    assert_eq!(rows[0].output_tokens, 50);
}

#[test]
fn llm_usage_accumulates_across_completions() {
    init_logging();
    let state = AppState::new();
    let state = send_llm_completed(state, "gpt-4o-mini", 100, 50, CacheStatus::Miss);
    let state = send_llm_completed(state, "gpt-4o-mini", 200, 80, CacheStatus::Miss);
    let rows = state.llm_usage_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].input_tokens, 300);
    assert_eq!(rows[0].output_tokens, 130);
}

#[test]
fn llm_usage_ignores_hit_validated() {
    init_logging();
    let state = AppState::new();
    let state = send_llm_completed(state, "gpt-4o-mini", 100, 50, CacheStatus::HitValidated);
    assert!(state.llm_usage_rows().is_empty());
}

#[test]
fn llm_usage_ignores_hit_unvalidated() {
    init_logging();
    let state = AppState::new();
    let state = send_llm_completed(state, "gpt-4o-mini", 100, 50, CacheStatus::HitUnvalidated);
    assert!(state.llm_usage_rows().is_empty());
}

#[test]
fn llm_usage_ignores_empty_model_name() {
    init_logging();
    let state = AppState::new();
    let state = send_llm_completed(state, "", 100, 50, CacheStatus::Miss);
    assert!(state.llm_usage_rows().is_empty());
}

#[test]
fn llm_usage_ignores_whitespace_only_model_name() {
    init_logging();
    let state = AppState::new();
    let state = send_llm_completed(state, "   ", 100, 50, CacheStatus::Miss);
    assert!(state.llm_usage_rows().is_empty());
}

#[test]
fn llm_usage_saturates_at_u64_max() {
    init_logging();
    let mut state = AppState::new();
    // Fill near u64::MAX via many u32::MAX additions
    for _ in 0..3 {
        state.record_llm_usage_from_metadata(&make_metadata(
            "m",
            u32::MAX,
            u32::MAX,
            CacheStatus::Miss,
        ));
    }
    let rows = state.llm_usage_rows();
    // Values must accumulate without overflow/panic for large u32 inputs.
    let expected = u64::from(u32::MAX) * 3;
    assert_eq!(rows[0].input_tokens, expected);
    assert_eq!(rows[0].output_tokens, expected);
}

#[test]
fn view_contains_sorted_llm_usage_rows() {
    init_logging();
    let state = AppState::new();
    let state = send_llm_completed(state, "zmodel", 10, 5, CacheStatus::Miss);
    let state = send_llm_completed(state, "amodel", 20, 8, CacheStatus::Miss);
    let view = state.view();
    assert_eq!(view.llm_usage_by_model.len(), 2);
    assert_eq!(view.llm_usage_by_model[0].model, "amodel");
    assert_eq!(view.llm_usage_by_model[1].model, "zmodel");
}

#[test]
fn llm_usage_tracks_multiple_models_separately() {
    init_logging();
    let state = AppState::new();
    let state = send_llm_completed(state, "gpt-4o-mini", 100, 50, CacheStatus::Miss);
    let state = send_llm_completed(state, "gpt-4o", 200, 80, CacheStatus::Miss);
    let rows = state.llm_usage_rows();
    assert_eq!(rows.len(), 2);
    // BTreeMap guarantees alphabetical order
    assert_eq!(rows[0].model, "gpt-4o");
    assert_eq!(rows[0].input_tokens, 200);
    assert_eq!(rows[1].model, "gpt-4o-mini");
    assert_eq!(rows[1].input_tokens, 100);
}
