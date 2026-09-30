use super::*;
use harvester_engine::llm::QuotaOrigin;

#[test]
fn every_summary_settle_path_saves_successes_and_emits_only_article_requests() {
    for path in [
        "success",
        "partial_failure",
        "all_failed",
        "session_quota",
        "provider_quota",
        "rate_limit",
        "stop",
        "cache_hits",
    ] {
        let mut state = start_briefing_after_triage(AppState::new(), loaded_articles().0);
        state.set_llm_max_in_flight(2);
        if path == "cache_hits" {
            for article in loaded_articles().0 {
                let key = state
                    .current_summary_cache_key(&article.content_hash)
                    .unwrap();
                state.store_summary_result(
                    key,
                    crate::ArticleSummaryResult {
                        title: "Cached".into(),
                        summary: "Paid summary".into(),
                        key_points: vec![],
                        input_tokens: 10,
                        output_tokens: 5,
                        entities: Default::default(),
                    },
                    "2026-09-30T00:00:00Z".into(),
                );
            }
        }
        let (mut state, mut effects) =
            crate::update::test_support::summarize(state, loaded_articles().0);
        let requests = effects
            .iter()
            .filter_map(|e| match e {
                Effect::RequestLlmCompletion {
                    request_id,
                    prompt_id: PromptId::ArticleSummary,
                    ..
                } => Some(*request_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut successes = if path == "cache_hits" { 2 } else { 0 };
        if path == "stop" {
            let (next, more) = update(state, Msg::StopFinishClicked);
            state = next;
            effects.extend(more);
        }
        for (index, request_id) in requests.into_iter().enumerate() {
            let result = match (path, index) {
                ("all_failed", _) | ("partial_failure", 1) => LlmResultKind::Failed {
                    reason: "fixture failure".into(),
                },
                ("session_quota", 0) => LlmResultKind::QuotaExhausted {
                    reason: "session limit".into(),
                    origin: QuotaOrigin::SessionBudget,
                },
                ("provider_quota", 0) => LlmResultKind::QuotaExhausted {
                    reason: "credits".into(),
                    origin: QuotaOrigin::Provider,
                },
                ("rate_limit", 0) => LlmResultKind::RateLimited {
                    reason: "rate limited".into(),
                },
                _ => {
                    successes += 1;
                    LlmResultKind::Success {
                        output_json: summary_json("Saved"),
                        input_tokens: 10,
                        output_tokens: 5,
                        prompt_version: 1,
                        resolved_model: "test-model".into(),
                    }
                }
            };
            let (next, more) = update(
                state,
                Msg::LlmCompleted {
                    request_id,
                    result,
                    metadata: None,
                },
            );
            state = next;
            effects.extend(more);
        }
        assert_eq!(state.summary_cache().len(), successes, "path={path}");
        assert_eq!(
            state.briefing().completed_summary_count(),
            successes,
            "path={path}"
        );
        assert!(
            matches!(
                state.briefing().phase(),
                BriefingPhase::Complete | BriefingPhase::Failed { .. }
            ),
            "path={path}"
        );
        assert!(
            effects.iter().any(|e| matches!(e, Effect::FlushResults)),
            "path={path}"
        );
        let saved = effects
            .iter()
            .filter_map(|e| match e {
                Effect::SaveResults { records } => Some(
                    records
                        .iter()
                        .filter(|r| matches!(r, crate::SavedResult::Summary(..)))
                        .count(),
                ),
                _ => None,
            })
            .sum::<usize>();
        assert_eq!(saved, successes, "path={path}");
        assert!(
            effects.iter().all(|e| match e {
                Effect::RequestLlmCompletion { prompt_id, .. } =>
                    *prompt_id == PromptId::ArticleSummary,
                _ => true,
            }),
            "path={path}"
        );
        let view = state.view();
        assert!(
            !view.briefing_generate_enabled && !view.next_item_enabled,
            "path={path}"
        );
        assert!(view.briefing_blocked_reason.is_none(), "path={path}");
    }
}
