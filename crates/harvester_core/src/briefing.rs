use crate::summary_cache::SummaryCacheKey;
use crate::triage::{ArticleTriageState, TriageSession};
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write;
use std::hash::{Hash, Hasher};

pub type BriefingArticleId = usize;
const MAX_BRIEFING_PREVIEW_CHARS: usize = 32_768;
const PREVIEW_TRUNCATE_MARKER: &str = "[...truncated]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BriefingPhase {
    Idle,
    WaitingForTriage,
    LoadingArticles,
    Summarizing,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorpusFingerprint(u64);

impl CorpusFingerprint {
    pub fn from_articles(articles: &[LoadedArticle]) -> Self {
        let mut pairs: Vec<_> = articles
            .iter()
            .map(|article| (article.url.as_str(), article.content_hash.as_str()))
            .collect();
        pairs.sort_unstable_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(b.1)));
        let mut hasher = DefaultHasher::new();
        pairs.hash(&mut hasher);
        Self(hasher.finish())
    }

    pub fn from_triage_results(triage: &TriageSession) -> Self {
        let mut pairs: Vec<_> = triage
            .articles()
            .iter()
            .filter_map(|article| match article.triage_state {
                ArticleTriageState::Completed { .. } => {
                    Some((article.url.as_str(), article.content_hash.as_str()))
                }
                _ => None,
            })
            .collect();
        pairs.sort_unstable_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(b.1)));
        let mut hasher = DefaultHasher::new();
        pairs.hash(&mut hasher);
        Self(hasher.finish())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriageSelectionPolicy {
    pub cutoff_exclusive: u8,
    pub exclude_untriaged: bool,
}

impl TriageSelectionPolicy {
    pub fn eligible_urls(&self, triage: &TriageSession) -> Vec<String> {
        let mut entries: Vec<_> = triage
            .articles()
            .iter()
            .filter_map(|article| match &article.triage_state {
                ArticleTriageState::Completed { result }
                    if result.priority > self.cutoff_exclusive =>
                {
                    Some((result.priority, article.url.clone()))
                }
                _ if self.exclude_untriaged => None,
                _ => None,
            })
            .collect();
        entries.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        entries.into_iter().map(|(_, url)| url).collect()
    }
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
    pub fn new_waiting_for_triage(started_at: Option<String>) -> Self {
        Self {
            phase: BriefingPhase::WaitingForTriage,
            articles: Vec::new(),
            collection_text: None,
            briefing_request_id: None,
            briefing_result: None,
            started_at,
        }
    }

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
                | BriefingPhase::Summarizing
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
            article.cache_key_snapshot = None;
        }
    }

    pub fn fail_article(&mut self, article_id: BriefingArticleId, reason: String) {
        if let Some(article) = self.articles.get_mut(article_id) {
            article.summary_state = ArticleSummaryState::Failed { reason };
            article.cache_key_snapshot = None;
        }
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
        self.articles
            .iter()
            .find_map(|article| match &article.summary_state {
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
            BriefingPhase::WaitingForTriage => "Waiting for triage...".to_string(),
            BriefingPhase::Summarizing => {
                let completed = self.completed_summary_count() + self.failed_summary_count();
                let total = self.total();
                format!("Summarizing {completed}/{total} articles...")
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
        let mut sections = Vec::new();
        sections.push("# Executive Briefing".to_string());
        sections.push(format!(
            "## Executive Summary\n\n{}",
            result.executive_summary.trim()
        ));

        let mut themes = String::from("## Themes");
        if result.themes.is_empty() {
            themes.push_str("\n\n(none)");
        } else {
            for (idx, theme) in result.themes.iter().enumerate() {
                let _ = writeln!(
                    themes,
                    "\n{}. **{}** - {}",
                    idx + 1,
                    theme.name,
                    theme.description
                );
            }
            themes.pop();
        }
        sections.push(themes);

        sections.push(format!(
            "## Session Info\n\nArticles: {} total, {} summarized, {} failed",
            self.articles.len(),
            self.completed_summary_count(),
            self.failed_summary_count()
        ));

        let preview = sections.join("\n\n");
        Some(truncate_preview(&preview))
    }
}

fn truncate_preview(text: &str) -> String {
    let total_chars = text.chars().count();
    if total_chars <= MAX_BRIEFING_PREVIEW_CHARS {
        return text.to_string();
    }
    let marker_chars = PREVIEW_TRUNCATE_MARKER.chars().count();
    if marker_chars >= MAX_BRIEFING_PREVIEW_CHARS {
        return PREVIEW_TRUNCATE_MARKER.to_string();
    }
    let keep_chars = MAX_BRIEFING_PREVIEW_CHARS - marker_chars;
    let cutoff = text
        .char_indices()
        .nth(keep_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    let mut truncated = text[..cutoff].to_string();
    truncated.push_str(PREVIEW_TRUNCATE_MARKER);
    truncated
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

    fn completed_session_with_themes(themes: Vec<BriefingThemeResult>) -> BriefingSession {
        let mut session = BriefingSession::new_loading(None);
        session.set_articles(
            vec![
                LoadedArticle {
                    url: "https://example.com/a".to_string(),
                    source_title: None,
                    prepared_text: "Article A".to_string(),
                    content_hash: "hash-a".to_string(),
                },
                LoadedArticle {
                    url: "https://example.com/b".to_string(),
                    source_title: None,
                    prepared_text: "Article B".to_string(),
                    content_hash: "hash-b".to_string(),
                },
            ],
            "collection".to_string(),
        );
        session.transition_to_summarizing();
        session.start_article(0, 1);
        session.complete_article(0, make_result());
        session.start_article(1, 2);
        session.fail_article(1, "network".to_string());
        session.set_briefing_request_id(3);
        session.complete_briefing(BriefingResult {
            executive_summary: "Concise executive summary".to_string(),
            themes,
            article_count: 2,
            input_tokens: 20,
            output_tokens: 10,
        });
        session
    }

    #[test]
    fn briefing_format_preview_contains_sections() {
        let session = completed_session_with_themes(vec![BriefingThemeResult {
            name: "Theme A".to_string(),
            description: "Description A".to_string(),
        }]);
        let preview = session.format_preview().expect("preview");
        assert!(preview.contains("# Executive Briefing"));
        assert!(preview.contains("## Executive Summary"));
        assert!(preview.contains("## Themes"));
        assert!(preview.contains("## Session Info"));
    }

    #[test]
    fn briefing_format_preview_theme_list_stable() {
        let session = completed_session_with_themes(vec![
            BriefingThemeResult {
                name: "First".to_string(),
                description: "A".to_string(),
            },
            BriefingThemeResult {
                name: "Second".to_string(),
                description: "B".to_string(),
            },
        ]);
        let preview = session.format_preview().expect("preview");
        let first_pos = preview.find("1. **First**").expect("first theme");
        let second_pos = preview.find("2. **Second**").expect("second theme");
        assert!(first_pos < second_pos);
    }

    #[test]
    fn briefing_format_preview_counts_correct() {
        let session = completed_session_with_themes(vec![]);
        let preview = session.format_preview().expect("preview");
        assert!(preview.contains("Articles: 2 total, 1 summarized, 1 failed"));
    }

    #[test]
    fn briefing_format_preview_none_when_not_complete() {
        let mut session = BriefingSession::new_loading(None);
        session.set_articles(
            vec![LoadedArticle {
                url: "https://example.com".to_string(),
                source_title: None,
                prepared_text: "text".to_string(),
                content_hash: "hash".to_string(),
            }],
            "collection".to_string(),
        );
        assert!(session.format_preview().is_none());
    }

    #[test]
    fn briefing_format_preview_truncates_at_limit() {
        let long_summary = "a".repeat(MAX_BRIEFING_PREVIEW_CHARS + 500);
        let mut session = completed_session_with_themes(vec![BriefingThemeResult {
            name: "Theme".to_string(),
            description: "Description".to_string(),
        }]);
        session.complete_briefing(BriefingResult {
            executive_summary: long_summary,
            themes: vec![BriefingThemeResult {
                name: "Theme".to_string(),
                description: "Description".to_string(),
            }],
            article_count: 2,
            input_tokens: 20,
            output_tokens: 10,
        });

        let preview = session.format_preview().expect("preview");
        assert!(preview.ends_with(PREVIEW_TRUNCATE_MARKER));
        assert_eq!(preview.chars().count(), MAX_BRIEFING_PREVIEW_CHARS);
    }

    fn make_loaded(url: &str, hash: &str) -> LoadedArticle {
        LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: "text".to_string(),
            content_hash: hash.to_string(),
        }
    }

    #[test]
    fn corpus_fingerprint_same_articles_different_order_are_equal() {
        let a = vec![
            make_loaded("https://a", "h1"),
            make_loaded("https://b", "h2"),
        ];
        let b = vec![
            make_loaded("https://b", "h2"),
            make_loaded("https://a", "h1"),
        ];
        assert_eq!(
            CorpusFingerprint::from_articles(&a),
            CorpusFingerprint::from_articles(&b)
        );
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
