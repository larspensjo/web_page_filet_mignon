use super::*;

fn triage_state_with_content_twins(shared_hash: &str) -> AppState {
    let mut state = complete_triage_state_for_test(3);
    let mut articles: Vec<_> = state
        .triage()
        .articles()
        .iter()
        .map(|article| LoadedArticle {
            url: article.url.clone(),
            source_title: article.source_title.clone(),
            prepared_text: "triage-content ".repeat(220),
            content_hash: article.content_hash.clone(),
            fetched_utc: article.fetched_utc.clone(),
        })
        .collect();
    articles[0].content_hash = shared_hash.into();
    articles[1].content_hash = shared_hash.into();
    let results: Vec<_> = articles
        .iter()
        .map(|article| state.triage().result_for_url(&article.url).unwrap().clone())
        .collect();
    state.set_pre_triage(crate::PreTriageSession::load_articles(
        articles.clone(),
        &crate::PreTriagePolicy::default(),
    ));
    let mut session = crate::triage::TriageSession::new_loading(None);
    session.set_articles(articles);
    session.transition_to_triaging();
    for (i, result) in results.into_iter().enumerate() {
        session.complete_article(i, result);
    }
    session.complete();
    state.set_triage(session);
    crate::fixture_support::save_session_results(&mut state);
    state
}

#[test]
fn signal_export_keeps_pinned_membership_and_submit_time_member_annotations_and_summaries() {
    use harvester_engine::{archive_url_key, ArchiveFoldReason};
    init_logging();
    let mut state =
        with_signal_candidate_metadata(with_summary_metadata(complete_triage_state_for_test(3)));
    seed_summaries_for_triage_hashes(&mut state, 3);
    let own_keys = [
        "anthropic-model-access-suspension-foreign-national-order",
        "anthropic-disable-advanced-models-foreign-access",
        "solo-event",
    ];
    for (i, key) in own_keys.iter().enumerate() {
        complete_signal_candidate(&mut state, i, 90 - i as u8, key);
    }
    crate::fixture_support::save_session_results(&mut state);
    let state = crate::fixture_support::complete_archive_meter_startup(state);
    let selected_count = state.view().archive_meter.selected_count;
    let final_selection = state.archive_final_selection();
    let (mut state, effects) = update(state, Msg::ArchiveClicked);
    let dialog_count = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::OpenArchiveDialog {
                signal_candidate_count,
                ..
            } => Some(*signal_candidate_count),
            _ => None,
        })
        .unwrap();
    let pin = state.pinned_signal_candidate_selection().unwrap().clone();
    assert_eq!(pin.folds, final_selection.folds);
    assert_eq!(pin.selected_urls, final_selection.ordered_urls);
    assert_eq!(selected_count, 2);
    assert_eq!(selected_count, dialog_count);
    let rep = "https://triage-complete.com/0";
    let member = "https://triage-complete.com/1";
    assert_eq!(pin.folds[&archive_url_key(rep)][0].url, member);
    assert_eq!(
        pin.folds[&archive_url_key(rep)][0].signal_key.as_deref(),
        Some(own_keys[1])
    );
    assert_eq!(
        pin.folds[&archive_url_key(rep)][0].reason,
        ArchiveFoldReason::SameEvent
    );
    // A new exclusion changes the live membership; the dialog pin stays fixed.
    state.signal_exclusions_mut().add_exclusion(OverrideKey {
        signal_key: own_keys[0].into(),
        prompt_id: "ArticleSignalCandidate".into(),
        prompt_version: 1,
    });
    assert!(state.signal_candidate_selection().folds.is_empty());
    assert_eq!(state.signal_candidate_selection().selected_urls.len(), 1);
    let request_id = state.archive_request_id();
    let (state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".into(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: true,
            use_signal_candidates: true,
        },
    );
    let effect = effects
        .iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .unwrap();
    if let Effect::ArchiveRequested {
        ordered_urls,
        folds,
        annotations,
        summaries,
        ..
    } = effect
    {
        assert_eq!(*ordered_urls, pin.selected_urls);
        assert_eq!(*folds, pin.folds);
        assert_eq!(ordered_urls.len(), selected_count);
        for (i, key) in own_keys.iter().enumerate() {
            let url = archive_url_key(&format!("https://triage-complete.com/{i}"));
            assert_eq!(annotations[&url].signal_key.as_deref(), Some(*key));
            assert!(annotations[&url].priority.is_some());
            assert!(summaries[&url].contains("## Summary"));
        }
    }
    assert!(state.pinned_full_archive_selection().is_none());
    assert!(state.pinned_signal_candidate_selection().is_none());
}

#[test]
fn merged_signal_export_keeps_pin_and_promotes_live_representative_on_exclusion() {
    use harvester_engine::{archive_url_key, ArchiveFoldReason};
    init_logging();
    let mut state =
        with_signal_candidate_metadata(with_summary_metadata(complete_triage_state_for_test(3)));
    seed_summaries_for_triage_hashes(&mut state, 3);
    let own_keys = [
        "vistra-4b-loan-nuclear-plant-upgrades",
        "doe-loan-vistra-nuclear-fleet-uprates",
        "solo-event",
    ];
    for (i, key) in own_keys.iter().enumerate() {
        complete_signal_candidate(&mut state, i, 90 - i as u8, key);
    }
    crate::fixture_support::save_session_results(&mut state);
    let state = crate::fixture_support::complete_archive_meter_startup(state);
    let selected_count = state.view().archive_meter.selected_count;
    let final_selection = state.archive_final_selection();
    let (mut state, effects) = update(state, Msg::ArchiveClicked);
    let dialog_count = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::OpenArchiveDialog {
                signal_candidate_count,
                ..
            } => Some(*signal_candidate_count),
            _ => None,
        })
        .unwrap();
    let pin = state.pinned_signal_candidate_selection().unwrap().clone();
    assert_eq!(pin.folds, final_selection.folds);
    assert_eq!(pin.selected_urls, final_selection.ordered_urls);
    assert_eq!(selected_count, 2);
    assert_eq!(selected_count, dialog_count);
    let rep = "https://triage-complete.com/0";
    let member = "https://triage-complete.com/1";
    assert_eq!(pin.folds[&archive_url_key(rep)][0].url, member);
    assert_eq!(
        pin.folds[&archive_url_key(rep)][0].signal_key.as_deref(),
        Some(own_keys[1])
    );
    assert_eq!(
        pin.folds[&archive_url_key(rep)][0].reason,
        ArchiveFoldReason::SameEvent
    );
    // A new exclusion changes the live membership; the dialog pin stays fixed.
    state.signal_exclusions_mut().add_exclusion(OverrideKey {
        signal_key: own_keys[0].into(),
        prompt_id: "ArticleSignalCandidate".into(),
        prompt_version: 1,
    });
    assert!(state.signal_candidate_selection().folds.is_empty());
    assert_eq!(state.signal_candidate_selection().selected_urls.len(), 2);
    assert!(state
        .signal_candidate_selection()
        .selected_urls
        .contains(&member.to_string()));
    let request_id = state.archive_request_id();
    let (state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".into(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: true,
            use_signal_candidates: true,
        },
    );
    let effect = effects
        .iter()
        .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
        .unwrap();
    if let Effect::ArchiveRequested {
        ordered_urls,
        folds,
        annotations,
        summaries,
        ..
    } = effect
    {
        assert_eq!(*ordered_urls, pin.selected_urls);
        assert_eq!(*folds, pin.folds);
        assert_eq!(ordered_urls.len(), selected_count);
        for (i, key) in own_keys.iter().enumerate() {
            let url = archive_url_key(&format!("https://triage-complete.com/{i}"));
            assert_eq!(annotations[&url].signal_key.as_deref(), Some(*key));
            assert!(annotations[&url].priority.is_some());
            assert!(summaries[&url].contains("## Summary"));
        }
    }
    assert!(state.pinned_full_archive_selection().is_none());
    assert!(state.pinned_signal_candidate_selection().is_none());
}

#[test]
fn full_mode_and_missing_signal_snapshot_export_folded_full_pin_with_identical_membership() {
    use harvester_engine::{archive_url_key, ArchiveFoldReason};
    init_logging();
    for use_signal_candidates in [false, true] {
        let mut state =
            with_signal_candidate_metadata(triage_state_with_content_twins("hash-tc-0"));
        let unavailable = state.archive_final_selection();
        assert_eq!(
            unavailable.source,
            crate::ArchiveSelectionSource::FullCorpusSignalUnavailable
        );
        assert_eq!(unavailable.ordered_urls.len(), 2);
        let unscored_member =
            &unavailable.folds[&archive_url_key("https://triage-complete.com/0")][0];
        assert_eq!(unscored_member.reason, ArchiveFoldReason::IdenticalContent);
        assert!(unscored_member.signal_key.is_none());
        for i in 0..3 {
            state = add_completed_job_with_tokens_for_test(
                state,
                &format!("https://triage-complete.com/{i}"),
                500,
            );
        }
        seed_summaries_for_triage_hashes(&mut state, 3);
        complete_signal_candidate(&mut state, 0, 90, "shared-event");
        complete_signal_candidate(&mut state, 1, 95, "different-twin-slug");
        complete_signal_candidate(&mut state, 2, 80, "shared-event");
        crate::fixture_support::save_session_results(&mut state);
        state.set_signal_candidate_threshold(100);
        let no_candidates = state.archive_final_selection();
        assert_eq!(
            no_candidates.source,
            crate::ArchiveSelectionSource::FullCorpusNoCandidates
        );
        assert_eq!(no_candidates.ordered_urls, unavailable.ordered_urls);
        assert_eq!(
            no_candidates.folds[&archive_url_key("https://triage-complete.com/0")][0].reason,
            ArchiveFoldReason::IdenticalContent
        );
        state.set_signal_candidate_threshold(60);
        let unfolded_estimate =
            state.archive_token_estimates(state.archive_corpus().ordered_urls());
        let (mut state, effects) = update(state, Msg::ArchiveClicked);
        let (article_count, estimates) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::OpenArchiveDialog {
                    article_count,
                    token_estimates,
                    ..
                } => Some((*article_count, *token_estimates)),
                _ => None,
            })
            .unwrap();
        let full_pin = state.pinned_full_archive_selection().unwrap().clone();
        assert_eq!(state.pinned_archive_corpus().unwrap().count(), 3);
        assert_eq!(
            full_pin.ordered_urls,
            vec![
                "https://triage-complete.com/0",
                "https://triage-complete.com/2"
            ]
        );
        assert_eq!(article_count, full_pin.ordered_urls.len());
        assert_eq!(article_count, 2);
        assert_eq!(unfolded_estimate.full_tokens, 1500);
        assert_eq!(estimates.full_tokens, unfolded_estimate.full_tokens - 500);
        assert_eq!(unfolded_estimate.summary_tokens, 15);
        assert_eq!(
            estimates.summary_tokens,
            unfolded_estimate.summary_tokens - 5
        );
        assert_eq!(estimates.summary_coverage, 2);
        assert!(!state
            .pinned_signal_candidate_selection()
            .unwrap()
            .folds
            .is_empty());
        let members = &full_pin.folds[&archive_url_key("https://triage-complete.com/0")];
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].url, "https://triage-complete.com/1");
        assert_eq!(
            members[0].signal_key.as_deref(),
            Some("different-twin-slug")
        );
        assert_eq!(members[0].reason, ArchiveFoldReason::IdenticalContent);
        assert_eq!(full_pin.folds.len(), 1);
        if use_signal_candidates {
            state.clear_pinned_signal_candidate_selection();
        }
        // A refreshed corpus must not replace the full-mode pin in either submit path.
        state.set_triage(crate::triage::TriageSession::new_loading(None));
        let request_id = state.archive_request_id();
        let (state, effects) = update(
            state,
            Msg::ArchiveDialogSubmitted {
                request_id,
                basename: "archive.md".into(),
                set_checkpoint: false,
                submitted_at: chrono::Utc::now(),
                use_summaries: true,
                use_signal_candidates,
            },
        );
        let effect = effects
            .iter()
            .find(|effect| matches!(effect, Effect::ArchiveRequested { .. }))
            .unwrap();
        if let Effect::ArchiveRequested {
            ordered_urls,
            folds,
            annotations,
            summaries,
            ..
        } = effect
        {
            assert_eq!(*ordered_urls, full_pin.ordered_urls);
            assert_eq!(*folds, full_pin.folds);
            assert_eq!(article_count, ordered_urls.len());
            assert_eq!(
                folds[&archive_url_key("https://triage-complete.com/0")][0].reason,
                ArchiveFoldReason::IdenticalContent
            );
            assert_eq!(
                annotations[&archive_url_key("https://triage-complete.com/1")]
                    .signal_key
                    .as_deref(),
                Some("different-twin-slug")
            );
            assert!(summaries.contains_key(&archive_url_key("https://triage-complete.com/1")));
        }
        assert!(state.pinned_full_archive_selection().is_none());
    }
}

#[test]
fn full_mode_and_final_selection_keep_empty_clean_text_separate() {
    let empty_hash = harvester_engine::llm::replay::content_hash("");
    // Startup corpus metadata carries the hash, not a stored clean-text length.
    let mut state = with_signal_candidate_metadata(triage_state_with_content_twins(&empty_hash));
    let unavailable = state.archive_final_selection();
    assert_eq!(
        unavailable.source,
        crate::ArchiveSelectionSource::FullCorpusSignalUnavailable
    );
    assert_eq!(unavailable.ordered_urls.len(), 3);
    assert!(unavailable.folds.is_empty());
    complete_signal_candidate(&mut state, 0, 50, "first");
    complete_signal_candidate(&mut state, 1, 40, "second");
    let no_candidates = state.archive_final_selection();
    assert_eq!(
        no_candidates.source,
        crate::ArchiveSelectionSource::FullCorpusNoCandidates
    );
    assert_eq!(no_candidates.ordered_urls, unavailable.ordered_urls);
    assert!(no_candidates.folds.is_empty());
    let (state, effects) = update(state, Msg::ArchiveClicked);
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenArchiveDialog {
            article_count: 3,
            ..
        }
    )));
    let pin = state.pinned_full_archive_selection().unwrap().clone();
    assert_eq!(pin.ordered_urls, unavailable.ordered_urls);
    assert!(pin.folds.is_empty());
    let request_id = state.archive_request_id();
    let (_, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "empty-text.md".into(),
            set_checkpoint: false,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: false,
        },
    );
    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::ArchiveRequested { ordered_urls, folds, .. }
            if *ordered_urls == pin.ordered_urls && folds.is_empty())
    ));
}

#[test]
fn archive_clicked_reports_signal_candidate_snapshot() {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(1);
    state.signal_candidate_mut().enqueue(
        "https://signal.example/a".to_string(),
        "fixture-input".to_string(),
    );
    state
        .signal_candidate_mut()
        .mark_scoring("https://signal.example/a", 7);
    state.signal_candidate_mut().complete(
        "https://signal.example/a",
        SignalCandidateResult {
            signal_score: 80,
            signal_key: "cluster-a".to_string(),
            themes: vec!["theme".to_string()],
            draft_gist: "gist".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "reason".to_string(),
            input_tokens: 10,
            output_tokens: 2,
        },
    );

    crate::fixture_support::save_session_results(&mut state);
    let (state, effects) = update(state, Msg::ArchiveClicked);
    let effect = effects
        .into_iter()
        .find(|e| matches!(e, Effect::OpenArchiveDialog { .. }))
        .expect("OpenArchiveDialog effect expected");
    match effect {
        Effect::OpenArchiveDialog {
            signal_candidate_default,
            signal_candidate_count,
            signal_candidate_scoring_done,
            signal_candidate_scoring_total,
            signal_candidate_token_estimates,
            ..
        } => {
            assert!(matches!(
                signal_candidate_default,
                crate::signal_candidate::SignalCandidateDialogDefault::OnAllSettled
            ));
            assert_eq!(signal_candidate_count, 1);
            assert_eq!(signal_candidate_scoring_done, 1);
            assert_eq!(signal_candidate_scoring_total, 1);
            assert_eq!(
                signal_candidate_token_estimates,
                crate::ArchiveTokenEstimates {
                    full_tokens: 0,
                    summary_tokens: 1,
                    summary_coverage: 1
                }
            );
        }
        _ => unreachable!(),
    }

    let pinned = state
        .pinned_signal_candidate_selection()
        .expect("signal candidate snapshot should be pinned at archive open");
    assert_eq!(
        pinned.selected_urls,
        vec!["https://signal.example/a".to_string()]
    );
}

#[test]
fn archive_dialog_submit_uses_pinned_signal_candidate_snapshot_and_clears_overrides() {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(1);
    state.signal_candidate_mut().enqueue(
        "https://signal.example/a".to_string(),
        "fixture-input".to_string(),
    );
    state
        .signal_candidate_mut()
        .mark_scoring("https://signal.example/a", 7);
    state.signal_candidate_mut().complete(
        "https://signal.example/a",
        SignalCandidateResult {
            signal_score: 80,
            signal_key: "cluster-a".to_string(),
            themes: vec!["theme".to_string()],
            draft_gist: "gist".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "reason".to_string(),
            input_tokens: 10,
            output_tokens: 2,
        },
    );

    crate::fixture_support::save_session_results(&mut state);
    let (state, _) = update(
        state,
        Msg::ToggleSignalCandidateExclusion {
            signal_key: "cluster-x".to_string(),
        },
    );
    let (mut state, _) = update(state, Msg::ArchiveClicked);

    state.signal_candidate_mut().enqueue(
        "https://signal.example/b".to_string(),
        "fixture-input".to_string(),
    );
    state
        .signal_candidate_mut()
        .mark_scoring("https://signal.example/b", 8);
    state.signal_candidate_mut().complete(
        "https://signal.example/b",
        SignalCandidateResult {
            signal_score: 90,
            signal_key: "cluster-b".to_string(),
            themes: vec!["theme".to_string()],
            draft_gist: "gist".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "reason".to_string(),
            input_tokens: 10,
            output_tokens: 2,
        },
    );

    let request_id = state.archive_request_id();
    let (state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: true,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: true,
        },
    );

    let effect = effects
        .iter()
        .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
        .expect("ArchiveRequested effect expected");
    match effect {
        Effect::ArchiveRequested { ordered_urls, .. } => {
            assert_eq!(
                *ordered_urls,
                vec!["https://signal.example/a".to_string()],
                "submit must use the pinned signal-candidate snapshot from archive-open time"
            );
        }
        _ => unreachable!(),
    }

    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::PersistSignalCandidateOverrides { overrides } if overrides.is_empty()
    )));
    assert!(
        state.signal_exclusions().excluded().is_empty(),
        "checkpoint submit must clear signal-candidate overrides"
    );
}

#[test]
fn archive_dialog_submit_with_empty_candidate_snapshot_exports_empty_selection_when_explicitly_enabled(
) {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(1);
    state.signal_candidate_mut().enqueue(
        "https://triage-complete.com/0".to_string(),
        "fixture-input".to_string(),
    );
    state
        .signal_candidate_mut()
        .mark_scoring("https://triage-complete.com/0", 7);
    state.signal_candidate_mut().complete(
        "https://triage-complete.com/0",
        SignalCandidateResult {
            signal_score: 20,
            signal_key: "below-threshold".to_string(),
            themes: vec!["theme".to_string()],
            draft_gist: "gist".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "reason".to_string(),
            input_tokens: 10,
            output_tokens: 2,
        },
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
            use_signal_candidates: true,
        },
    );

    let archived = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::ArchiveRequested { ordered_urls, .. } => Some(ordered_urls),
            _ => None,
        })
        .expect("ArchiveRequested effect expected");
    assert!(
        archived.is_empty(),
        "explicit candidate export should honor the empty pinned candidate selection"
    );
}

#[test]
fn archive_dialog_checkpoint_submit_skips_override_persist_when_overrides_already_empty() {
    init_logging();
    let state = complete_triage_state_for_test(1);
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();

    let (_state, effects) = update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".to_string(),
            set_checkpoint: true,
            submitted_at: chrono::Utc::now(),
            use_summaries: false,
            use_signal_candidates: false,
        },
    );

    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::PersistSignalCandidateOverrides { .. })),
        "checkpoint submit should not write an already-empty override set"
    );
}

#[test]
fn signal_candidate_selection_applies_threshold_and_order() {
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);

    for (i, score, key) in [(0usize, 80u8, "cluster-a"), (1usize, 30u8, "cluster-b")] {
        let url = format!("https://triage-complete.com/{i}");
        state
            .signal_candidate_mut()
            .enqueue(url.clone(), "fixture-input".to_string());
        state
            .signal_candidate_mut()
            .mark_scoring(&url, i as u64 + 1);
        state.signal_candidate_mut().complete(
            &url,
            SignalCandidateResult {
                signal_score: score,
                signal_key: key.to_string(),
                themes: vec!["t".to_string()],
                draft_gist: "g".to_string(),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
    }

    crate::fixture_support::save_session_results(&mut state);
    let selection = state.signal_candidate_selection();
    assert_eq!(
        selection.selected_urls,
        vec!["https://triage-complete.com/0".to_string()],
        "only the above-threshold article is selected"
    );
}

#[test]
fn archive_final_selection_signal_filtered_matches_shared_selection() {
    use crate::signal_candidate::ArchiveSelectionSource;
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);
    for (i, score, key) in [(0usize, 80u8, "cluster-a"), (1usize, 30u8, "cluster-b")] {
        let url = format!("https://triage-complete.com/{i}");
        state
            .signal_candidate_mut()
            .enqueue(url.clone(), "fixture-input".to_string());
        state
            .signal_candidate_mut()
            .mark_scoring(&url, i as u64 + 1);
        state.signal_candidate_mut().complete(
            &url,
            SignalCandidateResult {
                signal_score: score,
                signal_key: key.to_string(),
                themes: vec!["t".to_string()],
                draft_gist: "g".to_string(),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
    }

    crate::fixture_support::save_session_results(&mut state);
    let final_selection = state.archive_final_selection();
    assert_eq!(
        final_selection.source,
        ArchiveSelectionSource::SignalFiltered
    );
    assert_eq!(
        final_selection.ordered_urls,
        state.signal_candidate_selection().selected_urls,
        "archive_final_selection must use the shared selection compute"
    );
    assert_eq!(
        final_selection.ordered_urls,
        vec!["https://triage-complete.com/0".to_string()]
    );
}

#[test]
fn archive_final_selection_settled_empty_falls_back_to_full_corpus() {
    use crate::signal_candidate::ArchiveSelectionSource;
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    init_logging();
    let mut state = complete_triage_state_for_test(2);
    state = with_signal_candidate_metadata(state);
    for i in 0..2usize {
        let url = format!("https://triage-complete.com/{i}");
        state
            .signal_candidate_mut()
            .enqueue(url.clone(), "fixture-input".to_string());
        state
            .signal_candidate_mut()
            .mark_scoring(&url, i as u64 + 1);
        state.signal_candidate_mut().complete(
            &url,
            SignalCandidateResult {
                signal_score: 10,
                signal_key: format!("k{i}"),
                themes: vec!["t".to_string()],
                draft_gist: "g".to_string(),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
    }

    crate::fixture_support::save_session_results(&mut state);
    let final_selection = state.archive_final_selection();
    assert_eq!(
        final_selection.source,
        ArchiveSelectionSource::FullCorpusNoCandidates
    );
    assert_eq!(
        final_selection.ordered_urls,
        state.archive_corpus().ordered_urls().to_vec()
    );
}

#[test]
fn archive_final_selection_no_candidates_falls_back_to_full_corpus() {
    use crate::signal_candidate::ArchiveSelectionSource;

    init_logging();
    let state = complete_triage_state_for_test(2);
    let final_selection = state.archive_final_selection();
    assert_eq!(
        final_selection.source,
        ArchiveSelectionSource::FullCorpusSignalUnavailable
    );
    assert_eq!(
        final_selection.ordered_urls,
        state.archive_corpus().ordered_urls().to_vec()
    );
}
