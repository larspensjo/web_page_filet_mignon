use super::*;

#[test]
fn archive_token_estimates_uses_summary_output_tokens_when_available() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let url = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(1);
    let mut state = add_completed_job_with_tokens_for_test(state, &url, 500);
    let key = SummaryCacheKey {
        content_hash: "hash-tc-0".to_string(),
        prompt_id: PromptId::ArticleSummary,
        prompt_version: 4,
        model_id: "claude-sonnet".to_string(),
        context_hash: "ctx".to_string(),
    };
    let result = ArticleSummaryResult {
        title: "Art".to_string(),
        summary: "summary text".to_string(),
        key_points: vec![],
        input_tokens: 100,
        output_tokens: 42,
        entities: SummaryEntities::default(),
    };
    state.store_summary_result(key, result, "2026-04-01T00:00:00Z".to_string());

    let estimates = state.archive_token_estimates(&[url]);

    assert_eq!(estimates.full_tokens, 500);
    assert_eq!(estimates.summary_tokens, 42);
    assert_eq!(estimates.summary_coverage, 1);
}

#[test]
fn view_job_rows_hide_stale_cached_summary_tokens() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let url = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(1);
    let mut state = add_completed_job_with_tokens_for_test(state, &url, 500);
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Art".to_string(),
            summary: "summary text".to_string(),
            key_points: vec![],
            input_tokens: 100,
            output_tokens: 42,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    let view = state.view();
    let row = view
        .desktop_job_list
        .rows
        .iter()
        .find(|job| job.url == url)
        .expect("expected matching job row");

    assert_eq!(row.tokens, Some(500));
    assert_eq!(row.summary_tokens, None);
    assert!(!row.has_summary);
}

#[test]
fn view_job_rows_hide_stale_pre_triage_cached_summary_tokens() {
    use crate::briefing::{ArticleSummaryResult, LoadedArticle};
    use crate::pre_triage_filter::{PreTriagePolicy, PreTriageSession};
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let url = "https://startup.example.com/a".to_string();
    let state = add_completed_job_with_tokens_for_test(AppState::new(), &url, 500);
    let mut state = state;
    let pre_triage = PreTriageSession::load_articles(
        vec![LoadedArticle {
            url: url.clone(),
            source_title: None,
            prepared_text: std::iter::repeat_n("startup", 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: "startup-hash".to_string(),
            fetched_utc: None,
        }],
        &PreTriagePolicy::default(),
    );
    state.set_pre_triage(pre_triage);
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "startup-hash".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Startup".to_string(),
            summary: "summary text".to_string(),
            key_points: vec![],
            input_tokens: 100,
            output_tokens: 42,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    let view = state.view();
    let row = view
        .desktop_job_list
        .rows
        .iter()
        .find(|job| job.url == url)
        .expect("expected matching job row");

    assert_eq!(row.tokens, Some(500));
    assert_eq!(row.summary_tokens, None);
    assert!(!row.has_summary);
}

#[test]
fn archive_token_estimates_falls_back_to_full_tokens_when_no_summary() {
    init_logging();
    let url = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(1);
    let state = add_completed_job_with_tokens_for_test(state, &url, 300);

    let estimates = state.archive_token_estimates(&[url]);

    assert_eq!(estimates.full_tokens, 300);
    assert_eq!(estimates.summary_tokens, 300);
    assert_eq!(estimates.summary_coverage, 0);
}

#[test]
fn archive_clicked_emits_real_token_estimates_from_state() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let url = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(1);
    let mut state = add_completed_job_with_tokens_for_test(state, &url, 400);
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Art".to_string(),
            summary: "s".to_string(),
            key_points: vec![],
            input_tokens: 10,
            output_tokens: 99,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    let (_, effects) = update(state, Msg::ArchiveClicked);
    let estimates = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::OpenArchiveDialog {
                token_estimates, ..
            } => Some(*token_estimates),
            _ => None,
        })
        .expect("OpenArchiveDialog expected");

    assert_eq!(estimates.full_tokens, 400);
    assert_eq!(estimates.summary_tokens, 99);
    assert_eq!(estimates.summary_coverage, 1);
}

#[test]
fn view_exposes_selected_meter_without_triage_or_download_fallback() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use crate::{JobResultKind, Stage};
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();

    // Local helper: enqueue a URL and return its job id (so we can drive it into
    // queued / in-flight / failed states the completed-job helper can't produce).
    fn enqueue(state: AppState, url: &str) -> (AppState, crate::JobId) {
        let (state, e1) = update(state, Msg::InputChanged(format!("{url}\n")));
        let (state, e2) = update(state, Msg::UrlsSubmitted);
        let job_id = e1
            .into_iter()
            .chain(e2)
            .find_map(|e| match e {
                Effect::EnqueueUrl { job_id, .. } => Some(job_id),
                _ => None,
            })
            .expect("EnqueueUrl effect");
        (state, job_id)
    }

    // (1) One article in the completed triage corpus, downloaded (500 tokens)
    //     and summarized (42 output tokens), but not scored.
    let triaged_url = "https://triage-complete.com/0".to_string();
    let state = complete_triage_state_for_test(1);
    let mut state = add_completed_job_with_tokens_for_test(state, &triaged_url, 500);
    state.store_summary_result(
        SummaryCacheKey {
            content_hash: "hash-tc-0".to_string(),
            prompt_id: PromptId::ArticleSummary,
            prompt_version: 4,
            model_id: "model".to_string(),
            context_hash: "ctx".to_string(),
        },
        ArticleSummaryResult {
            title: "Art".to_string(),
            summary: "summary text".to_string(),
            key_points: vec![],
            input_tokens: 100,
            output_tokens: 42,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    // (2) A successful download outside the triaged archive corpus. Neither
    //     download can count toward the meter without a saved current-key score.
    let state = add_completed_job_with_tokens_for_test(state, "https://fresh.example.com/new", 300);

    // (3) A FAILED job that still carries tokens (apply_done does not clear them).
    let (state, failed_id) = enqueue(state, "https://fail.example.com/x");
    let (state, _) = update(
        state,
        Msg::JobProgress {
            job_id: failed_id,
            stage: Stage::Tokenizing,
            tokens: Some(700),
            bytes: None,
        },
    );
    let (state, _) = update(
        state,
        Msg::JobDone {
            job_id: failed_id,
            result: JobResultKind::Failed {
                reason: "boom".to_string(),
            },

            extracted_links: Vec::new(),
            fetched_utc: None,
        },
    );

    // (4) An IN-FLIGHT job: tokens set via progress, never completed.
    let (state, inflight_id) = enqueue(state, "https://inflight.example.com/x");
    let (state, _) = update(
        state,
        Msg::JobProgress {
            job_id: inflight_id,
            stage: Stage::Tokenizing,
            tokens: Some(800),
            bytes: None,
        },
    );

    // (5) A QUEUED job: enqueued only, no tokens, not done.
    let (state, _queued_id) = enqueue(state, "https://queued.example.com/x");

    let state = crate::fixture_support::complete_archive_meter_startup(state);
    let view = state.view();

    assert_eq!(view.archive_meter.selected_count, 0);
    assert_eq!(view.archive_meter.token_estimate, 0);
    assert_eq!(
        view.archive_meter.status,
        crate::ArchiveMeterStatus::NotScoredYet
    );
    let (_, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::OpenArchiveDialog {
        article_count: 1, token_estimates, ..
    } if token_estimates.summary_tokens == 42)));
}
