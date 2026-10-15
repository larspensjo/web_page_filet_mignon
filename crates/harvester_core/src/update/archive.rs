use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::{archive_url_key, ArchiveDocAnnotations};

pub(super) fn handle_archive_clicked(state: &mut AppState) -> Vec<Effect> {
    if !state.export_available() {
        engine_warn!(
            "[archive-gate] action=open run_id={} run_state={:?}",
            state.run_progress().map_or(0, |run| run.run_id),
            state.run_state()
        );
        return Vec::new();
    }
    clear_export_unavailable_status(state);
    let request_id = state.allocate_next_archive_request_id();
    let corpus = state.archive_corpus(); // triage-only; pre-triage excluded
    let corpus_count = corpus.count();
    let fingerprint = corpus.fingerprint();
    let source = corpus.source();
    state.pin_archive_corpus(corpus);
    let full_selection = state
        .pinned_full_archive_selection()
        .expect("full-mode pin");
    let article_count = full_selection.ordered_urls.len();
    let token_estimates = state.archive_token_estimates(&full_selection.ordered_urls);
    let signal_candidate_snapshot = build_signal_candidate_snapshot(state);
    engine_info!(
        "[working-corpus] source={:?} count={} identical_content_folded={} fingerprint={:#010x} caller=archive-open request_id={}",
        source,
        corpus_count,
        corpus_count - article_count,
        fingerprint,
        request_id,
    );
    // Count pre-triage articles ready for review. resolved_included_urls() is phase-gated:
    // returns empty when phase is Idle (after consume) or during Reviewing (phase-gated).
    let pending_pre_triage_count = state.pre_triage().resolved_included_urls().len();
    let since_utc = state.briefing_since_utc();
    state.pin_signal_candidate_selection(signal_candidate_snapshot.clone());
    let signal_candidate_default = {
        crate::signal_candidate::compute_dialog_default(
            state.saved_signal_count(),
            state.signal_candidate().in_flight_count(),
            state.signal_candidate().failed_count(),
            signal_candidate_snapshot.selected_urls.len(),
        )
    };
    let signal_candidate_count = signal_candidate_snapshot.selected_urls.len();
    let (mode, selected, folds) = if signal_candidate_default
        == crate::signal_candidate::SignalCandidateDialogDefault::OnAllSettled
    {
        (
            "signal",
            signal_candidate_count,
            &signal_candidate_snapshot.folds,
        )
    } else {
        (
            "full",
            article_count,
            &state
                .pinned_full_archive_selection()
                .expect("full-mode pin")
                .folds,
        )
    };
    let same_event = folds
        .values()
        .flatten()
        .filter(|member| member.reason == harvester_engine::ArchiveFoldReason::SameEvent)
        .count();
    let identical_content = folds
        .values()
        .flatten()
        .filter(|member| member.reason == harvester_engine::ArchiveFoldReason::IdenticalContent)
        .count();
    engine_info!("[signal-archive] open request_id={} mode={} selected={} same_event={} identical_content={}", request_id, mode, selected, same_event, identical_content);
    let signal_candidate_scoring_done =
        state.saved_signal_count() + state.signal_candidate().failed_count();
    let signal_candidate_scoring_total = state
        .signal_candidate()
        .enqueued_count()
        .max(signal_candidate_scoring_done);
    vec![Effect::OpenArchiveDialog {
        request_id,
        article_count,
        since_utc,
        default_basename: "archive.md".to_string(),
        pending_pre_triage_count,
        token_estimates,
        signal_candidate_default,
        signal_candidate_count,
        signal_candidate_scoring_done,
        signal_candidate_scoring_total,
        signal_candidate_token_estimates: signal_candidate_snapshot.token_estimates,
    }]
}

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_dialog_ready(
    state: &mut AppState,
    request_id: u64,
    article_count: usize,
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
    default_basename: String,
    default_file_exists: bool,
    export_dir: std::path::PathBuf,
    pending_pre_triage_count: usize,
    token_estimates: crate::ArchiveTokenEstimates,
    signal_candidate_default: crate::signal_candidate::SignalCandidateDialogDefault,
    signal_candidate_count: usize,
    signal_candidate_scoring_done: u32,
    signal_candidate_scoring_total: u32,
    signal_candidate_token_estimates: crate::ArchiveTokenEstimates,
) -> Vec<Effect> {
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    vec![Effect::ShowArchiveDialog {
        request_id,
        article_count,
        since_utc,
        default_basename,
        default_file_exists,
        export_dir,
        pending_pre_triage_count,
        token_estimates,
        signal_candidate_default,
        signal_candidate_count,
        signal_candidate_scoring_done,
        signal_candidate_scoring_total,
        signal_candidate_token_estimates,
    }]
}

pub(super) fn handle_dialog_submitted(
    state: &mut AppState,
    request_id: u64,
    basename: String,
    set_checkpoint: bool,
    submitted_at: chrono::DateTime<chrono::Utc>,
    use_summaries: bool,
    use_signal_candidates: bool,
) -> Vec<Effect> {
    engine_info!(
        "[archive-dialog] submit received request_id={} current_request_id={} basename={} set_checkpoint={} use_summaries={} use_signal_candidates={}",
        request_id, state.archive_request_id(), basename, set_checkpoint, use_summaries, use_signal_candidates
    );
    if !state.export_available() {
        engine_warn!(
            "[archive-gate] action=submit request_id={} run_id={} run_state={:?}",
            request_id,
            state.run_progress().map_or(0, |run| run.run_id),
            state.run_state()
        );
        state.set_briefing_checkpoint_status_message(Some(
            crate::state::EXPORT_UNAVAILABLE_STATUS_MESSAGE.to_string(),
        ));
        return Vec::new();
    }
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    clear_export_unavailable_status(state);
    if !is_safe_archive_basename(&basename) {
        engine_warn!(
            "[archive-dialog] rejecting invalid basename request_id={} basename={}",
            request_id,
            basename
        );
        return Vec::new();
    }
    let pinned = state.pinned_archive_corpus();
    let (full_count, fingerprint) = match pinned {
        Some(corpus) => (corpus.count(), corpus.fingerprint()),
        None => {
            // Unreachable in normal operation: ArchiveClicked always precedes
            // ArchiveDialogSubmitted. Guard defensively rather than emit an empty
            // archive file to disk.
            engine_warn!(
                "[archive-dialog] no pinned corpus at submit time request_id={}; dropping submit",
                request_id
            );
            return Vec::new();
        }
    };
    let full_selection = state
        .pinned_full_archive_selection()
        .cloned()
        .expect("corpus and full-mode selection are pinned together");
    let folded_full_count = full_selection.ordered_urls.len();
    state.clear_pinned_archive_corpus();
    engine_info!(
        "[working-corpus] source=pinned count={} fingerprint={:#010x} caller=archive-submit request_id={}",
        full_count,
        fingerprint,
        request_id,
    );
    let (ordered_urls, folds) = if use_signal_candidates {
        match state.pinned_signal_candidate_selection().cloned() {
            Some(snapshot) => {
                engine_info!(
                    "[signal-archive] submit decision=use_candidates count={} threshold={} override_fp={} cache_fp={} scoring_in_progress={}",
                    snapshot.selected_urls.len(),
                    snapshot.threshold,
                    snapshot.override_fingerprint,
                    snapshot.cache_fingerprint,
                    snapshot.scoring_in_progress
                );
                (snapshot.selected_urls, snapshot.folds)
            }
            None => {
                engine_warn!(
                    "[signal-archive] use_signal_candidates=true but snapshot was missing; falling back to full corpus"
                );
                (full_selection.ordered_urls, full_selection.folds)
            }
        }
    } else {
        engine_info!(
            "[signal-archive] submit decision=use_full_corpus url_count={}",
            folded_full_count
        );
        (full_selection.ordered_urls, full_selection.folds)
    };
    state.clear_pinned_signal_candidate_selection();
    let since_utc = state.briefing_since_utc();
    let requested_checkpoint = set_checkpoint.then_some(submitted_at);
    let mut annotated_urls = ordered_urls.clone();
    let raw_urls_by_key: std::collections::HashMap<_, _> = ordered_urls
        .iter()
        .map(|url| (archive_url_key(url), url.as_str()))
        .collect();
    for (kept, members) in &folds {
        let kept_url = raw_urls_by_key.get(kept).copied().unwrap_or(kept);
        let kept_key = state
            .saved_results_for_url(kept_url)
            .and_then(|entry| entry.signal.as_ref())
            .map(|result| result.signal_key.as_str())
            .unwrap_or("-");
        for member in members {
            engine_info!("[signal-archive] fold request_id={} reason={} kept_url={} kept_signal_key={} folded_url={} folded_signal_key={}", request_id, member.reason.as_str(), kept, kept_key, member.url, member.signal_key.as_deref().unwrap_or("-"));
            annotated_urls.push(member.url.clone());
        }
    }
    let summaries = if use_summaries {
        build_summary_map(state, &annotated_urls)
    } else {
        std::collections::HashMap::new()
    };
    let annotations = build_annotation_map(state, &annotated_urls);
    let priority_snapshot = build_priority_snapshot(state);
    engine_info!(
        "[archive-dialog] export effect ready request_id={} basename={} urls={} summaries={} annotations={} priorities={} fold_groups={}",
        request_id, basename, ordered_urls.len(), summaries.len(), annotations.len(), priority_snapshot.len(), folds.len()
    );
    let mut effects = vec![Effect::ArchiveRequested {
        request_id,
        basename,
        ordered_urls,
        since_utc,
        requested_checkpoint,
        use_summaries,
        summaries,
        annotations,
        priority_snapshot,
        folds,
    }];
    let had_signal_candidate_overrides = !state.signal_exclusions().excluded().is_empty();
    if set_checkpoint && had_signal_candidate_overrides {
        state
            .signal_exclusions_mut()
            .set_excluded(Default::default());
        effects.push(Effect::PersistSignalCandidateOverrides {
            overrides: state.signal_exclusions().excluded().clone(),
        });
        engine_info!(
            "[signal-overrides] cleared at archive-checkpoint request_id={}",
            request_id
        );
        state.mark_dirty();
    }
    effects
}

fn clear_export_unavailable_status(state: &mut AppState) {
    if state.briefing_checkpoint_status_message()
        == Some(crate::state::EXPORT_UNAVAILABLE_STATUS_MESSAGE)
    {
        state.set_briefing_checkpoint_status_message(None);
    }
}

fn build_priority_snapshot(state: &AppState) -> std::collections::HashMap<String, u8> {
    state
        .saved_results_entries()
        .filter(|entry| entry.in_window)
        .filter_map(|entry| {
            entry
                .triage
                .as_ref()
                .map(|(_, result)| (archive_url_key(&entry.article.url), result.priority))
        })
        .collect()
}

fn build_annotation_map(
    state: &AppState,
    ordered_urls: &[String],
) -> std::collections::HashMap<String, ArchiveDocAnnotations> {
    let mut annotations = std::collections::HashMap::new();
    for url in ordered_urls {
        let mut annotation = ArchiveDocAnnotations::default();
        if let Some(entry) = state
            .saved_results_for_url(url)
            .filter(|entry| entry.in_window)
        {
            if let Some((key, result)) = &entry.triage {
                annotation.priority = Some(result.priority);
                annotation.tags = Some(result.tags.clone());
                annotation.triage_model = Some(key.model_id.clone());
            }
            if let Some(result) = &entry.signal {
                annotation.signal_key = Some(result.signal_key.clone());
                annotation.signal_score = Some(result.signal_score);
                annotation.themes = Some(result.themes.clone());
            }
        }
        if annotation != ArchiveDocAnnotations::default() {
            annotations.insert(archive_url_key(url), annotation);
        }
    }
    annotations
}

pub(super) fn handle_export_completed(
    state: &mut AppState,
    request_id: u64,
    requested_checkpoint: Option<chrono::DateTime<chrono::Utc>>,
) -> Vec<Effect> {
    engine_info!(
        "[archive-dialog] completion received request_id={} current_request_id={} requested_checkpoint={:?}",
        request_id, state.archive_request_id(), requested_checkpoint
    );
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    // Clear any residual pin (idempotent, normally already cleared at submit time).
    state.clear_pinned_archive_corpus();
    state.clear_pinned_signal_candidate_selection();
    if let Some(checkpoint) = requested_checkpoint {
        let save_id = state.begin_briefing_checkpoint_save(Some(checkpoint), true);
        engine_info!(
            "[archive-dialog] checkpoint save requested request_id={} save_id={} since_utc={}",
            request_id,
            save_id,
            checkpoint
        );
        state.mark_dirty();
        vec![Effect::SaveBriefingCheckpoint {
            save_id,
            since_utc: Some(checkpoint),
        }]
    } else {
        Vec::new()
    }
}

pub(super) fn handle_export_failed(
    state: &mut AppState,
    request_id: u64,
    basename: String,
    reason: String,
) -> Vec<Effect> {
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    engine_warn!(
        "[archive-dialog] export failed request_id={} basename={} reason={}",
        request_id,
        basename,
        reason
    );
    // Clear any residual pin (idempotent, normally already cleared at submit time).
    state.clear_pinned_archive_corpus();
    state.clear_pinned_signal_candidate_selection();
    Vec::new()
}

fn is_safe_archive_basename(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    if name.contains(['/', '\\', '\0']) {
        return false;
    }
    !std::path::Path::new(name).is_absolute()
}

fn build_summary_map(
    state: &AppState,
    ordered_urls: &[String],
) -> std::collections::HashMap<String, String> {
    use harvester_engine::archive_url_key;

    let mut map = std::collections::HashMap::new();
    for url in ordered_urls {
        if let Some(summary) = state.newest_summary_for_url(url) {
            map.insert(archive_url_key(url), format_summary_body(summary));
        }
    }
    map
}

fn format_summary_body(result: &crate::briefing::ArticleSummaryResult) -> String {
    let mut body = format!("## Summary\n{}\n", result.summary.trim_end());
    if !result.key_points.is_empty() {
        body.push_str("\n## Key Points\n");
        for point in &result.key_points {
            body.push_str(&format!("- {}\n", point.trim_end()));
        }
    }
    body
}

fn build_signal_candidate_snapshot(
    state: &AppState,
) -> crate::signal_candidate::SignalCandidateArchiveSelection {
    let selection = state.signal_candidate_selection();
    let token_estimates = state.archive_token_estimates(&selection.selected_urls);
    let cache_fingerprint = signal_candidate_selection_fingerprint(state, &selection.selected_urls);

    crate::signal_candidate::SignalCandidateArchiveSelection::new(
        selection.selected_urls,
        selection.folds,
        state.signal_candidate_threshold(),
        state.signal_exclusions().override_fingerprint(),
        cache_fingerprint,
        token_estimates,
        state.signal_candidate().in_flight_count() > 0,
    )
}

fn signal_candidate_selection_fingerprint(state: &AppState, urls: &[String]) -> String {
    use sha2::Digest;

    let mut hasher = sha2::Sha256::new();
    for url in urls {
        hasher.update(url.as_bytes());
        hasher.update(b"|");
        if let Some(result) = state
            .saved_results_for_url(url)
            .and_then(|entry| entry.signal.as_ref())
        {
            hasher.update(result.signal_key.as_bytes());
            hasher.update(b"|");
            hasher.update([result.signal_score]);
            hasher.update(b"|");
            hasher.update(format!("{:?}", result.source_tier).as_bytes());
            hasher.update(b"|");
            hasher.update(format!("{:?}", result.confidence).as_bytes());
            hasher.update(b"|");
            hasher.update(result.themes.join("\0").as_bytes());
            hasher.update(b"|");
            hasher.update(result.draft_gist.as_bytes());
        }
        hasher.update(b"\n");
    }
    crate::cache_utils::hex_digest(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefing::LoadedArticle;
    use crate::triage::{ArticleTriageResult, TriageSession};
    use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};

    #[test]
    fn annotations_include_triage_empty_tags_and_zero_score_but_omit_unavailable_values() {
        let url = "https://example.com/article";
        let mut state = AppState::new();
        let mut triage = TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: url.into(),
            source_title: None,
            prepared_text: String::new(),
            content_hash: "hash".into(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        triage.complete_article_with_model(
            0,
            ArticleTriageResult {
                category: "news".into(),
                priority: 4,
                tags: vec![],
                rationale: String::new(),
                input_tokens: 0,
                output_tokens: 0,
            },
            Some("stored-model".into()),
        );
        state.set_triage(triage);
        state
            .signal_candidate_mut()
            .enqueue(url.into(), "fixture-input".to_string());
        state.signal_candidate_mut().complete(
            url,
            SignalCandidateResult {
                signal_score: 0,
                signal_key: "event".into(),
                themes: vec![],
                draft_gist: String::new(),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::Low,
                reasoning: String::new(),
                input_tokens: 0,
                output_tokens: 0,
            },
        );
        crate::fixture_support::save_session_results(&mut state);
        let annotations =
            build_annotation_map(&state, &[url.into(), "https://example.com/none".into()]);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[&archive_url_key(url)].priority, Some(4));
        assert_eq!(annotations[&archive_url_key(url)].tags, Some(vec![]));
        assert_eq!(
            annotations[&archive_url_key(url)].triage_model.as_deref(),
            Some("stored-model")
        );
        assert_eq!(annotations[&archive_url_key(url)].signal_score, Some(0));
        assert_eq!(annotations[&archive_url_key(url)].themes, Some(vec![]));
    }
}
