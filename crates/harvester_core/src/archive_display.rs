use crate::briefing::TriageSelectionPolicy;
use crate::working_corpus::CurrentWorkingCorpus;

/// Provenance for the archive counts shown by the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveCoverage {
    /// Counts come from a live triage run completed in this session.
    LiveComplete,
    /// Counts come from current-key triage cache hits before live triage ran.
    CacheDerived {
        triaged: usize,
        actionable_total: usize,
    },
}

/// Display-only archive counts and the ordered URLs used for token estimation.
///
/// This is intentionally a distinct type from [`CurrentWorkingCorpus`]. It must
/// not be used by action paths such as archive export or briefing generation.
/// `filtered` is the count of archive-eligible URLs; cache-derived coverage is
/// the separate triage-cache-hit count used for the partial-coverage indicator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveDisplayCounts {
    ordered_urls: Vec<String>,
    coverage: ArchiveCoverage,
}

/// Cache-derived archive data before it is wrapped in the display read-model.
pub(crate) struct CacheDerivedArchive {
    cache_hit_count: usize,
    eligible_urls: Vec<String>,
}

impl CacheDerivedArchive {
    pub(crate) fn from_scored(scored: Vec<(u8, String)>, policy: TriageSelectionPolicy) -> Self {
        let cache_hit_count = scored.len();
        let eligible_urls = policy.rank_eligible(scored);
        Self {
            cache_hit_count,
            eligible_urls,
        }
    }

    pub(crate) fn cache_hit_count(&self) -> usize {
        self.cache_hit_count
    }

    pub(crate) fn into_eligible_urls(self) -> Vec<String> {
        self.eligible_urls
    }
}

impl ArchiveDisplayCounts {
    pub(crate) fn live(corpus: CurrentWorkingCorpus) -> Self {
        Self {
            ordered_urls: corpus.ordered_urls().to_vec(),
            coverage: ArchiveCoverage::LiveComplete,
        }
    }

    pub(crate) fn cache_derived(
        cache_derived: CacheDerivedArchive,
        actionable_total: usize,
    ) -> Self {
        debug_assert!(cache_derived.cache_hit_count() > 0);
        let triaged = cache_derived.cache_hit_count;
        Self {
            ordered_urls: cache_derived.into_eligible_urls(),
            coverage: ArchiveCoverage::CacheDerived {
                triaged,
                actionable_total,
            },
        }
    }

    pub(crate) fn ordered_urls(&self) -> &[String] {
        &self.ordered_urls
    }

    pub(crate) fn filtered_count(&self) -> usize {
        self.ordered_urls.len()
    }

    pub(crate) fn coverage(&self) -> &ArchiveCoverage {
        &self.coverage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_derived_counts_keep_cache_coverage_separate_from_eligibility() {
        let derived = CacheDerivedArchive::from_scored(
            vec![
                (5, "https://high.example".to_string()),
                (0, "https://low.example".to_string()),
            ],
            TriageSelectionPolicy {
                cutoff_exclusive: 0,
                exclude_untriaged: true,
            },
        );
        let display = ArchiveDisplayCounts::cache_derived(derived, 2);

        assert_eq!(display.filtered_count(), 1);
        assert_eq!(
            display.ordered_urls(),
            &["https://high.example".to_string()]
        );
        assert_eq!(
            display.coverage(),
            &ArchiveCoverage::CacheDerived {
                triaged: 2,
                actionable_total: 2,
            }
        );
    }

    #[test]
    fn zero_cache_coverage_is_not_represented_as_cache_derived() {
        let derived = CacheDerivedArchive::from_scored(
            Vec::new(),
            TriageSelectionPolicy {
                cutoff_exclusive: 0,
                exclude_untriaged: true,
            },
        );
        assert_eq!(derived.cache_hit_count(), 0);
        assert!(derived.eligible_urls.is_empty());
    }
}
