use super::*;

#[test]
fn pre_triage_refresh_dispatch_includes_briefing_checkpoint_since_utc() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    state.set_briefing_since_utc(Some(since));

    for _ in 0..200 {
        let (next, effects) = update(state, Msg::tick_at(chrono::Utc::now()));
        state = next;
        if let Some(effect_since_utc) = effects.iter().find_map(|e| match e {
            Effect::LoadArticlesForTriage { since_utc, .. } => *since_utc,
            _ => None,
        }) {
            assert_eq!(effect_since_utc, since);
            return;
        }
    }

    panic!("no LoadArticlesForTriage dispatch within 200 ticks");
}

#[test]
fn archive_clicked_emits_open_dialog_with_request_id_and_article_count() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, effects) = update(state, Msg::ArchiveClicked);
    assert_eq!(state.archive_request_id(), 1);
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::OpenArchiveDialog { .. }))
        .expect("OpenArchiveDialog effect expected");
    match effect {
        Effect::OpenArchiveDialog {
            request_id,
            article_count,
            since_utc,
            default_basename,
            ..
        } => {
            assert_eq!(request_id, 1);
            assert_eq!(article_count, 0);
            assert!(since_utc.is_none());
            assert_eq!(default_basename, "archive.md");
        }
        _ => unreachable!(),
    }
}

fn archive_submit_message(request_id: u64) -> Msg {
    Msg::ArchiveDialogSubmitted {
        request_id,
        basename: "archive.md".into(),
        set_checkpoint: false,
        submitted_at: chrono::Utc::now(),
        use_summaries: false,
        use_signal_candidates: false,
    }
}

fn archive_request_id(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::OpenArchiveDialog { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("ArchiveClicked opens a dialog")
}

#[test]
fn archive_submit_and_open_are_gated_while_run_is_active() {
    init_logging();
    let state = complete_triage_state_for_test(1);
    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = archive_request_id(&open_effects);
    let (state, _) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    assert_eq!(state.run_state(), crate::RunState::Active);
    assert!(!state.export_available());

    let (state, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects.is_empty());
    assert_eq!(state.archive_request_id(), request_id);

    let (state, effects) = update(state, archive_submit_message(request_id));
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::ArchiveRequested { .. })));
    assert_eq!(
        state.briefing_checkpoint_status_message(),
        Some("Export is unavailable while a run is in progress")
    );
}

#[test]
fn archive_submit_is_gated_until_a_stopping_run_finishes_draining() {
    init_logging();
    let state = complete_triage_state_for_test(1);
    let (state, _) = update(
        state,
        Msg::AiAvailabilityDetected {
            availability: crate::AiAvailability::Unavailable {
                reason: crate::AiUnavailableReason::MissingApiKey,
            },
        },
    );
    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = archive_request_id(&open_effects);
    let (state, _) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Full,
        },
    );
    let (state, _) = update(state, Msg::PollStarted { total: 1 });
    let (state, effects) = update(state, Msg::StopFinishClicked);
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::StopFinish { .. })));
    assert_eq!(
        state.run_state(),
        crate::RunState::Stopping { in_flight: 1 }
    );
    assert!(!state.export_available());

    let (state, effects) = update(state, archive_submit_message(request_id));
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::ArchiveRequested { .. })));
    assert_eq!(
        state.briefing_checkpoint_status_message(),
        Some("Export is unavailable while a run is in progress")
    );
    let (state, _) = update(state, Msg::AllSourcesPollEnded);
    assert_eq!(state.run_state(), crate::RunState::Idle);
    assert_eq!(state.briefing_checkpoint_status_message(), None);
}

#[test]
fn archive_export_in_flight_disables_archive_and_shows_progress_until_completion() {
    init_logging();
    let state = complete_triage_state_for_test(1);
    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = archive_request_id(&open_effects);
    let (state, effects) = update(state, archive_submit_message(request_id));
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::ArchiveRequested { .. })));
    assert!(!state.view().archive_enabled);
    assert_eq!(
        state.view().checkpoint_status_message.as_deref(),
        Some("Exporting archive...")
    );

    // A second click while the worker is writing must not start a competing export.
    let (state, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects.is_empty());
    assert_eq!(state.archive_request_id(), request_id);

    let (state, _) = update(
        state,
        Msg::ArchiveExportCompleted {
            request_id,
            path: "archive.md".into(),
            doc_count: 1,
            requested_checkpoint: None,
        },
    );
    assert!(state.view().archive_enabled);
    assert_eq!(state.view().checkpoint_status_message, None);
    let (_, effects) = update(state, Msg::ArchiveClicked);
    assert_eq!(archive_request_id(&effects), request_id + 1);
}

#[test]
fn archive_export_failure_reenables_archive_and_reports_reason_until_next_open() {
    init_logging();
    let state = complete_triage_state_for_test(1);
    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = archive_request_id(&open_effects);
    let (state, _) = update(state, archive_submit_message(request_id));
    let (state, effects) = update(
        state,
        Msg::ArchiveExportFailed {
            request_id,
            basename: "archive.md".into(),
            reason: "disk full".into(),
        },
    );
    assert!(effects.is_empty());
    assert!(state.view().archive_enabled);
    assert_eq!(
        state.view().checkpoint_status_message.as_deref(),
        Some("Archive export failed: disk full")
    );

    let (state, effects) = update(state, Msg::ArchiveClicked);
    assert_eq!(archive_request_id(&effects), request_id + 1);
    assert_eq!(state.view().checkpoint_status_message, None);
}

#[test]
fn archive_actions_preserve_pending_checkpoint_save_status() {
    init_logging();
    let (state, _) = update(
        AppState::new(),
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".into())),
    );
    assert_eq!(
        state.briefing_checkpoint_status_message(),
        Some("Checkpoint saving...")
    );
    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    assert_eq!(
        state.briefing_checkpoint_status_message(),
        Some("Checkpoint saving...")
    );
    let request_id = archive_request_id(&open_effects);
    let (state, _) = update(state, archive_submit_message(request_id));
    assert_eq!(
        state.briefing_checkpoint_status_message(),
        Some("Checkpoint saving...")
    );
}

#[test]
fn archive_succeeds_after_terminal_with_failed_summary_and_unfinished_work() {
    init_logging();
    let article = loaded_pre_triage_articles(&["https://archive-gate.invalid/article"]).remove(0);
    let mut state = with_signal_candidate_metadata(ready_pre_triage_state(&[&article.url]));
    state.set_llm_max_in_flight(1);
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let (state, effects) =
        crate::fixture_support::complete_processing_start(state, effects, 100_000);
    let triage_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: harvester_engine::llm::prompt::PromptId::ArticleTriage,
                ..
            } => Some(*request_id),
            _ => None,
        })
        .expect("triage starts");
    let (state, effects) = update(state, triage_success(triage_id));
    let summary_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: harvester_engine::llm::prompt::PromptId::ArticleSummary,
                ..
            } => Some(*request_id),
            _ => None,
        })
        .expect("summary starts");
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: summary_id,
            result: LlmResultKind::Failed {
                reason: "summary fixture failure".into(),
            },
            metadata: None,
        },
    );
    let (state, _) = update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: vec![article.url.clone()],
            triggered_by_job_done: false,
        },
    );
    let (state, refresh_id) = tick_until_dispatch(state);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: refresh_id,
            delta: harvester_engine::TriageArticleDelta::full_window(vec![article], 100_000),
        },
    );
    assert!(state.run_progress().unwrap().terminal);
    assert_eq!(state.briefing().failed_summary_count(), 1);
    let unfinished = state.unfinished_work();
    assert!(
        matches!(unfinished, crate::UnfinishedWork::Known(work) if work.articles_with_work > 0),
        "failed summary should leave nonzero unfinished work, got {unfinished:?}"
    );
    assert!(state.view().archive_enabled);

    let (state, effects) = update(state, Msg::ArchiveClicked);
    let request_id = archive_request_id(&effects);
    let (state, effects) = update(state, archive_submit_message(request_id));
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::ArchiveRequested { .. })));
    assert!(state.export_available());
}

#[test]
fn summary_failed_for_url_returns_true_for_failed_summary() {
    use crate::briefing::{BriefingSession, LoadedArticle};

    init_logging();
    let url = "https://triage-complete.com/0";
    let mut briefing = BriefingSession::new_loading();
    briefing.set_articles(vec![LoadedArticle {
        url: url.to_string(),
        source_title: None,
        prepared_text: "text".to_string(),
        content_hash: "hash-tc-0".to_string(),
        fetched_utc: None,
    }]);
    briefing.transition_to_summarizing();
    briefing.start_article(0, 1);
    briefing.fail_article(0, "network".to_string());

    assert!(briefing.summary_failed_for_url(url));
    assert!(!briefing.summary_failed_for_url("https://triage-complete.com/1"));
}

#[test]
fn summaries_can_start_true_when_triage_complete_and_briefing_idle() {
    init_logging();
    let state = complete_triage_state_for_test(2);
    assert!(state.summaries_can_start());
}

#[test]
fn summaries_can_start_false_when_triage_not_complete() {
    init_logging();
    let state = AppState::new();
    assert!(!state.summaries_can_start());
}

#[test]
fn summaries_can_start_false_when_briefing_active() {
    use crate::briefing::BriefingSession;

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state.set_briefing(BriefingSession::new_loading());
    assert!(!state.briefing().can_start());
    assert!(!state.summaries_can_start());
}
