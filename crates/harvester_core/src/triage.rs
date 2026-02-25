use crate::briefing::LoadedArticle;

pub type TriageArticleId = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriagePhase {
    Idle,
    LoadingArticles,
    Triaging,
    Complete,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleTriageState {
    Pending,
    InProgress { request_id: u64 },
    Completed { result: ArticleTriageResult },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub triage_state: ArticleTriageState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriageSession {
    phase: TriagePhase,
    articles: Vec<TriageArticle>,
    started_at: Option<String>,
}

impl Default for TriageSession {
    fn default() -> Self {
        Self {
            phase: TriagePhase::Idle,
            articles: Vec::new(),
            started_at: None,
        }
    }
}

impl TriageSession {
    pub fn new_loading(started_at: Option<String>) -> Self {
        Self {
            phase: TriagePhase::LoadingArticles,
            articles: Vec::new(),
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
                triage_state: ArticleTriageState::Pending,
            })
            .collect();
    }

    pub fn reset_with_articles(&mut self, loaded: Vec<LoadedArticle>) {
        self.articles.clear();
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

    pub fn complete_article(&mut self, article_id: TriageArticleId, result: ArticleTriageResult) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.triage_state = ArticleTriageState::Completed { result };
        }
    }

    pub fn fail_article(&mut self, article_id: TriageArticleId, reason: String) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.triage_state = ArticleTriageState::Failed { reason };
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
        self.articles
            .iter()
            .find_map(|article| match &article.triage_state {
                ArticleTriageState::Completed { result } if article.url == url => Some(result),
                _ => None,
            })
    }

    pub fn article_content_hash(&self, url: &str) -> Option<&str> {
        self.articles
            .iter()
            .find(|article| article.url == url)
            .map(|article| article.content_hash.as_str())
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
        results.sort_by(|a, b| b.priority.cmp(&a.priority));
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
}
