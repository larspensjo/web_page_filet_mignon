use super::{
    AppState, ArchiveTokenEstimates, BatchNextAction, BatchObservation, BatchStatus, JobResultKind,
    PreTriagePhase, TriagePhase,
};
use crate::archive_display::{ArchiveDisplayCounts, CacheDerivedArchive};
use crate::working_corpus::CurrentWorkingCorpus;

impl AppState {
    /// Returns a snapshot of batch processing state for headless monitoring.
    /// Provides metrics without UI dependencies.
    pub fn batch_observation(&self) -> BatchObservation {
        // Count jobs by outcome visibility for batch settling:
        // - in-flight includes queued and actively processing jobs (no final outcome yet)
        // - done counts successful completions
        // - failed counts terminal failures
        let jobs_total = self.jobs.len();
        let mut jobs_done = 0;
        let mut jobs_failed = 0;
        let mut jobs_in_flight = 0;

        for job in self.jobs.values() {
            match job.outcome.as_ref() {
                Some(JobResultKind::Success) => jobs_done += 1,
                Some(JobResultKind::Failed { .. }) => jobs_failed += 1,
                None => jobs_in_flight += 1,
            }
        }

        // Triage metrics
        let (triage_total, triage_pending, triage_in_flight, triage_completed, triage_failed) =
            self.triage.observation_counts();
        let pre_triage_total = self.pre_triage.entries().len();
        let pre_triage_included = self.pre_triage.resolved_included_articles().len();
        let pre_triage_review = self
            .pre_triage
            .entries()
            .iter()
            .filter(|entry| {
                matches!(
                    entry.auto_verdict,
                    crate::pre_triage_filter::AutoVerdict::Review
                ) && entry.manual_decision.is_none()
            })
            .count();
        let pre_triage_filtered =
            pre_triage_total.saturating_sub(pre_triage_included + pre_triage_review);
        let summary_total = self.briefing.articles().len();
        let summary_pending = self.briefing.pending_count();
        let summary_in_flight = self.briefing.in_progress_count();
        let summary_completed = self.briefing.completed_summary_count();
        let summary_failed = self.briefing.failed_summary_count();

        BatchObservation {
            poll_in_progress: self.source_states.is_poll_in_progress(),
            session_state: self.session,
            jobs_total,
            jobs_done,
            jobs_failed,
            jobs_in_flight,
            pre_triage_phase: self.pre_triage.phase().clone(),
            pre_triage_total,
            pre_triage_included,
            pre_triage_review,
            pre_triage_filtered,
            triage_phase: self.triage.phase().clone(),
            triage_total,
            triage_pending,
            triage_in_flight,
            triage_completed,
            triage_failed,
            summary_total,
            summary_pending,
            summary_in_flight,
            summary_completed,
            summary_failed,
            triage_cache_hits: self.triage_cache_run_metrics.hits() as usize,
            triage_cache_misses: self.triage_cache_run_metrics.misses() as usize,
            triage_cache_key_unavailable: self.triage_cache_run_metrics.key_unavailable() as usize,
            summary_cache_hits: self.summary_cache_metrics.hits(),
            summary_cache_misses: self.summary_cache_metrics.misses(),
            summary_cache_key_unavailable: self.summary_cache_metrics.key_unavailable(),
            import_phase: self.import_session.phase,
            imports_completed: self.import_session.imports_completed,
            imports_failed: self.import_session.imports_failed,
            import_in_flight: self.import_session.phase
                == crate::import_session::ImportPhase::Importing,
            source_poll_stats: self.source_states.last_completed_poll_stats().to_vec(),
        }
    }

    pub fn current_working_corpus(&self) -> CurrentWorkingCorpus {
        CurrentWorkingCorpus::select(
            self.pre_triage(),
            self.triage(),
            self.briefing_triage_policy(),
        )
    }

    pub fn batch_next_action(&self) -> BatchNextAction {
        let pre_triage_included = self.pre_triage.resolved_included_articles().len();

        if self.can_start_triage_from_pre_triage()
            && self.triage.can_start()
            && self.triage.total() < pre_triage_included
        {
            return BatchNextAction::DispatchTriage;
        }

        if matches!(self.triage.phase(), TriagePhase::Complete)
            && self.triage.completed_count() > 0
            && self.briefing.can_start()
            && !self.triage.is_active()
            && self.briefing.articles().is_empty()
        {
            return BatchNextAction::DispatchSummaries;
        }

        BatchNextAction::None
    }

    pub fn batch_status(&self) -> BatchStatus {
        let batch = self.batch_observation();
        let has_active_work = batch.poll_in_progress
            || matches!(batch.pre_triage_phase, PreTriagePhase::LoadingArticles)
            || matches!(
                batch.triage_phase,
                TriagePhase::LoadingArticles | TriagePhase::Triaging
            )
            || batch.jobs_in_flight > 0
            || batch.triage_in_flight > 0
            || batch.summary_in_flight > 0
            || batch.summary_pending > 0
            || self.briefing.is_active()
            || batch.import_in_flight;

        if has_active_work {
            BatchStatus::Running
        } else {
            BatchStatus::Settled
        }
    }

    /// Returns the live-triage-only corpus used by archive actions.
    ///
    /// Pre-triage articles (even when ready) are excluded - they need triage first.
    pub(crate) fn archive_corpus(&self) -> CurrentWorkingCorpus {
        CurrentWorkingCorpus::select_for_archive(self.triage(), self.briefing_triage_policy())
    }

    pub(in crate::state) fn archive_display_counts(&self) -> ArchiveDisplayCounts {
        if matches!(self.triage().phase(), TriagePhase::Complete) {
            return ArchiveDisplayCounts::live(CurrentWorkingCorpus::select_for_archive(
                self.triage(),
                self.briefing_triage_policy(),
            ));
        }

        if let Some(cache_derived) = self.cache_derived_archive_display() {
            let actionable_total = self.pre_triage().tentative_included_urls().len();
            if cache_derived.cache_hit_count() > 0 {
                return ArchiveDisplayCounts::cache_derived(cache_derived, actionable_total);
            }
        }

        ArchiveDisplayCounts::live(CurrentWorkingCorpus::select_for_archive(
            self.triage(),
            self.briefing_triage_policy(),
        ))
    }

    /// Derive the archive corpus URLs from the persisted triage cache for display
    /// when the live triage session has not run this session.
    ///
    /// Covers the actionable pre-triage corpus — both `ReadyToTriage` and the
    /// tentative `Reviewing` set, mirroring [`can_start_triage_from_pre_triage`] —
    /// and includes each article that already has a triage cache hit under the
    /// current prompt version, model, and context. Articles without a hit (never
    /// triaged, or triaged under a now-superseded prompt/model) are simply omitted,
    /// so the count reflects exactly the portion of the corpus that is already
    /// triaged rather than collapsing to zero when coverage is partial.
    ///
    /// Returns `None` only when triage metadata is not yet loaded (cache keys can't
    /// resolve) or there is no actionable pre-triage corpus, so the normal
    /// live-session path applies. This is read-only and never mutates the
    /// [`TriageSession`].
    fn cache_derived_archive_display(&self) -> Option<CacheDerivedArchive> {
        if !self.triage_metadata_ready() {
            return None;
        }
        if !self.can_start_triage_from_pre_triage() {
            return None;
        }
        let included = self.pre_triage().tentative_included_urls();
        if included.is_empty() {
            return None;
        }
        let scored: Vec<(u8, String)> = included
            .into_iter()
            .filter_map(|url| {
                let content_hash = self.pre_triage().article_content_hash(&url)?;
                match self.try_reuse_triage(content_hash) {
                    crate::state::TriageCacheLookupResult::Hit(result) => {
                        Some((result.priority, url))
                    }
                    _ => None,
                }
            })
            .collect();
        Some(CacheDerivedArchive::from_scored(
            scored,
            self.briefing_triage_policy(),
        ))
    }

    /// Compute token estimates for the two archive modes for the given ordered URL list.
    ///
    /// `filtered` is the number of archive-eligible URLs, `raw` is the eligible
    /// count minus summary coverage, and `tokens` use summary output tokens when
    /// available or full article tokens otherwise. The content hash is resolved
    /// from live triage first and pre-triage as the cache-derived fallback.
    ///
    /// **Limitation:** `full_tokens` aggregates `JobState::tokens`; articles whose job
    /// has been pruned, or imported articles without a job, contribute 0 and are likely
    /// underreported. Summary coverage uses the same live-triage-first,
    /// pre-triage-fallback content-hash resolver as cached summary rows.
    pub(crate) fn archive_token_estimates(&self, urls: &[String]) -> ArchiveTokenEstimates {
        use harvester_engine::archive_url_key;

        let url_tokens: std::collections::HashMap<String, u64> = self
            .jobs
            .values()
            .filter_map(|job| {
                job.tokens
                    .map(|tokens| (archive_url_key(&job.url), tokens as u64))
            })
            .collect();

        let mut full_tokens = 0u64;
        let mut summary_tokens = 0u64;
        let mut summary_coverage = 0usize;

        for url in urls {
            let article_tokens = url_tokens.get(&archive_url_key(url)).copied().unwrap_or(0);
            full_tokens = full_tokens.saturating_add(article_tokens);

            let maybe_summary = self
                .content_hash_for_url(url)
                .and_then(|hash| self.summary_cache().lookup_any_by_content_hash(hash));

            if let Some(entry) = maybe_summary {
                summary_tokens = summary_tokens.saturating_add(entry.result.output_tokens as u64);
                summary_coverage += 1;
            } else {
                summary_tokens = summary_tokens.saturating_add(article_tokens);
            }
        }

        ArchiveTokenEstimates {
            full_tokens,
            summary_tokens,
            summary_coverage,
        }
    }

    pub(crate) fn summary_output_tokens_for_url(&self, url: &str) -> Option<u32> {
        let hash = self.content_hash_for_url(url)?;
        self.summary_cache()
            .lookup_any_by_content_hash(hash)
            .map(|entry| entry.result.output_tokens)
    }

    pub(crate) fn content_hash_for_url(&self, url: &str) -> Option<&str> {
        self.triage()
            .article_content_hash(url)
            .or_else(|| self.pre_triage.article_content_hash(url))
    }

    pub fn allocate_next_archive_request_id(&mut self) -> u64 {
        self.archive_request_id = self.archive_request_id.saturating_add(1);
        self.archive_request_id
    }

    pub fn archive_request_id(&self) -> u64 {
        self.archive_request_id
    }

    /// Pin a corpus snapshot for the current archive dialog session.
    ///
    /// Called when the archive dialog is opened so that both the open and submit
    /// handlers operate on the identical corpus the user saw when confirming.
    pub fn pin_archive_corpus(&mut self, corpus: crate::working_corpus::CurrentWorkingCorpus) {
        self.pinned_archive_corpus = Some(corpus);
    }

    /// Returns the corpus pinned at archive-open time, or `None` if no dialog is active.
    pub fn pinned_archive_corpus(&self) -> Option<&crate::working_corpus::CurrentWorkingCorpus> {
        self.pinned_archive_corpus.as_ref()
    }

    /// Clears the pinned corpus after the archive dialog session ends (submit, export
    /// completion, or export failure).
    ///
    /// Note: there is no `ArchiveCancelled` message dispatched when the user cancels the
    /// dialog (the UI returns early without dispatching), so we cannot clear on cancel.
    /// A subsequent `ArchiveClicked` will naturally overwrite the pin.
    pub fn clear_pinned_archive_corpus(&mut self) {
        self.pinned_archive_corpus = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_display::ArchiveCoverage;
    use crate::briefing::LoadedArticle;
    use crate::pre_triage_filter::{PreTriagePolicy, PreTriageSession};
    use crate::triage::TriageSession;
    use crate::ArticleTriageResult;
    use harvester_engine::llm::prompt::PromptId;
    use std::collections::HashMap;

    #[test]
    fn live_archive_display_counts_match_archive_selection() {
        let mut triage = TriageSession::new_loading(None);
        triage.set_articles(vec![LoadedArticle {
            url: "https://batch-display.example/article".to_string(),
            source_title: None,
            prepared_text: "body".to_string(),
            content_hash: "hash".to_string(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        triage.complete_article(
            0,
            ArticleTriageResult {
                category: "tech".to_string(),
                priority: 3,
                tags: vec![],
                rationale: "test".to_string(),
                input_tokens: 0,
                output_tokens: 0,
            },
        );
        triage.complete();

        let mut state = AppState::new();
        state.set_triage(triage);
        let expected = state.archive_corpus();
        let display = state.archive_display_counts();

        assert_eq!(display.coverage(), &ArchiveCoverage::LiveComplete);
        assert_eq!(display.ordered_urls(), expected.ordered_urls());
        assert_eq!(display.filtered_count(), expected.count());
    }

    #[test]
    fn zero_cache_hits_use_live_complete_archive_display_counts() {
        let url = "https://batch-display.example/pre-triage";
        let pre_triage = PreTriageSession::load_articles(
            vec![LoadedArticle {
                url: url.to_string(),
                source_title: None,
                prepared_text: std::iter::repeat_n("contentword", 220)
                    .collect::<Vec<_>>()
                    .join(" "),
                content_hash: "pre-triage-hash".to_string(),
                fetched_utc: None,
            }],
            &PreTriagePolicy::default(),
        );
        assert_eq!(pre_triage.phase(), PreTriagePhase::ReadyToTriage);

        let mut active_versions = HashMap::new();
        active_versions.insert(PromptId::ArticleTriage, 1);
        let mut effective_models = HashMap::new();
        effective_models.insert(PromptId::ArticleTriage, "test-model".to_string());

        let mut state = AppState::new();
        state.set_pre_triage(pre_triage);
        state.set_llm_metadata(active_versions, effective_models, HashMap::new());
        state.set_prompt_contexts(HashMap::new());
        state.mark_triage_metadata_ready();

        let display = state.archive_display_counts();

        assert_eq!(display.coverage(), &ArchiveCoverage::LiveComplete);
        assert_eq!(display.filtered_count(), 0);
    }
}
