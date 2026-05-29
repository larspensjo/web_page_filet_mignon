use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use harvester_core::{EntityIndex, SummaryCache, SummaryCacheEntry};
use harvester_engine::llm::LlmProvider;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{Implementation, ServerInfo};
use rmcp::{tool_handler, ServerHandler};

use crate::article_index::ArticleIndex;
use crate::smart_query;
use crate::smart_query::SmartQueryEngine;

#[derive(Parser, Debug)]
#[command(name = "harvester_mcp", about = "Harvester MCP server")]
pub(crate) struct Args {
    /// Output directory containing harvested data
    #[arg(long, default_value = "./output")]
    pub(crate) output_dir: PathBuf,

    /// LLM model ID to use for agent operations
    #[arg(long, default_value = harvester_engine::llm::DEFAULT_TRIAGE_MODEL)]
    pub(crate) agent_model: String,

    /// Context budget in tokens
    #[arg(long, default_value_t = 4000)]
    pub(crate) context_budget: usize,

    /// Maximum number of candidates to send to LLM scoring
    #[arg(long, default_value_t = smart_query::DEFAULT_MAX_SCORING_CANDIDATES)]
    pub(crate) scoring_candidate_cap: usize,

    /// Broad-query threshold on eligible candidates before early return
    #[arg(long, default_value_t = smart_query::DEFAULT_TOO_BROAD_THRESHOLD)]
    pub(crate) too_broad_threshold: usize,

    /// Minimum triage priority required for articles to be eligible
    #[arg(long, default_value_t = smart_query::DEFAULT_MIN_TRIAGE_PRIORITY)]
    pub(crate) min_triage_priority: u8,

    /// Number of previous log runs to retain as mcp.log.N archives
    #[arg(long, default_value_t = 9)]
    pub(crate) retain_log_runs: usize,

    /// Directory for log files (defaults to <output-dir>/logs)
    #[arg(long)]
    pub(crate) log_dir: Option<PathBuf>,
}

#[derive(Clone)]
pub(crate) struct HarvesterMcpServer {
    #[allow(dead_code)]
    pub(crate) output_dir: PathBuf,
    pub(crate) entity_index: Arc<EntityIndex>,
    #[allow(dead_code)]
    pub(crate) summary_cache: SummaryCache,
    pub(crate) article_index: Arc<ArticleIndex>,
    pub(crate) summary_index: Arc<HashMap<String, SummaryCacheEntry>>,
    pub(crate) smart_query_engine: Arc<SmartQueryEngine>,
    pub(crate) tool_router: ToolRouter<Self>,
}

pub(crate) struct SmartQueryConfig {
    pub(crate) agent_model: String,
    pub(crate) context_budget: usize,
    pub(crate) scoring_candidate_cap: usize,
    pub(crate) too_broad_threshold: usize,
    pub(crate) min_triage_priority: u8,
    pub(crate) agent_provider: Option<Arc<dyn LlmProvider>>,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for HarvesterMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_server_info(Implementation::new(
            "harvester-mcp",
            env!("CARGO_PKG_VERSION"),
        ))
    }
}
