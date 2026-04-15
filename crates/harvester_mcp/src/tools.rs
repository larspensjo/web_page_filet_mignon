use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use harvester_core::{ArticleTriageResult, EntityIndex, SummaryCache, SummaryCacheEntry};
use regex::Regex;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::tool_router;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use smart_query::{QueryKnowledgeBaseInput, SmartQueryEngine, SmartQueryOptions};

use crate::article_index::ArticleIndex;
use crate::server::{HarvesterMcpServer, SmartQueryConfig};
use crate::smart_query;
use crate::util;

// ── Parameter structs ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SearchArticlesParams {
    /// Regex pattern to search for in article content
    pub(crate) pattern: String,
    /// ISO date filter on fetched_utc (inclusive lower bound)
    pub(crate) date_from: Option<String>,
    /// ISO date filter on fetched_utc (inclusive upper bound)
    pub(crate) date_to: Option<String>,
    /// Maximum number of results (default 20)
    pub(crate) max_results: Option<usize>,
    /// Maximum characters per returned snippet (default 320, max 1200)
    pub(crate) snippet_chars: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ReadArticleParams {
    /// Article filename (e.g. "my-article.md")
    pub(crate) filename: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ListArticlesParams {
    /// ISO date filter on fetched_utc (inclusive lower bound)
    pub(crate) date_from: Option<String>,
    /// ISO date filter on fetched_utc (inclusive upper bound)
    pub(crate) date_to: Option<String>,
    /// Regex filter on article title
    pub(crate) title_pattern: Option<String>,
    /// Maximum number of articles to return (default 500)
    pub(crate) max_results: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SearchEntitiesParams {
    /// Substring to match against company names (case-insensitive)
    pub(crate) company: Option<String>,
    /// Substring to match against technology names (case-insensitive)
    pub(crate) technology: Option<String>,
    /// Substring to match against product names (case-insensitive)
    pub(crate) product: Option<String>,
    /// Substring to match against themes (case-insensitive)
    pub(crate) theme: Option<String>,
    /// Maximum number of results to return (default 200)
    pub(crate) max_results: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetArticleSummaryParams {
    /// Article URL
    pub(crate) url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct QueryKnowledgeBaseParams {
    /// Free-text question to answer from the article corpus
    pub(crate) question: String,
    /// Maximum number of ranked articles to include in the digest (default 10)
    pub(crate) max_results: Option<usize>,
    /// Continue through scoring and synthesis even when the query is too broad
    pub(crate) allow_broad: Option<bool>,
    /// Optional entity terms used to limit the search scope
    pub(crate) scope_entities: Option<Vec<String>>,
    /// ISO date filter on fetched_utc (inclusive lower bound)
    pub(crate) scope_date_from: Option<String>,
    /// ISO date filter on fetched_utc (inclusive upper bound)
    pub(crate) scope_date_to: Option<String>,
}

// ── Result structs ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct EntitySearchResult {
    pub(crate) url: String,
    pub(crate) fetched_utc: Option<String>,
    pub(crate) companies: Vec<String>,
    pub(crate) technologies: Vec<String>,
    pub(crate) products: Vec<String>,
    pub(crate) themes: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct ArticleSummaryResponse {
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) key_points: Vec<String>,
    pub(crate) created_at_utc: String,
}

#[derive(Serialize)]
pub(crate) struct SearchMatch {
    pub(crate) filename: String,
    pub(crate) title: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) fetched_utc: Option<String>,
    pub(crate) snippet: String,
}

#[derive(Serialize)]
pub(crate) struct ArticleSummary {
    pub(crate) filename: String,
    pub(crate) title: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) fetched_utc: Option<String>,
    pub(crate) token_count: Option<u32>,
}

// ── Tool helpers ─────────────────────────────────────────────────────────────

pub(crate) const DEFAULT_SEARCH_SNIPPET_CHARS: usize = 320;
const MAX_SEARCH_SNIPPET_CHARS: usize = 1200;
const MAX_SEARCH_RESULTS: usize = 200;
const MAX_READ_ARTICLE_CHARS: usize = 100_000;
const MAX_LIST_ARTICLES_RESULTS: usize = 500;
const MAX_SEARCH_ENTITIES_RESULTS: usize = 200;

fn clamp_search_snippet_chars(snippet_chars: Option<usize>) -> usize {
    snippet_chars
        .unwrap_or(DEFAULT_SEARCH_SNIPPET_CHARS)
        .clamp(80, MAX_SEARCH_SNIPPET_CHARS)
}

fn compact_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_frontmatter_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed == "---" {
        return true;
    }

    [
        "url:",
        "title:",
        "fetched_utc:",
        "token_count:",
        "summary_created_at_utc:",
        "summary_input_tokens:",
        "summary_output_tokens:",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
}

fn truncate_text_boundary(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let mut end = text.len();
    for (char_count, (index, _)) in text.char_indices().enumerate() {
        if char_count == max_chars {
            end = index;
            break;
        }
    }

    let candidate = &text[..end];
    let trimmed = candidate
        .rfind(|ch: char| ch.is_whitespace() || [',', ';', ':', '.'].contains(&ch))
        .filter(|index| *index >= candidate.len() / 2)
        .map(|index| &candidate[..index])
        .unwrap_or(candidate)
        .trim();

    if trimmed.is_empty() {
        format!("{}...", candidate.trim())
    } else {
        format!("{trimmed}...")
    }
}

/// Build a compact snippet from nearby regex matches for chat-oriented MCP tool responses.
///
/// Unlike the LLM-context snippet in `smart_query`, this version compacts whitespace,
/// filters frontmatter lines, and truncates to a character budget.
fn build_search_snippet(content: &str, re: &Regex, max_chars: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut included: Vec<usize> = Vec::new();
    let mut match_count = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if re.is_match(line) {
            let start = i.saturating_sub(1);
            let end = (i + 1).min(lines.len().saturating_sub(1));
            for j in start..=end {
                if !included.contains(&j) {
                    included.push(j);
                }
            }
            match_count += 1;
            if match_count >= 3 {
                break;
            }
        }
    }
    included.sort_unstable();
    let compact = included
        .iter()
        .map(|&i| compact_whitespace(lines[i]))
        .filter(|line| !is_frontmatter_line(line))
        .collect::<Vec<_>>()
        .join(" ... ");

    truncate_text_boundary(&compact, max_chars)
}

// ── Tool implementations ─────────────────────────────────────────────────────

#[tool_router]
impl HarvesterMcpServer {
    /// Return the server version.
    #[rmcp::tool(description = "Return the harvester-mcp server version.")]
    async fn server_version(&self) -> String {
        let t = std::time::Instant::now();
        engine_logging::engine_info!("[tool] server_version called");
        let result = env!("CARGO_PKG_VERSION").to_string();
        engine_logging::engine_info!(
            "[tool] server_version returned {} bytes in {}ms",
            result.len(),
            t.elapsed().as_millis()
        );
        result
    }

    /// Search article content with a regex pattern, optional date range, and result cap.
    #[rmcp::tool(
        description = "Search article content using a regex pattern. Optionally filter by date range (date_from/date_to as ISO date strings), cap results with max_results (default 20, max 200), and control snippet size with snippet_chars (default 320, min 80, max 1200). Returns JSON array of matches with filename, title, url, fetched_utc, and a compact content snippet."
    )]
    async fn search_articles(&self, Parameters(p): Parameters<SearchArticlesParams>) -> String {
        let t = std::time::Instant::now();
        let snippet_chars = clamp_search_snippet_chars(p.snippet_chars);
        engine_logging::engine_info!(
            "[tool] search_articles called with pattern={:?} date_from={:?} date_to={:?} max_results={:?} snippet_chars={}",
            p.pattern, p.date_from, p.date_to, p.max_results, snippet_chars
        );
        let re = match Regex::new(&p.pattern) {
            Ok(r) => r,
            Err(e) => {
                return serde_json::json!({"error": format!("invalid regex: {}", e)}).to_string()
            }
        };
        let max = p.max_results.unwrap_or(20).min(MAX_SEARCH_RESULTS);
        let mut results: Vec<SearchMatch> = Vec::new();

        for entry in &self.article_index.articles {
            if results.len() >= max {
                break;
            }
            if !util::date_in_range(
                entry.fetched_utc.as_deref(),
                p.date_from.as_deref(),
                p.date_to.as_deref(),
            ) {
                continue;
            }
            if re.is_match(&entry.content) {
                let snippet = build_search_snippet(&entry.content, &re, snippet_chars);
                results.push(SearchMatch {
                    filename: entry.filename.clone(),
                    title: entry.title.clone(),
                    url: entry.url.clone(),
                    fetched_utc: entry.fetched_utc.clone(),
                    snippet,
                });
            }
        }

        let result = serde_json::to_string(&results)
            .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}).to_string());
        engine_logging::engine_info!(
            "[tool] search_articles returned {} matches {} bytes in {}ms",
            results.len(),
            result.len(),
            t.elapsed().as_millis()
        );
        result
    }

    /// Return the markdown content of an article by filename, truncated at 100k characters.
    #[rmcp::tool(
        description = "Read the markdown content of an article. Pass the filename (e.g. \"my-article.md\") as returned by list_articles or search_articles. Content is returned as-is up to 100,000 characters; longer articles are truncated with a note."
    )]
    async fn read_article(&self, Parameters(p): Parameters<ReadArticleParams>) -> String {
        let t = std::time::Instant::now();
        engine_logging::engine_info!("[tool] read_article called with filename={:?}", p.filename);
        let result = match self
            .article_index
            .articles
            .iter()
            .find(|a| a.filename == p.filename)
        {
            Some(entry) => {
                let content = &entry.content;
                let char_count = content.chars().count();
                if char_count > MAX_READ_ARTICLE_CHARS {
                    let truncated: String = content.chars().take(MAX_READ_ARTICLE_CHARS).collect();
                    format!(
                        "{truncated}\n\n[Article truncated at {MAX_READ_ARTICLE_CHARS} characters. Full article is {char_count} characters.]",
                    )
                } else {
                    content.clone()
                }
            }
            None => serde_json::json!({"error": format!("article not found: {}", p.filename)})
                .to_string(),
        };
        engine_logging::engine_info!(
            "[tool] read_article returned {} bytes in {}ms",
            result.len(),
            t.elapsed().as_millis()
        );
        result
    }

    /// Search articles by entity tags (company, technology, product, theme).
    #[rmcp::tool(
        description = "Search articles by entity tags. Provide at least one of: company, technology, product, theme (all case-insensitive substring matches). All provided filters must match (AND logic). Limit results with max_results (default 200, max 200). Returns JSON array with url, fetched_utc, companies, technologies, products, themes."
    )]
    async fn search_entities(&self, Parameters(p): Parameters<SearchEntitiesParams>) -> String {
        let t = std::time::Instant::now();
        engine_logging::engine_info!(
            "[tool] search_entities called with company={} technology={} product={} theme={}",
            p.company.as_deref().unwrap_or("None"),
            p.technology.as_deref().unwrap_or("None"),
            p.product.as_deref().unwrap_or("None"),
            p.theme.as_deref().unwrap_or("None"),
        );
        if p.company.is_none() && p.technology.is_none() && p.product.is_none() && p.theme.is_none()
        {
            return serde_json::json!({"error": "at least one search parameter required"})
                .to_string();
        }

        let max_entities = p
            .max_results
            .unwrap_or(MAX_SEARCH_ENTITIES_RESULTS)
            .min(MAX_SEARCH_ENTITIES_RESULTS);

        let results: Vec<EntitySearchResult> = self
            .entity_index
            .entries
            .iter()
            .filter_map(|(url, entry)| {
                let matches_company = p.company.as_ref().is_none_or(|q| {
                    let q = q.to_lowercase();
                    entry
                        .companies
                        .iter()
                        .any(|c| c.to_lowercase().contains(&q))
                });
                let matches_technology = p.technology.as_ref().is_none_or(|q| {
                    let q = q.to_lowercase();
                    entry
                        .technologies
                        .iter()
                        .any(|c| c.to_lowercase().contains(&q))
                });
                let matches_product = p.product.as_ref().is_none_or(|q| {
                    let q = q.to_lowercase();
                    entry.products.iter().any(|c| c.to_lowercase().contains(&q))
                });
                let matches_theme = p.theme.as_ref().is_none_or(|q| {
                    let q = q.to_lowercase();
                    entry.themes.iter().any(|c| c.to_lowercase().contains(&q))
                });
                if matches_company && matches_technology && matches_product && matches_theme {
                    Some(EntitySearchResult {
                        url: url.clone(),
                        fetched_utc: entry.fetched_utc.clone(),
                        companies: entry.companies.clone(),
                        technologies: entry.technologies.clone(),
                        products: entry.products.clone(),
                        themes: entry.themes.clone(),
                    })
                } else {
                    None
                }
            })
            .take(max_entities)
            .collect();

        let result = serde_json::to_string(&results)
            .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}).to_string());
        engine_logging::engine_info!(
            "[tool] search_entities returned {} bytes in {}ms",
            result.len(),
            t.elapsed().as_millis()
        );
        result
    }

    /// Return the summary for an article by URL.
    #[rmcp::tool(
        description = "Get the LLM-generated summary for an article by its URL. Returns title, summary, key_points, and created_at_utc, or a status object if no summary is available."
    )]
    async fn get_article_summary(
        &self,
        Parameters(p): Parameters<GetArticleSummaryParams>,
    ) -> String {
        let t = std::time::Instant::now();
        engine_logging::engine_info!("[tool] get_article_summary called with url={:?}", p.url);
        let result = match self.summary_index.get(&p.url) {
            None => serde_json::json!({"status": "no summary available"}).to_string(),
            Some(entry) => {
                let resp = ArticleSummaryResponse {
                    title: entry.result.title.clone(),
                    summary: entry.result.summary.clone(),
                    key_points: entry.result.key_points.clone(),
                    created_at_utc: entry.created_at_utc.clone(),
                };
                serde_json::to_string(&resp)
                    .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}).to_string())
            }
        };
        engine_logging::engine_info!(
            "[tool] get_article_summary returned {} bytes in {}ms",
            result.len(),
            t.elapsed().as_millis()
        );
        result
    }

    /// Query the knowledge base using the smart agent layer.
    #[rmcp::tool(
        description = "Answer a free-text question from the article corpus. Uses a cheap-model agent layer for query expansion, relevance scoring, and digest assembly. Returns JSON with mode, message, synthesis, ranked_articles, warnings, total_token_count, and optional breadth diagnostics. If a query is too broad, the tool returns mode=too_broad unless allow_broad=true."
    )]
    async fn query_knowledge_base(
        &self,
        Parameters(p): Parameters<QueryKnowledgeBaseParams>,
    ) -> String {
        let t = std::time::Instant::now();
        engine_logging::engine_info!(
            "[tool] query_knowledge_base called with question={:?} max_results={:?} allow_broad={:?} scope_entities={:?} scope_date_from={:?} scope_date_to={:?}",
            p.question,
            p.max_results,
            p.allow_broad,
            p.scope_entities,
            p.scope_date_from,
            p.scope_date_to
        );

        let response = self
            .smart_query_engine
            .query(QueryKnowledgeBaseInput {
                question: p.question,
                max_results: p.max_results.unwrap_or(10),
                allow_broad: p.allow_broad.unwrap_or(false),
                scope_entities: p.scope_entities.unwrap_or_default(),
                scope_date_from: p.scope_date_from,
                scope_date_to: p.scope_date_to,
            })
            .await;

        let result = serde_json::to_string(&response)
            .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}).to_string());
        engine_logging::engine_info!(
            "[tool] query_knowledge_base returned {} bytes in {}ms",
            result.len(),
            t.elapsed().as_millis()
        );
        result
    }

    /// List articles, optionally filtered by date range and/or title regex.
    #[rmcp::tool(
        description = "List articles in the corpus. Optionally filter by date_from/date_to (ISO date strings, inclusive) and/or title_pattern (regex on title). Limit results with max_results (default 500, max 500). Returns JSON array with filename, title, url, fetched_utc, token_count."
    )]
    async fn list_articles(&self, Parameters(p): Parameters<ListArticlesParams>) -> String {
        let t = std::time::Instant::now();
        engine_logging::engine_info!(
            "[tool] list_articles called with date_from={:?} date_to={:?} title_pattern={:?}",
            p.date_from,
            p.date_to,
            p.title_pattern
        );
        let title_re = if let Some(pat) = &p.title_pattern {
            match Regex::new(pat) {
                Ok(r) => Some(r),
                Err(e) => {
                    return serde_json::json!({"error": format!("invalid title_pattern regex: {}", e)}).to_string()
                }
            }
        } else {
            None
        };

        let max_list = p
            .max_results
            .unwrap_or(MAX_LIST_ARTICLES_RESULTS)
            .min(MAX_LIST_ARTICLES_RESULTS);

        let results: Vec<ArticleSummary> = self
            .article_index
            .articles
            .iter()
            .filter(|entry| {
                util::date_in_range(
                    entry.fetched_utc.as_deref(),
                    p.date_from.as_deref(),
                    p.date_to.as_deref(),
                )
            })
            .filter(|entry| match &title_re {
                None => true,
                Some(re) => entry
                    .title
                    .as_deref()
                    .map(|t| re.is_match(t))
                    .unwrap_or(false),
            })
            .map(|entry| ArticleSummary {
                filename: entry.filename.clone(),
                title: entry.title.clone(),
                url: entry.url.clone(),
                fetched_utc: entry.fetched_utc.clone(),
                token_count: entry.token_count,
            })
            .take(max_list)
            .collect();

        let result = serde_json::to_string(&results)
            .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}).to_string());
        engine_logging::engine_info!(
            "[tool] list_articles returned {} entries {} bytes in {}ms",
            results.len(),
            result.len(),
            t.elapsed().as_millis()
        );
        result
    }
}

impl HarvesterMcpServer {
    pub(crate) fn new(
        output_dir: PathBuf,
        entity_index: EntityIndex,
        summary_cache: SummaryCache,
        article_index: ArticleIndex,
        summary_index: HashMap<String, SummaryCacheEntry>,
        triage_index: HashMap<String, ArticleTriageResult>,
        smart_query_config: SmartQueryConfig,
    ) -> Self {
        let entity_index = Arc::new(entity_index);
        let article_index = Arc::new(article_index);
        let summary_index = Arc::new(summary_index);
        let smart_query_engine = Arc::new(SmartQueryEngine::new(
            article_index.clone(),
            entity_index.clone(),
            summary_index.clone(),
            Arc::new(triage_index.clone()),
            smart_query_config.agent_provider,
            SmartQueryOptions::new(
                smart_query_config.agent_model,
                smart_query_config.context_budget,
            )
            .with_scoring_candidate_cap(smart_query_config.scoring_candidate_cap)
            .with_too_broad_threshold(smart_query_config.too_broad_threshold)
            .with_min_triage_priority(smart_query_config.min_triage_priority),
        ));
        Self {
            output_dir,
            entity_index,
            summary_cache,
            article_index,
            summary_index,
            smart_query_engine,
            tool_router: Self::tool_router(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article_index::ArticleEntry;
    use crate::server::{HarvesterMcpServer, SmartQueryConfig};
    use harvester_core::{EntityIndex, SummaryCache};
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn test_server_with_articles(articles: Vec<ArticleEntry>) -> HarvesterMcpServer {
        HarvesterMcpServer::new(
            PathBuf::from("output"),
            EntityIndex {
                schema_version: 1,
                entries: BTreeMap::new(),
            },
            SummaryCache::new(),
            crate::article_index::ArticleIndex { articles },
            HashMap::new(),
            HashMap::new(),
            SmartQueryConfig {
                agent_model: "gpt-5.4-nano".to_string(),
                context_budget: 4000,
                scoring_candidate_cap: smart_query::DEFAULT_MAX_SCORING_CANDIDATES,
                too_broad_threshold: smart_query::DEFAULT_TOO_BROAD_THRESHOLD,
                min_triage_priority: smart_query::DEFAULT_MIN_TRIAGE_PRIORITY,
                agent_provider: None,
            },
        )
    }

    fn test_server_with_entities(
        entries: Vec<(String, harvester_core::EntityIndexEntry)>,
    ) -> HarvesterMcpServer {
        let mut map = BTreeMap::new();
        for (url, entry) in entries {
            map.insert(url, entry);
        }
        HarvesterMcpServer::new(
            PathBuf::from("output"),
            EntityIndex {
                schema_version: 1,
                entries: map,
            },
            SummaryCache::new(),
            crate::article_index::ArticleIndex { articles: vec![] },
            HashMap::new(),
            HashMap::new(),
            SmartQueryConfig {
                agent_model: "gpt-5.4-nano".to_string(),
                context_budget: 4000,
                scoring_candidate_cap: smart_query::DEFAULT_MAX_SCORING_CANDIDATES,
                too_broad_threshold: smart_query::DEFAULT_TOO_BROAD_THRESHOLD,
                min_triage_priority: smart_query::DEFAULT_MIN_TRIAGE_PRIORITY,
                agent_provider: None,
            },
        )
    }

    fn test_server_with_summary(
        url: &str,
        entry: harvester_core::SummaryCacheEntry,
    ) -> HarvesterMcpServer {
        let mut summary_index = HashMap::new();
        summary_index.insert(url.to_string(), entry);
        HarvesterMcpServer::new(
            PathBuf::from("output"),
            EntityIndex {
                schema_version: 1,
                entries: BTreeMap::new(),
            },
            SummaryCache::new(),
            crate::article_index::ArticleIndex { articles: vec![] },
            summary_index,
            HashMap::new(),
            SmartQueryConfig {
                agent_model: "gpt-5.4-nano".to_string(),
                context_budget: 4000,
                scoring_candidate_cap: smart_query::DEFAULT_MAX_SCORING_CANDIDATES,
                too_broad_threshold: smart_query::DEFAULT_TOO_BROAD_THRESHOLD,
                min_triage_priority: smart_query::DEFAULT_MIN_TRIAGE_PRIORITY,
                agent_provider: None,
            },
        )
    }

    fn sample_article(filename: &str, title: &str, body: &str) -> ArticleEntry {
        ArticleEntry {
            filename: filename.to_string(),
            path: PathBuf::from(filename),
            title: Some(title.to_string()),
            url: Some(format!("https://example.com/{filename}")),
            fetched_utc: Some("2026-04-14T12:00:00Z".to_string()),
            token_count: Some(100),
            content: body.to_string(),
        }
    }

    #[tokio::test]
    async fn search_articles_returns_compact_snippets_by_default() {
        let body = format!(
            "---\nurl: \"https://example.com/alpha\"\ntitle: \"Alpha\"\n---\n{}\n{}\n",
            "Microsoft and OpenAI are renegotiating the partnership while expanding data center capacity. ".repeat(6),
            "This second paragraph should not all fit into the default snippet budget. ".repeat(6)
        );
        let server = test_server_with_articles(vec![sample_article("alpha.md", "Alpha", &body)]);

        let result = server
            .search_articles(Parameters(SearchArticlesParams {
                pattern: "Microsoft".to_string(),
                date_from: None,
                date_to: None,
                max_results: Some(5),
                snippet_chars: None,
            }))
            .await;

        let payload: Value = serde_json::from_str(&result).expect("valid json");
        let snippet = payload[0]["snippet"].as_str().expect("snippet string");
        assert!(snippet.len() <= DEFAULT_SEARCH_SNIPPET_CHARS + 3);
        assert!(snippet.contains("Microsoft and OpenAI"));
        assert!(!snippet.contains("url:"));
        assert!(snippet.ends_with("..."));
    }

    #[tokio::test]
    async fn read_article_returns_content_when_found() {
        let server =
            test_server_with_articles(vec![sample_article("beta.md", "Beta", "Hello from beta.")]);
        let result = server
            .read_article(Parameters(ReadArticleParams {
                filename: "beta.md".to_string(),
            }))
            .await;
        assert!(result.contains("Hello from beta."));
    }

    #[tokio::test]
    async fn read_article_returns_error_when_not_found() {
        let server = test_server_with_articles(vec![]);
        let result = server
            .read_article(Parameters(ReadArticleParams {
                filename: "ghost.md".to_string(),
            }))
            .await;
        let payload: Value = serde_json::from_str(&result).expect("valid json");
        assert!(payload["error"].as_str().unwrap().contains("ghost.md"));
    }

    #[tokio::test]
    async fn read_article_truncates_at_cap() {
        let long_content = "x".repeat(MAX_READ_ARTICLE_CHARS + 500);
        let server =
            test_server_with_articles(vec![sample_article("long.md", "Long", &long_content)]);
        let result = server
            .read_article(Parameters(ReadArticleParams {
                filename: "long.md".to_string(),
            }))
            .await;
        assert!(result.contains("[Article truncated"));
        let note_start = result.find("\n\n[Article truncated").unwrap();
        assert_eq!(result[..note_start].chars().count(), MAX_READ_ARTICLE_CHARS);
    }

    #[tokio::test]
    async fn list_articles_filters_by_date() {
        let mut old = sample_article("old.md", "Old", "old content");
        old.fetched_utc = Some("2025-01-01T00:00:00Z".to_string());
        let mut new = sample_article("new.md", "New", "new content");
        new.fetched_utc = Some("2026-03-01T00:00:00Z".to_string());
        let server = test_server_with_articles(vec![old, new]);

        let result = server
            .list_articles(Parameters(ListArticlesParams {
                date_from: Some("2026-01-01".to_string()),
                date_to: None,
                title_pattern: None,
                max_results: None,
            }))
            .await;

        let payload: Value = serde_json::from_str(&result).expect("valid json");
        let arr = payload.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["filename"].as_str().unwrap(), "new.md");
    }

    #[tokio::test]
    async fn list_articles_filters_by_title_regex() {
        let server = test_server_with_articles(vec![
            sample_article("alpha.md", "Alpha Article", "content"),
            sample_article("beta.md", "Beta Post", "content"),
        ]);

        let result = server
            .list_articles(Parameters(ListArticlesParams {
                date_from: None,
                date_to: None,
                title_pattern: Some("Alpha".to_string()),
                max_results: None,
            }))
            .await;

        let payload: Value = serde_json::from_str(&result).expect("valid json");
        let arr = payload.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["filename"].as_str().unwrap(), "alpha.md");
    }

    #[tokio::test]
    async fn search_entities_matches_company() {
        use harvester_core::EntityIndexEntry;
        let entry = EntityIndexEntry {
            companies: vec!["Acme Corp".to_string()],
            ..Default::default()
        };
        let server =
            test_server_with_entities(vec![("https://example.com/acme".to_string(), entry)]);
        let result = server
            .search_entities(Parameters(SearchEntitiesParams {
                company: Some("acme".to_string()),
                technology: None,
                product: None,
                theme: None,
                max_results: None,
            }))
            .await;
        let payload: Value = serde_json::from_str(&result).expect("valid json");
        let arr = payload.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["url"].as_str().unwrap(), "https://example.com/acme");
    }

    #[tokio::test]
    async fn search_entities_and_logic_requires_all_filters() {
        use harvester_core::EntityIndexEntry;
        let acme_tech = EntityIndexEntry {
            companies: vec!["Acme".to_string()],
            technologies: vec!["Rust".to_string()],
            ..Default::default()
        };
        let acme_only = EntityIndexEntry {
            companies: vec!["Acme".to_string()],
            ..Default::default()
        };
        let server = test_server_with_entities(vec![
            ("https://example.com/both".to_string(), acme_tech),
            ("https://example.com/company-only".to_string(), acme_only),
        ]);
        let result = server
            .search_entities(Parameters(SearchEntitiesParams {
                company: Some("acme".to_string()),
                technology: Some("Rust".to_string()),
                product: None,
                theme: None,
                max_results: None,
            }))
            .await;
        let payload: Value = serde_json::from_str(&result).expect("valid json");
        let arr = payload.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["url"].as_str().unwrap(), "https://example.com/both");
    }

    #[tokio::test]
    async fn get_article_summary_returns_no_summary_when_missing() {
        let server = test_server_with_articles(vec![]);
        let result = server
            .get_article_summary(Parameters(GetArticleSummaryParams {
                url: "https://example.com/unknown".to_string(),
            }))
            .await;
        let payload: Value = serde_json::from_str(&result).expect("valid json");
        assert_eq!(payload["status"].as_str().unwrap(), "no summary available");
    }

    #[tokio::test]
    async fn get_article_summary_returns_summary_when_present() {
        use harvester_core::{ArticleSummaryResult, SummaryCacheEntry, SummaryEntities};
        let entry = SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "Known Article".to_string(),
                summary: "A test summary.".to_string(),
                key_points: vec!["Point A".to_string()],
                input_tokens: 100,
                output_tokens: 50,
                entities: SummaryEntities::default(),
            },
            created_at_utc: "2026-04-01T00:00:00Z".to_string(),
        };
        let server = test_server_with_summary("https://example.com/known", entry);
        let result = server
            .get_article_summary(Parameters(GetArticleSummaryParams {
                url: "https://example.com/known".to_string(),
            }))
            .await;
        let payload: Value = serde_json::from_str(&result).expect("valid json");
        assert_eq!(payload["title"].as_str().unwrap(), "Known Article");
        assert_eq!(payload["summary"].as_str().unwrap(), "A test summary.");
        assert_eq!(payload["key_points"][0].as_str().unwrap(), "Point A");
    }

    #[tokio::test]
    async fn search_articles_respects_max_results_parameter() {
        let server = test_server_with_articles(vec![
            sample_article("a.md", "A", "Rust programming language"),
            sample_article("b.md", "B", "Rust programming language"),
            sample_article("c.md", "C", "Rust programming language"),
        ]);
        let result = server
            .search_articles(Parameters(SearchArticlesParams {
                pattern: "Rust".to_string(),
                date_from: None,
                date_to: None,
                max_results: Some(2),
                snippet_chars: None,
            }))
            .await;
        let payload: Value = serde_json::from_str(&result).expect("valid json");
        assert_eq!(payload.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn list_articles_respects_max_results_parameter() {
        let server = test_server_with_articles(vec![
            sample_article("a.md", "A", "content"),
            sample_article("b.md", "B", "content"),
            sample_article("c.md", "C", "content"),
        ]);
        let result = server
            .list_articles(Parameters(ListArticlesParams {
                date_from: None,
                date_to: None,
                title_pattern: None,
                max_results: Some(2),
            }))
            .await;
        let payload: Value = serde_json::from_str(&result).expect("valid json");
        assert_eq!(payload.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn search_entities_respects_max_results_parameter() {
        use harvester_core::EntityIndexEntry;
        let entries = (0..5)
            .map(|i| {
                let entry = EntityIndexEntry {
                    companies: vec!["Acme".to_string()],
                    ..Default::default()
                };
                (format!("https://example.com/{i}"), entry)
            })
            .collect::<Vec<_>>();
        let server = test_server_with_entities(entries);
        let result = server
            .search_entities(Parameters(SearchEntitiesParams {
                company: Some("acme".to_string()),
                technology: None,
                product: None,
                theme: None,
                max_results: Some(3),
            }))
            .await;
        let payload: Value = serde_json::from_str(&result).expect("valid json");
        assert_eq!(payload.as_array().unwrap().len(), 3);
    }
}
