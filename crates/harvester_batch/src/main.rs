//! Harvester batch runner - headless CLI for scheduled execution

mod cli;
mod lock;
mod progress;
mod runner;

use cli::Args;
use engine_logging::{engine_error, engine_info};
use std::fs::File;
use std::process;

fn main() {
    let args = Args::parse();

    // Batch runs should always start with a fresh per-run log file.
    let _ = File::create("engine.log");

    // Batch mode is file-only to keep stderr/stdout clean during scheduled runs.
    engine_logging::initialize_file_only();

    engine_info!("[batch] Starting harvester_batch");
    engine_info!("[batch] output_dir: {:?}", args.output_dir);
    engine_info!("[batch] sources: {:?}", args.sources);
    engine_info!("[batch] dry_run: {}", args.dry_run);
    engine_info!("[batch] single_shot: {}", args.single_shot);
    engine_info!(
        "[batch] refresh_stale_summaries_limit: {:?}",
        args.refresh_stale_summaries_limit
    );

    let exit_code = match runner::run(args) {
        Ok(code) => code,
        Err(err) => {
            engine_error!("[batch] Fatal error: {}", err);
            eprintln!("harvester_batch: {}", err);
            2
        }
    };

    engine_info!("[batch] Exiting with code {}", exit_code);
    process::exit(exit_code);
}
