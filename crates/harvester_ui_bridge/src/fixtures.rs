use std::collections::HashMap;

use chrono::{DateTime, Utc};
use harvester_core::{
    update, AiAvailability, AiUnavailableReason, AppState, CompletedJobSnapshot, Effect,
    LinkSnapshotRecord, LlmResultKind, Msg, SelectedJobVisibility,
};
use harvester_engine::{llm::prompt::PromptId, SourceId, SourceKind};

use crate::{project, SnapshotEnvelope};

const FIXTURE_TIME: i64 = 1_700_000_000;

pub fn named_snapshots() -> Vec<(&'static str, SnapshotEnvelope)> {
    let empty = reduce(AppState::new(), Msg::tick_at(time(0)));
    let with_corpus = idle_with_corpus(&empty);
    let with_selection = selected_fixture_state(&empty);
    let run_in_progress = run_in_progress_with_failures();
    let run_finished = completed_run_state();
    let ai_unavailable = reduce(
        empty.clone(),
        Msg::AiAvailabilityDetected {
            availability: AiAvailability::Unavailable {
                reason: AiUnavailableReason::MissingApiKey,
            },
        },
    );
    let states = [
        ("idle_empty_corpus", empty),
        ("idle_with_corpus", with_corpus),
        ("idle_with_selection", with_selection),
        ("run_in_progress_with_failures", run_in_progress),
        ("run_finished_with_notice", run_finished),
        ("ai_unavailable", ai_unavailable),
    ];
    states
        .into_iter()
        .map(|(name, state)| (name, project(&state.desktop_view()).0.with_generation(1)))
        .collect()
}

fn idle_with_corpus(empty: &AppState) -> AppState {
    let articles = vec![
        harvester_core::LoadedArticle {
            url: "https://example.invalid/lower-priority".into(),
            source_title: Some("Lower-priority fixture".into()),
            prepared_text: "lower-priority fixture article text ".repeat(200),
            content_hash: "fixture-content-hash-lower".into(),
            fetched_utc: Some("1970-01-01T00:00:00Z".into()),
        },
        harvester_core::LoadedArticle {
            url: "https://example.invalid/higher-priority".into(),
            source_title: Some("Higher-priority fixture".into()),
            prepared_text: "higher-priority fixture article text ".repeat(200),
            content_hash: "fixture-content-hash-higher".into(),
            fetched_utc: Some("1970-01-01T00:00:01Z".into()),
        },
    ];
    let state = reduce(
        empty.clone(),
        Msg::RestoreCompletedJobs(vec![
            completed_job(&articles[0].url, 0),
            completed_job(&articles[1].url, 1),
        ]),
    );
    let state = reduce(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: articles.iter().map(|article| article.url.clone()).collect(),
            triggered_by_job_done: false,
        },
    );
    let (state, load_request_id) = triage_load_request(state);
    let state = reduce(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_request_id,
            articles,
        },
    );
    let state = add_llm_metadata(state);
    let (state, first_effects) = update(state, Msg::TriageClicked);
    let first_request_id = request_id(&first_effects, PromptId::ArticleTriage, "first triage");
    let (state, second_effects) = update(state, triage_success(first_request_id, 2));
    let second_request_id = request_id(&second_effects, PromptId::ArticleTriage, "second triage");
    let state = reduce(state, triage_success(second_request_id, 5));

    let view = state.desktop_view();
    assert_eq!(
        view.desktop_job_list
            .rows
            .iter()
            .map(|row| row.job_id)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    state
}

fn selected_fixture_state(empty: &AppState) -> AppState {
    let state = reduce(
        empty.clone(),
        Msg::RestoreCompletedJobs(vec![
            CompletedJobSnapshot {
                url: "https://example.invalid/outside-checkpoint".into(),
                tokens: Some(42),
                bytes: Some(1024),
                links: vec![LinkSnapshotRecord {
                    url: "https://example.invalid/outside-checkpoint-link".into(),
                    downloaded_path: None,
                }],
                fetched_utc: Some("1970-01-01T00:00:00Z".into()),
            },
            CompletedJobSnapshot {
                url: "https://example.invalid/inside-checkpoint".into(),
                tokens: Some(43),
                bytes: Some(1025),
                links: Vec::new(),
                fetched_utc: Some("1970-01-01T00:00:02Z".into()),
            },
        ]),
    );
    let state = reduce(
        state,
        Msg::BriefingCheckpointSet(Some("1970-01-01T00:00:01Z".into())),
    );
    let state = reduce(state, Msg::JobSelected { job_id: 1 });
    let selected_view = state.desktop_view();
    assert_eq!(selected_view.desktop_job_list.rows.len(), 1);
    assert!(matches!(
        selected_view
            .desktop_job_list
            .selected_job
            .as_ref()
            .map(|selected| selected.list_visibility),
        Some(SelectedJobVisibility::OutsideScope)
    ));
    state
}

fn run_in_progress_with_failures() -> AppState {
    let (state, _, _) = prepared_article_state_with_source_failure(true);
    let state = add_llm_metadata(state);
    let state = reduce(state, Msg::PipelineRunRequested);
    let state = reduce(state, Msg::PipelineRunAdvance);
    assert!(state.desktop_view().run_progress.run_active);
    state
}

fn completed_run_state() -> AppState {
    let (mut state, article, job_id) = prepared_article_state();
    state = add_llm_metadata(state);
    let (next, _) = update(state, Msg::PipelineRunRequested);
    state = next;
    let (next, triage_effects) = update(state, Msg::PipelineRunAdvance);
    state = next;
    let triage_request_id = request_id(&triage_effects, PromptId::ArticleTriage, "triage");
    state = reduce(state, triage_success(triage_request_id, 4));

    state = reduce(state, Msg::PipelineRunAdvance);
    let (next, summary_load_effects) = update(state, Msg::PipelineRunAdvance);
    state = next;
    assert!(summary_load_effects
        .iter()
        .any(|effect| matches!(effect, Effect::LoadArticlesForBriefing { .. })));
    state = add_llm_metadata(state);
    let (next, summary_effects) = update(
        state,
        Msg::ArticlesLoaded {
            articles: vec![article],
            collection_text: "fixture collection".into(),
        },
    );
    state = next;
    state = reduce(state, Msg::PipelineRunAdvance);
    let summary_request_id = request_id(&summary_effects, PromptId::ArticleSummary, "summary");
    let (next, signal_effects) = update(state, summary_success(summary_request_id));
    state = next;
    let signal_request_id = request_id(
        &signal_effects,
        PromptId::ArticleSignalCandidate,
        "signal candidate",
    );
    state = reduce(state, signal_success(signal_request_id));
    state = reduce(state, Msg::PipelineRunAdvance);
    state = reduce(state, Msg::JobSelected { job_id });
    assert!(state.desktop_view().run_completion_notice.is_some());
    assert!(!state.desktop_view().signal_candidate_rows.is_empty());
    state
}

fn prepared_article_state() -> (AppState, harvester_core::LoadedArticle, u64) {
    prepared_article_state_with_source_failure(false)
}

fn prepared_article_state_with_source_failure(
    with_source_failure: bool,
) -> (AppState, harvester_core::LoadedArticle, u64) {
    let article = harvester_core::LoadedArticle {
        url: "https://fixture.invalid/article".into(),
        source_title: Some("Fixture article".into()),
        prepared_text: "fixture article text ".repeat(200),
        content_hash: "fixture-content-hash".into(),
        fetched_utc: Some("2023-11-14T22:13:20Z".into()),
    };
    let state = reduce(AppState::new(), Msg::tick_at(time(0)));
    let state = reduce(state, Msg::PollSourcesClicked);
    let state = reduce(
        state,
        Msg::PollStarted {
            total: if with_source_failure { 2 } else { 1 },
        },
    );
    let state = if with_source_failure {
        reduce(
            state,
            Msg::SourcePollFailed {
                source_id: SourceId::new("fixture-failed-source").expect("fixture source id"),
                error: "fixture source failure".into(),
            },
        )
    } else {
        state
    };
    let (state, effects) = update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("fixture-source").expect("fixture source id"),
            urls: vec![article.url.clone()],
            kind: SourceKind::Rss,
            parsed: 1,
            dedup_filtered: 0,
        },
    );
    let job_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::EnqueueUrl { job_id, .. } => Some(*job_id),
            _ => None,
        })
        .expect("fixture job is enqueued");
    let state = reduce(state, Msg::PipelineRunRequested);
    let state = reduce(state, Msg::AllSourcesPollEnded);
    let state = reduce(
        state,
        Msg::JobProgress {
            job_id,
            stage: harvester_core::Stage::Downloading,
            tokens: None,
            bytes: Some(1024),
            content_preview: None,
        },
    );
    let state = reduce(
        state,
        Msg::JobDone {
            job_id,
            result: harvester_core::JobResultKind::Success,
            content_preview: Some(
                "# Fixture raw text\n\n[Fixture link](https://fixture.invalid/link)".into(),
            ),
            extracted_links: vec![harvester_engine::ExtractedLink {
                url: "https://fixture.invalid/link".into(),
                text: Some("Fixture link".into()),
                kind: harvester_engine::LinkKind::Hyperlink,
            }],
            fetched_utc: article.fetched_utc.clone(),
        },
    );
    let state = reduce(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: vec![article.url.clone()],
            triggered_by_job_done: true,
        },
    );
    let (state, load_request_id) = triage_load_request(state);
    let state = reduce(
        state,
        Msg::TriageArticlesLoadProgress {
            request_id: load_request_id,
            files_scanned: 0,
            files_total: 1,
        },
    );
    let state = reduce(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_request_id,
            articles: vec![article.clone()],
        },
    );
    (state, article, job_id)
}

fn triage_load_request(mut state: AppState) -> (AppState, u64) {
    for second in 1..=20 {
        let (next, effects) = update(state, Msg::tick_at(time(second)));
        state = next;
        if let Some(request_id) = effects.iter().find_map(|effect| match effect {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        }) {
            return (state, request_id);
        }
    }
    panic!("fixture triage load request was not emitted");
}

fn add_llm_metadata(state: AppState) -> AppState {
    let active_versions = HashMap::from([
        (PromptId::ArticleTriage, 1),
        (PromptId::ArticleSummary, 1),
        (PromptId::ArticleSignalCandidate, 1),
    ]);
    let effective_models = HashMap::from([
        (PromptId::ArticleTriage, "fixture-triage-model".to_string()),
        (
            PromptId::ArticleSummary,
            "fixture-summary-model".to_string(),
        ),
        (
            PromptId::ArticleSignalCandidate,
            "fixture-signal-model".to_string(),
        ),
    ]);
    let state = reduce(
        state,
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
            templates: HashMap::new(),
        },
    );
    let state = reduce(state, Msg::PromptTemplateFilesLoaded);
    reduce(
        state,
        Msg::PromptContextsLoaded {
            contexts: HashMap::new(),
        },
    )
}

fn request_id(effects: &[Effect], prompt_id: PromptId, stage: &str) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: actual,
                ..
            } if *actual == prompt_id => Some(*request_id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("fixture {stage} LLM request: {effects:?}"))
}

fn triage_success(request_id: u64, priority: u8) -> Msg {
    Msg::LlmCompleted {
        request_id,
        result: LlmResultKind::Success {
            output_json: format!(
                r#"{{"category":"news","priority":{priority},"tags":["fixture","review"],"rationale":"fixture"}}"#
            ),
            input_tokens: 10,
            output_tokens: 5,
            prompt_version: 1,
            resolved_model: "fixture-triage-model".into(),
        },
        metadata: None,
    }
}

fn summary_success(request_id: u64) -> Msg {
    Msg::LlmCompleted {
        request_id,
        result: LlmResultKind::Success {
            output_json: r#"{"title":"Fixture summary","summary":"A fixture **summary** with a [Fixture link](https://fixture.invalid/link).","key_points":["Fixture point"]}"#.into(),
            input_tokens: 10,
            output_tokens: 5,
            prompt_version: 1,
            resolved_model: "fixture-summary-model".into(),
        },
        metadata: None,
    }
}

fn signal_success(request_id: u64) -> Msg {
    Msg::LlmCompleted {
        request_id,
        result: LlmResultKind::Success {
            output_json: r#"{"signal_score":84,"signal_key":"fixture-signal","themes":["fixture-theme"],"draft_gist":"Fixture outlet reports a concrete event with enough detail for review.","source_tier":"Tier1","confidence":"High","reasoning":"Fixture evidence is concrete and relevant."}"#.into(),
            input_tokens: 10,
            output_tokens: 5,
            prompt_version: 1,
            resolved_model: "fixture-signal-model".into(),
        },
        metadata: None,
    }
}

fn completed_job(url: &str, seconds: i64) -> CompletedJobSnapshot {
    CompletedJobSnapshot {
        url: url.into(),
        tokens: Some(42),
        bytes: Some(1024),
        links: vec![LinkSnapshotRecord {
            url: "https://example.invalid/link".into(),
            downloaded_path: None,
        }],
        fetched_utc: Some(format!("1970-01-01T00:00:0{seconds}Z")),
    }
}

fn reduce(state: AppState, msg: Msg) -> AppState {
    update(state, msg).0
}

fn time(offset_seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(FIXTURE_TIME + offset_seconds, 0)
        .expect("fixture timestamp")
        .with_timezone(&Utc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_snapshot_fixtures_match_core_projection() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/snapshots");
        for (name, envelope) in named_snapshots() {
            let path = root.join(format!("{name}.json"));
            let actual = serde_json::to_string_pretty(&envelope).unwrap() + "\n";
            if std::env::var_os("UPDATE_UI_FIXTURES").is_some() {
                std::fs::create_dir_all(&root).unwrap();
                std::fs::write(&path, actual).unwrap();
            } else {
                assert_eq!(
                    std::fs::read_to_string(&path).expect("checked-in UI fixture"),
                    actual,
                    "regenerate with UPDATE_UI_FIXTURES=1 cargo test -p harvester_ui_bridge"
                );
            }
        }
    }
}
