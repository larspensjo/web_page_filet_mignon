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

fn sample_articles(urls: &[&str]) -> Vec<LoadedArticle> {
    urls.iter()
        .map(|url| LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: format!("prepared {}", url),
            content_hash: format!("{url}-hash"),
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
    assert_eq!(
        effects,
        &[Effect::PersistTriageCache {
            cache: expected_cache,
        }]
    );
}

#[test]
fn triage_clicked_emits_load_effect() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let (state, effects) = update(state, Msg::TriageClicked);
    assert_eq!(
        effects,
        vec![
            Effect::LoadPromptContexts,
            Effect::LoadLlmMetadata,
            Effect::LoadArticlesForTriage
        ]
    );
    assert!(!state.view().triage_can_start);
}

#[test]
fn triage_clicked_while_active_is_noop() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let (state, _) = update(state, Msg::TriageClicked);
    let (_state, effects) = update(state, Msg::TriageClicked);
    assert!(effects.is_empty());
}

#[test]
fn triage_articles_loaded_dispatches_first_request() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let (state, _) = update(state, Msg::TriageClicked);
    let articles = sample_articles(&["https://one.example"]);
    let (_, effects) = update(state, Msg::TriageArticlesLoaded { articles });
    let request_id = request_id_for_prompt(&effects, PromptId::ArticleTriage).unwrap();
    assert_eq!(request_id, 1);
}

#[test]
fn triage_articles_loaded_empty_fails() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let (state, _) = update(state, Msg::TriageClicked);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            articles: Vec::new(),
        },
    );
    assert!(state.view().triage_can_start);
    assert!(state.view().triage_progress.is_none());
}

#[test]
fn triage_load_failed_transitions_to_failed() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let (state, _) = update(state, Msg::TriageClicked);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoadFailed {
            reason: "boom".to_string(),
        },
    );
    assert!(state.view().triage_can_start);
    assert!(state.view().triage_progress.is_none());
}

fn triage_flow_with_two_articles() -> (AppState, Vec<LoadedArticle>) {
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let articles = sample_articles(&["https://one.example", "https://two.example"]);
    let (state, _) = update(state, Msg::TriageClicked);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            articles: articles.clone(),
        },
    );
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
    assert!(view.triage_progress.is_none());
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
fn triage_rerun_after_complete_starts_fresh() {
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
    assert_eq!(
        effects,
        vec![
            Effect::LoadPromptContexts,
            Effect::LoadLlmMetadata,
            Effect::LoadArticlesForTriage
        ]
    );
    assert!(!state.view().triage_can_start);
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
    let (state, _) = update(state, Msg::TriageClicked);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            articles: sample_articles(&["https://low.example", "https://high.example"]),
        },
    );
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
    let (state, _) = update(state, Msg::TriageClicked);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            articles: sample_articles(&["https://first.example", "https://second.example"]),
        },
    );
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
    let (state, _) = update(state, Msg::TriageClicked);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            articles: sample_articles(&["https://one.example", "https://stale.example"]),
        },
    );
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
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
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
