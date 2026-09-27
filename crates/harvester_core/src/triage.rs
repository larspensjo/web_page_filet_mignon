use crate::briefing::LoadedArticle;
use crate::triage_cache::TriageCacheKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type TriageArticleId = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriagePhase {
    Idle,
    LoadingArticles,
    Triaging,
    AwaitingBatch,
    Complete,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleTriageState {
    Pending,
    InProgress { request_id: u64 },
    Deferred,
    Completed { result: ArticleTriageResult },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArticleTriageResult {
    pub category: String,
    pub priority: u8,
    pub tags: Vec<String>,
    pub rationale: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriageArticle {
    pub url: String,
    pub source_title: Option<String>,
    pub prepared_text: String,
    pub content_hash: String,
    pub fetched_utc: Option<String>,
    /// Provenance belongs to the selected session result, not the serialized result.
    pub triage_model: Option<String>,
    pub preparation_budget: Option<usize>,
    /// Key used by the current admitted triage attempt, retained so unfinished
    /// work can distinguish current-key requests from stale requests.
    pub cache_key_snapshot: Option<TriageCacheKey>,
    pub triage_state: ArticleTriageState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriageSession {
    phase: TriagePhase,
    articles: Vec<TriageArticle>,
    article_indices_by_url: HashMap<String, Vec<usize>>,
    started_at: Option<String>,
}

impl Default for TriageSession {
    fn default() -> Self {
        Self {
            phase: TriagePhase::Idle,
            articles: Vec::new(),
            article_indices_by_url: HashMap::new(),
            started_at: None,
        }
    }
}

impl TriageSession {
    pub(crate) fn index_for_identity(&self, url: &str, hash: &str) -> Option<usize> {
        self.article_indices_by_url
            .get(url)?
            .iter()
            .copied()
            .find(|&i| self.articles[i].content_hash == hash)
    }
    pub(crate) fn admit(
        &mut self,
        loaded: LoadedArticle,
        key: Option<TriageCacheKey>,
        budget: Option<usize>,
    ) {
        if let Some(index) = self.index_for_identity(&loaded.url, &loaded.content_hash) {
            let article = &mut self.articles[index];
            article.prepared_text = loaded.prepared_text;
            article.preparation_budget = budget;
            if matches!(
                article.triage_state,
                ArticleTriageState::Pending
                    | ArticleTriageState::InProgress { .. }
                    | ArticleTriageState::Deferred
            ) || (matches!(article.triage_state, ArticleTriageState::Completed { .. })
                && article.cache_key_snapshot == key)
            {
                return;
            }
            article.triage_state = ArticleTriageState::Pending;
            article.triage_model = None;
            article.cache_key_snapshot = key;
        } else {
            let index = self.articles.len();
            self.article_indices_by_url
                .entry(loaded.url.clone())
                .or_default()
                .push(index);
            self.articles.push(TriageArticle {
                url: loaded.url,
                source_title: loaded.source_title,
                prepared_text: loaded.prepared_text,
                content_hash: loaded.content_hash,
                fetched_utc: loaded.fetched_utc,
                triage_model: None,
                preparation_budget: budget,
                cache_key_snapshot: key,
                triage_state: ArticleTriageState::Pending,
            });
        }
        self.phase = TriagePhase::Triaging;
    }

    pub(crate) fn retain_members(&mut self, members: &std::collections::HashSet<(String, String)>) {
        let previous_len = self.articles.len();
        self.articles
            .retain(|a| members.contains(&(a.url.clone(), a.content_hash.clone())));
        self.article_indices_by_url.clear();
        for (index, a) in self.articles.iter().enumerate() {
            self.article_indices_by_url
                .entry(a.url.clone())
                .or_default()
                .push(index);
        }
        if self.articles.len() != previous_len {
            self.phase = if self.pending_count() + self.in_progress_count() > 0 {
                TriagePhase::Triaging
            } else if self.deferred_count() > 0 {
                TriagePhase::AwaitingBatch
            } else if self.completed_count() > 0 {
                TriagePhase::Complete
            } else {
                TriagePhase::Failed {
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
                .filter(|a| !matches!(a.triage_state, ArticleTriageState::Pending))
                .map(|a| (a.url.clone(), a.content_hash.clone()))
                .collect();
            self.retain_members(&retained);
        }
        pending
    }
    pub fn new_loading(started_at: Option<String>) -> Self {
        Self {
            phase: TriagePhase::LoadingArticles,
            articles: Vec::new(),
            article_indices_by_url: HashMap::new(),
            started_at,
        }
    }

    pub fn phase(&self) -> &TriagePhase {
        &self.phase
    }

    pub fn can_start(&self) -> bool {
        matches!(
            self.phase,
            TriagePhase::Idle | TriagePhase::Complete | TriagePhase::Failed { .. }
        )
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.phase,
            TriagePhase::LoadingArticles | TriagePhase::Triaging
        )
    }

    pub fn articles(&self) -> &[TriageArticle] {
        &self.articles
    }

    pub fn set_articles(&mut self, loaded: Vec<LoadedArticle>) {
        self.articles = loaded
            .into_iter()
            .map(|article| TriageArticle {
                url: article.url,
                source_title: article.source_title,
                prepared_text: article.prepared_text,
                content_hash: article.content_hash,
                fetched_utc: article.fetched_utc,
                triage_model: None,
                preparation_budget: None,
                cache_key_snapshot: None,
                triage_state: ArticleTriageState::Pending,
            })
            .collect();
        self.article_indices_by_url.clear();
        for (index, article) in self.articles.iter().enumerate() {
            self.article_indices_by_url
                .entry(article.url.clone())
                .or_default()
                .push(index);
        }
    }

    pub(crate) fn refresh_preparation(&mut self, articles: &[LoadedArticle], budget: usize) {
        let prepared: HashMap<_, _> = articles
            .iter()
            .filter(|a| a.prepared_text.len() <= budget)
            .map(|a| ((a.url.as_str(), a.content_hash.as_str()), a))
            .collect();
        for article in &mut self.articles {
            if let Some(prepared) =
                prepared.get(&(article.url.as_str(), article.content_hash.as_str()))
            {
                article.prepared_text = prepared.prepared_text.clone();
                article.preparation_budget = Some(budget);
            }
        }
    }

    pub fn reset_with_articles(&mut self, loaded: Vec<LoadedArticle>) {
        self.set_articles(loaded);
        self.started_at = None;
        self.phase = TriagePhase::LoadingArticles;
    }

    pub fn transition_to_triaging(&mut self) {
        if self.articles.is_empty() {
            self.phase = TriagePhase::Failed {
                reason: "no articles to triage".to_string(),
            };
            return;
        }
        self.phase = TriagePhase::Triaging;
    }

    pub fn start_article(&mut self, article_id: TriageArticleId, request_id: u64) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.triage_state = ArticleTriageState::InProgress { request_id };
        }
    }

    pub(crate) fn set_article_cache_key(
        &mut self,
        article_id: TriageArticleId,
        key: Option<TriageCacheKey>,
    ) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.cache_key_snapshot = key;
        }
    }

    pub fn complete_article(&mut self, article_id: TriageArticleId, result: ArticleTriageResult) {
        self.complete_article_with_model(article_id, result, None);
    }

    pub fn complete_article_with_model(
        &mut self,
        article_id: TriageArticleId,
        result: ArticleTriageResult,
        triage_model: Option<String>,
    ) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.triage_state = ArticleTriageState::Completed { result };
            article.triage_model = triage_model;
        }
    }

    pub fn fail_article(&mut self, article_id: TriageArticleId, reason: String) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.triage_state = ArticleTriageState::Failed { reason };
        }
    }

    pub fn defer_article(&mut self, article_id: TriageArticleId) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.triage_state = ArticleTriageState::Deferred;
        }
    }

    pub fn deferred_count(&self) -> usize {
        self.articles
            .iter()
            .filter(|article| matches!(article.triage_state, ArticleTriageState::Deferred))
            .count()
    }

    pub fn rearm_deferred(&mut self) {
        for article in &mut self.articles {
            if matches!(article.triage_state, ArticleTriageState::Deferred) {
                article.triage_state = ArticleTriageState::Pending;
            }
        }
        if matches!(self.phase, TriagePhase::AwaitingBatch) {
            self.phase = TriagePhase::Triaging;
        }
    }

    pub fn total(&self) -> usize {
        self.articles.len()
    }

    pub fn in_progress_count(&self) -> usize {
        self.articles
            .iter()
            .filter(|article| matches!(article.triage_state, ArticleTriageState::InProgress { .. }))
            .count()
    }

    pub fn pending_count(&self) -> usize {
        self.articles
            .iter()
            .filter(|article| matches!(article.triage_state, ArticleTriageState::Pending))
            .count()
    }

    pub fn can_dispatch_more(&self, limit: usize) -> bool {
        self.in_progress_count() < limit && self.pending_count() > 0
    }

    pub fn completed_count(&self) -> usize {
        self.articles
            .iter()
            .filter(|article| matches!(article.triage_state, ArticleTriageState::Completed { .. }))
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.articles
            .iter()
            .filter(|article| matches!(article.triage_state, ArticleTriageState::Failed { .. }))
            .count()
    }

    pub fn next_pending_index(&self) -> Option<TriageArticleId> {
        self.articles
            .iter()
            .position(|article| matches!(article.triage_state, ArticleTriageState::Pending))
    }

    pub fn find_article_by_request_id(&self, request_id: u64) -> Option<TriageArticleId> {
        self.articles
            .iter()
            .position(|article| match &article.triage_state {
                ArticleTriageState::InProgress { request_id: id } => *id == request_id,
                _ => false,
            })
    }

    pub fn fail_all_pending(&mut self, reason: &str) {
        for article in self.articles.iter_mut() {
            if matches!(article.triage_state, ArticleTriageState::Pending) {
                article.triage_state = ArticleTriageState::Failed {
                    reason: reason.to_string(),
                };
            }
        }
    }

    pub fn fail(&mut self, reason: String) {
        self.phase = TriagePhase::Failed { reason };
    }

    /// Returns tuple of (total, pending, in_flight, completed, failed) counts.
    /// Used for batch observation without exposing internal structure.
    pub fn observation_counts(&self) -> (usize, usize, usize, usize, usize) {
        (
            self.total(),
            self.pending_count(),
            self.in_progress_count(),
            self.completed_count(),
            self.failed_count(),
        )
    }

    pub fn complete(&mut self) {
        self.phase = TriagePhase::Complete;
    }

    pub fn set_awaiting_batch(&mut self) {
        self.phase = TriagePhase::AwaitingBatch;
    }

    pub fn progress_text(&self) -> Option<String> {
        let text = match self.phase {
            TriagePhase::LoadingArticles => "Loading articles...".to_string(),
            TriagePhase::Triaging => {
                let completed = self.completed_count() + self.failed_count();
                let total = self.total();
                format!("Triaging {completed}/{total} articles...")
            }
            _ => return None,
        };
        Some(text)
    }

    pub fn result_for_url(&self, url: &str) -> Option<&ArticleTriageResult> {
        self.article_indices_by_url
            .get(url)?
            .iter()
            .find_map(|&index| match &self.articles[index].triage_state {
                ArticleTriageState::Completed { result } => Some(result),
                _ => None,
            })
    }

    pub fn iter_completed_priorities(&self) -> impl Iterator<Item = (&str, u8)> {
        self.articles
            .iter()
            .filter_map(|article| match &article.triage_state {
                ArticleTriageState::Completed { result } => {
                    Some((article.url.as_str(), result.priority))
                }
                _ => None,
            })
    }

    pub fn triage_model_for_url(&self, url: &str) -> Option<&str> {
        self.article_indices_by_url
            .get(url)?
            .iter()
            .map(|&index| &self.articles[index])
            .find(|article| matches!(article.triage_state, ArticleTriageState::Completed { .. }))
            .and_then(|article| article.triage_model.as_deref())
    }

    pub fn source_title_for_url(&self, url: &str) -> Option<&str> {
        self.article_for_url(url)
            .and_then(|article| article.source_title.as_deref())
            .map(str::trim)
            .filter(|title| !title.is_empty())
    }

    pub fn fetched_utc_for_url(&self, url: &str) -> Option<&str> {
        self.article_for_url(url)
            .and_then(|article| article.fetched_utc.as_deref())
    }

    pub fn article_content_hash(&self, url: &str) -> Option<&str> {
        self.article_for_url(url)
            .map(|article| article.content_hash.as_str())
    }

    fn article_for_url(&self, url: &str) -> Option<&TriageArticle> {
        self.article_indices_by_url
            .get(url)
            .and_then(|indices| indices.first())
            .map(|&index| &self.articles[index])
    }

    pub fn sorted_results(&self) -> Vec<&ArticleTriageResult> {
        let mut results: Vec<_> = self
            .articles
            .iter()
            .filter_map(|article| match &article.triage_state {
                ArticleTriageState::Completed { result } => Some(result),
                _ => None,
            })
            .collect();
        results.sort_by_key(|result| std::cmp::Reverse(result.priority));
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn loaded_article_with_url(url: &str) -> LoadedArticle {
        LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: String::new(),
            content_hash: String::new(),
            fetched_utc: None,
        }
    }

    fn result_for_priority(priority: u8) -> ArticleTriageResult {
        ArticleTriageResult {
            category: "security".to_string(),
            priority,
            tags: vec!["tag".to_string()],
            rationale: "reason".to_string(),
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    #[test]
    fn default_is_idle_and_can_start() {
        let session = TriageSession::default();
        assert!(matches!(session.phase(), TriagePhase::Idle));
        assert!(session.can_start());
    }

    #[test]
    fn transition_to_triaging_without_articles_fails() {
        let mut session = TriageSession::new_loading(None);
        session.transition_to_triaging();
        assert!(matches!(session.phase(), TriagePhase::Failed { .. }));
    }

    #[test]
    fn start_article_sets_in_progress() {
        let mut session = TriageSession::new_loading(None);
        session.set_articles(vec![loaded_article_with_url("https://example.com")]);
        session.transition_to_triaging();
        session.start_article(0, 123);
        assert!(matches!(session.phase(), TriagePhase::Triaging));
        assert_eq!(session.in_progress_count(), 1);
        assert_eq!(session.pending_count(), 0);
        assert_eq!(session.total(), 1);
    }

    #[test]
    fn sorted_results_order_by_priority_desc() {
        let mut session = TriageSession::new_loading(None);
        session.set_articles(vec![
            loaded_article_with_url("a"),
            loaded_article_with_url("b"),
        ]);
        session.transition_to_triaging();
        session.complete_article(0, result_for_priority(3));
        session.complete_article(1, result_for_priority(5));
        let sorted = session.sorted_results();
        assert_eq!(sorted[0].priority, 5);
        assert_eq!(sorted[1].priority, 3);
    }

    #[test]
    fn fetched_utc_for_url_returns_timestamp_when_present() {
        let mut session = TriageSession::new_loading(None);
        session.set_articles(vec![LoadedArticle {
            url: "https://example.com/a".to_string(),
            source_title: None,
            prepared_text: String::new(),
            content_hash: "h".to_string(),
            fetched_utc: Some("2026-06-10T00:00:00Z".to_string()),
        }]);
        assert_eq!(
            session.fetched_utc_for_url("https://example.com/a"),
            Some("2026-06-10T00:00:00Z")
        );
        assert_eq!(session.fetched_utc_for_url("https://nope"), None);
    }

    #[test]
    fn url_index_matches_article_scan_after_set_completion_and_replacement() {
        let mut session = TriageSession::new_loading(None);
        let first = "https://example.com/first";
        let second = "https://example.com/second";
        for urls in [vec![first, second, first], vec![second, first], vec![first]] {
            session.set_articles(
                urls.iter()
                    .enumerate()
                    .map(|(index, url)| LoadedArticle {
                        url: (*url).into(),
                        source_title: Some(format!(" Title {index} ")),
                        prepared_text: String::new(),
                        content_hash: format!("hash-{index}"),
                        fetched_utc: Some(format!("time-{index}")),
                    })
                    .collect(),
            );
            for index in (0..session.total()).rev() {
                session.complete_article(index, result_for_priority(index as u8));
            }
            for url in [first, second, "https://example.com/missing"] {
                let first_match = session.articles.iter().find(|article| article.url == url);
                assert_eq!(
                    session.article_content_hash(url),
                    first_match.map(|article| article.content_hash.as_str())
                );
                assert_eq!(
                    session.source_title_for_url(url),
                    first_match
                        .and_then(|article| article.source_title.as_deref())
                        .map(str::trim)
                );
                assert_eq!(
                    session.fetched_utc_for_url(url),
                    first_match.and_then(|article| article.fetched_utc.as_deref())
                );
                let completed =
                    session
                        .articles
                        .iter()
                        .find_map(|article| match &article.triage_state {
                            ArticleTriageState::Completed { result } if article.url == url => {
                                Some(result)
                            }
                            _ => None,
                        });
                assert_eq!(session.result_for_url(url), completed);
            }
        }
        session.reset_with_articles(vec![loaded_article_with_url(second)]);
        assert_eq!(session.article_content_hash(first), None);
        assert_eq!(session.article_content_hash(second), Some(""));
    }
}
