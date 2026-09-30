use super::*;
use crate::ProviderAlert;
use harvester_engine::llm::QuotaOrigin;

fn summary_articles(count: usize) -> (Vec<crate::briefing::LoadedArticle>, String) {
    let articles = (0..count)
        .map(|i| crate::briefing::LoadedArticle {
            url: format!("https://provider-alert.example/{i}"),
            source_title: Some(format!("Article {i}")),
            prepared_text: std::iter::repeat_n(format!("article-{i}-content"), 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("provider-alert-hash-{i}"),
            fetched_utc: None,
        })
        .collect();
    (articles, "Provider alert collection".to_string())
}

fn start_summary_run(count: usize, max_in_flight: usize) -> (AppState, Vec<u64>) {
    let mut state = AppState::new();
    state.set_llm_max_in_flight(max_in_flight);
    let (articles, _collection_text) = summary_articles(count);
    let state = start_briefing_after_triage(state, articles.clone());
    let (state, effects) = crate::update::test_support::summarize(state, articles);
    let request_ids = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: harvester_engine::llm::prompt::PromptId::ArticleSummary,
                ..
            } => Some(*request_id),
            _ => None,
        })
        .collect();
    (state, request_ids)
}

fn rate_limited_result() -> LlmResultKind {
    LlmResultKind::RateLimited {
        reason: "provider rate limited".to_string(),
    }
}

fn success_result(title: &str) -> LlmResultKind {
    LlmResultKind::Success {
        output_json: summary_json(title),
        input_tokens: 10,
        output_tokens: 5,
        prompt_version: 1,
        resolved_model: "test-model".to_string(),
    }
}

fn quota_result(origin: QuotaOrigin) -> LlmResultKind {
    LlmResultKind::QuotaExhausted {
        reason: match origin {
            QuotaOrigin::Provider => "provider quota exhausted: billing",
            QuotaOrigin::SessionBudget => "session call quota exhausted",
        }
        .to_string(),
        origin,
    }
}

fn deliver(state: AppState, request_id: u64, result: LlmResultKind) -> (AppState, Vec<Effect>) {
    update(
        state,
        Msg::LlmCompleted {
            request_id,
            result,
            metadata: None,
        },
    )
}

fn next_summary_request(effects: &[Effect]) -> Option<u64> {
    request_id_for_prompt(
        effects,
        harvester_engine::llm::prompt::PromptId::ArticleSummary,
    )
}

#[test]
fn three_consecutive_rate_limited_summaries_stop_run_and_raise_banner() {
    init_logging();
    let (state, requests) = start_summary_run(5, 1);
    let request_id = requests[0];
    let (state, effects) = deliver(state, request_id, rate_limited_result());
    let request_id = next_summary_request(&effects).expect("second summary request");
    let (state, effects) = deliver(state, request_id, rate_limited_result());
    let request_id = next_summary_request(&effects).expect("third summary request");
    let (state, _) = deliver(state, request_id, rate_limited_result());

    assert_eq!(state.briefing().pending_count(), 0);
    assert!(state.briefing().failed_summary_count() >= 3);
    assert!(matches!(
        state.provider_alert(),
        Some(ProviderAlert::RateLimited)
    ));
    let banner = state.view().ai_warning_banner.expect("warning banner");
    assert!(banner.title.contains("rate limiting"));
}

#[test]
fn single_rate_limited_summary_does_not_stop_run() {
    init_logging();
    let (state, requests) = start_summary_run(5, 1);
    let (state, effects) = deliver(state, requests[0], rate_limited_result());
    let next = next_summary_request(&effects).expect("next summary request");
    let (state, _) = deliver(state, next, success_result("Article 1"));

    assert!(state.provider_alert().is_none());
    assert_eq!(state.briefing().failed_summary_count(), 1);
    assert!(state.briefing().pending_count() > 0 || state.briefing().in_progress_count() > 0);
}

#[test]
fn interleaved_unrelated_success_does_not_reset_counter() {
    init_logging();
    let (state, requests) = start_summary_run(5, 1);
    let (state, effects) = deliver(state, requests[0], rate_limited_result());
    let second = next_summary_request(&effects).expect("second summary request");
    let (state, effects) = deliver(state, second, rate_limited_result());
    let third = next_summary_request(&effects).expect("third summary request");
    let (state, _) = deliver(state, 99_999, success_result("unrelated"));
    let (state, _) = deliver(state, third, rate_limited_result());

    assert!(matches!(
        state.provider_alert(),
        Some(ProviderAlert::RateLimited)
    ));
}

#[test]
fn provider_quota_exhausted_summary_raises_credits_banner_immediately() {
    init_logging();
    let (state, requests) = start_summary_run(5, 1);
    let (state, _) = deliver(state, requests[0], quota_result(QuotaOrigin::Provider));

    assert!(matches!(
        state.provider_alert(),
        Some(ProviderAlert::OutOfCredits { .. })
    ));
    let banner = state.view().ai_warning_banner.expect("warning banner");
    assert!(banner.body.contains("credits"));
    assert_eq!(state.briefing().pending_count(), 0);
    assert_eq!(state.briefing().failed_summary_count(), 5);
}

#[test]
fn session_budget_quota_exhausted_stops_run_with_session_limit_banner() {
    init_logging();
    let (state, requests) = start_summary_run(5, 1);
    let (state, _) = deliver(state, requests[0], quota_result(QuotaOrigin::SessionBudget));

    assert!(state.provider_alert().is_none());
    assert!(state
        .view()
        .ai_warning_banner
        .unwrap()
        .title
        .contains("session call limit"));
    assert_eq!(state.briefing().pending_count(), 0);
    assert_eq!(state.briefing().failed_summary_count(), 5);
}

#[test]
fn prepare_summaries_start_clears_provider_alert() {
    init_logging();
    let mut state = with_summary_metadata(complete_triage_state_for_test(2));
    state.note_provider_out_of_credits("provider quota exhausted: billing".to_string());

    let (state, _) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );

    assert!(state.provider_alert().is_none());
    assert!(state.view().ai_warning_banner.is_none());
}

#[test]
fn triage_start_clears_provider_alert() {
    init_logging();
    let mut state = AppState::new();
    state.note_provider_out_of_credits("provider quota exhausted: billing".to_string());

    let (state, _) = start_triage_for_test(state, loaded_triage_articles(3));

    assert!(state.provider_alert().is_none());
    assert!(state.view().ai_warning_banner.is_none());
}

#[test]
fn stale_quota_completion_after_new_run_start_does_not_raise_banner() {
    init_logging();
    let (mut state, requests) = start_summary_run(3, 2);
    assert_eq!(requests.len(), 2);
    // Replacing the session leaves the old worker completion unowned.
    state.set_briefing(crate::briefing::BriefingSession::new_loading());
    assert!(state.provider_alert().is_none());
    let (state, _) = deliver(state, requests[1], quota_result(QuotaOrigin::SessionBudget));

    assert!(state.provider_alert().is_none());
    assert!(state.model_dispatch_halt_reason().is_none());
    assert!(state.view().ai_warning_banner.is_none());
}

#[test]
fn provider_credit_halt_resets_when_triage_starts_again() {
    init_logging();
    let mut state = AppState::new();
    state.set_llm_max_in_flight(1);
    let (state, effects) = start_triage_for_test(state, loaded_triage_articles(3));
    let id = request_id_for_prompt(
        &effects,
        harvester_engine::llm::prompt::PromptId::ArticleTriage,
    )
    .unwrap();
    let (state, _) = deliver(state, id, quota_result(QuotaOrigin::Provider));
    assert!(state.model_dispatch_halt_reason().is_some());
    let (state, effects) = start_triage_for_test(state, loaded_triage_articles(2));
    assert!(request_id_for_prompt(
        &effects,
        harvester_engine::llm::prompt::PromptId::ArticleTriage
    )
    .is_some());
    assert!(state.model_dispatch_halt_reason().is_none());
    assert!(state.provider_alert().is_none());
}

#[test]
fn session_quota_halt_persists_across_triage_start_with_visible_reason() {
    init_logging();
    let mut state = AppState::new();
    state.set_llm_max_in_flight(1);
    let (state, effects) = start_triage_for_test(state, loaded_triage_articles(3));
    let id = request_id_for_prompt(
        &effects,
        harvester_engine::llm::prompt::PromptId::ArticleTriage,
    )
    .unwrap();
    let (state, _) = deliver(state, id, quota_result(QuotaOrigin::SessionBudget));
    let (state, effects) = start_triage_for_test(state, loaded_triage_articles(2));
    assert!(request_id_for_prompt(
        &effects,
        harvester_engine::llm::prompt::PromptId::ArticleTriage
    )
    .is_none());
    assert!(state.model_dispatch_halt_reason().is_some());
    let banner = state.view().ai_warning_banner.unwrap();
    assert!(banner.title.contains("session call limit"));
    assert!(banner.body.contains("Restart Harvester"));
}

#[test]
fn three_consecutive_rate_limited_triage_results_stop_triage_run() {
    init_logging();
    let mut state = AppState::new();
    state.set_llm_max_in_flight(1);
    let (mut state, effects) = start_triage_for_test(state, loaded_triage_articles(5));
    let mut request_id = request_id_for_prompt(
        &effects,
        harvester_engine::llm::prompt::PromptId::ArticleTriage,
    )
    .expect("first triage request");

    for _ in 0..3 {
        let (next_state, effects) = deliver(state, request_id, rate_limited_result());
        state = next_state;
        if let Some(next) = request_id_for_prompt(
            &effects,
            harvester_engine::llm::prompt::PromptId::ArticleTriage,
        ) {
            request_id = next;
        }
    }

    assert_eq!(state.triage().pending_count(), 0);
    assert!(matches!(
        state.provider_alert(),
        Some(ProviderAlert::RateLimited)
    ));
    let (state, effects) = start_triage_for_test(state, loaded_triage_articles(2));
    assert!(request_id_for_prompt(
        &effects,
        harvester_engine::llm::prompt::PromptId::ArticleTriage
    )
    .is_some());
    assert!(state.model_dispatch_halt_reason().is_none());
    assert!(state.provider_alert().is_none());
}
