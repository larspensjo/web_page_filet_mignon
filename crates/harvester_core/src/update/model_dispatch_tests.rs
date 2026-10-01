use super::*;
use crate::briefing::{ArticleSummaryResult, BriefingSession, LoadedArticle};
use crate::signal_candidate::SignalCandidateState;
use crate::triage::{ArticleTriageResult, TriageSession};
use crate::update::signal_candidate::{input_key, SignalCandidateInputSnapshot};
use crate::{LlmResultKind, Msg, SummaryCacheKey};
use harvester_engine::llm::{QuotaOrigin, SignalCandidateResult};

fn article(name: &str) -> LoadedArticle {
    LoadedArticle {
        url: format!("https://example.com/{name}"),
        source_title: Some(name.into()),
        prepared_text: name.into(),
        content_hash: name.into(),
        fetched_utc: None,
    }
}

fn state_with_work(triage: &[&str], summaries: &[&str], scores: &[&str]) -> AppState {
    let mut state = AppState::new();
    crate::update::test_support::arm_admitted(&mut state);
    state.set_llm_metadata(
        STAGE_PRIORITY.iter().map(|id| (*id, 1)).collect(),
        STAGE_PRIORITY
            .iter()
            .map(|id| (*id, "model".into()))
            .collect(),
    );
    state.mark_triage_metadata_ready();
    state.start_summary_cache_run();
    state.mark_briefing_metadata_ready();
    if !triage.is_empty() {
        let mut session = TriageSession::new_loading(None);
        session.set_articles(triage.iter().map(|name| article(name)).collect());
        session.transition_to_triaging();
        state.set_triage(session);
    }
    if !summaries.is_empty() {
        let mut session = BriefingSession::new_loading();
        session.set_articles(summaries.iter().map(|name| article(name)).collect());
        session.transition_to_summarizing();
        state.set_briefing(session);
    }
    for name in scores {
        admit_score(&mut state, name);
    }
    state
}

fn admit_score(state: &mut AppState, name: &str) {
    let url = article(name).url;
    let snapshot = SignalCandidateInputSnapshot {
        outlet: "example".into(),
        title: name.into(),
        published_at: String::new(),
        triage_priority: 3,
        triage_tags_sorted: vec![],
        summary: "summary".into(),
        key_points: vec![],
        upstream_summary_cache_digest: "summary-key".into(),
        context: vec![],
        prompt_version: 1,
        model_id: "model".into(),
    };
    let key = input_key(&url, &snapshot).unwrap();
    state
        .signal_candidate_mut()
        .enqueue(url.clone(), key.digest());
    state.set_signal_candidate_input_snapshot(&url, snapshot);
}

fn requests(effects: &[Effect]) -> Vec<(u64, PromptId, String)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                input_content,
                ..
            } => Some((*request_id, *prompt_id, input_content.clone())),
            _ => None,
        })
        .collect()
}

fn completion(state: AppState, id: u64, result: LlmResultKind) -> (AppState, Vec<Effect>) {
    crate::update(
        state,
        Msg::LlmCompleted {
            request_id: id,
            result,
            metadata: None,
        },
    )
}

fn triage_result() -> ArticleTriageResult {
    ArticleTriageResult {
        category: "news".into(),
        priority: 3,
        tags: vec![],
        rationale: "relevant".into(),
        input_tokens: 0,
        output_tokens: 0,
    }
}

fn summary_result(text: &str) -> ArticleSummaryResult {
    ArticleSummaryResult {
        title: "title".into(),
        summary: text.into(),
        key_points: vec![],
        input_tokens: 0,
        output_tokens: 0,
        entities: Default::default(),
    }
}

fn summary_key(hash: &str, version: u32) -> SummaryCacheKey {
    SummaryCacheKey::try_new(
        hash,
        PromptId::ArticleSummary,
        Some(version),
        Some("model"),
        &[],
    )
    .unwrap()
}

fn score_result() -> SignalCandidateResult {
    use harvester_engine::llm::dto::{Confidence, SourceTier};
    SignalCandidateResult {
        signal_score: 80,
        signal_key: "event".into(),
        themes: vec![],
        draft_gist: "event".into(),
        source_tier: SourceTier::Tier1,
        confidence: Confidence::High,
        reasoning: "event".into(),
        input_tokens: 5,
        output_tokens: 5,
    }
}

#[test]
fn synchronous_budget_is_clamped_and_zero_dispatches_one_triage_request() {
    let mut state = state_with_work(&["one", "two", "three", "four"], &[], &[]);
    state.set_llm_max_in_flight(usize::MAX);
    assert_eq!(
        state.llm_max_in_flight(),
        harvester_engine::llm::MAX_LLM_CONCURRENT_REQUESTS
    );
    state.set_llm_max_in_flight(0);
    assert_eq!(state.llm_max_in_flight(), 1);
    let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    let emitted = requests(&effects);
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].1, PromptId::ArticleTriage);
    assert_eq!(state.triage().pending_count(), 3);
    assert_eq!(state.article_model_requests_in_flight(), 1);
}

#[test]
fn shared_budget_dispatches_scoring_summary_triage_and_never_exceeds_three() {
    let mut state = state_with_work(&["triage-z", "triage-a"], &["summary"], &["score"]);
    state.set_llm_max_in_flight(3);
    let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    let emitted = requests(&effects);
    assert_eq!(
        emitted.iter().map(|r| r.1).collect::<Vec<_>>(),
        STAGE_PRIORITY
    );
    assert_eq!(state.article_model_requests_in_flight(), 3);
    assert_eq!(state.triage().pending_count(), 1);
    let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    assert!(requests(&effects).is_empty());
    assert_eq!(state.article_model_requests_in_flight(), 3);
}

#[test]
fn completion_gives_slot_to_new_highest_priority_work() {
    let state = state_with_work(&["triage-z", "triage-a"], &[], &[]);
    let (mut state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    let id = requests(&effects)[0].0;
    admit_score(&mut state, "score");
    let (state, effects) = completion(
        state,
        id,
        LlmResultKind::Failed {
            reason: "failed".into(),
        },
    );
    assert_eq!(requests(&effects)[0].1, PromptId::ArticleSignalCandidate);
    assert_eq!(state.triage().pending_count(), 1);
    assert_eq!(state.article_model_requests_in_flight(), 1);
}

#[test]
fn summary_cache_completion_yields_to_scoring_before_next_summary() {
    let mut state = state_with_work(&["cached"], &["cached", "uncached"], &[]);
    state.store_triage_result("cached", triage_result());
    state.store_summary_result(
        summary_key("cached", 1),
        summary_result("cached"),
        "now".into(),
    );
    let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    assert_eq!(requests(&effects).len(), 1);
    assert_eq!(requests(&effects)[0].1, PromptId::ArticleSignalCandidate);
    assert_eq!(state.briefing().pending_count(), 1);
    assert_eq!(state.triage().completed_count(), 1);
}

#[test]
fn scoring_uses_run_frozen_triage_key_after_live_metadata_changes() {
    let mut state = state_with_work(&["article"], &[], &[]);
    state.store_triage_result("article", triage_result());
    state.triage_mut().complete_article(0, triage_result());
    state.store_summary_result(
        summary_key("article", 1),
        summary_result("current"),
        "now".into(),
    );
    state.set_llm_metadata(
        STAGE_PRIORITY
            .iter()
            .map(|id| (*id, if *id == PromptId::ArticleTriage { 2 } else { 1 }))
            .collect(),
        STAGE_PRIORITY
            .iter()
            .map(|id| (*id, "model".into()))
            .collect(),
    );
    assert!(try_enqueue(&mut state, &article("article").url));
}

#[test]
fn changed_in_flight_score_settles_and_caches_old_request_before_readmission() {
    let mut state = state_with_work(&["article"], &[], &[]);
    let url = article("article").url;
    state.store_triage_result("article", triage_result());
    state.store_summary_result(
        summary_key("article", 1),
        summary_result("first"),
        "now".into(),
    );
    let (mut state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    let first_id = requests(&effects)[0].0;
    let first_key = input_key(&url, state.signal_candidate_input_snapshot(&url).unwrap()).unwrap();
    state.store_summary_result(
        summary_key("article", 1),
        summary_result("changed"),
        "later".into(),
    );
    assert!(!try_enqueue(&mut state, &url));
    let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    assert!(requests(&effects).is_empty());
    assert_eq!(state.article_model_requests_in_flight(), 1);
    let (mut state, effects) = completion(
        state,
        first_id,
        LlmResultKind::Success {
            output_json: r#"{"signal_score":84,"signal_key":"example-signal-key","themes":["ai-infrastructure"],"draft_gist":"Example outlet reports a concrete AI infrastructure event.","source_tier":"Tier1","confidence":"High","reasoning":"Concrete event."}"#.into(),
            input_tokens: 1,
            output_tokens: 1,
            prompt_version: 1,
            resolved_model: "model".into(),
        },
    );
    assert!(requests(&effects).is_empty());
    assert!(matches!(
        state.signal_candidate().state_for(&url),
        Some(SignalCandidateState::Completed { .. })
    ));
    assert!(state.try_reuse_signal_candidate(&first_key).is_some());
    assert_eq!(state.signal_candidate().enqueued_count(), 1);
    crate::update::test_support::arm_admitted(&mut state);
    assert!(try_enqueue(&mut state, &url));
    assert_eq!(state.signal_candidate().enqueued_count(), 2);
    let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    assert_eq!(requests(&effects)[0].1, PromptId::ArticleSignalCandidate);
    assert_eq!(
        state.signal_candidate_input_snapshot(&url).unwrap().summary,
        "changed"
    );
}

#[test]
fn every_stage_preserves_admission_order() {
    for stage in STAGE_PRIORITY {
        let names = ["z", "a", "m"];
        let mut state = match stage {
            PromptId::ArticleTriage => state_with_work(&names, &[], &[]),
            PromptId::ArticleSummary => state_with_work(&[], &names, &[]),
            _ => state_with_work(&[], &[], &names),
        };
        state.set_llm_max_in_flight(3);
        let (_, effects) = crate::update(state, Msg::PipelineRunAdvance);
        let emitted = requests(&effects);
        assert_eq!(emitted.len(), 3);
        for (request, name) in emitted.iter().zip(names) {
            assert_eq!(request.1, *stage);
            if *stage == PromptId::ArticleSignalCandidate {
                assert!(request.2.contains(&format!("https://example.com/{name}")));
            } else {
                assert_eq!(request.2, name);
            }
        }
    }
}

#[test]
fn cache_hits_in_all_stages_take_no_slot() {
    let mut state = state_with_work(
        &["triage-hit", "triage-miss"],
        &["summary-hit"],
        &["score-hit"],
    );
    state.store_triage_result("triage-hit", triage_result());
    state.store_summary_result(
        summary_key("summary-hit", 1),
        summary_result("cached"),
        "now".into(),
    );
    let url = article("score-hit").url;
    let key = input_key(&url, state.signal_candidate_input_snapshot(&url).unwrap()).unwrap();
    state.store_signal_candidate_result(key, score_result(), "now".into());
    let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    assert_eq!(requests(&effects).len(), 1);
    assert_eq!(requests(&effects)[0].2, "triage-miss");
    assert_eq!(state.article_model_requests_in_flight(), 1);
    assert_eq!(state.triage().completed_count(), 1);
    assert_eq!(state.briefing().completed_summary_count(), 1);
    assert_eq!(state.signal_candidate().completed_count(), 1);
}

#[test]
fn triage_cache_hit_rejects_old_summary_then_current_completion_admits_scoring() {
    let mut state = state_with_work(&["article"], &[], &[]);
    state.store_triage_result("article", triage_result());
    state.store_summary_result(
        summary_key("article", 0),
        summary_result("old"),
        "now".into(),
    );
    let (mut state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    let url = article("article").url;
    assert!(requests(&effects).is_empty());
    assert!(state.signal_candidate().state_for(&url).is_none());
    let mut summaries = BriefingSession::new_loading();
    summaries.set_articles(vec![article("article")]);
    summaries.transition_to_summarizing();
    state.set_briefing(summaries);
    crate::update::test_support::arm_admitted(&mut state);
    let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
    let id = requests(&effects)[0].0;
    let (state, effects) = completion(
        state,
        id,
        LlmResultKind::Success {
            output_json:
                r#"{"title":"current","summary":"current summary","key_points":["point"]}"#.into(),
            input_tokens: 1,
            output_tokens: 1,
            prompt_version: 1,
            resolved_model: "model".into(),
        },
    );
    assert_eq!(requests(&effects)[0].1, PromptId::ArticleSignalCandidate);
    let snapshot = state.signal_candidate_input_snapshot(&url).unwrap();
    assert_eq!(snapshot.summary, "current summary");
    assert_eq!(
        snapshot.upstream_summary_cache_digest,
        summary_key("article", 1).digest()
    );
    assert_eq!(
        state.signal_candidate().input_digest_for(&url),
        Some(input_key(&url, snapshot).unwrap().digest().as_str())
    );
}

#[test]
fn changed_summary_replaces_completed_and_failed_scores_but_same_digest_is_refused() {
    for failed in [false, true] {
        let mut state = state_with_work(&["article"], &[], &[]);
        let url = article("article").url;
        state.store_triage_result("article", triage_result());
        state.store_summary_result(
            summary_key("article", 1),
            summary_result("first"),
            "now".into(),
        );
        let effects: Vec<Effect> = vec![];
        assert!(try_enqueue(&mut state, &url));
        assert!(effects.is_empty(), "admission is queue-only");
        let first = state
            .signal_candidate()
            .input_digest_for(&url)
            .unwrap()
            .to_owned();
        if failed {
            state.signal_candidate_mut().fail(&url, "provider failure");
        } else {
            state.signal_candidate_mut().complete(&url, score_result());
        }
        assert!(!try_enqueue(&mut state, &url));
        state.store_summary_result(
            summary_key("article", 1),
            summary_result("changed"),
            "later".into(),
        );
        assert!(!try_enqueue(&mut state, &url));
        crate::update::test_support::arm_admitted(&mut state);
        assert!(try_enqueue(&mut state, &url));
        assert_ne!(
            state.signal_candidate().input_digest_for(&url),
            Some(first.as_str())
        );
        assert!(matches!(
            state.signal_candidate().state_for(&url),
            Some(SignalCandidateState::Pending)
        ));
        assert!(!try_enqueue(&mut state, &url));
    }
}

#[test]
fn quota_halts_all_stages_and_future_admissions_with_provider_reason() {
    for origin in [QuotaOrigin::Provider, QuotaOrigin::SessionBudget] {
        let state = state_with_work(&["triage"], &["summary"], &["score", "pending-score"]);
        let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
        let id = requests(&effects)[0].0;
        let (mut state, effects) = completion(
            state,
            id,
            LlmResultKind::QuotaExhausted {
                reason: "quota reason".into(),
                origin,
            },
        );
        assert!(requests(&effects).is_empty());
        assert_eq!(state.triage().pending_count(), 0);
        assert_eq!(state.briefing().pending_count(), 0);
        assert!(
            matches!(state.signal_candidate().state_for(&article("pending-score").url), Some(SignalCandidateState::Failed { reason }) if reason == "quota reason")
        );
        assert!(
            matches!(&state.triage().articles()[0].triage_state, crate::triage::ArticleTriageState::Failed { reason } if reason == "quota reason")
        );
        assert!(
            matches!(&state.briefing().articles()[0].summary_state, crate::briefing::ArticleSummaryState::Failed { reason } if reason == "quota reason")
        );
        state.reset_provider_rate_limit_failures();
        admit_score(&mut state, "later");
        let (state, effects) = crate::update(state, Msg::PipelineRunAdvance);
        assert!(requests(&effects).is_empty());
        assert!(
            matches!(state.signal_candidate().state_for(&article("later").url), Some(SignalCandidateState::Failed { reason }) if reason == "quota reason")
        );
    }
}

#[test]
fn existing_three_rate_limit_threshold_halts_all_stages() {
    let state = state_with_work(
        &["triage"],
        &["summary"],
        &["score-1", "score-2", "score-3", "score-4"],
    );
    let (mut state, mut effects) = crate::update(state, Msg::PipelineRunAdvance);
    for count in 1..=3 {
        let id = requests(&effects)[0].0;
        (state, effects) = completion(
            state,
            id,
            LlmResultKind::RateLimited {
                reason: "slow down".into(),
            },
        );
        assert_eq!(state.model_dispatch_halt_reason().is_some(), count == 3);
    }
    assert!(requests(&effects).is_empty());
    assert_eq!(state.triage().pending_count(), 0);
    assert_eq!(state.briefing().pending_count(), 0);
    assert_eq!(state.signal_candidate().in_flight_count(), 0);
}
