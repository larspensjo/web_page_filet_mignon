use super::{
    AppState, ArchiveTokenEstimates, BatchObservation, BatchStatus, JobResultKind, TriagePhase,
};
use crate::archive_display::{ArchiveDisplayCounts, CacheDerivedArchive};
#[cfg(test)]
use crate::fixture_support::ManualPreTriageDecisions;
use crate::working_corpus::CurrentWorkingCorpus;
#[cfg(test)]
use crate::PreTriagePhase;
use std::collections::{BTreeMap, HashMap};

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
        let mut pre_triage_included = 0;
        let mut pre_triage_review = 0;
        let mut pre_triage_filtered = 0;
        for entry in self.pre_triage.entries() {
            match (entry.manual_decision, entry.auto_verdict) {
                (Some(crate::ManualDecision::Include), _)
                | (None, crate::pre_triage_filter::AutoVerdict::Include) => {
                    pre_triage_included += 1;
                }
                (Some(crate::ManualDecision::Exclude), _)
                | (None, crate::pre_triage_filter::AutoVerdict::HardExclude) => {
                    pre_triage_filtered += 1;
                }
                (None, crate::pre_triage_filter::AutoVerdict::Review) => {
                    pre_triage_review += 1;
                }
            }
        }
        let summary_total = self.briefing.articles().len();
        let summary_pending = self.briefing.pending_count();
        let summary_in_flight = self.briefing.in_progress_count();
        let summary_completed = self.briefing.completed_summary_count();
        let summary_failed = self.briefing.failed_summary_count();
        let signal_counts = self.signal_candidate.observation_counts();

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
            signal_total: signal_counts.total,
            signal_pending_or_in_flight: signal_counts.pending_or_in_flight,
            signal_completed: signal_counts.completed,
            signal_failed: signal_counts.failed,
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

    pub fn batch_status(&self) -> BatchStatus {
        if self.pipeline_activity().is_settled() {
            BatchStatus::Settled
        } else {
            BatchStatus::Running
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
            if cache_derived.cache_hit_count() > 0 {
                return ArchiveDisplayCounts::cache_derived(cache_derived);
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
        let actionable_total = self.cache_derived_archive_index.urls.len();
        if actionable_total == 0 {
            return None;
        }
        let scored = self
            .cache_derived_archive_index
            .urls
            .iter()
            .zip(&self.cache_derived_archive_index.hash_positions)
            .filter_map(|(url, position)| {
                self.cache_derived_archive_index.priorities[position.as_ref().copied()?]
                    .map(|priority| (priority, url.as_str()))
            });
        Some(CacheDerivedArchive::from_scored(
            scored,
            actionable_total,
            self.briefing_triage_policy(),
        ))
    }

    pub(crate) fn rebuild_cache_derived_archive_index(&mut self) {
        let mut index = CacheDerivedArchiveIndex::default();
        if self.triage_metadata_ready() && self.can_start_triage_from_pre_triage() {
            for url in self.pre_triage().tentative_included_url_refs() {
                let content_hash = self.pre_triage().article_content_hash(url);
                let hash_position = content_hash.map(|hash| {
                    if let Some(&position) = index.index_by_hash.get(hash) {
                        position
                    } else {
                        let position = index.priorities.len();
                        index.index_by_hash.insert(hash.to_string(), position);
                        index.priorities.push(self.cached_triage_priority(hash));
                        position
                    }
                });
                index.hash_positions.push(hash_position);
                index.urls.push(url.to_string());
            }
        }
        self.cache_derived_archive_index = index;
    }

    pub(in crate::state) fn refresh_cache_derived_archive_hash(&mut self, content_hash: &str) {
        let Some(&position) = self
            .cache_derived_archive_index
            .index_by_hash
            .get(content_hash)
        else {
            return;
        };
        self.cache_derived_archive_index.priorities[position] =
            self.cached_triage_priority(content_hash);
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
        if urls.is_empty() {
            return ArchiveTokenEstimates::default();
        }
        let url_tokens = self.archive_article_token_lookup();
        archive_token_estimates_from_parts(urls, url_tokens, |url| {
            self.content_hash_for_url(url)
                .and_then(|hash| self.summary_cache().lookup_any_by_content_hash(hash))
                .map(|entry| entry.result.output_tokens)
        })
    }

    pub(in crate::state) fn archive_article_token_lookup(&self) -> &ArchiveArticleTokenLookup {
        &self.archive_article_tokens
    }

    pub(in crate::state) fn record_archive_job_tokens(&mut self, job_id: super::JobId) {
        if let Some(job) = self.jobs.get(&job_id) {
            self.archive_article_tokens.record(job_id, job);
        }
    }

    pub(in crate::state) fn rebuild_archive_job_tokens(&mut self) {
        self.archive_article_tokens = ArchiveArticleTokenLookup::default();
        for (&job_id, job) in &self.jobs {
            self.archive_article_tokens.record(job_id, job);
        }
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

#[derive(Debug, Clone, PartialEq, Default)]
pub(in crate::state) struct ArchiveArticleTokenLookup {
    tokens_by_key: HashMap<String, BTreeMap<super::JobId, u64>>,
    key_by_exact_url: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(in crate::state) struct CacheDerivedArchiveIndex {
    urls: Vec<String>,
    hash_positions: Vec<Option<usize>>,
    priorities: Vec<Option<u8>>,
    index_by_hash: HashMap<String, usize>,
}

impl ArchiveArticleTokenLookup {
    fn record(&mut self, job_id: super::JobId, job: &super::JobState) {
        let Some(tokens) = job.tokens else { return };
        let key = job.archive_url_key().into_owned();
        self.key_by_exact_url.insert(job.url.clone(), key.clone());
        self.tokens_by_key
            .entry(key)
            .or_default()
            .insert(job_id, tokens as u64);
    }

    pub(in crate::state) fn tokens_for_url(&self, url: &str) -> u64 {
        if self.tokens_by_key.is_empty() {
            return 0;
        }
        let key = self.key_by_exact_url.get(url).map(String::as_str);
        let canonical;
        let key = if let Some(key) = key {
            key
        } else {
            canonical = harvester_engine::archive_url_key(url);
            &canonical
        };
        self.tokens_by_key
            .get(key)
            .and_then(|jobs| jobs.last_key_value().map(|(_, &tokens)| tokens))
            .unwrap_or(0)
    }
}

pub(in crate::state) fn archive_token_estimates_from_parts<F>(
    urls: &[String],
    url_tokens: &ArchiveArticleTokenLookup,
    summary_tokens_for_url: F,
) -> ArchiveTokenEstimates
where
    F: Fn(&str) -> Option<u32>,
{
    let mut full_tokens = 0u64;
    let mut summary_tokens = 0u64;
    let mut summary_coverage = 0usize;

    for url in urls {
        let article_tokens = url_tokens.tokens_for_url(url);
        full_tokens = full_tokens.saturating_add(article_tokens);

        if let Some(tokens) = summary_tokens_for_url(url) {
            summary_tokens = summary_tokens.saturating_add(tokens as u64);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_display::ArchiveCoverage;
    use crate::briefing::LoadedArticle;
    use crate::pre_triage_filter::{PreTriagePolicy, PreTriageSession};
    use crate::triage::TriageSession;
    use crate::{update, ArticleTriageResult, Msg, TriageCache, TriageCacheKey};
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
        state.set_llm_metadata(active_versions, effective_models);
        state.set_prompt_contexts(HashMap::new());
        state.mark_triage_metadata_ready();

        let display = state.archive_display_counts();

        assert_eq!(display.coverage(), &ArchiveCoverage::LiveComplete);
        assert_eq!(display.filtered_count(), 0);
    }

    #[test]
    fn cache_derived_archive_display_tracks_delta_verdict_cache_and_metadata_changes() {
        let url_a = "https://batch-display.example/a";
        let url_b = "https://batch-display.example/b";
        let url_c = "https://batch-display.example/c";
        let rich_text = std::iter::repeat_n("contentword", 220)
            .collect::<Vec<_>>()
            .join(" ");
        let articles = vec![
            LoadedArticle {
                url: url_a.into(),
                source_title: None,
                prepared_text: rich_text.clone(),
                content_hash: "hash-a".into(),
                fetched_utc: None,
            },
            LoadedArticle {
                url: url_b.into(),
                source_title: None,
                prepared_text: rich_text.clone(),
                content_hash: "hash-b-old".into(),
                fetched_utc: None,
            },
            LoadedArticle {
                url: url_c.into(),
                source_title: None,
                prepared_text: rich_text.clone(),
                content_hash: "hash-c".into(),
                fetched_utc: None,
            },
        ];
        let mut state = AppState::new();
        state.set_pre_triage(PreTriageSession::load_articles(
            articles.clone(),
            &PreTriagePolicy::default(),
        ));
        set_test_triage_metadata(&mut state, 1);
        state.set_triage_cache(test_triage_cache(
            1,
            &[("hash-a", 4), ("hash-b-old", 2), ("hash-c", 3)],
        ));
        assert_cache_derived_display_matches_scratch(&mut state);
        assert_eq!(
            state.archive_display_counts().ordered_urls(),
            &[url_a.to_string(), url_c.to_string(), url_b.to_string()]
        );

        let a_key = state
            .pre_triage()
            .entry_for_url(url_a)
            .expect("article A entry")
            .key
            .clone();
        state
            .pre_triage_mut()
            .set_manual_decision(&a_key, crate::ManualDecision::Exclude)
            .expect("included article can be excluded");
        assert_cache_derived_display_matches_scratch(&mut state);
        assert_eq!(
            state.archive_display_counts().ordered_urls(),
            &[url_c.to_string(), url_b.to_string()]
        );

        let delta = harvester_engine::TriageArticleDelta::full_window(
            vec![
                articles[0].clone(),
                LoadedArticle {
                    url: url_b.into(),
                    source_title: None,
                    prepared_text: rich_text.clone(),
                    content_hash: "hash-b-new".into(),
                    fetched_utc: None,
                },
                articles[2].clone(),
            ],
            100_000,
        );
        state
            .pre_triage_mut()
            .merge_delta(delta, &PreTriagePolicy::default());
        assert_cache_derived_display_matches_scratch(&mut state);
        assert_eq!(
            state.archive_display_counts().ordered_urls(),
            &[url_c.to_string()]
        );

        set_test_triage_metadata(&mut state, 2);
        assert_cache_derived_display_matches_scratch(&mut state);
        assert_eq!(state.archive_display_counts().filtered_count(), 0);

        state.set_triage_cache(test_triage_cache(2, &[("hash-b-new", 5), ("hash-c", 3)]));
        assert_cache_derived_display_matches_scratch(&mut state);
        assert_eq!(
            state.archive_display_counts().ordered_urls(),
            &[url_b.to_string(), url_c.to_string()]
        );

        state.set_triage_cache(test_triage_cache(2, &[("hash-b-new", 2), ("hash-c", 3)]));
        assert_cache_derived_display_matches_scratch(&mut state);
        assert_eq!(
            state.archive_display_counts().ordered_urls(),
            &[url_c.to_string(), url_b.to_string()]
        );
    }

    fn set_test_triage_metadata(state: &mut AppState, version: u32) {
        let mut versions = HashMap::new();
        versions.insert(PromptId::ArticleTriage, version);
        let mut models = HashMap::new();
        models.insert(PromptId::ArticleTriage, "test-model".to_string());
        state.set_llm_metadata(versions, models);
        state.set_prompt_contexts(HashMap::new());
        state.mark_triage_metadata_ready();
    }

    fn test_triage_cache(version: u32, hashes_and_priorities: &[(&str, u8)]) -> TriageCache {
        let mut cache = TriageCache::new();
        for (content_hash, priority) in hashes_and_priorities {
            let key = TriageCacheKey::try_new(
                content_hash,
                PromptId::ArticleTriage,
                Some(version),
                Some("test-model"),
                &[],
            )
            .expect("complete triage cache key");
            cache.insert_entry(
                key,
                crate::TriageCacheEntry {
                    result: ArticleTriageResult {
                        category: "news".into(),
                        priority: *priority,
                        tags: Vec::new(),
                        rationale: "fixture".into(),
                        input_tokens: 1,
                        output_tokens: 1,
                    },
                    created_at_utc: "2026-09-25T12:00:00Z".into(),
                },
            );
        }
        cache
    }

    fn assert_cache_derived_display_matches_scratch(state: &mut AppState) {
        state.rebuild_cache_derived_archive_index();
        assert_indexed_cache_display_matches_scratch(state);
    }

    fn assert_indexed_cache_display_matches_scratch(state: &AppState) {
        let display = state.archive_display_counts();
        if matches!(state.triage().phase(), TriagePhase::Complete) {
            let corpus = state.archive_corpus();
            assert_eq!(display.coverage(), &ArchiveCoverage::LiveComplete);
            assert_eq!(display.ordered_urls(), corpus.ordered_urls());
            assert_eq!(display.filtered_count(), corpus.count());
            return;
        }

        if !state.triage_metadata_ready() || !state.can_start_triage_from_pre_triage() {
            let corpus = state.archive_corpus();
            assert_eq!(display.coverage(), &ArchiveCoverage::LiveComplete);
            assert_eq!(display.ordered_urls(), corpus.ordered_urls());
            return;
        }

        let included = state
            .pre_triage()
            .tentative_included_url_refs()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let scored = included
            .iter()
            .filter_map(|url| {
                let content_hash = state.pre_triage().article_content_hash(url)?;
                let key = state.current_triage_cache_key(content_hash)?;
                state
                    .triage_cache()
                    .lookup(&key)
                    .map(|(_, result)| (result.priority, url.clone()))
            })
            .collect::<Vec<_>>();
        if scored.is_empty() {
            let corpus = state.archive_corpus();
            assert_eq!(display.coverage(), &ArchiveCoverage::LiveComplete);
            assert_eq!(display.ordered_urls(), corpus.ordered_urls());
            return;
        }

        let expected_urls = state.briefing_triage_policy().rank_eligible(scored.clone());
        assert_eq!(
            display.coverage(),
            &ArchiveCoverage::CacheDerived {
                triaged: scored.len(),
                actionable_total: included.len(),
            }
        );
        assert_eq!(display.ordered_urls(), expected_urls);
        assert_eq!(display.filtered_count(), expected_urls.len());
    }

    #[test]
    fn cache_derived_index_tracks_triage_cache_writes_without_full_rebuild() {
        let url = "https://batch-display.example/write";
        let text = std::iter::repeat_n("contentword", 220)
            .collect::<Vec<_>>()
            .join(" ");
        let mut state = AppState::new();
        state.set_pre_triage(PreTriageSession::load_articles(
            [url, "https://batch-display.example/write-two"]
                .into_iter()
                .map(|url| LoadedArticle {
                    url: url.into(),
                    source_title: None,
                    prepared_text: text.clone(),
                    content_hash: "write-hash".into(),
                    fetched_utc: None,
                })
                .collect(),
            &PreTriagePolicy::default(),
        ));
        set_test_triage_metadata(&mut state, 1);
        state.rebuild_cache_derived_archive_index();
        assert_eq!(state.cache_derived_archive_index.priorities.len(), 1);
        assert_indexed_cache_display_matches_scratch(&state);

        state.store_triage_result(
            "write-hash",
            ArticleTriageResult {
                category: "news".into(),
                priority: 3,
                tags: Vec::new(),
                rationale: "fixture".into(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
        assert_indexed_cache_display_matches_scratch(&state);
        assert_eq!(state.archive_display_counts().filtered_count(), 2);

        state.store_triage_result(
            "write-hash",
            ArticleTriageResult {
                category: "news".into(),
                priority: 0,
                tags: Vec::new(),
                rationale: "fixture".into(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
        assert_indexed_cache_display_matches_scratch(&state);
        assert!(state.archive_display_counts().ordered_urls().is_empty());
    }

    #[test]
    fn automatic_pre_triage_preserves_batch_article_set_without_manual_overrides() {
        let auto_include_url = "https://batch.example/automatic";
        let hard_exclude_url = "https://batch.example/hard-exclude";
        let review_url = "https://batch.example/review";
        let articles = vec![
            LoadedArticle {
                url: auto_include_url.to_string(),
                source_title: None,
                prepared_text: std::iter::repeat_n("contentword", 220)
                    .collect::<Vec<_>>()
                    .join(" "),
                content_hash: "automatic-hash".to_string(),
                fetched_utc: None,
            },
            LoadedArticle {
                url: hard_exclude_url.to_string(),
                source_title: None,
                prepared_text: "too short".to_string(),
                content_hash: "hard-exclude-hash".to_string(),
                fetched_utc: None,
            },
            LoadedArticle {
                url: review_url.to_string(),
                source_title: None,
                prepared_text: std::iter::repeat_n("contentword", 100)
                    .collect::<Vec<_>>()
                    .join(" "),
                content_hash: "review-hash".to_string(),
                fetched_utc: None,
            },
        ];

        let mut state = AppState::new();
        let request_id = state.alloc_triage_request_id();
        state.set_triage_in_flight(request_id);
        let (mut state, effects) = update(
            state,
            Msg::TriageArticlesLoaded {
                request_id,
                delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
            },
        );
        assert!(effects.is_empty());

        let observation = state.batch_observation();
        assert_eq!(observation.pre_triage_included, 1);
        assert_eq!(observation.pre_triage_review, 1);
        assert_eq!(observation.pre_triage_filtered, 1);

        let mut active_versions = HashMap::new();
        active_versions.insert(PromptId::ArticleTriage, 1);
        let mut effective_models = HashMap::new();
        effective_models.insert(PromptId::ArticleTriage, "test-model".to_string());
        state.set_llm_metadata(active_versions, effective_models);
        state.set_prompt_contexts(HashMap::new());
        state.mark_triage_metadata_ready();

        let (state, _) = crate::update::test_support::update(
            state,
            Msg::PipelineRunRequested {
                scope: crate::PipelineRunScope::Resume,
            },
        );
        assert_eq!(
            state
                .triage()
                .articles()
                .iter()
                .map(|article| article.url.as_str())
                .collect::<Vec<_>>(),
            vec![auto_include_url, review_url]
        );
    }
}
