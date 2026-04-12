mod article_index;
mod smart_query;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use article_index::ArticleIndex;
use clap::Parser;
use harvester_core::{EntityIndex, SummaryCache, SummaryCacheEntry};
use harvester_engine::llm::{LlmProvider, OpenAiProvider};
use harvester_io::{load_entity_index, load_summary_cache};
use regex::Regex;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{tool_handler, tool_router, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use smart_query::{QueryKnowledgeBaseInput, SmartQueryEngine};

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

/// Build a snippet: up to 3 matching lines with 1 line of context before + after.
fn build_snippet(content: &str, re: &Regex) -> String {
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
    included
        .iter()
        .map(|&i| lines[i])
        .collect::<Vec<_>>()
        .join("\n")
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
        description = "Search article content using a regex pattern. Optionally filter by date range (date_from/date_to as ISO date strings) and cap results with max_results (default 20). Returns JSON array of matches with filename, title, url, fetched_utc, and a content snippet."
    )]
    async fn search_articles(&self, Parameters(p): Parameters<SearchArticlesParams>) -> String {
        let t = std::time::Instant::now();
        engine_logging::engine_info!(
            "[tool] search_articles called with pattern={:?} date_from={:?} date_to={:?} max_results={:?}",
            p.pattern, p.date_from, p.date_to, p.max_results
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
                let snippet = build_snippet(&entry.content, &re);
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
            "[tool] search_articles returned {} bytes in {}ms",
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
        description = "Answer a free-text question from the article corpus. Uses a cheap-model agent layer for query expansion, relevance scoring, and digest assembly. Returns JSON with mode, synthesis, ranked_articles, warnings, and total_token_count."
    )]
    async fn query_knowledge_base(
        &self,
        Parameters(p): Parameters<QueryKnowledgeBaseParams>,
    ) -> String {
        let t = std::time::Instant::now();
        engine_logging::engine_info!(
            "[tool] query_knowledge_base called with question={:?} max_results={:?} scope_entities={:?} scope_date_from={:?} scope_date_to={:?}",
            p.question,
            p.max_results,
            p.scope_entities,
            p.scope_date_from,
            p.scope_date_to
        );

        let response = self
            .smart_query_engine
            .query(QueryKnowledgeBaseInput {
                question: p.question,
                max_results: p.max_results.unwrap_or(10),
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
        smart_query_config: SmartQueryConfig,
    ) -> Self {
        let entity_index = Arc::new(entity_index);
        let article_index = Arc::new(article_index);
        let summary_index = Arc::new(summary_index);
        let smart_query_engine = Arc::new(SmartQueryEngine::new(
            article_index.clone(),
            entity_index.clone(),
            summary_index.clone(),
            smart_query_config.agent_provider,
            smart_query_config.agent_model,
            smart_query_config.context_budget,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let overall_start = Instant::now();

    let args = Args::parse();

    let log_dir = args.log_dir.unwrap_or_else(|| args.output_dir.join("logs"));
    let log_path = log_dir.join("mcp.log");

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

    let t0 = Instant::now();
    let entity_index_path = args.output_dir.join(".entity_index.ron");
    let entity_index = load_entity_index(&entity_index_path);
    engine_logging::engine_info!("entity index loaded in {}ms", t0.elapsed().as_millis());

    let t1 = Instant::now();
    let summary_cache_path = args.output_dir.join(".summary_cache.ron");
    let summary_cache = load_summary_cache(&summary_cache_path);
    engine_logging::engine_info!("summary cache loaded in {}ms", t1.elapsed().as_millis());

    let t2 = Instant::now();
    let article_index = ArticleIndex::load(&args.output_dir);
    engine_logging::engine_info!(
        "article index: loaded {} articles in {}ms",
        article_index.articles.len(),
        t2.elapsed().as_millis()
    );

    engine_logging::engine_info!(
        "output_dir={:?} agent_model={} context_budget={}",
        args.output_dir,
        args.agent_model,
        args.context_budget
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
        SmartQueryConfig {
            agent_model: args.agent_model,
            context_budget: args.context_budget,
            agent_provider,
        },
    );
    let transport = stdio();
    server.serve(transport).await?.waiting().await?;

    Ok(())
}
