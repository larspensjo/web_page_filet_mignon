use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    harvester_eval::cli::run_async(harvester_eval::cli::Cli::parse()).await
}
