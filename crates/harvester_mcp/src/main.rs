mod article_index;

use std::path::PathBuf;
use std::time::Instant;

use article_index::{ArticleIndex};
use clap::Parser;
use harvester_core::{EntityIndex, SummaryCache};
use harvester_io::{load_entity_index, load_summary_cache};
use regex::Regex;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerInfo};
use rmcp::{ServerHandler, ServiceExt, tool_handler, tool_router};
use rmcp::transport::stdio;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    #[allow(dead_code)]
    entity_index: EntityIndex,
    #[allow(dead_code)]
    summary_cache: SummaryCache,
    article_index: std::sync::Arc<ArticleIndex>,
    tool_router: ToolRouter<Self>,
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

// ── Result structs ───────────────────────────────────────────────────────────

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
    for (i, line) in lines.iter().enumerate() {
        if re.is_match(line) {
            let start = i.saturating_sub(1);
            let end = (i + 1).min(lines.len() - 1);
            for j in start..=end {
                if !included.contains(&j) {
                    included.push(j);
                }
            }
            if included.iter().filter(|&&j| re.is_match(lines[j])).count() >= 3 {
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
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// Search article content with a regex pattern, optional date range, and result cap.
    #[rmcp::tool(description = "Search article content using a regex pattern. Optionally filter by date range (date_from/date_to as ISO date strings) and cap results with max_results (default 20). Returns JSON array of matches with filename, title, url, fetched_utc, and a content snippet.")]
    async fn search_articles(&self, Parameters(p): Parameters<SearchArticlesParams>) -> String {
        let re = match Regex::new(&p.pattern) {
            Ok(r) => r,
            Err(e) => return format!("{{\"error\": \"invalid regex: {}\"}}", e),
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

        serde_json::to_string(&results).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
    }

    /// Return the full markdown content of an article by filename.
    #[rmcp::tool(description = "Read the full markdown content of an article. Pass the filename (e.g. \"my-article.md\") as returned by list_articles or search_articles.")]
    async fn read_article(&self, Parameters(p): Parameters<ReadArticleParams>) -> String {
        match self
            .article_index
            .articles
            .iter()
            .find(|a| a.filename == p.filename)
        {
            Some(entry) => entry.content.clone(),
            None => format!("{{\"error\": \"article not found: {}\"}}", p.filename),
        }
    }

    /// List articles, optionally filtered by date range and/or title regex.
    #[rmcp::tool(description = "List articles in the corpus. Optionally filter by date_from/date_to (ISO date strings, inclusive) and/or title_pattern (regex on title). Returns JSON array with filename, title, url, fetched_utc, token_count.")]
    async fn list_articles(&self, Parameters(p): Parameters<ListArticlesParams>) -> String {
        let title_re = if let Some(pat) = &p.title_pattern {
            match Regex::new(pat) {
                Ok(r) => Some(r),
                Err(e) => {
                    return format!("{{\"error\": \"invalid title_pattern regex: {}\"}}", e)
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

        serde_json::to_string(&results).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
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
    ) -> Self {
        Self {
            output_dir,
            entity_index,
            summary_cache,
            article_index: std::sync::Arc::new(article_index),
            tool_router: Self::tool_router(),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let overall_start = Instant::now();

    let args = Args::parse();

    let log_dir = args
        .log_dir
        .unwrap_or_else(|| args.output_dir.join("logs"));
    let log_path = log_dir.join("mcp.log");

    engine_logging::initialize_to_path(&log_path);

    engine_logging::engine_info!("harvester_mcp starting");

    let t0 = Instant::now();
    let entity_index_path = args.output_dir.join(".entity_index.ron");
    let entity_index = load_entity_index(&entity_index_path);
    engine_logging::engine_info!(
        "entity index loaded in {}ms",
        t0.elapsed().as_millis()
    );

    let t1 = Instant::now();
    let summary_cache_path = args.output_dir.join(".summary_cache.ron");
    let summary_cache = load_summary_cache(&summary_cache_path);
    engine_logging::engine_info!(
        "summary cache loaded in {}ms",
        t1.elapsed().as_millis()
    );

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

    engine_logging::engine_info!(
        "startup complete in {}ms",
        overall_start.elapsed().as_millis()
    );

    let server = HarvesterMcpServer::new(args.output_dir, entity_index, summary_cache, article_index);
    let transport = stdio();
    server.serve(transport).await?.waiting().await?;

    Ok(())
}
