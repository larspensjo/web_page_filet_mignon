//! Working corpus selector: determines the current set of article URLs to act on.
//!
//! # Source precedence (most important first)
//!
//! 1. `PreTriageReady`     — pre-triage session in `ReadyToTriage` phase with ≥1 included URL.
//! 2. `PreTriageReviewing` — pre-triage session in `Reviewing` phase; informational only, NOT
//!    action-ready. Lets the UI show progress while the user resolves edge cases.
//! 3. `TriageComplete`     — completed triage session with ≥1 eligible URL, used as fallback when
//!    pre-triage is unavailable or loading.
//! 4. `Unavailable`        — no actionable corpus exists.
//!
//! `LoadingArticles` pre-triage state is never treated as ready; it falls through to triage.

use std::hash::{Hash, Hasher};

use crate::{
    briefing::TriageSelectionPolicy,
    pre_triage_filter::{AutoVerdict, ManualDecision, PreTriagePhase, PreTriageSession},
    triage::{TriagePhase, TriageSession},
};

/// Where the current working corpus came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentWorkingCorpusSource {
    /// Pre-triage resolved and ready: all review items settled, ≥1 article passes filters.
    PreTriageReady,
    /// Pre-triage session is in the `Reviewing` phase (unresolved items remain).
    /// The corpus is informational only — callers must not dispatch actions against it.
    PreTriageReviewing,
    /// Triage completed and at least one article met the selection policy priority cutoff.
    TriageComplete,
    /// No corpus is available from any source.
    Unavailable,
}

/// A snapshot of the current working corpus with its origin.
///
/// Created by [`CurrentWorkingCorpus::select`]; consumers read URLs and metadata
/// without reconstructing workflow rules at call sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentWorkingCorpus {
    source: CurrentWorkingCorpusSource,
    ordered_urls: Vec<String>,
}

impl CurrentWorkingCorpus {
    /// Select the best available corpus given the current session state.
    ///
    /// This is a pure function — deterministic, no IO, suitable for use inside the reducer.
    pub(crate) fn select(
        pre_triage: &PreTriageSession,
        triage: &TriageSession,
        triage_policy: TriageSelectionPolicy,
    ) -> Self {
        // Precedence 1: pre-triage ready.
        if matches!(pre_triage.phase(), PreTriagePhase::ReadyToTriage) {
            let urls = pre_triage.resolved_included_urls();
            if !urls.is_empty() {
                return Self {
                    source: CurrentWorkingCorpusSource::PreTriageReady,
                    ordered_urls: urls,
                };
            }
            // ReadyToTriage but resolved to empty — treat as Unavailable rather than silently
            // falling through to triage. This state is unreachable via normal transitions but
            // the guard prevents a mis-classification if state is ever constructed directly.
            return Self {
                source: CurrentWorkingCorpusSource::Unavailable,
                ordered_urls: Vec::new(),
            };
        }

        // Precedence 2: pre-triage reviewing (informational, not action-ready).
        if matches!(pre_triage.phase(), PreTriagePhase::Reviewing) {
            let urls = provisional_included_urls(pre_triage);
            // Even if provisionally empty we still report PreTriageReviewing — the user is
            // actively working through the list; there is no corpus to fall back to while
            // they are mid-review.
            return Self {
                source: CurrentWorkingCorpusSource::PreTriageReviewing,
                ordered_urls: urls,
            };
        }

        // Precedence 3: triage complete (fallback when pre-triage is loading, idle, failed, etc.).
        // If pre-triage is still loading we log a note for traceability.
        if matches!(pre_triage.phase(), PreTriagePhase::LoadingArticles) {
            engine_logging::engine_info!(
                "[working-corpus] pre-triage is LoadingArticles; \
                 falling through to triage corpus"
            );
        }

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

    /// Whether this corpus can be acted on (e.g., start briefing, export).
    ///
    /// `PreTriageReviewing` is explicitly excluded because manual review is still pending.
    pub fn is_ready_for_actions(&self) -> bool {
        matches!(
            self.source,
            CurrentWorkingCorpusSource::PreTriageReady | CurrentWorkingCorpusSource::TriageComplete
        )
    }

    /// A stable hash of the ordered URLs. Changes when membership or order changes.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.ordered_urls.hash(&mut hasher);
        hasher.finish()
    }

    /// Whether the corpus has no articles.
    pub fn is_empty(&self) -> bool {
        self.ordered_urls.is_empty()
    }
}

/// Compute the "would-be included" URLs from a `Reviewing` pre-triage session.
///
/// Mirrors the resolution logic in `PreTriageSession` without requiring access to private
/// internals: manual decisions override auto-verdicts; in the absence of a manual decision,
/// `Include` and `Review` auto-verdicts are treated as tentatively included.
fn provisional_included_urls(pre_triage: &PreTriageSession) -> Vec<String> {
    pre_triage
        .entries()
        .iter()
        .filter_map(|entry| {
            let included = match entry.manual_decision {
                Some(ManualDecision::Include) => true,
                Some(ManualDecision::Exclude) => false,
                None => !matches!(entry.auto_verdict, AutoVerdict::HardExclude),
            };
            if included {
                Some(entry.key.url.clone())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        briefing::{LoadedArticle, TriageSelectionPolicy},
        pre_triage_filter::{ManualDecision, PreTriagePhase, PreTriagePolicy},
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
            prepared_text: std::iter::repeat_n("word", 300).collect::<Vec<_>>().join(" "),
            content_hash: format!("hash-{url}"),
            fetched_utc: None,
        }
    }

    /// Construct a `LoadedArticle` with a body that triggers a `Review` auto-verdict.
    ///
    /// Uses 100 repetitions of an 8-character word so the body is ~900 chars (above the 500-char
    /// hard-exclude threshold) and ~100 words (inside the 60–179 SmallMediumContent review band).
    fn review_article(url: &str) -> LoadedArticle {
        LoadedArticle {
            url: url.to_string(),
            source_title: None,
            // ~100 words × 8 chars + spaces = ~900 chars; word count 100 ∈ [60, 180)
            prepared_text: std::iter::repeat_n("longword", 100)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("hash-{url}"),
            fetched_utc: None,
        }
    }

    /// Build a `PreTriageSession` in `ReadyToTriage` phase from a list of URLs.
    ///
    /// Each URL is given a rich-enough body to auto-include without review.
    fn ready_pre_triage(urls: &[&str]) -> PreTriageSession {
        let policy = PreTriagePolicy::default();
        let articles: Vec<_> = urls.iter().map(|u| rich_article(u)).collect();
        PreTriageSession::load_articles(articles, &policy)
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
    // Test 1: pre-triage ready + stale triage → PreTriageReady
    // -----------------------------------------------------------------------------------------
    #[test]
    fn pre_triage_ready_wins_over_stale_triage() {
        init_logging();
        let pre_triage = ready_pre_triage(&["https://a.com", "https://b.com"]);
        assert_eq!(pre_triage.phase(), &PreTriagePhase::ReadyToTriage);

        let triage = idle_triage(); // "stale": no completed run

        let corpus = CurrentWorkingCorpus::select(&pre_triage, &triage, default_policy());

        assert_eq!(corpus.source(), CurrentWorkingCorpusSource::PreTriageReady);
        assert_eq!(
            corpus.ordered_urls(),
            &["https://a.com".to_string(), "https://b.com".to_string()]
        );
        assert!(corpus.is_ready_for_actions());
    }

    // -----------------------------------------------------------------------------------------
    // Test 2: pre-triage ready + triage empty → PreTriageReady
    // -----------------------------------------------------------------------------------------
    #[test]
    fn pre_triage_ready_wins_over_empty_triage() {
        init_logging();
        let pre_triage = ready_pre_triage(&["https://x.com"]);
        // Triage complete but no articles ⟹ complete() after empty set_articles; stays in
        // LoadingArticles after transition_to_triaging would fail, so build it manually.
        let mut triage = TriageSession::new_loading(None);
        triage.complete(); // complete with no articles
        assert_eq!(triage.phase(), &TriagePhase::Complete);

        let corpus = CurrentWorkingCorpus::select(&pre_triage, &triage, default_policy());

        assert_eq!(corpus.source(), CurrentWorkingCorpusSource::PreTriageReady);
        assert_eq!(corpus.ordered_urls(), &["https://x.com".to_string()]);
        assert!(corpus.is_ready_for_actions());
    }

    // -----------------------------------------------------------------------------------------
    // Test 3: pre-triage reviewing + triage complete → PreTriageReviewing
    // -----------------------------------------------------------------------------------------
    #[test]
    fn pre_triage_reviewing_takes_precedence_over_complete_triage() {
        init_logging();
        // `Reviewing` phase is entered when `set_manual_decision` is called while other
        // Review-verdict entries remain unresolved.  We load two Review-verdict articles
        // (both short/borderline), then resolve one — the other stays unresolved → Reviewing.
        let policy = PreTriagePolicy::default();
        let mut pre_triage = PreTriageSession::load_articles(
            vec![
                review_article("https://review1.com"),
                review_article("https://review2.com"),
            ],
            &policy,
        );
        // After load the session is ReadyToTriage (Review articles are tentatively included).
        // Resolve the first article; the second remains unresolved → Reviewing.
        let key1 = pre_triage.entries()[0].key.clone();
        pre_triage
            .set_manual_decision(&key1, ManualDecision::Include)
            .expect("set_manual_decision must succeed while ReadyToTriage");
        assert_eq!(
            pre_triage.phase(),
            &PreTriagePhase::Reviewing,
            "session must be in Reviewing while the second article is still unresolved"
        );

        let triage = complete_triage(&["https://t.com"], 5);

        let corpus = CurrentWorkingCorpus::select(&pre_triage, &triage, default_policy());

        assert_eq!(
            corpus.source(),
            CurrentWorkingCorpusSource::PreTriageReviewing
        );
        assert!(!corpus.is_ready_for_actions());
        // Both URLs are provisionally included.
        assert!(corpus
            .ordered_urls()
            .contains(&"https://review1.com".to_string()));
        assert!(corpus
            .ordered_urls()
            .contains(&"https://review2.com".to_string()));
    }

    // -----------------------------------------------------------------------------------------
    // Test 4: pre-triage loading + triage complete → TriageComplete (documented fallback)
    // -----------------------------------------------------------------------------------------
    #[test]
    fn pre_triage_loading_falls_through_to_triage_complete() {
        init_logging();
        // LoadingArticles must never be treated as ready; selector falls through to triage.
        let pre_triage = PreTriageSession::new_loading();
        assert_eq!(pre_triage.phase(), &PreTriagePhase::LoadingArticles);

        let triage = complete_triage(&["https://t.com"], 5);

        let corpus = CurrentWorkingCorpus::select(&pre_triage, &triage, default_policy());

        // Explicit assertion: the fallback is deliberate.
        assert_eq!(
            corpus.source(),
            CurrentWorkingCorpusSource::TriageComplete,
            "LoadingArticles pre-triage must fall through to TriageComplete, not be treated as ready"
        );
        assert_eq!(corpus.ordered_urls(), &["https://t.com".to_string()]);
        assert!(corpus.is_ready_for_actions());
    }

    // -----------------------------------------------------------------------------------------
    // Test 5: triage complete, pre-triage unavailable (Idle) → TriageComplete
    // -----------------------------------------------------------------------------------------
    #[test]
    fn triage_complete_used_when_pre_triage_unavailable() {
        init_logging();
        let pre_triage = PreTriageSession::default(); // Idle phase
        let triage = complete_triage(&["https://t1.com", "https://t2.com"], 3);

        let corpus = CurrentWorkingCorpus::select(&pre_triage, &triage, default_policy());

        assert_eq!(
            corpus.source(),
            CurrentWorkingCorpusSource::TriageComplete
        );
        assert!(corpus.is_ready_for_actions());
        assert_eq!(corpus.count(), 2);
    }

    // -----------------------------------------------------------------------------------------
    // Test 6: both unavailable → Unavailable
    // -----------------------------------------------------------------------------------------
    #[test]
    fn both_unavailable_yields_unavailable() {
        init_logging();
        let pre_triage = PreTriageSession::default(); // Idle
        let triage = idle_triage(); // Idle

        let corpus = CurrentWorkingCorpus::select(&pre_triage, &triage, default_policy());

        assert_eq!(corpus.source(), CurrentWorkingCorpusSource::Unavailable);
        assert!(corpus.is_empty());
        assert!(!corpus.is_ready_for_actions());
    }

    // -----------------------------------------------------------------------------------------
    // Test 7: ready pre-triage produces Unavailable when all articles below cutoff (triage side)
    // and separately: Failed pre-triage (all excluded) with idle triage → Unavailable
    // -----------------------------------------------------------------------------------------
    #[test]
    fn failed_pre_triage_and_idle_triage_yields_unavailable() {
        init_logging();
        // Pre-triage where all articles are hard-excluded (YouTube blocked host).
        let policy = PreTriagePolicy::default();
        let pre_triage = PreTriageSession::load_articles(
            vec![LoadedArticle {
                url: "https://youtube.com/watch?v=test".to_string(),
                source_title: None,
                prepared_text: "x".to_string(),
                content_hash: "h".to_string(),
                fetched_utc: None,
            }],
            &policy,
        );
        assert!(
            matches!(pre_triage.phase(), PreTriagePhase::Failed { .. }),
            "all-excluded pre-triage must be Failed, not ReadyToTriage"
        );

        let triage = idle_triage();

        let corpus = CurrentWorkingCorpus::select(&pre_triage, &triage, default_policy());

        assert_eq!(
            corpus.source(),
            CurrentWorkingCorpusSource::Unavailable,
            "Failed pre-triage with idle triage must yield Unavailable"
        );
        assert!(corpus.is_empty());
    }

    #[test]
    fn triage_complete_but_all_below_cutoff_yields_unavailable() {
        init_logging();
        // Cutoff is 0; priority 0 is not > 0 so no eligible URLs.
        let triage = complete_triage(&["https://low.com"], 0);
        let pre_triage = PreTriageSession::default();

        let corpus = CurrentWorkingCorpus::select(&pre_triage, &triage, default_policy());

        assert_eq!(
            corpus.source(),
            CurrentWorkingCorpusSource::Unavailable,
            "triage complete but all articles at or below cutoff must yield Unavailable"
        );
        assert!(corpus.is_empty());
    }

    // -----------------------------------------------------------------------------------------
    // Test 8: fingerprint changes when corpus membership/order changes
    // -----------------------------------------------------------------------------------------
    #[test]
    fn fingerprint_changes_on_membership_and_order_change() {
        init_logging();
        let pre_triage_ab = ready_pre_triage(&["https://a.com", "https://b.com"]);
        let pre_triage_a = ready_pre_triage(&["https://a.com"]);
        let pre_triage_ba = ready_pre_triage(&["https://b.com", "https://a.com"]);
        let triage = idle_triage();
        let policy = default_policy();

        let corpus_ab = CurrentWorkingCorpus::select(&pre_triage_ab, &triage, policy);
        let corpus_a = CurrentWorkingCorpus::select(&pre_triage_a, &triage, policy);
        let corpus_ba = CurrentWorkingCorpus::select(&pre_triage_ba, &triage, policy);

        assert_eq!(corpus_ab.source(), CurrentWorkingCorpusSource::PreTriageReady);
        assert_eq!(corpus_a.source(), CurrentWorkingCorpusSource::PreTriageReady);
        assert_eq!(corpus_ba.source(), CurrentWorkingCorpusSource::PreTriageReady);

        // Different membership → different fingerprint.
        assert_ne!(
            corpus_ab.fingerprint(),
            corpus_a.fingerprint(),
            "removing a URL must change the fingerprint"
        );
        // Different order → different fingerprint.
        assert_ne!(
            corpus_ab.fingerprint(),
            corpus_ba.fingerprint(),
            "reordering URLs must change the fingerprint"
        );
        // Same content → same fingerprint.
        let corpus_ab2 = CurrentWorkingCorpus::select(&pre_triage_ab, &triage, policy);
        assert_eq!(
            corpus_ab.fingerprint(),
            corpus_ab2.fingerprint(),
            "identical corpus must produce the same fingerprint"
        );
    }
}
