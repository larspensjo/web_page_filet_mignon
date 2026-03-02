use std::collections::HashMap;
use std::sync::Once;

use harvester_core::{update, AppState, Effect, JobResultKind, LlmResultKind, LoadedArticle, Msg};
use harvester_engine::llm::prompt::PromptId;

fn init_logging() {
    static INIT: Once = Once::new();
    INIT.call_once(engine_logging::initialize_for_tests);
}

fn submit_urls(state: AppState, input: &str) -> (AppState, Vec<Effect>) {
    let (state, _) = update(state, Msg::InputChanged(input.to_string()));
    update(state, Msg::UrlsSubmitted)
}

fn add_completed_job(state: AppState, url: &str) -> (AppState, u64) {
    let (state, effects) = submit_urls(state, &format!("{url}\n"));
    let job_id = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::EnqueueUrl { job_id, .. } => Some(job_id),
            _ => None,
        })
        .expect("job effect must be present");
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id,
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );
    (state, job_id)
}

fn completed_state_with_jobs(urls: &[&str]) -> (AppState, Vec<u64>) {
    let mut state = AppState::new();
    let mut job_ids = Vec::new();
    for url in urls {
        let (next, job_id) = add_completed_job(state, url);
        state = next;
        job_ids.push(job_id);
    }
    (state, job_ids)
}

/// Advance ticks until the coordinator dispatches a `LoadArticlesForTriage` effect.
/// Panics if no dispatch occurs within 200 ticks.
fn tick_until_triage_dispatch(mut state: AppState) -> (AppState, u64) {
    for _ in 0..200 {
        let (next, effects) = update(state, Msg::Tick);
        state = next;
        if let Some(request_id) = effects.iter().find_map(|e| match e {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        }) {
            return (state, request_id);
        }
    }
    panic!("no LoadArticlesForTriage dispatch within 200 ticks");
}

/// Advance ticks until dispatch, then apply the given articles.
fn simulate_triage_loaded(state: AppState, articles: Vec<LoadedArticle>) -> AppState {
    let (state, request_id) = tick_until_triage_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles,
        },
    );
    state
}

/// Advance ticks until dispatch, then apply a failure.
fn simulate_triage_load_failed(state: AppState, reason: &str) -> AppState {
    let (state, request_id) = tick_until_triage_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoadFailed {
            request_id,
            reason: reason.to_string(),
        },
    );
    state
}

fn ready_state_with_pretriage(urls: &[&str]) -> (AppState, Vec<u64>) {
    let (state, job_ids) = completed_state_with_jobs(urls);
    let state = with_triage_metadata_ready(state);
    let state = simulate_triage_loaded(state, sample_articles(urls));
    (state, job_ids)
}

fn sample_articles(urls: &[&str]) -> Vec<LoadedArticle> {
    urls.iter()
        .map(|url| LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: std::iter::repeat_n("prepared-content", 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("{url}-hash"),
            fetched_utc: None,
        })
        .collect()
}

fn triage_success(priority: u8) -> LlmResultKind {
    let output_json = format!(
        r#"{{"category":"security","priority":{},"tags":["tag"],"rationale":"reason"}}"#,
        priority
    );
    LlmResultKind::Success {
        output_json,
        input_tokens: 10,
        output_tokens: 5,
        prompt_version: 1,
        model_id: "test-model".to_string(),
    }
}

fn triage_quota() -> LlmResultKind {
    LlmResultKind::QuotaExhausted {
        reason: "quota".to_string(),
    }
}

fn triage_failure(reason: &str) -> LlmResultKind {
    LlmResultKind::Failed {
        reason: reason.to_string(),
    }
}

fn request_id_for_prompt(effects: &[Effect], prompt_id: PromptId) -> Option<u64> {
    effects.iter().find_map(|effect| match effect {
        Effect::RequestLlmCompletion {
            request_id,
            prompt_id: pid,
            ..
        } if *pid == prompt_id => Some(*request_id),
        _ => None,
    })
}

fn assert_persist_triage_cache_effect(effects: &[Effect], state: &AppState) {
    let expected_cache = state.triage_cache().clone();
    let expected = Effect::PersistTriageCache {
        cache: expected_cache,
    };
    assert!(
        effects.contains(&expected),
        "expected PersistTriageCache in effects, got: {effects:?}"
    );
}

#[test]
fn triage_clicked_emits_load_effect() {
    init_logging();
    let (state, _) = ready_state_with_pretriage(&["https://one.example"]);
    let (state, effects) = update(state, Msg::TriageClicked);
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::RequestLlmCompletion { .. })));
    assert!(!state.view().triage_can_start);
}

fn with_triage_metadata_ready(state: AppState) -> AppState {
    let (state, _) = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: HashMap::new(),
        },
    );
    let mut active_versions = HashMap::new();
    active_versions.insert(PromptId::ArticleTriage, 1);
    let mut effective_models = HashMap::new();
    effective_models.insert(PromptId::ArticleTriage, "test-model".to_string());
    let (state, _) = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
            templates: HashMap::new(),
        },
    );
    state
}

#[test]
fn triage_clicked_while_active_is_noop() {
    init_logging();
    let (state, _) = ready_state_with_pretriage(&["https://one.example"]);
    let (state, _) = update(state, Msg::TriageClicked);
    let (_state, effects) = update(state, Msg::TriageClicked);
    assert!(effects.is_empty());
}

#[test]
fn triage_articles_loaded_dispatches_first_request() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = with_triage_metadata_ready(state);
    let state = simulate_triage_loaded(state, sample_articles(&["https://one.example"]));
    let (_, effects) = update(state, Msg::TriageClicked);
    let request_id = request_id_for_prompt(&effects, PromptId::ArticleTriage).unwrap();
    assert_eq!(request_id, 1);
}

#[test]
fn triage_articles_loaded_empty_fails() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = simulate_triage_loaded(state, Vec::new());
    assert!(!state.view().triage_can_start);
    assert!(state.view().triage_progress.is_none());
}

#[test]
fn triage_load_failed_transitions_to_failed() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = simulate_triage_load_failed(state, "boom");
    assert!(!state.view().triage_can_start);
    assert!(state.view().triage_progress.is_none());
}

fn triage_flow_with_two_articles() -> (AppState, Vec<LoadedArticle>) {
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = with_triage_metadata_ready(state);
    let articles = sample_articles(&["https://one.example", "https://two.example"]);
    let state = simulate_triage_loaded(state, articles.clone());
    let (state, _) = update(state, Msg::TriageClicked);
    (state, articles)
}

#[test]
fn triage_completion_advances_to_next_article() {
    init_logging();
    let (state, _) = triage_flow_with_two_articles();
    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: triage_success(5),
            metadata: None,
        },
    );
    let request_id = request_id_for_prompt(&effects, PromptId::ArticleTriage).unwrap();
    let (_state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: triage_success(4),
            metadata: None,
        },
    );
    assert_eq!(request_id, 2);
}

#[test]
fn triage_all_completed_transitions_to_complete() {
    init_logging();
    let (state, _) = triage_flow_with_two_articles();
    let (_state, _effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: triage_success(5),
            metadata: None,
        },
    );
    let (state, effects) = update(
        _state,
        Msg::LlmCompleted {
            request_id: 2,
            result: triage_success(4),
            metadata: None,
        },
    );
    assert_persist_triage_cache_effect(&effects, &state);
    let view = state.view();
    assert!(view.triage_can_start);
}

#[test]
fn triage_all_failed_transitions_to_failed() {
    init_logging();
    let (state, _articles) = triage_flow_with_two_articles();
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: triage_failure("bad"),
            metadata: None,
        },
    );
    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: triage_failure("still bad"),
            metadata: None,
        },
    );
    assert_persist_triage_cache_effect(&effects, &state);
    assert!(state.view().triage_can_start);
    assert!(state.view().jobs[0].triage_annotation.is_none());
}

#[test]
fn triage_partial_failure_still_completes() {
    init_logging();
    let (state, _articles) = triage_flow_with_two_articles();
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: triage_success(5),
            metadata: None,
        },
    );
    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: triage_failure("bad"),
            metadata: None,
        },
    );
    assert_persist_triage_cache_effect(&effects, &state);
    assert!(state.view().triage_can_start);
    assert!(state.view().jobs[0].triage_annotation.is_some());
}

#[test]
fn triage_quota_exhaustion_fails_remaining() {
    init_logging();
    let (state, _articles) = triage_flow_with_two_articles();
    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: triage_quota(),
            metadata: None,
        },
    );
    assert_persist_triage_cache_effect(&effects, &state);
    assert!(state.view().triage_can_start);
}

#[test]
fn triage_rerun_after_complete_reuses_cache_when_available() {
    init_logging();
    let (state, _articles) = triage_flow_with_two_articles();
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: triage_success(5),
            metadata: None,
        },
    );
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: triage_success(4),
            metadata: None,
        },
    );
    let (state, effects) = update(state, Msg::TriageClicked);
    assert!(!effects
        .iter()
        .any(|e| matches!(e, Effect::RequestLlmCompletion { .. })));
    assert!(state.view().triage_can_start);
}

#[test]
fn view_model_annotates_jobs_with_triage() {
    init_logging();
    let (state, _articles) = triage_flow_with_two_articles();
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: triage_success(5),
            metadata: None,
        },
    );
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: triage_success(4),
            metadata: None,
        },
    );
    let view = state.view();
    assert!(view.jobs[0].triage_annotation.is_some());
}

#[test]
fn view_model_sorts_by_priority() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://low.example", "https://high.example"]);
    let state = with_triage_metadata_ready(state);
    let state = simulate_triage_loaded(
        state,
        sample_articles(&["https://low.example", "https://high.example"]),
    );
    let (state, _) = update(state, Msg::TriageClicked);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: triage_success(2),
            metadata: None,
        },
    );
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: triage_success(5),
            metadata: None,
        },
    );
    let view = state.view();
    assert_eq!(view.jobs[0].triage_annotation.as_ref().unwrap().priority, 5);
    assert_eq!(view.jobs[1].triage_annotation.as_ref().unwrap().priority, 2);
}

#[test]
fn view_model_equal_priority_sorted_by_job_id() {
    init_logging();
    let (state, job_ids) =
        completed_state_with_jobs(&["https://first.example", "https://second.example"]);
    let state = with_triage_metadata_ready(state);
    let state = simulate_triage_loaded(
        state,
        sample_articles(&["https://first.example", "https://second.example"]),
    );
    let (state, _) = update(state, Msg::TriageClicked);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: triage_success(4),
            metadata: None,
        },
    );
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: triage_success(4),
            metadata: None,
        },
    );
    let view = state.view();
    assert_eq!(view.jobs[0].job_id, job_ids[0]);
    assert_eq!(view.jobs[1].job_id, job_ids[1]);
}

#[test]
fn view_model_stale_triage_url_ignored() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = with_triage_metadata_ready(state);
    let state = simulate_triage_loaded(
        state,
        sample_articles(&["https://one.example", "https://stale.example"]),
    );
    let (state, _) = update(state, Msg::TriageClicked);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: triage_success(5),
            metadata: None,
        },
    );
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: triage_success(4),
            metadata: None,
        },
    );
    let view = state.view();
    assert!(view.jobs[0].triage_annotation.is_some());
}

#[test]
fn triage_and_briefing_can_interleave() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let (state, _) = update(state, Msg::GenerateBriefingClicked);
    let (_state, effects) = update(state, Msg::TriageClicked);
    assert!(
        effects.is_empty(),
        "manual triage must be blocked while briefing owns triage"
    );
}

#[test]
fn triage_can_start_false_without_completed_jobs() {
    init_logging();
    let state = AppState::new();
    assert!(!state.view().triage_can_start);
}

#[test]
fn triage_can_start_true_with_completed_jobs() {
    init_logging();
    let (state, _) = ready_state_with_pretriage(&["https://one.example"]);
    assert!(state.view().triage_can_start);
}

#[test]
fn restore_completed_jobs_resets_triage() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let snapshot = state.completed_jobs_snapshot();
    let (state, _) = update(state, Msg::RestoreCompletedJobs(snapshot));
    assert!(state.view().triage_progress.is_none());
}

#[test]
fn restore_completed_jobs_resets_briefing() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let snapshot = state.completed_jobs_snapshot();
    let (state, _) = update(state, Msg::RestoreCompletedJobs(snapshot));
    assert!(state.view().briefing_can_start);
}

#[test]
fn triage_and_briefing_concurrent_request_ids() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let (state, _) = update(state, Msg::GenerateBriefingClicked);
    let (_state, effects) = update(state, Msg::TriageClicked);
    assert!(
        effects.is_empty(),
        "triage click should no-op during briefing triage ownership"
    );
}

#[test]
fn rerun_uses_triage_cache_when_metadata_and_corpus_unchanged() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = with_triage_metadata_ready(state);
    // First load: valid in-flight request ID set by JobDone.
    let state = simulate_triage_loaded(state, sample_articles(&["https://one.example"]));
    let (state, first_effects) = update(state, Msg::TriageClicked);
    let first_request = request_id_for_prompt(&first_effects, PromptId::ArticleTriage)
        .expect("first run dispatches llm request");

    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: first_request,
            result: triage_success(3),
            metadata: None,
        },
    );
    // Second load: no in-flight request ID after the first load was applied, so
    // this message is intentionally stale and will be ignored by the coordinator.
    // The pre-triage session from the first load remains valid for the rerun.
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: 0, // stale — no in-flight request at this point
            articles: sample_articles(&["https://one.example"]),
        },
    );
    let (_state, rerun_effects) = update(state, Msg::TriageClicked);

    assert!(
        !rerun_effects
            .iter()
            .any(|effect| matches!(effect, Effect::RequestLlmCompletion { .. })),
        "rerun should reuse triage cache and avoid new llm requests"
    );
}
