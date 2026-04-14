mod article_index;
mod smart_query;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use article_index::ArticleIndex;
use clap::Parser;
use harvester_core::{ArticleTriageResult, EntityIndex, SummaryCache, SummaryCacheEntry};
use harvester_engine::llm::{LlmProvider, OpenAiProvider};
use harvester_io::{load_entity_index, load_summary_cache, load_triage_cache};
use regex::Regex;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{tool_handler, tool_router, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use smart_query::{QueryKnowledgeBaseInput, SmartQueryEngine, SmartQueryOptions};

#[derive(Parser, Debug)]
#[command(name = "harvester_mcp", about = "Harvester MCP server")]
struct Args {
    /// Output directory containing harvested data
    #[arg(long, default_value = "./output")]
    output_dir: PathBuf,

    /// LLM model ID to use for agent operations
    #[arg(long, default_value = harvester_engine::llm::DEFAULT_TRIAGE_MODEL)]
    agent_model: String,

    /// Context budget in tokens
    #[arg(long, default_value_t = 4000)]
    context_budget: usize,

    /// Maximum number of candidates to send to LLM scoring
    #[arg(long, default_value_t = smart_query::DEFAULT_MAX_SCORING_CANDIDATES)]
    scoring_candidate_cap: usize,

    /// Broad-query threshold on eligible candidates before early return
    #[arg(long, default_value_t = smart_query::DEFAULT_TOO_BROAD_THRESHOLD)]
    too_broad_threshold: usize,

    /// Minimum triage priority required for articles to be eligible
    #[arg(long, default_value_t = smart_query::DEFAULT_MIN_TRIAGE_PRIORITY)]
    min_triage_priority: u8,

    /// Number of previous log runs to retain as mcp.log.N archives
    #[arg(long, default_value_t = 9)]
    retain_log_runs: usize,

    /// Directory for log files (defaults to <output-dir>/logs)
    #[arg(long)]
    log_dir: Option<PathBuf>,
}

#[derive(Clone)]
struct HarvesterMcpServer {
    #[allow(dead_code)]
    output_dir: PathBuf,
    entity_index: Arc<EntityIndex>,
    #[allow(dead_code)]
    summary_cache: SummaryCache,
    article_index: Arc<ArticleIndex>,
    summary_index: Arc<HashMap<String, SummaryCacheEntry>>,
    smart_query_engine: Arc<SmartQueryEngine>,
    tool_router: ToolRouter<Self>,
}

struct SmartQueryConfig {
    agent_model: String,
    context_budget: usize,
    scoring_candidate_cap: usize,
    too_broad_threshold: usize,
    min_triage_priority: u8,
    agent_provider: Option<Arc<dyn LlmProvider>>,
}

// ── Parameter structs ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchArticlesParams {
    /// Regex pattern to search for in article content
    pattern: String,
    /// ISO date filter on fetched_utc (inclusive lower bound)
    date_from: Option<String>,
    /// ISO date filter on fetched_utc (inclusive upper bound)
    date_to: Option<String>,
    /// Maximum number of results (default 20)
    max_results: Option<usize>,
    /// Maximum characters per returned snippet (default 320, max 1200)
    snippet_chars: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadArticleParams {
    /// Article filename (e.g. "my-article.md")
    filename: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListArticlesParams {
    /// ISO date filter on fetched_utc (inclusive lower bound)
    date_from: Option<String>,
    /// ISO date filter on fetched_utc (inclusive upper bound)
    date_to: Option<String>,
    /// Regex filter on article title
    title_pattern: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchEntitiesParams {
    /// Substring to match against company names (case-insensitive)
    company: Option<String>,
    /// Substring to match against technology names (case-insensitive)
    technology: Option<String>,
    /// Substring to match against product names (case-insensitive)
    product: Option<String>,
    /// Substring to match against themes (case-insensitive)
    theme: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetArticleSummaryParams {
    /// Article URL
    url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct QueryKnowledgeBaseParams {
    /// Free-text question to answer from the article corpus
    question: String,
    /// Maximum number of ranked articles to include in the digest (default 10)
    max_results: Option<usize>,
    /// Continue through scoring and synthesis even when the query is too broad
    allow_broad: Option<bool>,
    /// Optional entity terms used to limit the search scope
    scope_entities: Option<Vec<String>>,
    /// ISO date filter on fetched_utc (inclusive lower bound)
    scope_date_from: Option<String>,
    /// ISO date filter on fetched_utc (inclusive upper bound)
    scope_date_to: Option<String>,
}

// ── Result structs ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct EntitySearchResult {
    url: String,
    fetched_utc: Option<String>,
    companies: Vec<String>,
    technologies: Vec<String>,
    products: Vec<String>,
    themes: Vec<String>,
}

#[derive(Serialize)]
struct ArticleSummaryResponse {
    title: String,
    summary: String,
    key_points: Vec<String>,
    created_at_utc: String,
}

#[derive(Serialize)]
struct SearchMatch {
    filename: String,
    title: Option<String>,
    url: Option<String>,
    fetched_utc: Option<String>,
    snippet: String,
}

#[derive(Serialize)]
struct ArticleSummary {
    filename: String,
    title: Option<String>,
    url: Option<String>,
    fetched_utc: Option<String>,
    token_count: Option<u32>,
}

// ── Tool implementations ─────────────────────────────────────────────────────

/// Check whether fetched_utc matches the date range filters.
fn date_in_range(
    fetched_utc: Option<&str>,
    date_from: Option<&str>,
    date_to: Option<&str>,
) -> bool {
    if date_from.is_none() && date_to.is_none() {
        return true;
    }
    match fetched_utc {
        None => false,
        Some(ts) => {
            if let Some(from) = date_from {
                if ts < from {
                    return false;
                }
            }
            if let Some(to) = date_to {
                if ts > to {
                    return false;
                }
            }
            true
        }
    }
}

const DEFAULT_SEARCH_SNIPPET_CHARS: usize = 320;
const MAX_SEARCH_SNIPPET_CHARS: usize = 1200;

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

/// Build a snippet from nearby matches and compress it for chat-oriented payloads.
fn build_snippet(content: &str, re: &Regex, max_chars: usize) -> String {
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
        description = "Search article content using a regex pattern. Optionally filter by date range (date_from/date_to as ISO date strings), cap results with max_results (default 20), and control snippet size with snippet_chars (default 320, max 1200). Returns JSON array of matches with filename, title, url, fetched_utc, and a compact content snippet."
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
        let max = p.max_results.unwrap_or(20);
        let mut results: Vec<SearchMatch> = Vec::new();

        for entry in &self.article_index.articles {
            if results.len() >= max {
                break;
            }
            if !date_in_range(
                entry.fetched_utc.as_deref(),
                p.date_from.as_deref(),
                p.date_to.as_deref(),
            ) {
                continue;
            }
            if re.is_match(&entry.content) {
                let snippet = build_snippet(&entry.content, &re, snippet_chars);
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

    /// Return the full markdown content of an article by filename.
    #[rmcp::tool(
        description = "Read the full markdown content of an article. Pass the filename (e.g. \"my-article.md\") as returned by list_articles or search_articles."
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
            Some(entry) => entry.content.clone(),
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
        description = "Search articles by entity tags. Provide at least one of: company, technology, product, theme (all case-insensitive substring matches). All provided filters must match (AND logic). Returns JSON array with url, fetched_utc, companies, technologies, products, themes."
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
        description = "List articles in the corpus. Optionally filter by date_from/date_to (ISO date strings, inclusive) and/or title_pattern (regex on title). Returns JSON array with filename, title, url, fetched_utc, token_count."
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

        let results: Vec<ArticleSummary> = self
            .article_index
            .articles
            .iter()
            .filter(|entry| {
                date_in_range(
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
            .collect();

        let result = serde_json::to_string(&results)
            .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}).to_string());
        engine_logging::engine_info!(
            "[tool] list_articles returned {} bytes in {}ms",
            result.len(),
            t.elapsed().as_millis()
        );
        result
    }
}

#[tool_handler]
impl ServerHandler for HarvesterMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(rmcp::model::ServerCapabilities::default()).with_server_info(
            Implementation::new("harvester-mcp", env!("CARGO_PKG_VERSION")),
        )
    }
}

impl HarvesterMcpServer {
    fn new(
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

fn rotated_log_path(log_path: &std::path::Path, index: usize) -> PathBuf {
    let file_name = log_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("mcp.log"));
    let mut rotated_name = file_name.to_os_string();
    rotated_name.push(format!(".{index}"));
    log_path.with_file_name(rotated_name)
}

fn rotate_log_files(
    log_path: &std::path::Path,
    retain_runs: usize,
) -> std::io::Result<Vec<String>> {
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut actions = Vec::new();
    if retain_runs == 0 {
        if log_path.exists() {
            std::fs::remove_file(log_path)?;
            actions.push(format!("removed previous log {:?}", log_path));
        }
        return Ok(actions);
    }

    let oldest_archive = rotated_log_path(log_path, retain_runs);
    if oldest_archive.exists() {
        std::fs::remove_file(&oldest_archive)?;
        actions.push(format!("removed oldest archive {:?}", oldest_archive));
    }

    for index in (1..retain_runs).rev() {
        let source = rotated_log_path(log_path, index);
        if !source.exists() {
            continue;
        }
        let target = rotated_log_path(log_path, index + 1);
        std::fs::rename(&source, &target)?;
        actions.push(format!("rotated {:?} -> {:?}", source, target));
    }

    if log_path.exists() {
        let target = rotated_log_path(log_path, 1);
        std::fs::rename(log_path, &target)?;
        actions.push(format!("rotated {:?} -> {:?}", log_path, target));
    }

    Ok(actions)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let overall_start = Instant::now();

    let args = Args::parse();

    let log_dir = args.log_dir.unwrap_or_else(|| args.output_dir.join("logs"));
    let log_path = log_dir.join("mcp.log");

    let rotation_actions = rotate_log_files(&log_path, args.retain_log_runs)?;
    engine_logging::initialize_to_path(&log_path);

    let session_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    engine_logging::engine_info!(
        "===== harvester_mcp session start pid={} session_id={} =====",
        std::process::id(),
        session_id
    );
    engine_logging::engine_info!("harvester_mcp starting");
    for action in rotation_actions {
        engine_logging::engine_info!("[logging] {}", action);
    }

    let t0 = Instant::now();
    let entity_index_path = args.output_dir.join(".entity_index.ron");
    let entity_index = load_entity_index(&entity_index_path);
    engine_logging::engine_info!("entity index loaded in {}ms", t0.elapsed().as_millis());

    let t1 = Instant::now();
    let summary_cache_path = args.output_dir.join(".summary_cache.ron");
    let summary_cache = load_summary_cache(&summary_cache_path);
    engine_logging::engine_info!("summary cache loaded in {}ms", t1.elapsed().as_millis());

    let t1b = Instant::now();
    let triage_cache_path = args.output_dir.join(".triage_cache.ron");
    let triage_cache = load_triage_cache(&triage_cache_path);
    engine_logging::engine_info!("triage cache loaded in {}ms", t1b.elapsed().as_millis());

    let t2 = Instant::now();
    let article_index = ArticleIndex::load(&args.output_dir);
    engine_logging::engine_info!(
        "article index: loaded {} articles in {}ms",
        article_index.articles.len(),
        t2.elapsed().as_millis()
    );

    engine_logging::engine_info!(
        "output_dir={:?} agent_model={} context_budget={} scoring_candidate_cap={} too_broad_threshold={} min_triage_priority={} retain_log_runs={}",
        args.output_dir,
        args.agent_model,
        args.context_budget,
        args.scoring_candidate_cap,
        args.too_broad_threshold,
        args.min_triage_priority,
        args.retain_log_runs
    );

    // Build URL → summary index
    let t3 = Instant::now();
    let mut summary_index: HashMap<String, SummaryCacheEntry> = HashMap::new();
    for (url, entity_entry) in &entity_index.entries {
        let Some(ref content_hash) = entity_entry.content_hash else {
            continue;
        };
        let newest = summary_cache
            .iter()
            .filter(|(k, _)| k.content_hash == *content_hash)
            .max_by_key(|(_, v)| &v.created_at_utc);
        if let Some((_, entry)) = newest {
            summary_index.insert(url.clone(), entry.clone());
        }
    }
    engine_logging::engine_info!(
        "summary index: built {} entries in {}ms",
        summary_index.len(),
        t3.elapsed().as_millis()
    );

    let t4 = Instant::now();
    let mut triage_by_hash: HashMap<String, (String, ArticleTriageResult)> = HashMap::new();
    for (key, entry) in triage_cache.iter() {
        let should_replace = triage_by_hash
            .get(&key.content_hash)
            .map(|(created_at, _)| entry.created_at_utc > *created_at)
            .unwrap_or(true);
        if should_replace {
            triage_by_hash.insert(
                key.content_hash.clone(),
                (entry.created_at_utc.clone(), entry.result.clone()),
            );
        }
    }

    let mut triage_index: HashMap<String, ArticleTriageResult> = HashMap::new();
    for (url, entity_entry) in &entity_index.entries {
        let Some(content_hash) = entity_entry.content_hash.as_ref() else {
            continue;
        };
        if let Some((_, result)) = triage_by_hash.get(content_hash) {
            triage_index.insert(url.clone(), result.clone());
        }
    }
    engine_logging::engine_info!(
        "triage index: built {} entries in {}ms",
        triage_index.len(),
        t4.elapsed().as_millis()
    );

    let agent_provider: Option<Arc<dyn LlmProvider>> = match OpenAiProvider::from_env() {
        Ok(provider) => {
            engine_logging::engine_info!("smart-query provider initialized");
            Some(Arc::new(provider))
        }
        Err(err) => {
            engine_logging::engine_warn!(
                "smart-query provider unavailable; query_knowledge_base will degrade to raw results: {}",
                err
            );
            None
        }
    };

    engine_logging::engine_info!(
        "startup complete in {}ms",
        overall_start.elapsed().as_millis()
    );

    let server = HarvesterMcpServer::new(
        args.output_dir,
        entity_index,
        summary_cache,
        article_index,
        summary_index,
        triage_index,
        SmartQueryConfig {
            agent_model: args.agent_model,
            context_budget: args.context_budget,
            scoring_candidate_cap: args.scoring_candidate_cap,
            too_broad_threshold: args.too_broad_threshold,
            min_triage_priority: args.min_triage_priority,
            agent_provider,
        },
    );
    let transport = stdio();
    server.serve(transport).await?.waiting().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article_index::ArticleEntry;
    use harvester_core::{EntityIndex, SummaryCache};
    use serde_json::Value;
    use std::collections::BTreeMap;

    fn test_server_with_articles(articles: Vec<ArticleEntry>) -> HarvesterMcpServer {
        HarvesterMcpServer::new(
            PathBuf::from("output"),
            EntityIndex {
                schema_version: 1,
                entries: BTreeMap::new(),
            },
            SummaryCache::new(),
            ArticleIndex { articles },
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
}
