use super::summary_cache_support::summary_cache_model_ids_compatible;
use super::support::*;
use super::*;
use crate::LlmResultKind;
use harvester_engine::llm::{OPENAI_MODEL_GPT_4O, OPENAI_MODEL_GPT_4O_MINI};

#[test]
fn triage_clicked_emits_load_effects() {
    init_logging();
    let state = AppState::new();
    let (_state, effects) = update(state, Msg::TriageClicked);
    assert!(effects.is_empty());
}

#[test]
fn summary_cache_model_id_compatibility_accepts_resolved_suffix() {
    assert!(summary_cache_model_ids_compatible(
        OPENAI_MODEL_GPT_4O_MINI,
        "gpt-4o-mini-2024-07-18",
    ));
    assert!(!summary_cache_model_ids_compatible(
        OPENAI_MODEL_GPT_4O_MINI,
        OPENAI_MODEL_GPT_4O,
    ));
}

#[test]
fn briefing_blocked_when_triage_in_progress() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage(crate::triage::TriageSession::new_loading(None));
    let (next_state, effects) = update(state.clone(), Msg::GenerateBriefingClicked);
    assert!(effects.is_empty());
    assert!(matches!(
        next_state.briefing().phase(),
        crate::briefing::BriefingPhase::Failed { reason }
            if reason == "No completed triage. Run triage before generating a briefing."
    ));
    assert_eq!(next_state.active_tab(), AppTab::Briefing);
}

#[test]
fn triage_click_blocked_when_briefing_owns_triage() {
    init_logging();
    let state = AppState::new();
    let (state, _) = update(state, Msg::GenerateBriefingClicked);
    let (_state, effects) = update(state, Msg::TriageClicked);
    assert!(effects.is_empty());
}

#[test]
fn triage_articles_loaded_dispatches_up_to_limit_requests() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(2);
    let (state, effects) = start_triage_for_test(state, loaded_triage_articles(3));
    let llm_effects: Vec<_> = effects
        .iter()
        .filter(|e| matches!(e, Effect::RequestLlmCompletion { .. }))
        .collect();
    assert_eq!(
        llm_effects.len(),
        2,
        "should dispatch 2 requests for limit=2"
    );
    assert_eq!(state.triage().in_progress_count(), 2);
    assert_eq!(state.triage().pending_count(), 1);
}

#[test]
fn triage_completion_backfills_one_slot() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(2);
    let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));
    assert_eq!(state.triage().in_progress_count(), 2);

    let (state, effects) = update(state, triage_success(1));
    let llm_effects: Vec<_> = effects
        .iter()
        .filter(|e| matches!(e, Effect::RequestLlmCompletion { .. }))
        .collect();
    assert_eq!(llm_effects.len(), 1, "backfill dispatches 1 new request");
    assert_eq!(state.triage().in_progress_count(), 2);
    assert_eq!(state.triage().completed_count(), 1);
}

#[test]
fn triage_out_of_order_completion_routes_correctly() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(3);
    let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));
    assert_eq!(state.triage().in_progress_count(), 3);

    let (state, _) = update(state, triage_success(3));
    let (state, _) = update(state, triage_success(1));
    let (state, _) = update(state, triage_success(2));

    assert_eq!(state.triage().completed_count(), 3);
    assert_eq!(state.triage().failed_count(), 0);
    assert!(matches!(
        state.triage().phase(),
        crate::triage::TriagePhase::Complete
    ));
}

#[test]
fn triage_progress_text_counts_settled_articles() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(1);
    let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));
    let text = state.triage().progress_text().unwrap();
    assert!(
        text.contains("0/3"),
        "initial progress shows 0 settled: got '{text}'"
    );

    let (state, _) = update(state, triage_success(1));
    let text = state.triage().progress_text().unwrap();
    assert!(
        text.contains("1/3"),
        "after 1 complete shows 1 settled: got '{text}'"
    );
}

#[test]
fn triage_quota_exhausted_fails_all_pending() {
    init_logging();
    let mut state = AppState::new();
    state.set_triage_max_in_flight(1);
    let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));

    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: LlmResultKind::QuotaExhausted {
                reason: "too many calls".to_string(),
            },
            metadata: None,
        },
    );
    assert_eq!(state.triage().failed_count(), 3);
}
