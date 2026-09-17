use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    harvester_eval::cli::run(harvester_eval::cli::Cli::parse())
}
