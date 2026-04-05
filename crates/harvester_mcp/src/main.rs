use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use harvester_core::{EntityIndex, SummaryCache};
use harvester_io::{load_entity_index, load_summary_cache};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{Implementation, ServerInfo};
use rmcp::{ServerHandler, ServiceExt, tool_handler, tool_router};
use rmcp::transport::stdio;

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
    output_dir: PathBuf,
    entity_index: EntityIndex,
    summary_cache: SummaryCache,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl HarvesterMcpServer {
    /// Return the server version. Placeholder tool for initial skeleton.
    #[rmcp::tool(description = "Return the harvester-mcp server version.")]
    async fn server_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
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
    fn new(output_dir: PathBuf, entity_index: EntityIndex, summary_cache: SummaryCache) -> Self {
        Self {
            output_dir,
            entity_index,
            summary_cache,
            tool_router: Self::tool_router(),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    engine_logging::engine_info!(
        "output_dir={:?} agent_model={} context_budget={}",
        args.output_dir,
        args.agent_model,
        args.context_budget
    );

    let server = HarvesterMcpServer::new(args.output_dir, entity_index, summary_cache);
    let transport = stdio();
    server.serve(transport).await?.waiting().await?;

    Ok(())
}
