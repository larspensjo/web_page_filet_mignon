use std::collections::HashMap;
use std::sync::Once;

use super::*;
use crate::briefing::LoadedArticle;
use crate::LlmResultKind;
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::run_metadata::{CacheStatus, LlmRunMetadata, LlmRunMetadataInit};

pub(super) fn init_logging() {
    static INIT: Once = Once::new();
    INIT.call_once(engine_logging::initialize_for_tests);
}

pub(super) fn loaded_articles() -> (Vec<LoadedArticle>, String) {
    fn long_text(prefix: &str) -> String {
        format!(
            "{prefix} {}",
            std::iter::repeat_n("content", 220)
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
    let articles = vec![
        LoadedArticle {
            url: "https://example.com/a".to_string(),
            source_title: Some("Article A".to_string()),
            prepared_text: long_text("Article A text"),
            content_hash: "hash-a".to_string(),
            fetched_utc: None,
        },
        LoadedArticle {
            url: "https://example.com/b".to_string(),
            source_title: Some("Article B".to_string()),
            prepared_text: long_text("Article B text"),
            content_hash: "hash-b".to_string(),
            fetched_utc: None,
        },
    ];
    (articles, "Collection text".to_string())
}

pub(super) fn loaded_single_article() -> (Vec<LoadedArticle>, String) {
    let articles = vec![LoadedArticle {
        url: "https://example.com/a".to_string(),
        source_title: Some("Article A".to_string()),
        prepared_text: format!(
            "Article A text {}",
            std::iter::repeat_n("content", 220)
                .collect::<Vec<_>>()
                .join(" ")
        ),
        content_hash: "hash-a".to_string(),
        fetched_utc: None,
    }];
    (articles, "Collection text".to_string())
}

pub(super) fn with_summary_metadata(state: AppState) -> AppState {
    let mut active_versions = HashMap::new();
    active_versions.insert(PromptId::ArticleTriage, 1);
    active_versions.insert(PromptId::ArticleSummary, 1);
    let mut effective_models = HashMap::new();
    effective_models.insert(PromptId::ArticleTriage, "test-triage-model".to_string());
    effective_models.insert(PromptId::ArticleSummary, "test-model".to_string());
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

pub(super) fn summary_json(title: &str) -> String {
    format!("{{\"title\":\"{title}\",\"summary\":\"Summary\",\"key_points\":[\"p1\"]}}")
}

pub(super) fn briefing_json(article_count: u32) -> String {
    format!(
        "{{\"executive_summary\":\"Exec\",\"top_stories\":[{{\"headline\":\"Story\",\"body\":\"Desc\"}}],\"article_count\":{article_count}}}"
    )
}

pub(super) fn aggregate_briefing_metadata(
    model: &str,
    input_tokens: u32,
    output_tokens: u32,
) -> LlmRunMetadata {
    LlmRunMetadata::new(LlmRunMetadataInit {
        prompt_id: PromptId::AggregateBriefing,
        prompt_version: 1,
        resolved_model: model.to_string(),
        input_bytes: 100,
        input_tokens,
        output_tokens,
        cached_input_tokens: 0,
        cost_microdollars: 0,
        wall_ms: 10,
        parse_ok: true,
        validation_error: None,
        cache_status: CacheStatus::Miss,
        timestamp_utc: "2026-01-01T00:00:00Z".to_string(),
    })
}

pub(super) fn request_id_for_prompt(effects: &[Effect], prompt_id: PromptId) -> Option<u64> {
    effects.iter().find_map(|effect| match effect {
        Effect::RequestLlmCompletion {
            request_id,
            prompt_id: pid,
            ..
        } if *pid == prompt_id => Some(*request_id),
        _ => None,
    })
}

pub(super) fn start_briefing_after_triage(
    state: AppState,
    articles: Vec<LoadedArticle>,
) -> AppState {
    let (state, _) = update(state, Msg::GenerateBriefingClicked);
    let state = with_summary_metadata(state);
    let (mut state, effects) = update(
        state,
        Msg::BriefingPrereqArticlesLoaded {
            articles: articles.clone(),
        },
    );
    let mut triage_request_id =
        request_id_for_prompt(&effects, PromptId::ArticleTriage).expect("triage request");

    for idx in 0..articles.len() {
        let priority = (articles.len() - idx + 1) as u8;
        let (next_state, next_effects) = update(
            state,
            Msg::LlmCompleted {
                request_id: triage_request_id,
                result: LlmResultKind::Success {
                    output_json: format!(
                        "{{\"category\":\"security\",\"priority\":{},\"tags\":[\"tag\"],\"rationale\":\"reason\"}}",
                        priority
                    ),
                    input_tokens: 10,
                    output_tokens: 5,
                    prompt_version: 1,
                    model_id: "test-model".to_string(),
                },
                metadata: None,
            },
        );
        state = next_state;
        if idx + 1 < articles.len() {
            triage_request_id = request_id_for_prompt(&next_effects, PromptId::ArticleTriage)
                .expect("next triage request");
        } else {
            assert!(
                next_effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::LoadArticlesForBriefing { .. })),
                "final triage completion should trigger filtered briefing load"
            );
        }
    }
    state
}

pub(super) fn make_state_with_summarized_job_for_update() -> AppState {
    use crate::briefing::ArticleSummaryResult;
    let mut state = AppState::new();
    let url = "https://open-browser.example/article".to_string();
    state.restore_completed_jobs(vec![crate::CompletedJobSnapshot {
        url: url.clone(),
        tokens: None,
        bytes: None,
        links: vec![],
        fetched_utc: None,
    }]);
    let mut briefing = crate::briefing::BriefingSession::new_loading(None);
    briefing.set_articles(
        vec![LoadedArticle {
            url: url.clone(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash".to_string(),
            fetched_utc: None,
        }],
        "collection".to_string(),
    );
    briefing.transition_to_summarizing();
    briefing.start_article(0, 1);
    briefing.complete_article(
        0,
        ArticleSummaryResult {
            title: "Article".to_string(),
            summary: "Summary".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        },
    );
    state.set_briefing(briefing);
    let job_id = state.view().jobs.first().map(|j| j.job_id).unwrap_or(1);
    state.select_job(job_id);
    state
}

pub(super) fn loaded_triage_articles(count: usize) -> Vec<LoadedArticle> {
    (0..count)
        .map(|i| LoadedArticle {
            url: format!("https://example.com/{i}"),
            source_title: None,
            prepared_text: std::iter::repeat_n(format!("article-{i}-content"), 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("hash-{i}"),
            fetched_utc: None,
        })
        .collect()
}

pub(super) fn triage_json() -> String {
    r#"{"category":"news","priority":3,"tags":["tag"],"rationale":"ok"}"#.to_string()
}

pub(super) fn triage_success(request_id: u64) -> Msg {
    Msg::LlmCompleted {
        request_id,
        result: LlmResultKind::Success {
            output_json: triage_json(),
            input_tokens: 10,
            output_tokens: 5,
            prompt_version: 1,
            model_id: "test-model".to_string(),
        },
        metadata: None,
    }
}

pub(super) fn start_triage_for_test(
    state: AppState,
    articles: Vec<LoadedArticle>,
) -> (AppState, Vec<Effect>) {
    let state = prime_llm_metadata(state);
    let mut state = state;
    let triage_request_id = state.alloc_triage_request_id();
    state.set_triage_in_flight(triage_request_id);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: triage_request_id,
            articles,
        },
    );
    update(state, Msg::TriageClicked)
}

pub(super) fn prepare_type_url_snapshot(state: &mut AppState, snapshot: &str) {
    state
        .prompt_lab_mut()
        .select_input_source(crate::prompt_lab::PromptLabInputSource::TypeUrl);
    state
        .prompt_lab_mut()
        .set_url_input("https://example.com".to_string());
    let resolve_id = state.allocate_next_prompt_lab_resolve_id();
    state.prompt_lab_mut().begin_url_resolution(resolve_id);
    state
        .prompt_lab_mut()
        .finish_url_resolution(resolve_id, Ok(snapshot.to_string()));
}

pub(super) fn dispatch_lab_run(state: AppState) -> (AppState, u64) {
    let mut state = state;
    prepare_type_url_snapshot(&mut state, "article content");
    let (state, effects) = update(state, Msg::PromptLabRunRequested);
    let request_id = effects
        .iter()
        .find_map(|e| {
            if let Effect::RequestLlmCompletion { request_id, .. } = e {
                Some(*request_id)
            } else {
                None
            }
        })
        .expect("expected RequestLlmCompletion effect");
    (state, request_id)
}

pub(super) fn prime_llm_metadata(state: AppState) -> AppState {
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
    let (state, _) = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: HashMap::new(),
        },
    );
    state
}

pub(super) fn tick_until_dispatch(mut state: AppState) -> (AppState, u64) {
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

pub(super) fn add_completed_job_for_test(state: AppState, url: &str) -> AppState {
    use crate::JobResultKind;
    let (state, effects) = update(state, Msg::InputChanged(format!("{url}\n")));
    let (state, effects2) = update(state, Msg::UrlsSubmitted);
    let job_id = effects
        .into_iter()
        .chain(effects2)
        .find_map(|e| match e {
            Effect::EnqueueUrl { job_id, .. } => Some(job_id),
            _ => None,
        })
        .expect("EnqueueUrl effect expected");
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
    apply_pending_pre_triage_refresh_evaluation(state)
}

pub(super) fn add_completed_job_with_tokens_for_test(
    state: AppState,
    url: &str,
    tokens: u32,
) -> AppState {
    use crate::JobResultKind;
    let (state, effects) = update(state, Msg::InputChanged(format!("{url}\n")));
    let (state, effects2) = update(state, Msg::UrlsSubmitted);
    let job_id = effects
        .into_iter()
        .chain(effects2)
        .find_map(|e| match e {
            Effect::EnqueueUrl { job_id, .. } => Some(job_id),
            _ => None,
        })
        .expect("EnqueueUrl effect expected");
    let (state, _) = update(
        state,
        Msg::JobProgress {
            job_id,
            stage: crate::Stage::Tokenizing,
            tokens: Some(tokens),
            bytes: None,
            content_preview: None,
        },
    );
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
    apply_pending_pre_triage_refresh_evaluation(state)
}

pub(super) fn apply_pending_pre_triage_refresh_evaluation(mut state: AppState) -> AppState {
    if let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request() {
        let ordered_urls = state.ordered_completed_job_urls_snapshot();
        let (next, _) = update(
            state,
            Msg::EvaluatePreTriageRefresh {
                ordered_urls,
                triggered_by_job_done,
            },
        );
        next
    } else {
        state
    }
}

pub(super) fn loaded_pre_triage_articles(urls: &[&str]) -> Vec<LoadedArticle> {
    urls.iter()
        .map(|url| LoadedArticle {
            url: (*url).to_string(),
            source_title: None,
            prepared_text: std::iter::repeat_n("pre-triage-content", 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("hash-{url}"),
            fetched_utc: None,
        })
        .collect()
}

pub(super) fn ready_pre_triage_state(urls: &[&str]) -> AppState {
    let mut state = AppState::new();
    for url in urls {
        state = add_completed_job_for_test(state, url);
    }
    let (state, request_id) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id,
            articles: loaded_pre_triage_articles(urls),
        },
    );
    assert!(matches!(
        state.pre_triage().phase(),
        crate::pre_triage_filter::PreTriagePhase::ReadyToTriage
    ));
    state
}
