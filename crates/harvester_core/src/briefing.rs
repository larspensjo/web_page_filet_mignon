use crate::summary_cache::SummaryCacheKey;
use crate::triage::{ArticleTriageState, TriageSession};
use harvester_engine::llm::SummaryEntities;
use serde::{Deserialize, Serialize};

pub type BriefingArticleId = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BriefingPhase {
    Idle,
    LoadingArticles,
    Summarizing,
    Complete,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArticleSummaryState {
    Pending,
    InProgress { request_id: u64 },
    Completed { result: ArticleSummaryResult },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArticleSummaryResult {
    pub title: String,
    pub summary: String,
    pub key_points: Vec<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Structured entity lists extracted by V4+ summary prompt. Empty for V3 cache hits.
    pub entities: SummaryEntities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefingArticle {
    pub url: String,
    pub source_title: Option<String>,
    pub prepared_text: String,
    pub content_hash: String,
    pub fetched_utc: Option<String>,
    pub summary_state: ArticleSummaryState,
    pub cache_key_snapshot: Option<SummaryCacheKey>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefingSession {
    phase: BriefingPhase,
    articles: Vec<BriefingArticle>,
    article_indices: std::collections::HashMap<(String, String), usize>,
}

pub use harvester_engine::LoadedArticle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriageSelectionPolicy {
    pub cutoff_exclusive: u8,
    pub exclude_untriaged: bool,
}

impl TriageSelectionPolicy {
    pub fn eligible_urls(&self, triage: &TriageSession) -> Vec<String> {
        let scored = triage
            .articles()
            .iter()
            .filter_map(|article| match &article.triage_state {
                ArticleTriageState::Completed { result } => {
                    Some((result.priority, article.url.clone()))
                }
                _ => None,
            });
        self.rank_eligible(scored)
    }

    /// Apply the priority cutoff and deterministic ordering to `(priority, url)`
    /// pairs. Shared by [`eligible_urls`] (live triage session) and the
    /// cache-derived archive corpus so both rank identically.
    ///
    /// Orders by descending priority, then ascending URL for a stable tie-break.
    pub fn rank_eligible<S>(&self, scored: impl IntoIterator<Item = (u8, S)>) -> Vec<String>
    where
        S: AsRef<str> + Into<String>,
    {
        let mut entries: Vec<_> = scored
            .into_iter()
            .filter(|(priority, _)| *priority > self.cutoff_exclusive)
            .collect();
        entries.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.as_ref().cmp(b.1.as_ref())));
        entries.into_iter().map(|(_, url)| url.into()).collect()
    }
}

impl Default for BriefingSession {
    fn default() -> Self {
        Self {
            phase: BriefingPhase::Idle,
            articles: Vec::new(),
            article_indices: Default::default(),
        }
    }
}

impl BriefingSession {
    pub fn new_loading() -> Self {
        Self {
            phase: BriefingPhase::LoadingArticles,
            article_indices: Default::default(),
            articles: Vec::new(),
        }
    }

    pub fn phase(&self) -> &BriefingPhase {
        &self.phase
    }

    pub fn can_start(&self) -> bool {
        matches!(
            self.phase,
            BriefingPhase::Idle | BriefingPhase::Complete | BriefingPhase::Failed { .. }
        )
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.phase,
            BriefingPhase::LoadingArticles | BriefingPhase::Summarizing
        )
    }

    pub fn articles(&self) -> &[BriefingArticle] {
        &self.articles
    }

    pub fn set_articles(&mut self, loaded: Vec<LoadedArticle>) {
        self.articles = loaded
            .into_iter()
            .map(|article| BriefingArticle {
                url: article.url,
                source_title: article.source_title,
                prepared_text: article.prepared_text,
                content_hash: article.content_hash,
                fetched_utc: article.fetched_utc,
                summary_state: ArticleSummaryState::Pending,
                cache_key_snapshot: None,
            })
            .collect();
        self.article_indices = self
            .articles
            .iter()
            .enumerate()
            .map(|(i, a)| ((a.url.clone(), a.content_hash.clone()), i))
            .collect();
    }

    pub fn transition_to_summarizing(&mut self) {
        if self.articles.is_empty() {
            self.phase = BriefingPhase::Failed {
                reason: "no articles to summarize".to_string(),
            };
            return;
        }
        self.phase = BriefingPhase::Summarizing;
    }

    pub fn start_article(&mut self, article_id: BriefingArticleId, request_id: u64) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.summary_state = ArticleSummaryState::InProgress { request_id };
        }
    }

    pub fn complete_article(
        &mut self,
        article_id: BriefingArticleId,
        result: ArticleSummaryResult,
    ) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.summary_state = ArticleSummaryState::Completed { result };
        }
    }

    pub fn fail_article(&mut self, article_id: BriefingArticleId, reason: String) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.summary_state = ArticleSummaryState::Failed { reason };
        }
    }

    pub(crate) fn admit(&mut self, loaded: LoadedArticle, key: Option<SummaryCacheKey>) {
        if let Some(&index) = self
            .article_indices
            .get(&(loaded.url.clone(), loaded.content_hash.clone()))
        {
            let article = &mut self.articles[index];
            article.prepared_text = loaded.prepared_text;
            if matches!(
                article.summary_state,
                ArticleSummaryState::Pending | ArticleSummaryState::InProgress { .. }
            ) || (matches!(article.summary_state, ArticleSummaryState::Completed { .. })
                && article.cache_key_snapshot == key)
            {
                return;
            }
            article.summary_state = ArticleSummaryState::Pending;
            article.cache_key_snapshot = key;
        } else {
            self.article_indices.insert(
                (loaded.url.clone(), loaded.content_hash.clone()),
                self.articles.len(),
            );
            self.articles.push(BriefingArticle {
                url: loaded.url,
                source_title: loaded.source_title,
                prepared_text: loaded.prepared_text,
                content_hash: loaded.content_hash,
                fetched_utc: loaded.fetched_utc,
                summary_state: ArticleSummaryState::Pending,
                cache_key_snapshot: key,
            });
        }
        self.phase = BriefingPhase::Summarizing;
    }

    pub(crate) fn index_for_identity(&self, url: &str, hash: &str) -> Option<usize> {
        self.article_indices
            .get(&(url.to_owned(), hash.to_owned()))
            .copied()
    }

    pub(crate) fn retain_members(&mut self, members: &std::collections::HashSet<(String, String)>) {
        let previous_len = self.articles.len();
        self.articles
            .retain(|a| members.contains(&(a.url.clone(), a.content_hash.clone())));
        self.article_indices = self
            .articles
            .iter()
            .enumerate()
            .map(|(i, a)| ((a.url.clone(), a.content_hash.clone()), i))
            .collect();
        if self.articles.len() != previous_len {
            self.phase = if self.pending_count() + self.in_progress_count() > 0 {
                BriefingPhase::Summarizing
            } else if self.completed_summary_count() > 0 {
                BriefingPhase::Complete
            } else {
                BriefingPhase::Failed {
                    reason: "no successful admitted articles remain".into(),
                }
            };
        }
    }

    pub(crate) fn withdraw_pending(&mut self) -> usize {
        let pending = self.pending_count();
        if pending > 0 {
            let retained = self
                .articles
                .iter()
                .filter(|a| !matches!(a.summary_state, ArticleSummaryState::Pending))
                .map(|a| (a.url.clone(), a.content_hash.clone()))
                .collect();
            self.retain_members(&retained);
        }
        pending
    }

    pub fn total(&self) -> usize {
        self.articles.len()
    }

    pub fn in_progress_count(&self) -> usize {
        self.articles
            .iter()
            .filter(|article| {
                matches!(
                    article.summary_state,
                    ArticleSummaryState::InProgress { .. }
                )
            })
            .count()
    }

    pub fn pending_count(&self) -> usize {
        self.articles
            .iter()
            .filter(|article| matches!(article.summary_state, ArticleSummaryState::Pending))
            .count()
    }

    pub fn can_dispatch_more(&self, limit: usize) -> bool {
        self.in_progress_count() < limit && self.pending_count() > 0
    }

    pub fn completed_summary_count(&self) -> usize {
        self.articles
            .iter()
            .filter(|article| {
                matches!(article.summary_state, ArticleSummaryState::Completed { .. })
            })
            .count()
    }

    pub fn failed_summary_count(&self) -> usize {
        self.articles
            .iter()
            .filter(|article| matches!(article.summary_state, ArticleSummaryState::Failed { .. }))
            .count()
    }

    pub fn next_pending_index(&self) -> Option<BriefingArticleId> {
        self.articles
            .iter()
            .position(|article| matches!(article.summary_state, ArticleSummaryState::Pending))
    }

    pub fn set_article_cache_key(
        &mut self,
        article_id: BriefingArticleId,
        key: Option<SummaryCacheKey>,
    ) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.cache_key_snapshot = key;
        }
    }

    pub fn article_cache_key(&self, article_id: BriefingArticleId) -> Option<&SummaryCacheKey> {
        self.articles
            .get(article_id)
            .and_then(|article| article.cache_key_snapshot.as_ref())
    }

    pub fn find_article_by_request_id(&self, request_id: u64) -> Option<BriefingArticleId> {
        self.articles
            .iter()
            .position(|article| match article.summary_state {
                ArticleSummaryState::InProgress { request_id: id } => id == request_id,
                _ => false,
            })
    }

    pub fn complete_without_briefing(&mut self) {
        self.phase = BriefingPhase::Complete;
    }

    pub fn fail(&mut self, reason: String) {
        self.phase = BriefingPhase::Failed { reason };
    }

    pub fn fail_all_pending(&mut self, reason: &str) {
        for article in self.articles.iter_mut() {
            if matches!(article.summary_state, ArticleSummaryState::Pending) {
                article.summary_state = ArticleSummaryState::Failed {
                    reason: reason.to_string(),
                };
            }
        }
    }

    /// Returns the completed summary result for an article URL, if available.
    pub fn summary_for_url(&self, url: &str) -> Option<&ArticleSummaryResult> {
        self.articles
            .iter()
            .find_map(|article| match &article.summary_state {
                ArticleSummaryState::Completed { result } if article.url == url => Some(result),
                _ => None,
            })
    }

    /// Completed per-article summaries, keyed by their source URL.
    pub fn completed_summaries(&self) -> impl Iterator<Item = (&str, &ArticleSummaryResult)> {
        self.articles
            .iter()
            .filter_map(|article| match &article.summary_state {
                ArticleSummaryState::Completed { result } => Some((article.url.as_str(), result)),
                ArticleSummaryState::Pending
                | ArticleSummaryState::InProgress { .. }
                | ArticleSummaryState::Failed { .. } => None,
            })
    }

    /// Returns true when the briefing session has recorded a terminal failure
    /// for the given article URL.
    pub fn summary_failed_for_url(&self, url: &str) -> bool {
        self.articles.iter().any(|article| {
            article.url == url
                && matches!(article.summary_state, ArticleSummaryState::Failed { .. })
        })
    }

    pub fn progress_text(&self) -> Option<String> {
        let text = match self.phase {
            BriefingPhase::LoadingArticles => "Loading articles...".to_string(),
            BriefingPhase::Summarizing => {
                let completed = self.completed_summary_count() + self.failed_summary_count();
                let total = self.total();
                format!("Summarizing {completed}/{total} articles...")
            }
            BriefingPhase::Failed { ref reason } => format!("Briefing failed: {reason}"),
            _ => return None,
        };
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result() -> ArticleSummaryResult {
        ArticleSummaryResult {
            title: "Title".to_string(),
            summary: "Summary".to_string(),
            key_points: vec!["Point 1".to_string()],
            input_tokens: 10,
            output_tokens: 5,
            entities: Default::default(),
        }
    }

    fn make_session_with_article(url: &str, state: ArticleSummaryState) -> BriefingSession {
        BriefingSession {
            articles: vec![BriefingArticle {
                url: url.to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
                fetched_utc: None,
                summary_state: state,
                cache_key_snapshot: None,
            }],
            ..BriefingSession::default()
        }
    }

    #[test]
    fn summary_for_url_returns_none_when_no_articles() {
        let session = BriefingSession::default();
        assert!(session.summary_for_url("https://example.com").is_none());
    }

    #[test]
    fn summary_for_url_returns_none_when_pending() {
        let session =
            make_session_with_article("https://example.com", ArticleSummaryState::Pending);
        assert!(session.summary_for_url("https://example.com").is_none());
    }

    #[test]
    fn summary_for_url_returns_none_when_failed() {
        let session = make_session_with_article(
            "https://example.com",
            ArticleSummaryState::Failed {
                reason: "err".to_string(),
            },
        );
        assert!(session.summary_for_url("https://example.com").is_none());
    }

    #[test]
    fn summary_for_url_returns_result_when_completed() {
        let result = make_result();
        let session = make_session_with_article(
            "https://example.com",
            ArticleSummaryState::Completed {
                result: result.clone(),
            },
        );
        let found = session.summary_for_url("https://example.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "Title");
    }

    #[test]
    fn summary_for_url_returns_none_for_wrong_url() {
        let result = make_result();
        let session = make_session_with_article(
            "https://example.com",
            ArticleSummaryState::Completed { result },
        );
        assert!(session.summary_for_url("https://other.com").is_none());
    }

    #[test]
    fn briefing_progress_text_shows_failure_reason() {
        let mut session = BriefingSession::new_loading();
        session.fail("request timed out".to_string());

        assert_eq!(
            session.progress_text().as_deref(),
            Some("Briefing failed: request timed out")
        );
    }

    fn make_loaded(url: &str, hash: &str) -> LoadedArticle {
        LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: hash.to_string(),
            fetched_utc: None,
        }
    }

    #[test]
    fn policy_sorts_by_priority_desc_then_url_asc_and_excludes_cutoff() {
        let mut triage = crate::triage::TriageSession::new_loading(None);
        triage.set_articles(vec![
            make_loaded("https://b", "h1"),
            make_loaded("https://a", "h2"),
            make_loaded("https://c", "h3"),
        ]);
        triage.transition_to_triaging();
        triage.complete_article(
            0,
            crate::triage::ArticleTriageResult {
                category: "cat".to_string(),
                priority: 3,
                tags: vec![],
                rationale: "r".to_string(),
                input_tokens: 0,
                output_tokens: 0,
            },
        );
        triage.complete_article(
            1,
            crate::triage::ArticleTriageResult {
                category: "cat".to_string(),
                priority: 3,
                tags: vec![],
                rationale: "r".to_string(),
                input_tokens: 0,
                output_tokens: 0,
            },
        );
        triage.complete_article(
            2,
            crate::triage::ArticleTriageResult {
                category: "cat".to_string(),
                priority: 1,
                tags: vec![],
                rationale: "r".to_string(),
                input_tokens: 0,
                output_tokens: 0,
            },
        );
        let policy = TriageSelectionPolicy {
            cutoff_exclusive: 1,
            exclude_untriaged: true,
        };
        assert_eq!(
            policy.eligible_urls(&triage),
            vec!["https://a".to_string(), "https://b".to_string()]
        );
    }
}
