use crate::{AppState, Effect};
use engine_logging::{engine_info, engine_warn};

pub(super) fn handle_archive_clicked(state: &mut AppState) -> Vec<Effect> {
    let request_id = state.allocate_next_archive_request_id();
    let corpus = state.archive_corpus(); // triage-only; pre-triage excluded
    let article_count = corpus.count();
    let fingerprint = corpus.fingerprint();
    let source = corpus.source();
    let token_estimates = state.archive_token_estimates(corpus.ordered_urls());
    let signal_candidate_snapshot = build_signal_candidate_snapshot(state);
    engine_info!(
        "[working-corpus] source={:?} count={} fingerprint={:#010x} caller=archive-open request_id={}",
        source,
        article_count,
        fingerprint,
        request_id,
    );
    // Count pre-triage articles ready for review. resolved_included_urls() is phase-gated:
    // returns empty when phase is Idle (after consume) or during Reviewing (phase-gated).
    let pending_pre_triage_count = state.pre_triage().resolved_included_urls().len();
    state.pin_archive_corpus(corpus);
    let since_utc = state.briefing_since_utc();
    state.pin_signal_candidate_selection(signal_candidate_snapshot.clone());
    let signal_candidate_default = crate::signal_candidate::compute_dialog_default(
        state.signal_candidate().completed_count(),
        state.signal_candidate().in_flight_count(),
        state.signal_candidate().failed_count(),
        signal_candidate_snapshot.selected_urls.len(),
    );
    let signal_candidate_count = signal_candidate_snapshot.selected_urls.len();
    let signal_candidate_scoring_done =
        state.signal_candidate().completed_count() + state.signal_candidate().failed_count();
    let signal_candidate_scoring_total = state.signal_candidate().enqueued_count();
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
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    if !is_safe_archive_basename(&basename) {
        engine_warn!(
            "[archive-dialog] rejecting invalid basename request_id={} basename={}",
            request_id,
            basename
        );
        return Vec::new();
    }
    let pinned = state.pinned_archive_corpus();
    let (full_urls, fingerprint) = match pinned {
        Some(corpus) => (corpus.ordered_urls().to_vec(), corpus.fingerprint()),
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
    state.clear_pinned_archive_corpus();
    engine_info!(
        "[working-corpus] source=pinned count={} fingerprint={:#010x} caller=archive-submit request_id={}",
        full_urls.len(),
        fingerprint,
        request_id,
    );
    let ordered_urls = if use_signal_candidates {
        match state.pinned_signal_candidate_selection().cloned() {
            Some(snapshot) => {
                engine_info!(
                    "[signal-archive] submit decision=use_candidates count={} threshold={} cap={} override_fp={} cache_fp={} scoring_in_progress={}",
                    snapshot.selected_urls.len(),
                    snapshot.threshold,
                    snapshot.cap,
                    snapshot.override_fingerprint,
                    snapshot.cache_fingerprint,
                    snapshot.scoring_in_progress
                );
                snapshot.selected_urls
            }
            None => {
                engine_warn!(
                    "[signal-archive] use_signal_candidates=true but snapshot was missing; falling back to full corpus"
                );
                full_urls
            }
        }
    } else {
        engine_info!(
            "[signal-archive] submit decision=use_full_corpus url_count={}",
            full_urls.len()
        );
        full_urls
    };
    state.clear_pinned_signal_candidate_selection();
    let since_utc = state.briefing_since_utc();
    let requested_checkpoint = set_checkpoint.then_some(submitted_at);
    let summaries = if use_summaries {
        build_summary_map(state, &ordered_urls)
    } else {
        std::collections::HashMap::new()
    };
    let mut effects = vec![Effect::ArchiveRequested {
        request_id,
        basename,
        ordered_urls,
        since_utc,
        requested_checkpoint,
        use_summaries,
        summaries,
    }];
    let had_signal_candidate_overrides = !state.signal_candidate().excluded().is_empty();
    if set_checkpoint && had_signal_candidate_overrides {
        state
            .signal_candidate_mut()
            .set_excluded(Default::default());
        effects.push(Effect::PersistSignalCandidateOverrides {
            overrides: state.signal_candidate().excluded().clone(),
        });
        engine_info!(
            "[signal-overrides] cleared at archive-checkpoint request_id={}",
            request_id
        );
        state.mark_dirty();
    }
    effects
}

pub(super) fn handle_export_completed(
    state: &mut AppState,
    request_id: u64,
    requested_checkpoint: Option<chrono::DateTime<chrono::Utc>>,
) -> Vec<Effect> {
    if request_id != state.archive_request_id() {
        return Vec::new();
    }
    // Clear any residual pin (idempotent, normally already cleared at submit time).
    state.clear_pinned_archive_corpus();
    state.clear_pinned_signal_candidate_selection();
    if let Some(checkpoint) = requested_checkpoint {
        let save_id = state.begin_briefing_checkpoint_save(Some(checkpoint));
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
        if let Some(hash) = state.triage().article_content_hash(url) {
            if let Some(entry) = state.summary_cache().lookup_any_by_content_hash(hash) {
                map.insert(archive_url_key(url), format_summary_body(&entry.result));
            }
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
    use crate::signal_candidate::{ScoredCandidate, SelectionPolicy, SignalCandidateSelection};

    let scored: Vec<ScoredCandidate> = state
        .signal_candidate()
        .iter_completed()
        .map(|(url, result)| ScoredCandidate {
            url: url.to_string(),
            result: result.clone(),
        })
        .collect();
    let policy = SelectionPolicy {
        threshold: state.signal_candidate_threshold(),
        cap: state.signal_candidate_cap(),
        active_prompt_version: state
            .active_version_for(harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate)
            .unwrap_or_default(),
        excluded: state.signal_candidate().excluded().clone(),
    };
    let selection = SignalCandidateSelection::compute(&scored, policy);
    let token_estimates = state.archive_token_estimates(&selection.selected_urls);
    let cache_fingerprint = signal_candidate_selection_fingerprint(state, &selection.selected_urls);

    crate::signal_candidate::SignalCandidateArchiveSelection::new(
        selection.selected_urls,
        state.signal_candidate_threshold(),
        state.signal_candidate_cap(),
        state.signal_candidate().override_fingerprint(),
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
        if let Some((_, result)) = state
            .signal_candidate()
            .iter_completed()
            .find(|(candidate_url, _)| *candidate_url == url)
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
