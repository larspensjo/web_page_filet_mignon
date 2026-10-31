use super::*;

#[test]
fn archive_export_completed_only_saves_checkpoint_for_current_request() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, effects) = update(
        state,
        Msg::ArchiveExportCompleted {
            request_id: 1,
            path: std::path::PathBuf::from("archive.md"),
            doc_count: 1,
            requested_checkpoint: Some(checkpoint),
        },
    );
    assert_eq!(
        effects,
        vec![Effect::SaveBriefingCheckpoint {
            save_id: 1,
            since_utc: Some(checkpoint)
        }]
    );
    assert_eq!(state.briefing_since_utc(), Some(checkpoint));
    assert_eq!(
        state.briefing_checkpoint_status_message(),
        Some("Checkpoint saving...")
    );
}

#[test]
fn successful_archive_checkpoint_switches_from_24h_to_empty_since_archive_and_persists_mode() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-28T13:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/archived");
    let state = update(state, Msg::tick_at(now)).0;
    let state = update(
        state,
        Msg::JobListModeSet {
            mode: crate::JobListMode::Last24Hours,
        },
    )
    .0;
    assert_eq!(state.view().desktop_job_list.rows.len(), 1);
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (state, effects) = update(
        state,
        Msg::ArchiveExportCompleted {
            request_id,
            path: "archive.md".into(),
            doc_count: 1,
            requested_checkpoint: Some(now),
        },
    );
    assert_eq!(state.job_list_mode(), crate::JobListMode::Last24Hours);
    let save_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SaveBriefingCheckpoint { save_id, .. } => Some(*save_id),
            _ => None,
        })
        .unwrap();
    // An unrelated acknowledgement must not switch views.
    let state = update(
        state,
        Msg::BriefingCheckpointSaveSucceeded {
            save_id: save_id + 1,
        },
    )
    .0;
    assert_eq!(state.job_list_mode(), crate::JobListMode::Last24Hours);
    let (state, effects) = update(state, Msg::BriefingCheckpointSaveSucceeded { save_id });
    assert_eq!(state.job_list_mode(), crate::JobListMode::SinceCheckpoint);
    assert!(state.view().desktop_job_list.rows.is_empty());
    assert!(effects.iter().any(|effect| matches!(effect,
        Effect::PersistRuntimeState { snapshot }
            if snapshot.job_list_mode == Some(crate::JobListMode::SinceCheckpoint)
    )));
}

#[test]
fn unsuccessful_or_checkpoint_free_archive_preserves_the_24h_view() {
    for scenario in [
        "export-failed",
        "checkpoint-failed",
        "no-checkpoint",
        "superseded",
    ] {
        let state = update(
            AppState::new(),
            Msg::JobListModeSet {
                mode: crate::JobListMode::Last24Hours,
            },
        )
        .0;
        let (state, _) = update(state, Msg::ArchiveClicked);
        let request_id = state.archive_request_id();
        let state = if scenario == "export-failed" {
            update(
                state,
                Msg::ArchiveExportFailed {
                    request_id,
                    basename: "archive.md".into(),
                    reason: "disk full".into(),
                },
            )
            .0
        } else {
            let (state, _) = update(
                state,
                Msg::ArchiveExportCompleted {
                    request_id,
                    path: "archive.md".into(),
                    doc_count: 1,
                    requested_checkpoint: (scenario != "no-checkpoint").then(chrono::Utc::now),
                },
            );
            match scenario {
                "checkpoint-failed" => {
                    update(
                        state,
                        Msg::BriefingCheckpointSaveFailed {
                            save_id: 1,
                            reason: "disk full".into(),
                        },
                    )
                    .0
                }
                "superseded" => {
                    let state = update(
                        state,
                        Msg::BriefingCheckpointSet(Some("2026-09-28T13:00:00Z".into())),
                    )
                    .0;
                    let state =
                        update(state, Msg::BriefingCheckpointSaveSucceeded { save_id: 1 }).0;
                    update(state, Msg::BriefingCheckpointSaveSucceeded { save_id: 2 }).0
                }
                _ => state,
            }
        };
        assert_eq!(
            state.job_list_mode(),
            crate::JobListMode::Last24Hours,
            "{scenario}"
        );
    }
}

#[test]
fn briefing_checkpoint_save_success_clears_pending_status() {
    init_logging();
    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, effects) = update(
        AppState::new(),
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert_eq!(
        effects,
        vec![Effect::SaveBriefingCheckpoint {
            save_id: 1,
            since_utc: Some(checkpoint)
        }]
    );
    assert_eq!(
        state.pending_briefing_checkpoint_save(),
        Some(crate::state::PendingBriefingCheckpointSaveSnapshot {
            save_id: 1,
            previous_since_utc: None,
            pending_since_utc: Some(checkpoint),
        })
    );

    let (state, follow_up) = update(state, Msg::BriefingCheckpointSaveSucceeded { save_id: 1 });
    assert!(follow_up.is_empty());
    assert_eq!(state.briefing_since_utc(), Some(checkpoint));
    assert_eq!(state.pending_briefing_checkpoint_save(), None);
    assert_eq!(state.briefing_checkpoint_status_message(), None);
}

#[test]
fn briefing_checkpoint_save_failure_reverts_in_memory_state() {
    init_logging();
    let initial = chrono::DateTime::parse_from_rfc3339("2026-03-20T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut state = AppState::new();
    state.set_briefing_since_utc(Some(initial));
    let (state, effects) = update(
        state,
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SaveBriefingCheckpoint { save_id: 1, .. }]
    ));
    let (state, follow_up) = update(
        state,
        Msg::BriefingCheckpointSaveFailed {
            save_id: 1,
            reason: "disk full".to_string(),
        },
    );
    assert!(follow_up.is_empty());
    assert_eq!(state.briefing_since_utc(), Some(initial));
    assert_eq!(state.pending_briefing_checkpoint_save(), None);
    assert_eq!(
        state.briefing_checkpoint_status_message(),
        Some("Checkpoint save failed: disk full")
    );
}

#[test]
fn briefing_checkpoint_loaded_clears_pending_save_tracking() {
    init_logging();
    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, _) = update(
        AppState::new(),
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert!(state.pending_briefing_checkpoint_save().is_some());

    let (state, follow_up) = update(
        state,
        Msg::BriefingCheckpointLoaded {
            since_utc: Some("2026-03-25T00:00:00Z".to_string()),
        },
    );
    assert!(follow_up.is_empty());
    assert_eq!(
        state.briefing_since_utc(),
        Some(
            chrono::DateTime::parse_from_rfc3339("2026-03-25T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc)
        )
    );
    assert_eq!(state.pending_briefing_checkpoint_save(), None);
    assert_eq!(state.briefing_checkpoint_status_message(), None);
    assert_ne!(state.briefing_since_utc(), Some(checkpoint));
}

#[test]
fn stale_checkpoint_save_failure_is_ignored_when_newer_save_is_pending() {
    init_logging();
    let first_checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-20T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let latest_checkpoint = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (state, _) = update(
        AppState::new(),
        Msg::BriefingCheckpointSet(Some("2026-03-20T00:00:00Z".to_string())),
    );
    let (state, effects) = update(
        state,
        Msg::BriefingCheckpointSet(Some("2026-03-22T00:00:00Z".to_string())),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::SaveBriefingCheckpoint { save_id: 2, .. }]
    ));

    let (state, follow_up) = update(
        state,
        Msg::BriefingCheckpointSaveFailed {
            save_id: 1,
            reason: "stale failure".to_string(),
        },
    );
    assert!(follow_up.is_empty());
    assert_eq!(state.briefing_since_utc(), Some(latest_checkpoint));
    assert_eq!(
        state.pending_briefing_checkpoint_save(),
        Some(crate::state::PendingBriefingCheckpointSaveSnapshot {
            save_id: 2,
            previous_since_utc: Some(first_checkpoint),
            pending_since_utc: Some(latest_checkpoint),
        })
    );
}
