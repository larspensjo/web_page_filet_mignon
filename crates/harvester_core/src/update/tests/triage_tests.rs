use super::summary_cache_support::summary_cache_model_ids_compatible;
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
fn fresh_triage_completion_records_snapshot_model_provenance() {
    init_logging();
    let (state, effects) = start_triage_for_test(AppState::new(), loaded_triage_articles(1));
    let request_id = request_id_for_prompt(
        &effects,
        harvester_engine::llm::prompt::PromptId::ArticleTriage,
    )
    .expect("triage request");
    let (state, _) = update(state, triage_success(request_id));

    assert_eq!(
        state.triage().triage_model_for_url("https://example.com/0"),
        Some("test-model")
    );
}

#[test]
fn key_unavailable_triage_completion_exports_priority_without_model_provenance() {
    use harvester_engine::archive_url_key;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let article = crate::briefing::LoadedArticle {
        url: "https://example.com/no-key".to_string(),
        source_title: None,
        prepared_text: "article content".to_string(),
        content_hash: String::new(),
        fetched_utc: None,
    };
    let mut state = prime_llm_metadata(AppState::new());
    let mut triage = crate::triage::TriageSession::new_loading(None);
    triage.set_articles(vec![article]);
    triage.transition_to_triaging();
    state.set_triage(triage);
    state.start_triage_cache_run();
    state.mark_triage_metadata_ready();
    let mut effects = Vec::new();
    crate::update::triage::dispatch_next_triage_step(&mut state, &mut effects);
    let request_id =
        request_id_for_prompt(&effects, PromptId::ArticleTriage).expect("triage request");
    let (state, _) = update(state, triage_success(request_id));
    assert_eq!(
        state
            .triage()
            .triage_model_for_url("https://example.com/no-key"),
        None
    );

    let (state, _) = update(state, Msg::ArchiveClicked);
    let archive_request_id = state.archive_request_id();
    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id: archive_request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let annotations = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ArchiveRequested { annotations, .. } => Some(annotations),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    let annotation = &annotations[&archive_url_key("https://example.com/no-key")];
    assert_eq!(annotation.priority, Some(3));
    assert!(annotation.triage_model.is_none());
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
                origin: harvester_engine::llm::QuotaOrigin::SessionBudget,
            },
            metadata: None,
        },
    );
    assert_eq!(state.triage().failed_count(), 3);
}
