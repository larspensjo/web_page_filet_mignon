use super::*;

#[test]
fn operation_progress_from_poll() {
    let mut state = AppState::new();
    assert!(state.start_poll());
    state.set_poll_total(3);

    let view = state.view();
    assert_eq!(
        view.operation_progress,
        Some(OperationProgress {
            label: "Scanning sources".to_string(),
            completed: 0,
            total: 3,
        })
    );
}

#[test]
fn operation_progress_from_triage() {
    use crate::briefing::LoadedArticle;
    use crate::triage::ArticleTriageResult;

    let mut state = AppState::new();
    let mut triage = crate::triage::TriageSession::new_loading(None);
    triage.set_articles(vec![
        LoadedArticle {
            url: "https://example.com/1".to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash-1".to_string(),
            fetched_utc: None,
        },
        LoadedArticle {
            url: "https://example.com/2".to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash-2".to_string(),
            fetched_utc: None,
        },
    ]);
    triage.transition_to_triaging();
    triage.start_article(0, 1);
    triage.complete_article(
        0,
        ArticleTriageResult {
            category: "News".to_string(),
            priority: 5,
            tags: Vec::new(),
            rationale: "ok".to_string(),
            input_tokens: 1,
            output_tokens: 1,
        },
    );
    state.set_triage(triage);

    let view = state.view();
    assert_eq!(
        view.operation_progress,
        Some(OperationProgress {
            label: "Triaging".to_string(),
            completed: 1,
            total: 2,
        })
    );
}

#[test]
fn operation_progress_from_briefing() {
    use crate::briefing::{ArticleSummaryResult, LoadedArticle};

    let mut state = AppState::new();
    let mut briefing = crate::briefing::BriefingSession::new_loading(None);
    briefing.set_articles(
        vec![
            LoadedArticle {
                url: "https://example.com/1".to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash-1".to_string(),
                fetched_utc: None,
            },
            LoadedArticle {
                url: "https://example.com/2".to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash-2".to_string(),
                fetched_utc: None,
            },
        ],
        "collection".to_string(),
    );
    briefing.transition_to_summarizing();
    briefing.start_article(0, 1);
    briefing.complete_article(
        0,
        ArticleSummaryResult {
            title: "Title".to_string(),
            summary: "Summary".to_string(),
            key_points: Vec::new(),
            input_tokens: 1,
            output_tokens: 1,
            entities: Default::default(),
        },
    );
    state.set_briefing(briefing);

    let view = state.view();
    assert_eq!(
        view.operation_progress,
        Some(OperationProgress {
            label: "Summarizing".to_string(),
            completed: 1,
            total: 2,
        })
    );
}

#[test]
fn operation_progress_from_pre_triage_loading() {
    let state = startup_pre_triage_loading_state();

    let view = state.view();
    assert_eq!(
        view.operation_progress,
        Some(OperationProgress {
            label: "Preparing triage list".to_string(),
            completed: 0,
            total: 1,
        })
    );
    assert_eq!(
        view.triage_blocked_reason,
        Some("Triage is unavailable while startup prepares the article set".to_string())
    );
}

#[test]
fn operation_progress_from_pre_triage_scan_progress() {
    let mut state = startup_pre_triage_loading_state();
    state.set_triage_in_flight(7);
    state.set_pre_triage_load_progress(7, 42, 190);

    let view = state.view();
    assert_eq!(
        view.operation_progress,
        Some(OperationProgress {
            label: "Preparing triage list".to_string(),
            completed: 42,
            total: 190,
        })
    );
}

#[test]
fn operation_progress_from_pre_triage_loading_falls_back_when_total_unknown() {
    let mut state = startup_pre_triage_loading_state();
    state.set_triage_in_flight(7);
    state.set_pre_triage_load_progress(7, 0, 0);

    let view = state.view();
    assert_eq!(
        view.operation_progress,
        Some(OperationProgress {
            label: "Preparing triage list".to_string(),
            completed: 0,
            total: 1,
        })
    );
}

#[test]
fn operation_progress_poll_takes_precedence() {
    use crate::briefing::LoadedArticle;

    let mut state = AppState::new();
    assert!(state.start_poll());
    state.set_poll_total(4);

    let mut triage = crate::triage::TriageSession::new_loading(None);
    triage.set_articles(vec![LoadedArticle {
        url: "https://example.com/1".to_string(),
        source_title: None,
        prepared_text: "text".to_string(),
        content_hash: "hash-1".to_string(),
        fetched_utc: None,
    }]);
    triage.transition_to_triaging();
    state.set_triage(triage);

    let view = state.view();
    assert_eq!(
        view.operation_progress,
        Some(OperationProgress {
            label: "Scanning sources".to_string(),
            completed: 0,
            total: 4,
        })
    );
}

#[test]
fn operation_progress_poll_still_takes_precedence_over_pre_triage_loading() {
    let mut state = startup_pre_triage_loading_state();
    assert!(state.start_poll());
    state.set_poll_total(4);

    let view = state.view();
    assert_eq!(
        view.operation_progress,
        Some(OperationProgress {
            label: "Scanning sources".to_string(),
            completed: 0,
            total: 4,
        })
    );
}

#[test]
fn operation_progress_from_poll_article_downloads_after_source_scan() {
    let mut state = AppState::new();
    assert!(state.start_poll());
    state.jobs.insert(
        1,
        JobState {
            url: "https://example.com/poll".to_string(),
            stage: Stage::Queued,
            outcome: None,
            ..Default::default()
        },
    );
    state.record_poll_pipeline_jobs(&[1]);
    state.end_poll();

    let view = state.view();
    assert_eq!(
        view.operation_progress,
        Some(OperationProgress {
            label: "Downloading articles".to_string(),
            completed: 0,
            total: 1,
        })
    );
    assert!(state.layout_view().operation_progress_visible);
}

#[test]
fn poll_article_download_progress_wins_over_background_candidate_update() {
    let mut state = startup_pre_triage_loading_state();
    assert!(state.start_poll());
    state.jobs.insert(
        1,
        JobState {
            url: "https://example.com/poll".to_string(),
            stage: Stage::Queued,
            outcome: None,
            ..Default::default()
        },
    );
    state.record_poll_pipeline_jobs(&[1]);
    state.end_poll();

    let view = state.view();
    assert_eq!(
        view.operation_progress,
        Some(OperationProgress {
            label: "Downloading articles".to_string(),
            completed: 0,
            total: 1,
        })
    );
}

#[test]
fn active_triage_progress_wins_over_poll_article_downloads() {
    use crate::briefing::LoadedArticle;

    let mut state = AppState::new();
    assert!(state.start_poll());
    state.jobs.insert(
        1,
        JobState {
            url: "https://example.com/poll".to_string(),
            stage: Stage::Queued,
            outcome: None,
            ..Default::default()
        },
    );
    state.record_poll_pipeline_jobs(&[1]);
    state.end_poll();

    let mut triage = crate::triage::TriageSession::new_loading(None);
    triage.set_articles(vec![LoadedArticle {
        url: "https://example.com/triage".to_string(),
        source_title: None,
        prepared_text: "text".to_string(),
        content_hash: "hash-triage".to_string(),
        fetched_utc: None,
    }]);
    triage.transition_to_triaging();
    state.set_triage(triage);

    assert_eq!(
        state.view().operation_progress,
        Some(OperationProgress {
            label: "Triaging".to_string(),
            completed: 0,
            total: 1,
        })
    );
}

#[test]
fn active_summary_progress_wins_over_poll_article_downloads() {
    use crate::briefing::LoadedArticle;

    let mut state = AppState::new();
    assert!(state.start_poll());
    state.jobs.insert(
        1,
        JobState {
            url: "https://example.com/poll".to_string(),
            stage: Stage::Queued,
            outcome: None,
            ..Default::default()
        },
    );
    state.record_poll_pipeline_jobs(&[1]);
    state.end_poll();

    let mut briefing = crate::briefing::BriefingSession::new_loading(None);
    briefing.set_articles(
        vec![LoadedArticle {
            url: "https://example.com/summary".to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: "hash-summary".to_string(),
            fetched_utc: None,
        }],
        "collection".to_string(),
    );
    briefing.transition_to_summarizing();
    state.set_briefing(briefing);

    assert_eq!(
        state.view().operation_progress,
        Some(OperationProgress {
            label: "Summarizing".to_string(),
            completed: 0,
            total: 1,
        })
    );
}

#[test]
fn poll_article_failed_jobs_count_as_settled() {
    let mut state = AppState::new();
    assert!(state.start_poll());
    state.jobs.insert(
        1,
        JobState {
            url: "https://example.com/poll".to_string(),
            stage: Stage::Done,
            outcome: Some(JobResultKind::Failed {
                reason: "fetch failed".to_string(),
            }),
            ..Default::default()
        },
    );
    state.record_poll_pipeline_jobs(&[1]);
    state.end_poll();

    assert!(state.view().operation_progress.is_none());
}

#[test]
fn settled_poll_article_jobs_clear_pipeline_tracker() {
    let mut state = AppState::new();
    assert!(state.start_poll());
    state.jobs.insert(
        1,
        JobState {
            url: "https://example.com/poll".to_string(),
            stage: Stage::Queued,
            outcome: None,
            ..Default::default()
        },
    );
    state.record_poll_pipeline_jobs(&[1]);
    state.end_poll();
    assert!(state.poll_pipeline.is_some());

    state.apply_done(1, JobResultKind::Success, None, Vec::new(), None);

    assert!(state.poll_pipeline.is_none());
    assert!(state.view().operation_progress.is_none());
}

#[test]
fn restored_jobs_do_not_inflate_current_poll_article_progress() {
    let mut state = AppState::new();
    state.jobs.insert(
        1,
        JobState {
            url: "https://example.com/restored".to_string(),
            stage: Stage::Done,
            outcome: Some(JobResultKind::Success),
            ..Default::default()
        },
    );
    assert!(state.start_poll());
    state.jobs.insert(
        2,
        JobState {
            url: "https://example.com/current".to_string(),
            stage: Stage::Queued,
            outcome: None,
            ..Default::default()
        },
    );
    state.record_poll_pipeline_jobs(&[2]);
    state.end_poll();

    assert_eq!(
        state.view().operation_progress,
        Some(OperationProgress {
            label: "Downloading articles".to_string(),
            completed: 0,
            total: 1,
        })
    );
}

#[test]
fn zero_emission_poll_has_no_download_phase_after_source_scan() {
    let mut state = AppState::new();
    assert!(state.start_poll());
    state.end_poll();

    assert!(state.poll_pipeline.is_none());
    assert!(state.view().operation_progress.is_none());
}

#[test]
fn indirect_ingestion_does_not_add_jobs_to_poll_pipeline_tracker() {
    let mut state = AppState::new();
    assert!(state.start_poll());
    let ingest = state.ingest_indirect_links(
        vec![IndirectLink {
            url: "https://example.com/indirect".to_string(),
            source_job_id: 99,
        }],
        chrono::Utc::now(),
    );
    assert_eq!(ingest.enqueued, 1);
    state.end_poll();

    assert!(state.view().operation_progress.is_none());
}

#[test]
fn operation_progress_triage_still_takes_precedence_over_pre_triage_loading() {
    use crate::briefing::LoadedArticle;

    let mut state = startup_pre_triage_loading_state();
    let mut triage = crate::triage::TriageSession::new_loading(None);
    triage.set_articles(vec![LoadedArticle {
        url: "https://example.com/1".to_string(),
        source_title: None,
        prepared_text: "text".to_string(),
        content_hash: "hash-1".to_string(),
        fetched_utc: None,
    }]);
    triage.transition_to_triaging();
    state.set_triage(triage);

    let view = state.view();
    assert_eq!(
        view.operation_progress,
        Some(OperationProgress {
            label: "Triaging".to_string(),
            completed: 0,
            total: 1,
        })
    );
}

#[test]
fn operation_progress_none_when_idle() {
    let state = AppState::new();
    let view = state.view();
    assert!(view.operation_progress.is_none());
}

#[test]
fn operation_progress_none_after_pre_triage_ready() {
    let mut state = startup_pre_triage_loading_state();
    let pre_triage = PreTriageSession::load_articles(
        vec![article_with_words("https://example.com/ready", 220)],
        &PreTriagePolicy::default(),
    );
    state.set_pre_triage(pre_triage);

    let view = state.view();
    assert!(view.operation_progress.is_none());
    assert!(!view.triage_results_reorder_suppressed);
}

#[test]
fn operation_progress_none_during_triage_loading() {
    let mut state = AppState::new();
    let triage = crate::triage::TriageSession::new_loading(None);
    state.set_triage(triage);

    let view = state.view();
    assert!(view.operation_progress.is_none());
}
