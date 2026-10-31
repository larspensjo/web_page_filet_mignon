use super::*;

#[test]
fn archive_dialog_ready_ignores_stale_request() {
    init_logging();
    let state = AppState::new();
    let (state, effects) = update(
        state,
        Msg::ArchiveDialogReady {
            request_id: 99,
            article_count: 0,
            since_utc: None,
            default_basename: "archive.md".to_string(),
            default_file_exists: false,
            export_dir: std::path::PathBuf::from("/tmp"),
            pending_pre_triage_count: 0,
            token_estimates: crate::ArchiveTokenEstimates::default(),
            signal_candidate_default:
                crate::signal_candidate::SignalCandidateDialogDefault::OffDisabled,
            signal_candidate_count: 0,
            signal_candidate_scoring_done: 0,
            signal_candidate_scoring_total: 0,
            signal_candidate_token_estimates: crate::ArchiveTokenEstimates::default(),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.archive_request_id(), 0);
}

#[test]
fn archive_dialog_ready_emits_show_dialog_for_current_request() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (state, effects) = update(
        state,
        Msg::ArchiveDialogReady {
            request_id,
            article_count: 1,
            since_utc: None,
            default_basename: "archive.md".to_string(),
            default_file_exists: false,
            export_dir: std::path::PathBuf::from("/tmp"),
            pending_pre_triage_count: 0,
            token_estimates: crate::ArchiveTokenEstimates::default(),
            signal_candidate_default:
                crate::signal_candidate::SignalCandidateDialogDefault::OffDisabled,
            signal_candidate_count: 0,
            signal_candidate_scoring_done: 0,
            signal_candidate_scoring_total: 0,
            signal_candidate_token_estimates: crate::ArchiveTokenEstimates::default(),
        },
    );
    assert_eq!(state.archive_request_id(), 1);
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ShowArchiveDialog { .. }))
        .expect("ShowArchiveDialog effect expected");
    match effect {
        Effect::ShowArchiveDialog {
            request_id,
            article_count,
            since_utc,
            default_basename,
            default_file_exists,
            export_dir,
            ..
        } => {
            assert_eq!(request_id, 1);
            assert_eq!(article_count, 1);
            assert!(since_utc.is_none());
            assert_eq!(default_basename, "archive.md");
            assert!(!default_file_exists);
            assert_eq!(export_dir, std::path::PathBuf::from("/tmp"));
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_dialog_submitted_validates_basename_and_checkpoint_flag() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "custom-archive.md".to_string(),
            set_checkpoint: true,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested {
            request_id,
            basename,
            ordered_urls,
            since_utc,
            requested_checkpoint,
            use_summaries,
            summaries,
            annotations: _,
            priority_snapshot: _,
            folds: _,
        } => {
            assert_eq!(request_id, 1);
            assert_eq!(basename, "custom-archive.md");
            assert_eq!(ordered_urls.len(), 0);
            assert!(since_utc.is_none());
            assert_eq!(requested_checkpoint, Some(since));
            assert!(!use_summaries);
            assert!(summaries.is_empty());
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_dialog_submitted_emits_triage_and_signal_annotations() {
    use harvester_engine::archive_url_key;

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    complete_signal_candidate(&mut state, 1, 0, "zero-event");
    crate::fixture_support::save_session_results(&mut state);
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: false,
        },
    );

    let (ordered_urls, annotations) = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ArchiveRequested {
                ordered_urls,
                annotations,
                ..
            } => Some((ordered_urls, annotations)),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(ordered_urls.len(), 2);
    let triaged = &annotations[&archive_url_key("https://triage-complete.com/0")];
    assert_eq!(triaged.priority, Some(3));
    assert_eq!(triaged.tags, Some(vec![]));
    assert_eq!(triaged.triage_model.as_deref(), Some("fixture-triage"));
    assert!(triaged.signal_key.is_none());
    let scored = &annotations[&archive_url_key("https://triage-complete.com/1")];
    assert_eq!(scored.priority, Some(3));
    assert_eq!(scored.signal_key.as_deref(), Some("zero-event"));
    assert_eq!(scored.signal_score, Some(0));
}

#[test]
fn archive_submit_priority_snapshot_includes_unselected_completed_result() {
    use harvester_engine::archive_url_key;

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    complete_signal_candidate(&mut state, 0, 80, "selected-event");
    crate::fixture_support::save_session_results(&mut state);
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: true,
        },
    );

    let (ordered_urls, priority_snapshot) = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ArchiveRequested {
                ordered_urls,
                priority_snapshot,
                ..
            } => Some((ordered_urls, priority_snapshot)),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(ordered_urls, vec!["https://triage-complete.com/0"]);
    assert_eq!(
        priority_snapshot.get(&archive_url_key("https://triage-complete.com/1")),
        Some(&3),
        "snapshot must include completed triage results outside the exported selection"
    );
    assert_eq!(
        priority_snapshot.get(&archive_url_key("https://triage-complete.com/0")),
        Some(&3),
        "snapshot must also include the exported selection's own triage results"
    );
    assert_eq!(
        priority_snapshot.len(),
        2,
        "snapshot must cover every completed triage result the session holds"
    );
}

#[test]
fn archive_submit_priority_snapshot_includes_manually_excluded_candidate() {
    use harvester_engine::archive_url_key;

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    complete_signal_candidate(&mut state, 0, 80, "selected-event");
    complete_signal_candidate(&mut state, 1, 90, "excluded-event");
    crate::fixture_support::save_session_results(&mut state);
    let (state, _) = update(
        state,
        Msg::ToggleSignalCandidateExclusion {
            signal_key: "excluded-event".to_string(),
        },
    );
    assert_eq!(state.signal_exclusions().excluded().len(), 1);

    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: true,
        },
    );

    let (ordered_urls, priority_snapshot) = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ArchiveRequested {
                ordered_urls,
                priority_snapshot,
                ..
            } => Some((ordered_urls, priority_snapshot)),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(ordered_urls, vec!["https://triage-complete.com/0"]);
    assert!(!ordered_urls.contains(&"https://triage-complete.com/1".to_string()));
    assert_eq!(
        priority_snapshot.get(&archive_url_key("https://triage-complete.com/1")),
        Some(&3),
        "manually excluded completed triage results must remain in the snapshot"
    );
}

#[test]
fn archive_annotations_are_read_at_submit_after_scoring_without_changing_pinned_selection() {
    use harvester_engine::archive_url_key;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let mut state = with_signal_candidate_metadata(complete_triage_state_for_test(2));
    seed_summaries_for_triage_hashes(&mut state, 2);
    let (mut state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let url = "https://triage-complete.com/0";
    state
        .signal_candidate_mut()
        .enqueue(url.to_string(), "fixture-input".to_string());
    state.signal_candidate_mut().mark_scoring(url, 77);
    state.record_pending_llm_request(77, PromptId::ArticleSignalCandidate);
    let snapshot = crate::update::signal_candidate::build_input_snapshot(&state, url)
        .expect("current upstream fixture");
    state.set_signal_candidate_input_snapshot(url, snapshot);

    crate::fixture_support::save_session_results(&mut state);
    let (state, _) = update(
        state,
        Msg::LlmCompleted {
            request_id: 77,
            result: LlmResultKind::Success {
                output_json: r#"{
                    "signal_score": 84,
                    "signal_key": "late-event",
                    "themes": ["late-theme"],
                    "draft_gist": "Example outlet reports a concrete AI infrastructure event.",
                    "source_tier": "Tier1",
                    "confidence": "High",
                    "reasoning": "Concrete event."
                }"#
                .to_string(),
                input_tokens: 10,
                output_tokens: 5,
                prompt_version: 1,
                resolved_model: "signal-model".to_string(),
            },
            metadata: None,
        },
    );
    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: false,
        },
    );

    let (ordered_urls, annotations) = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ArchiveRequested {
                ordered_urls,
                annotations,
                ..
            } => Some((ordered_urls, annotations)),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        ordered_urls,
        vec![
            "https://triage-complete.com/0".to_string(),
            "https://triage-complete.com/1".to_string()
        ]
    );
    let annotation = &annotations[&archive_url_key(url)];
    assert_eq!(annotation.signal_key.as_deref(), Some("late-event"));
    assert_eq!(annotation.signal_score, Some(84));
    assert_eq!(annotation.themes, Some(vec!["late-theme".to_string()]));
}

#[test]
fn compatible_triage_cache_hit_exports_the_stored_model_id() {
    use harvester_engine::archive_url_key;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let mut state = prime_llm_metadata(AppState::new());
    state.store_triage_result(
        "hash-0",
        crate::triage::ArticleTriageResult {
            category: "news".to_string(),
            priority: 4,
            tags: vec!["cached".to_string()],
            rationale: "cached".to_string(),
            input_tokens: 1,
            output_tokens: 1,
        },
    );
    let mut active_versions = std::collections::HashMap::new();
    active_versions.insert(PromptId::ArticleTriage, 1);
    let mut effective_models = std::collections::HashMap::new();
    effective_models.insert(PromptId::ArticleTriage, "test-model-2026-03-31".to_string());
    let (state, _) = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions,
            effective_models,
        },
    );
    let mut state = state;
    let load_request_id = state.alloc_triage_request_id();
    state.set_triage_in_flight(load_request_id);
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_request_id,
            delta: harvester_engine::TriageArticleDelta::full_window(
                loaded_triage_articles(1),
                100_000,
            ),
        },
    );
    let (state, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: crate::PipelineRunScope::Resume,
        },
    );
    let (mut state, effects) =
        crate::fixture_support::complete_processing_start(state, effects, 100_000);
    if let Some(summary_id) = effects.iter().find_map(|effect| match effect {
        Effect::RequestLlmCompletion {
            request_id,
            prompt_id: harvester_engine::llm::prompt::PromptId::ArticleSummary,
            ..
        } => Some(*request_id),
        _ => None,
    }) {
        let (next, _) = update(
            state,
            Msg::LlmCompleted {
                request_id: summary_id,
                result: LlmResultKind::Failed {
                    reason: "summary fixture failure".into(),
                },
                metadata: None,
            },
        );
        state = next;
    }
    assert!(state.run_progress().unwrap().terminal);
    assert_eq!(
        state.triage().triage_model_for_url("https://example.com/0"),
        Some("test-model")
    );

    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let annotations = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ArchiveRequested { annotations, .. } => Some(annotations),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        annotations[&archive_url_key("https://example.com/0")]
            .triage_model
            .as_deref(),
        Some("test-model")
    );
}

#[test]
fn archive_dialog_submitted_with_only_pre_triage_ready_exports_zero_urls() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let state = ready_pre_triage_state(&["https://example.com/1"]);
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested {
            ordered_urls,
            since_utc,
            requested_checkpoint,
            ..
        } => {
            assert_eq!(ordered_urls, Vec::<String>::new());
            assert!(since_utc.is_none());
            assert_eq!(requested_checkpoint, None);
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_submitted_with_use_summaries_true_and_cached_summary_populates_map() {
    use crate::briefing::ArticleSummaryResult;
    use crate::summary_cache::SummaryCacheKey;
    use harvester_engine::archive_url_key;
    use harvester_engine::llm::dto::SummaryEntities;
    use harvester_engine::llm::prompt::PromptId;

    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let url = "https://triage-complete.com/0".to_string();
    let mut state = complete_triage_state_for_test(1);
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
            summary: "compact".to_string(),
            key_points: vec!["point one".to_string()],
            input_tokens: 10,
            output_tokens: 5,
            entities: SummaryEntities::default(),
        },
        "2026-04-01T00:00:00Z".to_string(),
    );

    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: true,
            use_signal_candidates: false,
        },
    );
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested {
            use_summaries,
            summaries,
            ..
        } => {
            assert!(use_summaries);
            let body = summaries
                .get(&archive_url_key(&url))
                .expect("summary body for url");
            assert!(body.contains("## Summary\ncompact"));
            assert!(body.contains("## Key Points\n- point one"));
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_submitted_with_use_summaries_false_emits_empty_summary_map() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-21T18:17:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let state = complete_triage_state_for_test(1);
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let effect = effects
        .into_iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested {
            use_summaries,
            summaries,
            ..
        } => {
            assert!(!use_summaries);
            assert!(summaries.is_empty());
        }
        _ => unreachable!(),
    }
}

#[test]
fn archive_dialog_submitted_rejects_invalid_basename() {
    init_logging();
    let state = add_completed_job_for_test(AppState::new(), "https://example.com/1");
    let (state, _) = update(state, Msg::ArchiveClicked);
    let (_, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id: 1,
            basename: "../bad.md".to_string(),
            set_checkpoint: true,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    assert!(effects.is_empty());
}

#[test]
fn archive_open_and_submit_use_identical_pinned_corpus() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let state = complete_triage_state_for_test(2);

    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let open_effect = open_effects
        .into_iter()
        .find(|e| matches!(e, Effect::OpenArchiveDialog { .. }))
        .expect("OpenArchiveDialog effect expected");
    let open_count = match open_effect {
        Effect::OpenArchiveDialog { article_count, .. } => article_count,
        _ => unreachable!(),
    };
    assert_eq!(
        open_count, 2,
        "open dialog should report 2 pinned triage articles"
    );

    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let submit_effect = submit_effects
        .into_iter()
        .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match submit_effect {
        Effect::ArchiveRequested { ordered_urls, .. } => {
            assert_eq!(
                ordered_urls,
                vec![
                    "https://triage-complete.com/0".to_string(),
                    "https://triage-complete.com/1".to_string()
                ],
                "submitted export must use the pinned triage corpus"
            );
        }
        _ => unreachable!(),
    }
}

#[test]
fn refresh_between_open_and_submit_uses_pinned_snapshot() {
    init_logging();
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let triage_url = "https://triage-complete.com/0";
    let state = complete_triage_state_for_test(1);

    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let open_count = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { article_count, .. } => Some(article_count),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    assert_eq!(open_count, 1, "open dialog must see 1 triage article");

    let pre_triage_url = "https://pretriage.com/1";
    let state = add_completed_job_for_test(state, pre_triage_url);
    let (state, request_id2) = tick_until_dispatch(state);
    let retained = state
        .triage()
        .articles()
        .iter()
        .map(|a| LoadedArticle {
            url: a.url.clone(),
            source_title: a.source_title.clone(),
            prepared_text: a.prepared_text.clone(),
            content_hash: a.content_hash.clone(),
            fetched_utc: a.fetched_utc.clone(),
        })
        .collect::<Vec<_>>();
    let (state, _) = update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: request_id2,
            delta: harvester_engine::TriageArticleDelta::full_window(
                retained
                    .into_iter()
                    .chain(loaded_pre_triage_articles(&[pre_triage_url]))
                    .collect(),
                100_000,
            ),
        },
    );
    assert_eq!(
        state.archive_corpus().count(),
        1,
        "archive corpus must still see 1 triage article after pre-triage refresh"
    );

    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let submit_effect = submit_effects
        .into_iter()
        .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match submit_effect {
        Effect::ArchiveRequested { ordered_urls, .. } => {
            assert_eq!(
                ordered_urls,
                vec![triage_url.to_string()],
                "submit must use pinned triage corpus from open time, not pre-triage"
            );
        }
        _ => unreachable!(),
    }
}

#[test]
fn parity_a_pre_triage_ready_archive_count_is_zero_pending_count_is_nonzero() {
    init_logging();
    let urls = &["https://parity-a.com/1", "https://parity-a.com/2"];
    let state = ready_pre_triage_state(urls);
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let (archive_count, pending_count) = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog {
                article_count,
                pending_pre_triage_count,
                ..
            } => Some((article_count, pending_pre_triage_count)),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    assert_eq!(
        archive_count, 0,
        "archive must be empty when only pre-triage is ready"
    );
    assert_eq!(
        pending_count,
        urls.len(),
        "pending count must equal the number of pre-triage ready articles"
    );

    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let submit_urls = submit_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        submit_urls,
        Vec::<String>::new(),
        "submit must export zero URLs when only pre-triage is ready"
    );
}

#[test]
fn parity_b_triage_complete_corpus_count_dialog_count_urls_match() {
    init_logging();
    let state = complete_triage_state_for_test(2);
    let since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let expected_count = state.archive_corpus().count();
    let expected_urls: Vec<String> = state.archive_corpus().ordered_urls().to_vec();
    assert!(
        expected_count > 0,
        "corpus must be non-empty for a meaningful parity test"
    );

    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    let dialog_count = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { article_count, .. } => Some(article_count),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    assert_eq!(
        dialog_count, expected_count,
        "OpenArchiveDialog article_count must equal corpus.count()"
    );

    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let submit_urls = submit_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        submit_urls.len(),
        expected_count,
        "ArchiveRequested url count must equal corpus.count()"
    );
    assert_eq!(
        submit_urls, expected_urls,
        "ArchiveRequested URLs must match corpus.ordered_urls()"
    );
}

#[test]
fn checkpoint_set_does_not_reduce_corpus_count_to_zero() {
    init_logging();
    let mut state = complete_triage_state_for_test(2);

    let checkpoint = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    state.set_briefing_since_utc(Some(checkpoint));
    assert_eq!(
        state.briefing_since_utc(),
        Some(checkpoint),
        "checkpoint must be set before the test"
    );

    assert!(
        state.archive_corpus().count() > 0,
        "corpus count must be non-zero even when briefing checkpoint is set"
    );
    let expected_count = state.archive_corpus().count();
    let expected_urls: Vec<String> = state.archive_corpus().ordered_urls().to_vec();

    let (state, open_effects) = update(state, Msg::ArchiveClicked);
    let dialog_since_utc = open_effects
        .iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { since_utc, .. } => Some(*since_utc),
            _ => None,
        })
        .expect("OpenArchiveDialog effect expected");
    let dialog_count = open_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::OpenArchiveDialog { article_count, .. } => Some(article_count),
            _ => None,
        })
        .expect("OpenArchiveDialog effect (article_count) expected");

    assert_eq!(
        dialog_since_utc,
        Some(checkpoint),
        "OpenArchiveDialog must propagate the briefing checkpoint as since_utc"
    );
    assert_eq!(
        dialog_count, expected_count,
        "OpenArchiveDialog article_count must equal corpus.count() — checkpoint must not reduce it"
    );
    assert!(
        dialog_count > 0,
        "article_count must be non-zero when checkpoint is set and triage has completed articles"
    );

    let request_id = state.archive_request_id();
    let submit_since = chrono::DateTime::parse_from_rfc3339("2026-03-22T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let (_state, submit_effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: false,
            submitted_at: submit_since,
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    let submit_urls = submit_effects
        .into_iter()
        .find_map(|e| match e {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert_eq!(
        submit_urls.len(),
        expected_count,
        "ArchiveRequested url count must equal corpus.count() even with checkpoint set"
    );
    assert_eq!(
        submit_urls, expected_urls,
        "submit URLs must match corpus URLs even with checkpoint set"
    );
}
