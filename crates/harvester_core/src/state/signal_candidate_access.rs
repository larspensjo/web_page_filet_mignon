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
use crate::SummaryCacheKey;
use harvester_engine::llm::dto::SignalCandidateResult;
use harvester_engine::llm::prompt::PromptId;

/// Whether the briefing may generate now, and on what list.
///
/// The `Ready` variant carries the resolved selection so the entry point does
/// not recompute it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BriefingGenerateReadiness {
    Ready { selection: ArchiveFinalSelection },
    TriageOrCorpusNotReady,
    SummariesNotSettled,
    SignalScoringInProgress,
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
        &mut self.signal_candidate
    }

    pub fn signal_candidate_cache(&self) -> &SignalCandidateCache {
        &self.signal_candidate_cache
    }

    pub(crate) fn set_signal_candidate_cache(&mut self, cache: SignalCandidateCache) {
        self.signal_candidate_cache = cache;
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
        self.signal_candidate_cache.insert(
            key,
            SignalCandidateCacheEntry {
                result,
                created_at_utc: now_utc,
            },
        );
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

    pub fn summary_cache_key_for_url(&self, url: &str) -> Option<SummaryCacheKey> {
        let content_hash = self
            .triage()
            .article_content_hash(url)
            .or_else(|| self.pre_triage.article_content_hash(url))?;

        self.summary_cache()
            .iter()
            .filter(|(key, _)| {
                key.content_hash == content_hash && key.prompt_id == PromptId::ArticleSummary
            })
            .max_by(|(_, a), (_, b)| {
                let parsed_a = chrono::DateTime::parse_from_rfc3339(&a.created_at_utc)
                    .ok()
                    .map(|dt| dt.with_timezone(&chrono::Utc));
                let parsed_b = chrono::DateTime::parse_from_rfc3339(&b.created_at_utc)
                    .ok()
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                match (parsed_a, parsed_b) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    _ => a.created_at_utc.cmp(&b.created_at_utc),
                }
            })
            .map(|(key, _)| key.clone())
    }

    pub(crate) fn summary_result_for_url(&self, url: &str) -> Option<&ArticleSummaryResult> {
        if let Some(summary) = self.briefing.summary_for_url(url) {
            return Some(summary);
        }

        let content_hash = self
            .triage()
            .article_content_hash(url)
            .or_else(|| self.pre_triage.article_content_hash(url))?;

        self.summary_cache()
            .lookup_any_by_content_hash(content_hash)
            .map(|entry| &entry.result)
    }

    /// The live signal-candidate selection computed from the current session:
    /// the same threshold + exclusion logic the Archive dialog uses. Single
    /// source of truth shared by the dialog snapshot and the briefing selector.
    pub(crate) fn signal_candidate_selection(&self) -> SignalCandidateSelection {
        let scored: Vec<ScoredCandidate> = self
            .signal_candidate()
            .iter_completed()
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
    pub fn archive_final_selection(&self) -> ArchiveFinalSelection {
        let base = self.archive_corpus();
        let completed = self.signal_candidate().completed_count();
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
    /// `BriefingSession::can_start()` also blocks `WaitingForTriage`, which is
    /// intentionally treated as an outstanding briefing request.
    pub fn summaries_can_start(&self) -> bool {
        matches!(self.triage().phase(), crate::triage::TriagePhase::Complete)
            && !self.archive_corpus().is_empty()
            && self.briefing.can_start()
    }

    /// The corpus-relative readiness verdict for Generate Briefing.
    ///
    /// This deliberately does not include session/AI gates. The view and
    /// entry-point guards compose those with this verdict when they need the
    /// full "can generate now" answer.
    pub fn briefing_generate_readiness(&self) -> BriefingGenerateReadiness {
        let corpus = self.archive_corpus();
        if corpus.is_empty()
            || !matches!(self.triage().phase(), crate::triage::TriagePhase::Complete)
        {
            return BriefingGenerateReadiness::TriageOrCorpusNotReady;
        }

        let all_settled = corpus.ordered_urls().iter().all(|url| {
            self.summary_result_for_url(url).is_some() || self.briefing.summary_failed_for_url(url)
        });
        if !all_settled {
            return BriefingGenerateReadiness::SummariesNotSettled;
        }

        if self.signal_candidate().in_flight_count() > 0 {
            return BriefingGenerateReadiness::SignalScoringInProgress;
        }

        BriefingGenerateReadiness::Ready {
            selection: self.archive_final_selection(),
        }
    }
}
