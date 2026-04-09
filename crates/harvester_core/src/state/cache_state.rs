use super::{AppState, TriageCacheLookupResult};
use crate::context_hash;
use crate::summary_cache::SummaryCache;
use crate::triage::ArticleTriageResult;
use crate::triage_cache::{TriageCache, TriageCacheKey};
use harvester_engine::llm::prompt::{PromptId, PromptVersion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MetadataLoadState {
    Idle,
    Pending,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SummaryCacheMetadataSnapshot {
    pub(super) prompt_version: PromptVersion,
    pub(super) model_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TriageCacheMetadataSnapshot {
    pub(super) prompt_version: PromptVersion,
    pub(super) model_id: String,
    pub(super) context_hash: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SummaryCacheMetrics {
    hits: usize,
    misses: usize,
    key_unavailable: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TriageCacheRunMetrics {
    hits: u32,
    misses: u32,
    key_unavailable: u32,
}

impl TriageCacheRunMetrics {
    pub(crate) fn hits(&self) -> u32 {
        self.hits
    }

    pub(crate) fn misses(&self) -> u32 {
        self.misses
    }

    pub(crate) fn key_unavailable(&self) -> u32 {
        self.key_unavailable
    }

    pub(crate) fn total(&self) -> u32 {
        self.hits + self.misses + self.key_unavailable
    }
}

impl SummaryCacheMetrics {
    pub(crate) fn hits(&self) -> usize {
        self.hits
    }

    pub(crate) fn misses(&self) -> usize {
        self.misses
    }

    pub(crate) fn key_unavailable(&self) -> usize {
        self.key_unavailable
    }

    pub(crate) fn total(&self) -> usize {
        self.hits + self.misses + self.key_unavailable
    }
}

impl AppState {
    /// Try to reuse a cached summary result for the given cache key.
    /// Returns None if there is no cached entry for this key.
    pub(crate) fn try_reuse_summary(
        &self,
        key: &crate::summary_cache::SummaryCacheKey,
    ) -> Option<&crate::briefing::ArticleSummaryResult> {
        self.summary_cache.lookup(key).map(|entry| &entry.result)
    }

    /// Store a summary result in the cache with the given key.
    pub(crate) fn store_summary_result(
        &mut self,
        key: crate::summary_cache::SummaryCacheKey,
        result: crate::briefing::ArticleSummaryResult,
        created_at_utc: String,
    ) {
        let entry = crate::summary_cache::SummaryCacheEntry {
            result,
            created_at_utc,
        };
        self.summary_cache.insert(key, entry);
    }

    /// Replace the entire summary cache (used for hydration).
    pub(crate) fn set_summary_cache(&mut self, cache: SummaryCache) {
        self.summary_cache = cache;
    }

    pub(crate) fn start_summary_cache_run(&mut self) {
        self.summary_cache_metrics = SummaryCacheMetrics::default();
        self.summary_cache_metadata_snapshot = None;
        self.summary_cache_warmup_logged = false;
        self.briefing_metadata_state = MetadataLoadState::Pending;
    }

    pub(crate) fn mark_briefing_metadata_ready(&mut self) {
        if self.briefing_metadata_state != MetadataLoadState::Pending {
            return;
        }
        let snapshot = match (
            self.active_prompt_versions
                .get(&PromptId::ArticleSummary)
                .copied(),
            self.effective_models
                .get(&PromptId::ArticleSummary)
                .cloned(),
        ) {
            (Some(version), Some(model_id)) => Some(SummaryCacheMetadataSnapshot {
                prompt_version: version,
                model_id,
            }),
            _ => None,
        };
        self.summary_cache_metadata_snapshot = snapshot;
        self.briefing_metadata_state = MetadataLoadState::Ready;
    }

    pub(crate) fn is_briefing_metadata_ready(&self) -> bool {
        matches!(self.briefing_metadata_state, MetadataLoadState::Ready)
    }

    pub(crate) fn summary_cache_metadata(&self) -> Option<(PromptVersion, &str)> {
        self.summary_cache_metadata_snapshot
            .as_ref()
            .map(|snapshot| (snapshot.prompt_version, snapshot.model_id.as_str()))
    }

    pub(crate) fn summary_cache_warmup_logged(&self) -> bool {
        self.summary_cache_warmup_logged
    }

    pub(crate) fn mark_summary_cache_warmup_logged(&mut self) {
        self.summary_cache_warmup_logged = true;
    }

    pub(crate) fn record_summary_cache_hit(&mut self) {
        self.summary_cache_metrics.hits += 1;
    }

    pub(crate) fn record_summary_cache_miss(&mut self) {
        self.summary_cache_metrics.misses += 1;
    }

    pub(crate) fn record_summary_cache_key_unavailable(&mut self) {
        self.summary_cache_metrics.key_unavailable += 1;
    }

    pub(crate) fn summary_cache_metrics(&self) -> SummaryCacheMetrics {
        self.summary_cache_metrics
    }

    pub(crate) fn finalize_summary_cache_run(&mut self) {
        self.briefing_metadata_state = MetadataLoadState::Idle;
        self.summary_cache_metadata_snapshot = None;
        self.summary_cache_warmup_logged = false;
    }

    /// Get an immutable reference to the summary cache.
    pub(crate) fn summary_cache(&self) -> &SummaryCache {
        &self.summary_cache
    }

    pub(crate) fn set_triage_cache(&mut self, cache: TriageCache) {
        self.triage_cache = cache;
    }

    pub fn triage_cache(&self) -> &TriageCache {
        &self.triage_cache
    }

    pub(crate) fn start_triage_cache_run(&mut self) {
        self.triage_cache_run_metrics = TriageCacheRunMetrics::default();
        self.triage_cache_run_start_logged = false;
    }

    pub(crate) fn mark_triage_metadata_pending(&mut self) {
        self.triage_metadata_state = MetadataLoadState::Pending;
    }

    pub(crate) fn mark_triage_metadata_ready(&mut self) {
        let snapshot = match (
            self.active_prompt_versions
                .get(&PromptId::ArticleTriage)
                .copied(),
            self.effective_models.get(&PromptId::ArticleTriage).cloned(),
        ) {
            (Some(prompt_version), Some(model_id)) => Some(TriageCacheMetadataSnapshot {
                prompt_version,
                model_id,
                context_hash: context_hash(self.context_for(PromptId::ArticleTriage)),
            }),
            _ => {
                self.triage_metadata_state = MetadataLoadState::Pending;
                None
            }
        };
        if snapshot.is_some() {
            self.triage_metadata_state = MetadataLoadState::Ready;
        }
        self.triage_cache_metadata_snapshot = snapshot;
    }

    pub(crate) fn triage_metadata_ready(&self) -> bool {
        matches!(self.triage_metadata_state, MetadataLoadState::Ready)
    }

    pub(crate) fn triage_cache_metadata(&self) -> Option<(PromptVersion, &str, &str)> {
        self.triage_cache_metadata_snapshot
            .as_ref()
            .map(|snapshot| {
                (
                    snapshot.prompt_version,
                    snapshot.model_id.as_str(),
                    snapshot.context_hash.as_str(),
                )
            })
    }

    pub(crate) fn try_reuse_triage(&self, content_hash: &str) -> TriageCacheLookupResult<'_> {
        let snapshot = match &self.triage_cache_metadata_snapshot {
            Some(snapshot) => snapshot,
            None => return TriageCacheLookupResult::KeyUnavailable,
        };
        let key = match TriageCacheKey::try_new_with_context_hash(
            content_hash,
            PromptId::ArticleTriage,
            Some(snapshot.prompt_version),
            Some(snapshot.model_id.as_str()),
            &snapshot.context_hash,
        ) {
            Ok(key) => key,
            Err(_) => return TriageCacheLookupResult::KeyUnavailable,
        };
        match self.triage_cache.lookup(&key) {
            Some(result) => TriageCacheLookupResult::Hit(result),
            None => TriageCacheLookupResult::Miss,
        }
    }

    pub(crate) fn store_triage_result(&mut self, content_hash: &str, result: ArticleTriageResult) {
        let snapshot = match &self.triage_cache_metadata_snapshot {
            Some(snapshot) => snapshot,
            None => return,
        };
        let key = match TriageCacheKey::try_new_with_context_hash(
            content_hash,
            PromptId::ArticleTriage,
            Some(snapshot.prompt_version),
            Some(snapshot.model_id.as_str()),
            &snapshot.context_hash,
        ) {
            Ok(key) => key,
            Err(_) => return,
        };

        self.triage_cache.insert(key, result);
    }

    pub(crate) fn record_triage_cache_hit(&mut self) {
        self.triage_cache_run_metrics.hits = self.triage_cache_run_metrics.hits.saturating_add(1);
    }

    pub(crate) fn record_triage_cache_miss(&mut self) {
        self.triage_cache_run_metrics.misses =
            self.triage_cache_run_metrics.misses.saturating_add(1);
    }

    pub(crate) fn record_triage_cache_key_unavailable(&mut self) {
        self.triage_cache_run_metrics.key_unavailable = self
            .triage_cache_run_metrics
            .key_unavailable
            .saturating_add(1);
    }

    pub(crate) fn triage_cache_metrics(&self) -> &TriageCacheRunMetrics {
        &self.triage_cache_run_metrics
    }

    pub(crate) fn triage_cache_run_start_logged(&self) -> bool {
        self.triage_cache_run_start_logged
    }

    pub(crate) fn mark_triage_cache_run_started(&mut self) {
        self.triage_cache_run_start_logged = true;
    }

    pub(crate) fn finalize_triage_cache_run(&mut self) {
        self.triage_cache_run_start_logged = false;
    }
}
