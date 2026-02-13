use crate::summary_cache::SummaryCacheKey;
use std::fmt::Write;

pub type BriefingArticleId = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BriefingPhase {
    Idle,
    LoadingArticles,
    Summarizing { current_index: usize, total: usize },
    GeneratingBriefing,
    Complete,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleSummaryState {
    Pending,
    InProgress { request_id: u64 },
    Completed { result: ArticleSummaryResult },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleSummaryResult {
    pub title: String,
    pub summary: String,
    pub key_points: Vec<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingArticle {
    pub url: String,
    pub source_title: Option<String>,
    pub prepared_text: String,
    pub content_hash: String,
    pub summary_state: ArticleSummaryState,
    pub cache_key_snapshot: Option<SummaryCacheKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingThemeResult {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingResult {
    pub executive_summary: String,
    pub themes: Vec<BriefingThemeResult>,
    pub article_count: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl BriefingResult {
    pub fn theme_summary(&self) -> String {
        let mut buffer = String::new();
        for (idx, theme) in self.themes.iter().enumerate() {
            let _ = writeln!(buffer, "{}. {}: {}", idx + 1, theme.name, theme.description);
        }
        buffer
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BriefingSession {
    phase: BriefingPhase,
    articles: Vec<BriefingArticle>,
    collection_text: Option<String>,
    briefing_request_id: Option<u64>,
    briefing_result: Option<BriefingResult>,
    started_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedArticle {
    pub url: String,
    pub source_title: Option<String>,
    pub prepared_text: String,
    pub content_hash: String,
}

impl Default for BriefingSession {
    fn default() -> Self {
        Self {
            phase: BriefingPhase::Idle,
            articles: Vec::new(),
            collection_text: None,
            briefing_request_id: None,
            briefing_result: None,
            started_at: None,
        }
    }
}

impl BriefingSession {
    pub fn new_loading(started_at: Option<String>) -> Self {
        Self {
            phase: BriefingPhase::LoadingArticles,
            articles: Vec::new(),
            collection_text: None,
            briefing_request_id: None,
            briefing_result: None,
            started_at,
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
            BriefingPhase::LoadingArticles
                | BriefingPhase::Summarizing { .. }
                | BriefingPhase::GeneratingBriefing
        )
    }

    pub fn articles(&self) -> &[BriefingArticle] {
        &self.articles
    }

    pub fn collection_text(&self) -> Option<&str> {
        self.collection_text.as_deref()
    }

    pub fn set_articles(&mut self, loaded: Vec<LoadedArticle>, collection_text: String) {
        self.articles = loaded
            .into_iter()
            .map(|article| BriefingArticle {
                url: article.url,
                source_title: article.source_title,
                prepared_text: article.prepared_text,
                content_hash: article.content_hash,
                summary_state: ArticleSummaryState::Pending,
                cache_key_snapshot: None,
            })
            .collect();
        self.collection_text = Some(collection_text);
    }

    pub fn transition_to_summarizing(&mut self) {
        let total = self.articles.len();
        if total == 0 {
            self.phase = BriefingPhase::Failed {
                reason: "no articles to summarize".to_string(),
            };
            return;
        }
        self.phase = BriefingPhase::Summarizing {
            current_index: 0,
            total,
        };
    }

    pub fn start_article(&mut self, article_id: BriefingArticleId, request_id: u64) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.summary_state = ArticleSummaryState::InProgress { request_id };
            if let BriefingPhase::Summarizing { total, .. } = self.phase {
                self.phase = BriefingPhase::Summarizing {
                    current_index: article_id + 1,
                    total,
                };
            }
        }
    }

    pub fn complete_article(
        &mut self,
        article_id: BriefingArticleId,
        result: ArticleSummaryResult,
    ) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.summary_state = ArticleSummaryState::Completed { result };
            article.cache_key_snapshot = None;
        }
    }

    pub fn fail_article(&mut self, article_id: BriefingArticleId, reason: String) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.summary_state = ArticleSummaryState::Failed { reason };
            article.cache_key_snapshot = None;
        }
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

    pub fn is_briefing_request(&self, request_id: u64) -> bool {
        self.briefing_request_id == Some(request_id)
    }

    pub fn set_briefing_request_id(&mut self, request_id: u64) {
        self.briefing_request_id = Some(request_id);
        self.phase = BriefingPhase::GeneratingBriefing;
    }

    pub fn complete_briefing(&mut self, result: BriefingResult) {
        self.briefing_result = Some(result);
        self.phase = BriefingPhase::Complete;
        self.briefing_request_id = None;
    }

    pub fn complete_without_briefing(&mut self) {
        self.phase = BriefingPhase::Complete;
        self.briefing_request_id = None;
    }

    pub fn fail(&mut self, reason: String) {
        self.phase = BriefingPhase::Failed { reason };
        self.briefing_request_id = None;
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
        self.articles.iter().find_map(|article| match &article.summary_state {
            ArticleSummaryState::Completed { result } if article.url == url => Some(result),
            _ => None,
        })
    }

    pub fn briefing_result(&self) -> Option<&BriefingResult> {
        self.briefing_result.as_ref()
    }

    pub fn progress_text(&self) -> Option<String> {
        let text = match self.phase {
            BriefingPhase::LoadingArticles => "Loading articles...".to_string(),
            BriefingPhase::Summarizing {
                current_index,
                total,
            } => {
                format!("Summarizing {current_index}/{total} articles...")
            }
            BriefingPhase::GeneratingBriefing => "Generating briefing...".to_string(),
            _ => return None,
        };
        Some(text)
    }

    pub fn format_preview(&self) -> Option<String> {
        if self.phase != BriefingPhase::Complete {
            return None;
        }
        let result = match &self.briefing_result {
            Some(result) => result,
            None => return None,
        };
        let mut buffer = String::new();
        writeln!(&mut buffer, "=== Executive Briefing ===").ok();
        buffer.push('\n');
        writeln!(&mut buffer, "{}", result.executive_summary).ok();
        buffer.push('\n');
        if !result.themes.is_empty() {
            writeln!(&mut buffer, "=== Themes ===").ok();
            writeln!(&mut buffer, "{}", result.theme_summary()).ok();
            buffer.push('\n');
        }
        writeln!(&mut buffer, "=== Session Info ===").ok();
        writeln!(
            &mut buffer,
            "Articles: {} total, {} summarized, {} failed",
            self.articles.len(),
            self.completed_summary_count(),
            self.failed_summary_count()
        )
        .ok();
        Some(buffer)
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
        }
    }

    fn make_session_with_article(url: &str, state: ArticleSummaryState) -> BriefingSession {
        BriefingSession {
            articles: vec![BriefingArticle {
                url: url.to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
                summary_state: state,
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
        let session = make_session_with_article("https://example.com", ArticleSummaryState::Pending);
        assert!(session.summary_for_url("https://example.com").is_none());
    }

    #[test]
    fn summary_for_url_returns_none_when_failed() {
        let session = make_session_with_article(
            "https://example.com",
            ArticleSummaryState::Failed { reason: "err".to_string() },
        );
        assert!(session.summary_for_url("https://example.com").is_none());
    }

    #[test]
    fn summary_for_url_returns_result_when_completed() {
        let result = make_result();
        let session = make_session_with_article(
            "https://example.com",
            ArticleSummaryState::Completed { result: result.clone() },
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
}
