use std::collections::HashMap;
use std::sync::Once;

use harvester_core::{AppState, Effect, JobResultKind, LlmResultKind, LoadedArticle, Msg};
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
    // Notify the coordinator about the completed corpus, mirroring what the
    // tick handler does in production after the quiet period expires.
    let ordered_urls: Vec<String> = urls.iter().map(|u| u.to_string()).collect();
    let (state, _) = update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls,
            triggered_by_job_done: true,
        },
    );
    (state, job_ids)
}

/// Advance ticks until the coordinator dispatches a `LoadArticlesForTriage` effect.
/// Panics if no dispatch occurs within 200 ticks.
fn tick_until_triage_dispatch(mut state: AppState) -> (AppState, u64) {
    for _ in 0..200 {
        let (next, effects) = update(state, Msg::tick_at(chrono::Utc::now()));
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
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
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
        resolved_model: "test-model".to_string(),
    }
}

fn triage_quota() -> LlmResultKind {
    LlmResultKind::QuotaExhausted {
        reason: "quota".to_string(),
        origin: harvester_engine::llm::QuotaOrigin::SessionBudget,
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
    assert!(
        effects.iter().any(|effect| match effect {
            Effect::SaveResults { records } => records.iter().any(|record| match record {
                harvester_core::SavedResult::Triage(key, entry) => state
                    .triage_cache()
                    .iter()
                    .any(|(k, e)| k == key && e == entry),
                _ => false,
            }),
            _ => false,
        }),
        "expected incremental triage result, got: {effects:?}"
    );
}

#[test]
fn resume_run_triages_prepared_article() {
    init_logging();
    let (state, _) = ready_state_with_pretriage(&["https://one.example"]);
    let (state, effects) = request_resume(state, sample_articles(&["https://one.example"]));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::RequestLlmCompletion { .. })));
    assert!(!state.view().run_enabled);
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
        },
    );
    state
}

#[test]
fn duplicate_resume_request_joins_active_run_without_duplicate_dispatch() {
    init_logging();
    let (state, _) = ready_state_with_pretriage(&["https://one.example"]);
    let (state, _) = request_resume(state, sample_articles(&["https://one.example"]));
    let (_state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Resume,
        },
    );
    assert!(effects.is_empty());
}

#[test]
fn triage_articles_loaded_dispatches_first_request() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = with_triage_metadata_ready(state);
    let state = simulate_triage_loaded(state, sample_articles(&["https://one.example"]));
    let (_, effects) = request_resume(state, sample_articles(&["https://one.example"]));
    let request_id = request_id_for_prompt(&effects, PromptId::ArticleTriage).unwrap();
    assert!(request_id > 0);
}

#[test]
fn triage_articles_loaded_empty_fails() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = simulate_triage_loaded(state, Vec::new());
    assert!(matches!(
        state.batch_observation().pre_triage_phase,
        harvester_core::PreTriagePhase::Failed { .. }
    ));
    assert!(!state.triage_reorder_suppressed());
}

#[test]
fn triage_load_failed_transitions_to_failed() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = simulate_triage_load_failed(state, "boom");
    assert!(!state.can_start_triage_from_pre_triage());
    assert!(!state.triage_reorder_suppressed());
}

fn triage_flow_with_two_articles() -> (AppState, Vec<LoadedArticle>) {
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = with_triage_metadata_ready(state);
    let articles = sample_articles(&["https://one.example", "https://two.example"]);
    let state = simulate_triage_loaded(state, articles.clone());
    let (state, _) = request_resume(state, articles.clone());
    (state, articles)
}

#[test]
fn triage_completion_advances_to_next_article() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = with_triage_metadata_ready(state);
    let articles = sample_articles(&["https://one.example", "https://two.example"]);
    let state = simulate_triage_loaded(state, articles.clone());
    let (state, first_effects) = request_resume(state, articles);
    let first_request = request_id_for_prompt(&first_effects, PromptId::ArticleTriage)
        .expect("PipelineRunRequested(Resume) must dispatch the first LLM request");

    let (_, second_effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: first_request,
            result: triage_success(5),
            metadata: None,
        },
    );
    let second_request = request_id_for_prompt(&second_effects, PromptId::ArticleTriage)
        .expect("completing the first article must dispatch the second LLM request");
    assert!(
        second_request > first_request,
        "second request must have a distinct, greater ID"
    );
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
    // The second triage response arrives while its Resume run is still active.
    assert!(!view.run_enabled);
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
    assert!(effects.contains(&Effect::FlushResults));
    assert!(!effects
        .iter()
        .any(|e| matches!(e, Effect::SaveResults { .. })));
    // The run is terminal after every triage request fails, so the primary Run action is enabled.
    assert!(state.view().run_enabled);
    assert!(state.view().desktop_job_list.rows[0]
        .triage_annotation
        .is_none());
}

#[test]
fn triage_partial_failure_still_completes() {
    init_logging();
    let (state, _articles) = triage_flow_with_two_articles();
    let (state, completed_effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 1,
            result: triage_success(5),
            metadata: None,
        },
    );
    assert_persist_triage_cache_effect(&completed_effects, &state);
    let (state, effects) = update(
        state,
        Msg::LlmCompleted {
            request_id: 2,
            result: triage_failure("bad"),
            metadata: None,
        },
    );
    assert!(effects.contains(&Effect::FlushResults));
    assert!(!effects
        .iter()
        .any(|e| matches!(e, Effect::SaveResults { .. })));
    // The downstream stages are still part of the active run after partial triage failure.
    assert!(!state.view().run_enabled);
    assert!(state.view().desktop_job_list.rows[0]
        .triage_annotation
        .is_some());
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
    assert!(effects.contains(&Effect::FlushResults));
    assert!(!effects
        .iter()
        .any(|e| matches!(e, Effect::SaveResults { .. })));
    // Quota exhaustion terminalizes the run; Run stays available for a later retry.
    assert!(state.view().run_enabled);
}

#[test]
fn triage_rerun_after_complete_reuses_cache_when_available() {
    init_logging();
    // Build a fresh state with pre-triage ready (two articles loaded).
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = with_triage_metadata_ready(state);
    let articles = sample_articles(&["https://one.example", "https://two.example"]);
    let state = simulate_triage_loaded(state, articles.clone());
    // First triage run: PipelineRunRequested(Resume) consumes pre-triage and starts triage.
    let (state, _) = request_resume(state, articles.clone());
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
    // Pre-triage was consumed when PipelineRunRequested(Resume) started the session; it is now
    // Idle. Trigger a fresh pre-triage evaluation so the coordinator dispatches
    // a new LoadArticlesForTriage, then load the same articles to verify cache reuse.
    let (state, _) = update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: vec!["https://one.example".to_string()],
            triggered_by_job_done: false,
        },
    );
    let articles = sample_articles(&["https://one.example", "https://two.example"]);
    let state = simulate_triage_loaded(state, articles.clone());
    let (_state, effects) = request_resume(state, articles);
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, Effect::RequestLlmCompletion { .. })));
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
    assert!(view.desktop_job_list.rows[0].triage_annotation.is_some());
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
    let (state, _) = request_resume(
        state,
        sample_articles(&["https://first.example", "https://second.example"]),
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
    assert_eq!(view.desktop_job_list.rows[0].job_id, job_ids[0]);
    assert_eq!(view.desktop_job_list.rows[1].job_id, job_ids[1]);
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
    let (state, _) = request_resume(
        state,
        sample_articles(&["https://one.example", "https://stale.example"]),
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
    assert!(view.desktop_job_list.rows[0].triage_annotation.is_some());
}

#[test]
fn run_is_enabled_without_completed_jobs() {
    init_logging();
    let state = AppState::new();
    assert!(state.view().run_enabled);
}

#[test]
fn run_is_enabled_with_completed_jobs() {
    init_logging();
    let (state, _) = ready_state_with_pretriage(&["https://one.example"]);
    assert!(state.view().run_enabled);
}

#[test]
fn restore_completed_jobs_resets_triage() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let snapshot = state.completed_jobs_snapshot();
    let (state, _) = update(state, Msg::RestoreCompletedJobs(snapshot));
    assert!(!state.triage_reorder_suppressed());
}

#[test]
fn restore_completed_jobs_resets_briefing() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let snapshot = state.completed_jobs_snapshot();
    let (state, _) = update(state, Msg::RestoreCompletedJobs(snapshot));
    assert!(state.briefing_session_can_start());
}

#[test]
fn rerun_uses_triage_cache_when_metadata_and_corpus_unchanged() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = with_triage_metadata_ready(state);
    // First load: valid in-flight request ID set by JobDone.
    let state = simulate_triage_loaded(state, sample_articles(&["https://one.example"]));
    let (state, first_effects) = request_resume(state, sample_articles(&["https://one.example"]));
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
            delta: harvester_engine::TriageArticleDelta::full_window(
                sample_articles(&["https://one.example"]),
                100_000,
            ),
        },
    );
    let (_state, rerun_effects) = request_resume(state, sample_articles(&["https://one.example"]));

    assert!(
        !rerun_effects
            .iter()
            .any(|effect| matches!(effect, Effect::RequestLlmCompletion { .. })),
        "rerun should reuse triage cache and avoid new llm requests"
    );
}

fn update(state: AppState, msg: Msg) -> (AppState, Vec<Effect>) {
    let (state, effects) = harvester_core::update(state, msg);
    harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000)
}

/// Emulate the host's asynchronous fresh-window load after an explicit Resume.
fn request_resume(state: AppState, articles: Vec<LoadedArticle>) -> (AppState, Vec<Effect>) {
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Resume,
        },
    );
    let Some(request_id) = effects.iter().find_map(|effect| match effect {
        Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
        _ => None,
    }) else {
        return (state, effects);
    };
    update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    )
}

#[test]
fn resume_run_retries_failed_work_in_a_new_run() {
    let urls = ["https://retry.example/article"];
    let (state, _) = ready_state_with_pretriage(&urls);
    let (state, effects) = request_resume(state, sample_articles(&urls));
    assert!(state.pipeline_run_armed());
    let request_id = request_id_for_prompt(&effects, PromptId::ArticleTriage).unwrap();
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: triage_failure("temporary"),
            metadata: None,
        },
    );
    assert!(!state.view().run_progress.run_active);
    assert!(state.pipeline_activity().is_settled());
    let original_run = state.pipeline_waves().waves()[0].run_id;
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Resume,
        },
    );
    let request_id = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("Resume refreshes membership");
    assert!(state.pipeline_activity().intake_refresh_pending);
    let (state, effects) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(
                sample_articles(&urls),
                100_000,
            ),
        },
    );
    assert_eq!(state.batch_observation().triage_total, 1);
    assert_eq!(state.batch_observation().triage_failed, 0);
    assert_ne!(
        state.pipeline_waves().waves().last().unwrap().run_id,
        original_run
    );
    let request_id = request_id_for_prompt(&effects, PromptId::ArticleTriage).unwrap();
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: triage_success(1),
            metadata: None,
        },
    );
    assert!(!state.view().run_progress.run_active);
    assert!(state.pipeline_activity().is_settled());
}

#[test]
fn current_triage_survives_resume_and_departed_members_leave_the_session() {
    let urls = ["https://retained.example/article"];
    let (state, _) = ready_state_with_pretriage(&urls);
    let (state, effects) = request_resume(state, sample_articles(&urls));
    let request_id = request_id_for_prompt(&effects, PromptId::ArticleTriage).unwrap();
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id,
            result: triage_success(1),
            metadata: None,
        },
    );
    let original = state.triage_cache().clone();
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Resume,
        },
    );
    let load_id = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    let (state, effects) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_id,
            delta: harvester_engine::TriageArticleDelta::full_window(
                sample_articles(&urls),
                100_000,
            ),
        },
    );
    assert_eq!(state.triage_cache(), &original);
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    assert!(!state.view().run_progress.run_active);
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Resume,
        },
    );
    let load_id = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    let (state, effects) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_id,
            delta: harvester_engine::TriageArticleDelta::full_window(vec![], 100_000),
        },
    );
    assert_eq!(state.batch_observation().triage_total, 0);
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::RequestLlmCompletion { .. })));
    assert!(state.pipeline_activity().is_settled());
}

#[test]
fn resume_run_enters_the_pipeline_with_retired_briefing_controls_disabled() {
    init_logging();
    let (state, _) = completed_state_with_jobs(&["https://one.example"]);
    let state = with_triage_metadata_ready(state);
    let state = simulate_triage_loaded(state, sample_articles(&["https://one.example"]));
    let (state, effects) = request_resume(state, sample_articles(&["https://one.example"]));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::RequestLlmCompletion {
            prompt_id: PromptId::ArticleTriage,
            ..
        }
    )));
    assert!(!state.view().run_enabled);
}
