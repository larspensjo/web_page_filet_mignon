mod article_index;
mod log_rotation;
mod server;
mod smart_query;
mod tools;
mod util;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use article_index::ArticleIndex;
use clap::Parser;
use harvester_core::{ArticleTriageResult, SummaryCacheEntry};
use harvester_engine::llm::{LlmProvider, OpenAiProvider};
use harvester_io::{load_entity_index, load_summary_cache, load_triage_cache};
use log_rotation::rotate_log_files;
use rmcp::transport::stdio;
use rmcp::ServiceExt;
use server::{Args, HarvesterMcpServer, SmartQueryConfig};

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
