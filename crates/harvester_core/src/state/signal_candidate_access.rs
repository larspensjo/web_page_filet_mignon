use super::AppState;
use crate::briefing::ArticleSummaryResult;
use crate::signal_candidate::{
    ArchiveFinalSelection, ArchiveSelectionSource, ScoredCandidate, SelectionPolicy,
    SignalCandidateArchiveSelection, SignalCandidateSelection, SignalCandidateSession,
};
use crate::signal_candidate_cache::{
    SignalCandidateCache, SignalCandidateCacheEntry, SignalCandidateCacheKey,
};
use crate::update::signal_candidate::SignalCandidateInputSnapshot;
use harvester_engine::llm::dto::SignalCandidateResult;

pub(super) enum SignalCandidateDisplayState<'a> {
    Pending,
    Scoring,
    Failed { reason: &'a str },
    Completed { result: &'a SignalCandidateResult },
}

impl AppState {
    /// Pin the signal-candidate archive selection snapshot for the current dialog session.
    pub fn pin_signal_candidate_selection(&mut self, selection: SignalCandidateArchiveSelection) {
        self.pinned_signal_candidate_selection = Some(selection);
    }

    /// Returns the signal-candidate snapshot pinned at archive-open time, if any.
    pub fn pinned_signal_candidate_selection(&self) -> Option<&SignalCandidateArchiveSelection> {
        self.pinned_signal_candidate_selection.as_ref()
    }

    /// Clears the pinned signal-candidate snapshot after the archive dialog session ends.
    pub fn clear_pinned_signal_candidate_selection(&mut self) {
        self.pinned_signal_candidate_selection = None;
    }

    pub fn signal_candidate(&self) -> &SignalCandidateSession {
        &self.signal_candidate
    }

    pub fn signal_candidate_mut(&mut self) -> &mut SignalCandidateSession {
        self.note_unfinished_inputs_changed();
        &mut self.signal_candidate
    }

    pub fn signal_candidate_cache(&self) -> &SignalCandidateCache {
        &self.signal_candidate_cache
    }

    pub(crate) fn set_signal_candidate_cache(&mut self, cache: SignalCandidateCache) {
        self.note_unfinished_global_inputs_changed();
        self.signal_candidate_cache = cache;
        self.rebuild_saved_results();
    }

    pub fn try_reuse_signal_candidate(
        &self,
        key: &SignalCandidateCacheKey,
    ) -> Option<SignalCandidateResult> {
        self.signal_candidate_cache
            .get(key)
            .map(|entry| entry.result.clone())
    }

    pub fn store_signal_candidate_result(
        &mut self,
        key: SignalCandidateCacheKey,
        result: SignalCandidateResult,
        now_utc: String,
    ) {
        self.note_unfinished_inputs_changed();
        let entry = SignalCandidateCacheEntry {
            result,
            created_at_utc: now_utc,
        };
        self.pending_results
            .push(crate::SavedResult::SignalCandidate(
                key.clone(),
                entry.clone(),
            ));
        self.signal_candidate_cache.insert(key.clone(), entry);
        self.refresh_saved_signal(&key);
    }

    pub(crate) fn signal_candidate_input_snapshot(
        &self,
        url: &str,
    ) -> Option<&SignalCandidateInputSnapshot> {
        self.signal_candidate_inputs.get(url)
    }

    pub(crate) fn set_signal_candidate_input_snapshot(
        &mut self,
        url: &str,
        snapshot: SignalCandidateInputSnapshot,
    ) {
        self.signal_candidate_inputs
            .insert(url.to_string(), snapshot);
    }

    pub(crate) fn clear_signal_candidate_input_snapshot(&mut self, url: &str) {
        self.signal_candidate_inputs.remove(url);
    }

    pub fn signal_candidate_threshold(&self) -> u8 {
        self.signal_candidate_threshold
    }

    pub fn set_signal_candidate_threshold(&mut self, threshold: u8) {
        self.signal_candidate_threshold = threshold.clamp(0, 100);
    }

    pub(crate) fn summary_result_for_url(&self, url: &str) -> Option<&ArticleSummaryResult> {
        if let Some(summary) = self.briefing.summary_for_url(url) {
            return Some(summary);
        }

        self.newest_summary_for_url(url)
    }

    pub(crate) fn saved_signal_count(&self) -> u32 {
        self.saved_results
            .values()
            .filter(|entry| entry.in_window && entry.actionable && entry.signal.is_some())
            .count() as u32
    }

    pub(in crate::state) fn display_signal_states(
        &self,
    ) -> Vec<(&str, SignalCandidateDisplayState<'_>)> {
        use crate::signal_candidate::SignalCandidateState;
        let mut states: std::collections::BTreeMap<_, _> = self
            .saved_results
            .values()
            .filter_map(|entry| {
                entry.signal.as_ref().map(|result| {
                    (
                        entry.article.url.as_str(),
                        SignalCandidateDisplayState::Completed { result },
                    )
                })
            })
            .collect();
        for (url, state) in self.signal_candidate().iter_states() {
            let state = match state {
                SignalCandidateState::Pending => SignalCandidateDisplayState::Pending,
                SignalCandidateState::Scoring { .. } => SignalCandidateDisplayState::Scoring,
                SignalCandidateState::Failed { reason } => {
                    SignalCandidateDisplayState::Failed { reason }
                }
                SignalCandidateState::Completed { .. } => continue,
            };
            states.entry(url).or_insert(state);
        }
        states.into_iter().collect()
    }

    /// The signal-candidate selection computed from saved current-key window results:
    /// the same threshold + exclusion logic the Archive dialog uses. Single
    /// source of truth shared by the dialog snapshot and archive selection.
    pub(crate) fn signal_candidate_selection(&self) -> SignalCandidateSelection {
        let scored: Vec<ScoredCandidate> = self
            .saved_results
            .values()
            .filter(|entry| entry.in_window && entry.actionable)
            .filter_map(|entry| {
                entry
                    .signal
                    .as_ref()
                    .map(|result| (entry.article.url.as_str(), result))
            })
            .map(|(url, result)| ScoredCandidate {
                url: url.to_string(),
                result: result.clone(),
            })
            .collect();
        let policy = SelectionPolicy {
            threshold: self.signal_candidate_threshold(),
            active_prompt_version: self
                .active_version_for(harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate)
                .unwrap_or_default(),
            excluded: self.signal_candidate().excluded().clone(),
        };
        SignalCandidateSelection::compute(&scored, policy)
    }

    /// The exact ordered URL list the Archive would export right now: the triage
    /// base corpus narrowed by the settled signal-candidate selection, falling
    /// back to the full base corpus when the selection is empty or no candidates
    /// were scored.
    ///
    /// Note: in-flight scoring is intentionally not consulted here. Callers that
    /// must not act mid-scoring should gate before calling this accessor.
    /// Archive actions gate on run state; a completed live triage session is not required.
    pub fn archive_final_selection(&self) -> ArchiveFinalSelection {
        let base = self.archive_corpus();
        let completed = self.saved_signal_count();
        let failed = self.signal_candidate().failed_count();

        if completed == 0 && failed == 0 {
            return ArchiveFinalSelection {
                ordered_urls: base.ordered_urls().to_vec(),
                source: ArchiveSelectionSource::FullCorpusSignalUnavailable,
            };
        }

        let selection = self.signal_candidate_selection();
        if selection.selected_urls.is_empty() {
            return ArchiveFinalSelection {
                ordered_urls: base.ordered_urls().to_vec(),
                source: ArchiveSelectionSource::FullCorpusNoCandidates,
            };
        }

        ArchiveFinalSelection {
            ordered_urls: selection.selected_urls,
            source: ArchiveSelectionSource::SignalFiltered,
        }
    }

    /// The base corpus is ready to summarize when triage is complete, at least
    /// one eligible article exists, and no briefing run is already in flight.
    /// `BriefingSession::can_start()` blocks any active briefing run so the
    /// summarize entry point only arms a fresh load when the session is idle.
    pub fn summaries_can_start(&self) -> bool {
        matches!(self.triage().phase(), crate::triage::TriagePhase::Complete)
            && !self.archive_corpus().is_empty()
            && self.briefing.can_start()
    }
}
