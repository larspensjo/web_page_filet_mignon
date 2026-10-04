//! Archive corpus selected from completed triage under its priority policy.

#[cfg(test)]
use crate::{
    briefing::TriageSelectionPolicy,
    triage::{TriagePhase, TriageSession},
};

use crate::pre_triage_filter::stable_hash_u64;

/// Where the current working corpus came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentWorkingCorpusSource {
    /// Triage completed and at least one article met the selection policy priority cutoff.
    TriageComplete,
    /// No corpus is available from any source.
    Unavailable,
}

/// A snapshot of the current working corpus with its origin.
///
/// Created by the archive selector; consumers read URLs and metadata
/// without reconstructing workflow rules at call sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentWorkingCorpus {
    source: CurrentWorkingCorpusSource,
    ordered_urls: Vec<String>,
}

impl CurrentWorkingCorpus {
    pub(crate) fn from_saved_urls(ordered_urls: Vec<String>) -> Self {
        Self {
            source: if ordered_urls.is_empty() {
                CurrentWorkingCorpusSource::Unavailable
            } else {
                CurrentWorkingCorpusSource::TriageComplete
            },
            ordered_urls,
        }
    }
    /// Select the archive corpus: only `TriageComplete` articles.
    ///
    /// Archive exports curated articles only. Pre-triage articles (`ReadyToTriage`,
    /// `Reviewing`) are intentionally excluded — they require triage before archiving.
    #[cfg(test)]
    pub(crate) fn select_for_archive(
        triage: &TriageSession,
        triage_policy: TriageSelectionPolicy,
    ) -> Self {
        if matches!(triage.phase(), TriagePhase::Complete) {
            let urls = triage_policy.eligible_urls(triage);
            if !urls.is_empty() {
                return Self {
                    source: CurrentWorkingCorpusSource::TriageComplete,
                    ordered_urls: urls,
                };
            }
        }
        Self {
            source: CurrentWorkingCorpusSource::Unavailable,
            ordered_urls: Vec::new(),
        }
    }

    /// Number of articles in this corpus.
    pub fn count(&self) -> usize {
        self.ordered_urls.len()
    }

    /// Ordered article URLs.
    pub fn ordered_urls(&self) -> &[String] {
        &self.ordered_urls
    }

    /// Which source produced this corpus.
    pub fn source(&self) -> CurrentWorkingCorpusSource {
        self.source
    }

    #[cfg(test)]
    fn is_ready_for_actions(&self) -> bool {
        self.source == CurrentWorkingCorpusSource::TriageComplete
    }

    /// A stable hash of the ordered URLs. Changes when membership or order changes.
    ///
    /// Uses FNV-1a over the newline-joined URL list, which is deterministic across
    /// Rust versions and program invocations (unlike `DefaultHasher`).
    pub fn fingerprint(&self) -> u64 {
        stable_hash_u64(&self.ordered_urls.join("\n"))
    }

    /// Whether the corpus has no articles.
    pub fn is_empty(&self) -> bool {
        self.ordered_urls.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        briefing::{LoadedArticle, TriageSelectionPolicy},
        triage::{ArticleTriageResult, TriageSession},
    };

    fn init_logging() {
        engine_logging::initialize_for_tests();
    }

    /// Build a default `TriageSelectionPolicy` that includes anything with priority > 0.
    fn default_policy() -> TriageSelectionPolicy {
        TriageSelectionPolicy {
            cutoff_exclusive: 0,
            exclude_untriaged: true,
        }
    }

    /// Construct a `LoadedArticle` with enough body text to pass pre-triage auto-include.
    fn rich_article(url: &str) -> LoadedArticle {
        LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: std::iter::repeat_n("word", 300)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("hash-{url}"),
            fetched_utc: None,
        }
    }

    /// Build a `TriageSession` in `Complete` phase with all articles completed at the given
    /// priority.
    fn complete_triage(urls: &[&str], priority: u8) -> TriageSession {
        let mut session = TriageSession::new_loading(None);
        let articles: Vec<_> = urls.iter().map(|u| rich_article(u)).collect();
        session.set_articles(articles);
        if urls.is_empty() {
            // transition_to_triaging would fail; leave in LoadingArticles → Complete path unused.
            return session;
        }
        session.transition_to_triaging();
        for i in 0..urls.len() {
            session.complete_article(
                i,
                ArticleTriageResult {
                    category: "tech".to_string(),
                    priority,
                    tags: vec![],
                    rationale: "r".to_string(),
                    input_tokens: 0,
                    output_tokens: 0,
                },
            );
        }
        session.complete();
        session
    }

    /// Build an idle (empty) `TriageSession`.
    fn idle_triage() -> TriageSession {
        TriageSession::default()
    }

    // -----------------------------------------------------------------------------------------
    // Test 8: fingerprint changes when corpus membership/order changes
    // -----------------------------------------------------------------------------------------
    #[test]
    fn fingerprint_changes_on_membership_and_order_change() {
        let make = |urls: &[&str]| CurrentWorkingCorpus {
            source: CurrentWorkingCorpusSource::TriageComplete,
            ordered_urls: urls.iter().map(|u| u.to_string()).collect(),
        };
        let ab = make(&["https://a.com", "https://b.com"]);
        assert_ne!(ab.fingerprint(), make(&["https://a.com"]).fingerprint());
        assert_ne!(
            ab.fingerprint(),
            make(&["https://b.com", "https://a.com"]).fingerprint()
        );
        assert_eq!(
            ab.fingerprint(),
            make(&["https://a.com", "https://b.com"]).fingerprint()
        );
    }

    // ── select_for_archive tests ─

    #[test]
    fn triage_complete_but_all_below_cutoff_yields_unavailable() {
        init_logging();
        let triage = complete_triage(&["https://low.com"], 0);
        let corpus = CurrentWorkingCorpus::select_for_archive(&triage, default_policy());
        assert_eq!(corpus.source(), CurrentWorkingCorpusSource::Unavailable);
        assert!(corpus.is_empty());
    }

    // -----------------------------------------------------------------------------------------
    // Test: select_for_archive with TriageComplete and eligible URLs → returns TriageComplete
    // -----------------------------------------------------------------------------------------
    #[test]
    fn select_for_archive_with_complete_triage_and_eligible_urls() {
        init_logging();
        let triage = complete_triage(&["https://a.com", "https://b.com"], 5);
        assert_eq!(triage.phase(), &TriagePhase::Complete);

        let corpus = CurrentWorkingCorpus::select_for_archive(&triage, default_policy());

        assert_eq!(
            corpus.source(),
            CurrentWorkingCorpusSource::TriageComplete,
            "select_for_archive must return TriageComplete when triage is complete with eligible URLs"
        );
        assert_eq!(
            corpus.ordered_urls(),
            &["https://a.com".to_string(), "https://b.com".to_string()]
        );
        assert!(corpus.is_ready_for_actions());
    }

    // -----------------------------------------------------------------------------------------
    // Test: select_for_archive when triage is not Complete → returns Unavailable
    // -----------------------------------------------------------------------------------------
    #[test]
    fn select_for_archive_when_triage_not_complete() {
        init_logging();
        // Test with idle (LoadingArticles) triage
        let triage = idle_triage();
        assert_ne!(triage.phase(), &TriagePhase::Complete);

        let corpus = CurrentWorkingCorpus::select_for_archive(&triage, default_policy());

        assert_eq!(
            corpus.source(),
            CurrentWorkingCorpusSource::Unavailable,
            "select_for_archive must return Unavailable when triage is not Complete"
        );
        assert!(corpus.is_empty());
        assert!(!corpus.is_ready_for_actions());
    }

    // -----------------------------------------------------------------------------------------
    // Test: select_for_archive ignores pre-triage state (pre-triage not passed to function)
    // -----------------------------------------------------------------------------------------
    #[test]
    fn select_for_archive_ignores_pre_triage() {
        init_logging();
        // select_for_archive() does not receive pre-triage and checks only triage.
        // This test verifies that the function signature excludes pre_triage.

        let triage = complete_triage(&["https://archive.com"], 3);
        let corpus = CurrentWorkingCorpus::select_for_archive(&triage, default_policy());

        assert_eq!(
            corpus.source(),
            CurrentWorkingCorpusSource::TriageComplete,
            "archive corpus must use triage articles regardless of pre-triage state"
        );
        assert_eq!(corpus.ordered_urls(), &["https://archive.com".to_string()]);
    }

    // -----------------------------------------------------------------------------------------
    // Test: select_for_archive with Complete triage but all articles below cutoff → Unavailable
    // -----------------------------------------------------------------------------------------
    #[test]
    fn select_for_archive_triage_complete_but_below_cutoff() {
        init_logging();
        let triage = complete_triage(&["https://low.com"], 0); // priority 0, cutoff 0 → not eligible
        assert_eq!(triage.phase(), &TriagePhase::Complete);

        let corpus = CurrentWorkingCorpus::select_for_archive(&triage, default_policy());

        assert_eq!(
            corpus.source(),
            CurrentWorkingCorpusSource::Unavailable,
            "archive corpus must return Unavailable when no articles pass the selection policy"
        );
        assert!(corpus.is_empty());
    }
}
