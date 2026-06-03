use std::collections::{HashMap, HashSet};

use crate::briefing::{ArticleSummaryResult, BriefingSession, LoadedArticle};
use crate::signal_candidate::OverrideKey;
use crate::signal_candidate_cache::{
    SignalCandidateCache, SignalCandidateCacheKey, SignalCandidateInputBundle,
};
use crate::triage::{ArticleTriageResult, TriageSession};
use crate::update::update;
use crate::{AppState, Effect, LlmResultKind, Msg, SummaryCacheKey};
use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};
use harvester_engine::llm::prompt::PromptId;

use super::{
    loaded_single_article, loaded_triage_articles, request_id_for_prompt,
    start_briefing_after_triage, summary_json, with_signal_candidate_metadata,
};

fn signal_result_json() -> String {
    r#"{
        "signal_score": 84,
        "signal_key": "example-signal-key",
        "themes": ["ai-infrastructure"],
        "draft_gist": "Example outlet reports a concrete AI infrastructure event.",
        "source_tier": "Tier1",
        "confidence": "High",
        "reasoning": "Concrete event."
    }"#
    .to_string()
}

fn sample_summary_result() -> ArticleSummaryResult {
    ArticleSummaryResult {
        title: "Example article".to_string(),
        summary: "Example summary".to_string(),
        key_points: vec!["Point one".to_string(), "Point two".to_string()],
        input_tokens: 100,
        output_tokens: 55,
        entities: Default::default(),
    }
}

fn sample_triage_result(priority: u8) -> ArticleTriageResult {
    ArticleTriageResult {
        category: "news".to_string(),
        priority,
        tags: vec!["ai".to_string(), "chips".to_string()],
        rationale: "Relevant".to_string(),
        input_tokens: 10,
        output_tokens: 5,
    }
}

fn seed_completed_summary_state() -> AppState {
    let mut state = AppState::new();
    let article = LoadedArticle {
        url: "https://example.com/article".to_string(),
        source_title: Some("Example Outlet".to_string()),
        prepared_text: "Article text".to_string(),
        content_hash: "hash-1".to_string(),
        fetched_utc: Some("2026-05-25T12:00:00Z".to_string()),
    };

    let mut briefing = BriefingSession::new_loading(None);
    briefing.set_articles(vec![article.clone()], "collection".to_string());
    briefing.transition_to_summarizing();
    briefing.start_article(0, 11);
    briefing.complete_article(0, sample_summary_result());
    state.set_briefing(briefing);

    let mut triage = TriageSession::new_loading(None);
    triage.set_articles(vec![article]);
    triage.transition_to_triaging();
    triage.complete_article(0, sample_triage_result(3));
    state.set_triage(triage);

    let summary_key = SummaryCacheKey::try_new(
        "hash-1",
        PromptId::ArticleSummary,
        Some(1),
        Some("test-summary-model"),
        &[],
    )
    .expect("summary cache key");
    state.store_summary_result(
        summary_key,
        sample_summary_result(),
        "2026-05-25T12:00:10Z".into(),
    );

    state
}

fn seed_cached_summary_only_state_for_signal_candidate() -> AppState {
    let mut state = AppState::new();
    let article = LoadedArticle {
        url: "https://example.com/a".to_string(),
        source_title: Some("Article A".to_string()),
        prepared_text: "Article text".to_string(),
        content_hash: "hash-a".to_string(),
        fetched_utc: Some("2026-05-25T12:00:00Z".to_string()),
    };

    state.restore_completed_jobs(vec![crate::CompletedJobSnapshot {
        url: article.url.clone(),
        tokens: None,
        bytes: None,
        links: vec![],
        fetched_utc: article.fetched_utc.clone(),
    }]);

    let mut triage = TriageSession::new_loading(None);
    triage.set_articles(vec![article]);
    triage.transition_to_triaging();
    triage.complete_article(0, sample_triage_result(2));
    state.set_triage(triage);

    state.store_summary_result(
        SummaryCacheKey::try_new(
            "hash-a",
            PromptId::ArticleSummary,
            Some(1),
            Some("test-summary-model"),
            &[],
        )
        .expect("summary cache key"),
        sample_summary_result(),
        "2026-05-25T12:00:10Z".into(),
    );

    state
}

fn set_signal_candidate_metadata_without_sweep(state: &mut AppState) {
    let mut active_versions = HashMap::new();
    active_versions.insert(PromptId::ArticleTriage, 1);
    active_versions.insert(PromptId::ArticleSummary, 1);
    active_versions.insert(PromptId::ArticleSignalCandidate, 1);
    let mut effective_models = HashMap::new();
    effective_models.insert(PromptId::ArticleTriage, "test-triage-model".to_string());
    effective_models.insert(PromptId::ArticleSummary, "test-summary-model".to_string());
    effective_models.insert(
        PromptId::ArticleSignalCandidate,
        "test-signal-model".to_string(),
    );
    state.set_llm_metadata(active_versions, effective_models, HashMap::new());
}

fn prewarm_signal_candidate_cache(
    state: &mut AppState,
    article: &LoadedArticle,
    triage_priority: u8,
) {
    let url = article.url.as_str();
    let summary = state
        .summary_result_for_url(url)
        .expect("summary result for signal prewarm");
    let triage = state
        .triage()
        .result_for_url(url)
        .expect("triage result for signal prewarm");
    let key_points = summary.key_points.clone();
    let published_at = article.fetched_utc.clone().unwrap_or_default();
    let outlet = article.source_title.as_deref().unwrap_or(url);
    let title = article.source_title.as_deref().unwrap_or(url);
    let mut triage_tags_sorted = triage.tags.clone();
    triage_tags_sorted.sort();
    let triage_tags_sorted: Vec<_> = triage_tags_sorted.iter().map(String::as_str).collect();
    let bundle = SignalCandidateInputBundle {
        url,
        outlet,
        title,
        published_at: published_at.as_str(),
        triage_priority,
        triage_tags_sorted,
        summary: summary.summary.as_str(),
        key_points: &key_points,
        upstream_summary_cache_digest: state
            .summary_cache_key_for_url(url)
            .expect("summary cache key")
            .digest(),
    };
    let key = SignalCandidateCacheKey::try_new(
        &bundle,
        state.active_version_for(PromptId::ArticleSignalCandidate),
        state.effective_model_for(PromptId::ArticleSignalCandidate),
        state.context_for(PromptId::ArticleSignalCandidate),
    )
    .expect("signal cache key");
    state.store_signal_candidate_result(
        key,
        SignalCandidateResult {
            signal_score: 90,
            signal_key: "example-signal-key".to_string(),
            themes: vec!["ai-infrastructure".to_string()],
            draft_gist: "Concrete event.".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "Concrete event.".to_string(),
            input_tokens: 10,
            output_tokens: 5,
        },
        "2026-05-25T12:00:20Z".into(),
    );
}

fn prewarm_summary_cache_for_single_article(state: &mut AppState, summary_model: &str) {
    let summary_key = SummaryCacheKey::try_new(
        "hash-a",
        PromptId::ArticleSummary,
        Some(1),
        Some(summary_model),
        &[],
    )
    .expect("summary cache key");
    state.store_summary_result(
        summary_key,
        ArticleSummaryResult {
            title: "Article A".to_string(),
            summary: "Summary".to_string(),
            key_points: vec!["p1".to_string()],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        },
        "2026-05-25T12:00:00Z".to_string(),
    );
}

#[test]
fn signal_candidate_session_starts_empty() {
    let state = AppState::new();
    assert_eq!(state.signal_candidate().enqueued_count(), 0);
    assert!(state.signal_candidate_cache().is_empty());
}

#[test]
fn signal_candidate_completion_routes_to_session_and_persists_cache() {
    let mut state = AppState::new();
    state
        .signal_candidate_mut()
        .enqueue("https://example.com/article".to_string());
    state
        .signal_candidate_mut()
        .mark_scoring("https://example.com/article", 9);
    state.record_pending_llm_request(9, PromptId::ArticleSignalCandidate);
    state.set_signal_candidate_input_snapshot(
        "https://example.com/article",
        crate::update::signal_candidate::SignalCandidateInputSnapshot {
            outlet: "Example Outlet".to_string(),
            title: "Example article".to_string(),
            published_at: "2026-05-25T12:00:00Z".to_string(),
            triage_priority: 3,
            triage_tags_sorted: vec!["ai".to_string(), "chips".to_string()],
            summary: "Example summary".to_string(),
            key_points: vec!["Point one".to_string(), "Point two".to_string()],
            upstream_summary_cache_digest: "digest".to_string(),
            context: Vec::new(),
            prompt_version: 1,
            model_id: "test-signal-model".to_string(),
        },
    );

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 9,
            result: LlmResultKind::Success {
                output_json: signal_result_json(),
                input_tokens: 100,
                output_tokens: 10,
                prompt_version: 1,
                resolved_model: "test-signal-model".to_string(),
            },
            metadata: None,
        },
    );

    assert_eq!(state.signal_candidate().completed_count(), 1);
    assert!(state
        .signal_candidate_input_snapshot("https://example.com/article")
        .is_none());
    assert!(!state.signal_candidate_cache().is_empty());
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::PersistSignalCandidateCache { .. })));
}

#[test]
fn signal_candidate_validation_failure_marks_failed() {
    let mut state = AppState::new();
    state
        .signal_candidate_mut()
        .enqueue("https://example.com/article".to_string());
    state
        .signal_candidate_mut()
        .mark_scoring("https://example.com/article", 8);
    state.record_pending_llm_request(8, PromptId::ArticleSignalCandidate);

    let (state, _effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 8,
            result: LlmResultKind::Success {
                output_json: "{\"signal_score\": 999}".to_string(),
                input_tokens: 100,
                output_tokens: 10,
                prompt_version: 1,
                resolved_model: "test-signal-model".to_string(),
            },
            metadata: None,
        },
    );

    assert_eq!(state.signal_candidate().failed_count(), 1);
    assert!(state
        .signal_candidate_input_snapshot("https://example.com/article")
        .is_none());
}

#[test]
fn signal_candidate_cache_loaded_sweeps_eligible_summary() {
    let mut state = seed_completed_summary_state();
    set_signal_candidate_metadata_without_sweep(&mut state);
    let (state, effects) = update(
        state,
        Msg::SignalCandidateCacheLoaded {
            cache: SignalCandidateCache::default(),
        },
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSignalCandidate,
            ..
        }
    )));
    assert!(matches!(
        state
            .signal_candidate()
            .state_for("https://example.com/article"),
        Some(crate::signal_candidate::SignalCandidateState::Scoring { .. })
    ));
}

#[test]
fn signal_candidate_cache_loaded_reconstructs_from_cached_summary_without_briefing() {
    let mut state = seed_cached_summary_only_state_for_signal_candidate();
    set_signal_candidate_metadata_without_sweep(&mut state);

    let summary_key = SummaryCacheKey::try_new(
        "hash-a",
        PromptId::ArticleSummary,
        Some(1),
        Some("test-summary-model"),
        &[],
    )
    .expect("summary cache key");
    let key_points = vec!["Point one".to_string(), "Point two".to_string()];
    let bundle = SignalCandidateInputBundle {
        url: "https://example.com/a",
        outlet: "Article A",
        title: "Article A",
        published_at: "2026-05-25T12:00:00Z",
        triage_priority: 2,
        triage_tags_sorted: vec!["ai", "chips"],
        summary: "Example summary",
        key_points: &key_points,
        upstream_summary_cache_digest: summary_key.digest(),
    };
    let key = SignalCandidateCacheKey::try_new(&bundle, Some(1), Some("test-signal-model"), &[])
        .expect("signal cache key");
    state.store_signal_candidate_result(
        key,
        SignalCandidateResult {
            signal_score: 90,
            signal_key: "example-signal-key".to_string(),
            themes: vec!["ai-infrastructure".to_string()],
            draft_gist: "Concrete event.".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "Concrete event.".to_string(),
            input_tokens: 10,
            output_tokens: 5,
        },
        "2026-05-25T12:00:20Z".into(),
    );
    let persisted_cache = state.signal_candidate_cache().clone();

    let (state, effects) = update(
        state,
        Msg::SignalCandidateCacheLoaded {
            cache: persisted_cache,
        },
    );

    assert!(effects.iter().all(|effect| !matches!(
        effect,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSignalCandidate,
            ..
        }
    )));
    assert!(matches!(
        state.signal_candidate().state_for("https://example.com/a"),
        Some(crate::signal_candidate::SignalCandidateState::Completed { .. })
    ));
    assert!(!state.selected_job_has_summary());
}

#[test]
fn triage_cache_hydration_sweeps_signal_candidates_after_metadata_and_summary_restore() {
    let mut state = seed_cached_summary_only_state_for_signal_candidate();
    set_signal_candidate_metadata_without_sweep(&mut state);

    let (state, effects) = update(
        state,
        Msg::TriageCacheHydrated {
            cache: crate::triage_cache::TriageCache::default(),
        },
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSignalCandidate,
            ..
        }
    )));
    assert!(matches!(
        state.signal_candidate().state_for("https://example.com/a"),
        Some(crate::signal_candidate::SignalCandidateState::Scoring { .. })
    ));
}

#[test]
fn triage_cache_hit_enqueues_signal_candidate_scoring() {
    let mut state = with_signal_candidate_metadata(AppState::new());
    let articles = loaded_triage_articles(1);
    let article = articles[0].clone();

    state.store_summary_result(
        SummaryCacheKey::try_new(
            "hash-0",
            PromptId::ArticleSummary,
            Some(1),
            Some("test-summary-model"),
            &[],
        )
        .expect("summary cache key"),
        ArticleSummaryResult {
            title: "Article 0".to_string(),
            summary: "Summary".to_string(),
            key_points: vec!["p1".to_string()],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        },
        "2026-05-25T12:00:10Z".into(),
    );
    state.store_triage_result(&article.content_hash, sample_triage_result(3));

    let triage_request_id = state.alloc_triage_request_id();
    state.set_triage_in_flight(triage_request_id);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: triage_request_id,
            articles,
        },
    );
    let (state, effects) = update(state, Msg::TriageClicked);

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSignalCandidate,
            ..
        }
    )));
    assert!(matches!(
        state.signal_candidate().state_for(&article.url),
        Some(crate::signal_candidate::SignalCandidateState::Scoring { .. })
    ));
}

#[test]
fn summary_completion_enqueues_signal_scoring() {
    let state = start_briefing_after_triage(AppState::new(), loaded_single_article().0.clone());
    let state = with_signal_candidate_metadata(state);
    let (articles, collection_text) = loaded_single_article();

    let (state, effects) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );
    let summary_request_id =
        request_id_for_prompt(&effects, PromptId::ArticleSummary).expect("summary request");

    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: summary_request_id,
            result: LlmResultKind::Success {
                output_json: summary_json("Article A"),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "test-summary-model".to_string(),
            },
            metadata: None,
        },
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSignalCandidate,
            ..
        }
    )));
    assert!(matches!(
        state.signal_candidate().state_for("https://example.com/a"),
        Some(crate::signal_candidate::SignalCandidateState::Scoring { .. })
    ));
}

#[test]
fn summary_cache_hit_reuses_signal_candidate_cache_without_snapshot_leak() {
    super::init_logging();
    let mut state = start_briefing_after_triage(AppState::new(), loaded_single_article().0.clone());
    state = with_signal_candidate_metadata(state);
    state.start_summary_cache_run();
    state.mark_briefing_metadata_ready();
    let summary_model = state
        .summary_cache_metadata()
        .expect("summary metadata ready")
        .1
        .to_string();
    prewarm_summary_cache_for_single_article(&mut state, &summary_model);
    let (articles, _) = loaded_single_article();
    prewarm_signal_candidate_cache(&mut state, &articles[0], 3);
    let (articles, collection_text) = loaded_single_article();

    let (state, effects) = update(
        state,
        Msg::ArticlesLoaded {
            articles,
            collection_text,
        },
    );

    assert!(effects.iter().all(|effect| !matches!(
        effect,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSummary,
            ..
        }
    )));
    assert!(effects.iter().all(|effect| !matches!(
        effect,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleSignalCandidate,
            ..
        }
    )));
    assert!(matches!(
        state.signal_candidate().state_for("https://example.com/a"),
        Some(crate::signal_candidate::SignalCandidateState::Completed { .. })
    ));
    assert!(state
        .signal_candidate_input_snapshot("https://example.com/a")
        .is_none());

    let (_, result) = state
        .signal_candidate()
        .iter_completed()
        .find(|(url, _)| *url == "https://example.com/a")
        .expect("completed cached signal candidate");
    assert_eq!(result.input_tokens, 0);
    assert_eq!(result.output_tokens, 0);
}

#[test]
fn summary_cache_key_for_url_uses_parsed_rfc3339_order() {
    let mut state = AppState::new();
    let article = LoadedArticle {
        url: "https://example.com/article".to_string(),
        source_title: Some("Example".to_string()),
        prepared_text: "Article text".to_string(),
        content_hash: "shared-hash".to_string(),
        fetched_utc: None,
    };
    let mut triage = TriageSession::new_loading(None);
    triage.set_articles(vec![article]);
    triage.transition_to_triaging();
    triage.complete_article(0, sample_triage_result(3));
    state.set_triage(triage);

    let lexically_later_but_older = SummaryCacheKey::try_new(
        "shared-hash",
        PromptId::ArticleSummary,
        Some(1),
        Some("model-a"),
        &[],
    )
    .expect("older key");
    let lexically_earlier_but_newer = SummaryCacheKey::try_new(
        "shared-hash",
        PromptId::ArticleSummary,
        Some(1),
        Some("model-b"),
        &[],
    )
    .expect("newer key");

    state.store_summary_result(
        lexically_later_but_older,
        sample_summary_result(),
        "2026-05-25T09:00:00Z".to_string(),
    );
    state.store_summary_result(
        lexically_earlier_but_newer.clone(),
        sample_summary_result(),
        "2026-05-25T08:00:00-04:00".to_string(),
    );

    assert_eq!(
        state.summary_cache_key_for_url("https://example.com/article"),
        Some(lexically_earlier_but_newer)
    );
}

#[test]
fn signal_candidate_persisted_with_dated_model_variant_is_cache_hit_after_reload() {
    // Session 1: an eligible article gets enqueued, then completes with a dated
    // model variant ("test-signal-model-2026-03-17") even though the canonical
    // configured model is "test-signal-model". The persisted cache entry must
    // be keyed by the canonical model so the next session's lookup hits.
    let mut state = seed_completed_summary_state();
    set_signal_candidate_metadata_without_sweep(&mut state);

    let (state, effects) = update(
        state,
        Msg::SignalCandidateCacheLoaded {
            cache: SignalCandidateCache::default(),
        },
    );
    let signal_request_id = request_id_for_prompt(&effects, PromptId::ArticleSignalCandidate)
        .expect("signal request emitted by initial sweep");

    let (state, _effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: signal_request_id,
            result: LlmResultKind::Success {
                output_json: signal_result_json(),
                input_tokens: 100,
                output_tokens: 10,
                prompt_version: 1,
                resolved_model: "test-signal-model-2026-03-17".to_string(),
            },
            metadata: None,
        },
    );

    let persisted_cache = state.signal_candidate_cache().clone();
    assert!(
        !persisted_cache.is_empty(),
        "session 1 must persist a signal-candidate cache entry"
    );

    // Session 2: simulate process restart by building a fresh state with the
    // same article + metadata and hydrating the persisted cache. The sweep
    // must hit the cache and emit zero new signal-candidate LLM requests.
    let mut next_state = seed_completed_summary_state();
    set_signal_candidate_metadata_without_sweep(&mut next_state);

    let (next_state, effects) = update(
        next_state,
        Msg::SignalCandidateCacheLoaded {
            cache: persisted_cache,
        },
    );

    assert!(
        effects.iter().all(|effect| !matches!(
            effect,
            Effect::RequestLlmCompletion {
                prompt_id: PromptId::ArticleSignalCandidate,
                ..
            }
        )),
        "cache hit on restart must not issue a new signal-candidate LLM request"
    );
    assert!(matches!(
        next_state
            .signal_candidate()
            .state_for("https://example.com/article"),
        Some(crate::signal_candidate::SignalCandidateState::Completed { .. })
    ));
}

#[test]
fn signal_candidate_overrides_loaded_sets_exclusions() {
    let mut overrides = HashSet::new();
    overrides.insert(OverrideKey {
        signal_key: "drop-this-cluster".to_string(),
        prompt_id: "ArticleSignalCandidate".to_string(),
        prompt_version: 1,
    });

    let (state, effects) = update(
        AppState::new(),
        Msg::SignalCandidateOverridesLoaded { overrides },
    );

    assert!(effects.is_empty());
    assert_eq!(state.signal_candidate().excluded().len(), 1);
}
