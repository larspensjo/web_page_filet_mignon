use crate::summary_cache::SummaryCacheKey;
use crate::triage::{ArticleTriageState, TriageSession};
use harvester_engine::llm::SummaryEntities;
use std::fmt::Write;

pub type BriefingArticleId = usize;
const MAX_BRIEFING_PREVIEW_CHARS: usize = 32_768;
const PREVIEW_TRUNCATE_MARKER: &str = "[...truncated]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BriefingPhase {
    Idle,
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
    /// Structured entity lists extracted by V4+ summary prompt. Empty for V3 cache hits.
    pub entities: SummaryEntities,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingArticle {
    pub url: String,
    pub source_title: Option<String>,
    pub prepared_text: String,
    pub content_hash: String,
    pub fetched_utc: Option<String>,
    pub summary_state: ArticleSummaryState,
    pub cache_key_snapshot: Option<SummaryCacheKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingStoryResult {
    pub headline: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingResult {
    pub executive_summary: String,
    pub top_stories: Vec<BriefingStoryResult>,
    pub article_count: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl BriefingResult {
    pub fn story_summary(&self) -> String {
        let mut buffer = String::new();
        for (idx, story) in self.top_stories.iter().enumerate() {
            let _ = writeln!(buffer, "{}. {}: {}", idx + 1, story.headline, story.body);
        }
        buffer
    }
}

/// A single entry in the persisted briefing history.
/// Distinct from `BriefingResult` — this is the persisted/history version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingHistoryEntry {
    pub generated_at_utc: String, // RFC3339, UTC
    pub executive_summary: String,
    pub top_stories: Vec<BriefingHistoryStory>,
    pub article_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingHistoryStory {
    pub headline: String,
    pub body: String,
}

impl BriefingHistoryEntry {
    /// Creates a history entry from a completed briefing result.
    /// Returns `None` if the summary is blank (not worth storing).
    pub fn from_result(result: &BriefingResult, generated_at_utc: &str) -> Option<Self> {
        let summary = result.executive_summary.trim().to_string();
        if summary.is_empty() {
            return None;
        }
        Some(BriefingHistoryEntry {
            generated_at_utc: generated_at_utc.to_string(),
            executive_summary: summary,
            top_stories: result
                .top_stories
                .iter()
                .map(|story| BriefingHistoryStory {
                    headline: story.headline.clone(),
                    body: story.body.clone(),
                })
                .collect(),
            article_count: result.article_count,
        })
    }
}

/// Maximum number of Unicode scalar values for a single executive_summary in the history block.
const HISTORY_SUMMARY_MAX_CHARS: usize = 500;

/// Truncates `s` to at most `max_chars` Unicode scalar values, appending `…` if truncated.
/// Safe on all UTF-8 input — never panics on multibyte boundaries.
fn truncate_history_summary(s: &str, max_chars: usize) -> String {
    let mut char_indices = s.char_indices();
    match char_indices.nth(max_chars) {
        Some((byte_pos, _)) => format!("{}…", &s[..byte_pos]),
        None => s.to_string(),
    }
}

/// Formats the briefing history into the `{{previous_briefings}}` template variable value.
/// Returns `"(none)"` when history is empty.
/// Entries are rendered newest-first (index 0 = most recent).
pub fn format_previous_briefings_block(history: &[BriefingHistoryEntry]) -> String {
    if history.is_empty() {
        return "(none)".to_string();
    }
    let mut parts = Vec::new();
    for entry in history {
        let summary = if entry.executive_summary.chars().count() > HISTORY_SUMMARY_MAX_CHARS {
            truncate_history_summary(&entry.executive_summary, HISTORY_SUMMARY_MAX_CHARS)
        } else {
            entry.executive_summary.clone()
        };
        let top_stories_line = entry
            .top_stories
            .iter()
            .map(|story| format!("{}: {}", story.headline, story.body))
            .collect::<Vec<_>>()
            .join("; ");
        parts.push(format!(
            "[{}]\nSummary: {}\nTop Stories: {}",
            entry.generated_at_utc, summary, top_stories_line
        ));
    }
    parts.join("\n\n")
}

/// Formats a human-readable coverage window label for aggregate briefings.
/// This string is used both in prompt template variables and in briefing preview metadata.
pub fn format_briefing_time_window_label(
    since_utc: Option<chrono::DateTime<chrono::Utc>>,
) -> String {
    match since_utc {
        Some(dt) => format!(
            "Articles fetched on or after {} (briefing checkpoint filter).",
            dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        ),
        None => "All available articles (no briefing checkpoint filter).".to_string(),
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
    coverage_window_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedArticle {
    pub url: String,
    pub source_title: Option<String>,
    pub prepared_text: String,
    pub content_hash: String,
    /// RFC3339 UTC timestamp from the article's frontmatter; `None` if absent or unparseable.
    pub fetched_utc: Option<String>,
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
            coverage_window_label: None,
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
            coverage_window_label: None,
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
                fetched_utc: article.fetched_utc,
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

    pub fn set_coverage_window_label(&mut self, label: String) {
        self.coverage_window_label = Some(label);
    }

    pub fn coverage_window_label(&self) -> Option<&str> {
        self.coverage_window_label.as_deref()
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

    /// Returns true when the briefing session has recorded a terminal failure
    /// for the given article URL.
    pub fn summary_failed_for_url(&self, url: &str) -> bool {
        self.articles.iter().any(|article| {
            article.url == url
                && matches!(article.summary_state, ArticleSummaryState::Failed { .. })
        })
    }

    pub fn briefing_result(&self) -> Option<&BriefingResult> {
        self.briefing_result.as_ref()
    }

    pub fn progress_text(&self) -> Option<String> {
        let text = match self.phase {
            BriefingPhase::LoadingArticles => "Loading articles...".to_string(),
            BriefingPhase::Summarizing => {
                let completed = self.completed_summary_count() + self.failed_summary_count();
                let total = self.total();
                format!("Summarizing {completed}/{total} articles...")
            }
            BriefingPhase::GeneratingBriefing => "Generating briefing...".to_string(),
            BriefingPhase::Failed { ref reason } => format!("Briefing failed: {reason}"),
            _ => return None,
        };
        Some(text)
    }

    pub fn format_preview(&self) -> Option<String> {
        match &self.phase {
            BriefingPhase::Failed { reason } => {
                let mut sections = Vec::new();
                sections.push("# Executive Briefing".to_string());
                sections.push(format!("## Failed\n\n{reason}"));
                if let Some(label) = self.coverage_window_label.as_deref() {
                    sections.push(format!("## Session Info\n\nCoverage Window: {label}"));
                }
                return Some(truncate_preview(&sections.join("\n\n")));
            }
            BriefingPhase::Complete => {}
            _ => return None,
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

        let mut stories = String::from("## Top Stories");
        if result.top_stories.is_empty() {
            stories.push_str("\n\n(none)");
        } else {
            for (idx, story) in result.top_stories.iter().enumerate() {
                let indented_body = indent_markdown_list_item_body(&story.body);
                let _ = writeln!(
                    stories,
                    "\n{}. **{}**\n\n{}",
                    idx + 1,
                    story.headline,
                    indented_body
                );
            }
            stories.pop();
        }
        sections.push(stories);

        sections.push(format!(
            "## Session Info\n\n{}Articles: {} total, {} summarized, {} failed",
            self.coverage_window_label
                .as_deref()
                .map(|label| format!("Coverage Window: {label}\n"))
                .unwrap_or_default(),
            self.articles.len(),
            self.completed_summary_count(),
            self.failed_summary_count()
        ));

        let preview = sections.join("\n\n");
        Some(truncate_preview(&preview))
    }
}

fn indent_markdown_list_item_body(body: &str) -> String {
    body.lines()
        .map(|line| {
            if line.is_empty() {
                "   ".to_string()
            } else {
                format!("   {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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

    fn completed_session_with_stories(top_stories: Vec<BriefingStoryResult>) -> BriefingSession {
        let mut session = BriefingSession::new_loading(None);
        session.set_articles(
            vec![
                LoadedArticle {
                    url: "https://example.com/a".to_string(),
                    source_title: None,
                    prepared_text: "Article A".to_string(),
                    content_hash: "hash-a".to_string(),
                    fetched_utc: None,
                },
                LoadedArticle {
                    url: "https://example.com/b".to_string(),
                    source_title: None,
                    prepared_text: "Article B".to_string(),
                    content_hash: "hash-b".to_string(),
                    fetched_utc: None,
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
            top_stories,
            article_count: 2,
            input_tokens: 20,
            output_tokens: 10,
        });
        session
    }

    #[test]
    fn briefing_format_preview_contains_sections() {
        let session = completed_session_with_stories(vec![BriefingStoryResult {
            headline: "Story A".to_string(),
            body: "Description A".to_string(),
        }]);
        let preview = session.format_preview().expect("preview");
        assert!(preview.contains("# Executive Briefing"));
        assert!(preview.contains("## Executive Summary"));
        assert!(preview.contains("## Top Stories"));
        assert!(preview.contains("## Session Info"));
    }

    #[test]
    fn briefing_format_preview_story_list_stable() {
        let session = completed_session_with_stories(vec![
            BriefingStoryResult {
                headline: "First".to_string(),
                body: "A".to_string(),
            },
            BriefingStoryResult {
                headline: "Second".to_string(),
                body: "B".to_string(),
            },
        ]);
        let preview = session.format_preview().expect("preview");
        let first_pos = preview.find("1. **First**").expect("first story");
        let second_pos = preview.find("2. **Second**").expect("second story");
        assert!(first_pos < second_pos);
        assert!(preview.contains("1. **First**\n\n   A"));
        assert!(preview.contains("2. **Second**\n\n   B"));
    }

    #[test]
    fn briefing_format_preview_indents_multiline_story_body_under_list_item() {
        let session = completed_session_with_stories(vec![BriefingStoryResult {
            headline: "First".to_string(),
            body: "Line one\n\nLine two".to_string(),
        }]);
        let preview = session.format_preview().expect("preview");
        assert!(preview.contains("1. **First**\n\n   Line one\n   \n   Line two"));
    }

    #[test]
    fn briefing_format_preview_counts_correct() {
        let session = completed_session_with_stories(vec![]);
        let preview = session.format_preview().expect("preview");
        assert!(preview.contains("Articles: 2 total, 1 summarized, 1 failed"));
    }

    #[test]
    fn briefing_format_preview_includes_coverage_window_when_present() {
        let mut session = completed_session_with_stories(vec![]);
        session.set_coverage_window_label(
            "Articles fetched on or after 2026-02-24T00:00:00Z (briefing checkpoint filter)."
                .to_string(),
        );
        let preview = session.format_preview().expect("preview");
        assert!(preview.contains("Coverage Window:"));
        assert!(preview.contains("2026-02-24T00:00:00Z"));
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
                fetched_utc: None,
            }],
            "collection".to_string(),
        );
        assert!(session.format_preview().is_none());
    }

    #[test]
    fn briefing_format_preview_shows_failure_reason() {
        let mut session = BriefingSession::new_loading(None);
        session.fail("request timed out".to_string());

        let preview = session.format_preview().expect("preview");
        assert!(preview.contains("# Executive Briefing"));
        assert!(preview.contains("## Failed"));
        assert!(preview.contains("request timed out"));
    }

    #[test]
    fn briefing_progress_text_shows_failure_reason() {
        let mut session = BriefingSession::new_loading(None);
        session.fail("request timed out".to_string());

        assert_eq!(
            session.progress_text().as_deref(),
            Some("Briefing failed: request timed out")
        );
    }

    #[test]
    fn briefing_format_preview_truncates_at_limit() {
        let long_summary = "a".repeat(MAX_BRIEFING_PREVIEW_CHARS + 500);
        let mut session = completed_session_with_stories(vec![BriefingStoryResult {
            headline: "Story".to_string(),
            body: "Description".to_string(),
        }]);
        session.complete_briefing(BriefingResult {
            executive_summary: long_summary,
            top_stories: vec![BriefingStoryResult {
                headline: "Story".to_string(),
                body: "Description".to_string(),
            }],
            article_count: 2,
            input_tokens: 20,
            output_tokens: 10,
        });

        let preview = session.format_preview().expect("preview");
        assert!(preview.ends_with(PREVIEW_TRUNCATE_MARKER));
        assert_eq!(preview.chars().count(), MAX_BRIEFING_PREVIEW_CHARS);
    }

    #[test]
    fn format_briefing_time_window_label_formats_checkpoint_and_all_time() {
        let all_time = format_briefing_time_window_label(None);
        assert!(all_time.contains("All available articles"));

        let since = chrono::DateTime::parse_from_rfc3339("2026-02-24T12:34:56Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let filtered = format_briefing_time_window_label(Some(since));
        assert!(filtered.contains("2026-02-24T12:34:56Z"));
        assert!(filtered.contains("checkpoint"));
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

#[cfg(test)]
mod history_tests {
    use super::*;

    fn make_entry(ts: &str, summary: &str, top_stories: &[(&str, &str)]) -> BriefingHistoryEntry {
        BriefingHistoryEntry {
            generated_at_utc: ts.to_string(),
            executive_summary: summary.to_string(),
            top_stories: top_stories
                .iter()
                .map(|(headline, body)| BriefingHistoryStory {
                    headline: headline.to_string(),
                    body: body.to_string(),
                })
                .collect(),
            article_count: 5,
        }
    }

    #[test]
    fn format_empty_history_returns_sentinel() {
        let block = format_previous_briefings_block(&[]);
        assert_eq!(block, "(none)");
    }

    #[test]
    fn format_single_entry_contains_timestamp_summary_and_top_stories() {
        let entry = make_entry(
            "2026-02-21T08:00:00Z",
            "Markets rose sharply.",
            &[
                ("Economy", "Growth driven by tech."),
                ("Policy", "Rate cuts expected."),
            ],
        );
        let block = format_previous_briefings_block(&[entry]);
        assert!(block.contains("2026-02-21T08:00:00Z"), "missing timestamp");
        assert!(block.contains("Markets rose sharply."), "missing summary");
        assert!(block.contains("Economy"), "missing story headline");
        assert!(
            block.contains("Growth driven by tech."),
            "missing story body"
        );
        assert!(block.contains("Policy"), "missing second story");
    }

    #[test]
    fn format_three_entries_all_present() {
        let entries: Vec<BriefingHistoryEntry> = (1..=3)
            .map(|i| {
                make_entry(
                    &format!("2026-02-2{}T00:00:00Z", i),
                    &format!("Summary {i}"),
                    &[("Theme", &format!("Desc {i}"))],
                )
            })
            .collect();
        let block = format_previous_briefings_block(&entries);
        for i in 1..=3 {
            assert!(block.contains(&format!("Summary {i}")), "missing entry {i}");
        }
    }

    #[test]
    fn from_result_rejects_empty_summary() {
        let result = BriefingResult {
            executive_summary: "   ".to_string(),
            top_stories: vec![],
            article_count: 0,
            input_tokens: 0,
            output_tokens: 0,
        };
        assert!(BriefingHistoryEntry::from_result(&result, "2026-02-21T00:00:00Z").is_none());
    }

    #[test]
    fn truncation_is_safe_on_multibyte_characters() {
        // "é" is 2 bytes but 1 char — byte-slicing at byte boundary would panic.
        let multibyte: String = "é".repeat(600);
        let entry = make_entry("2026-02-21T00:00:00Z", &multibyte, &[]);
        let block = format_previous_briefings_block(&[entry]);
        // Must not panic and must contain the truncation marker
        assert!(block.contains('…'), "expected truncation marker");
    }
}
