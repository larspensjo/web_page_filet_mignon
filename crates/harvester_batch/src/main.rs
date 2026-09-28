//! Thin command-line entry point for the batch library.

use engine_logging::{engine_error, engine_info};
use harvester_batch::Args;
use std::fs::File;
use std::process;

fn main() {
    let args = Args::parse();

    // Batch runs should always start with a fresh per-run log file.
    let _ = File::create("engine.log");

    // Batch mode is file-only to keep stderr/stdout clean during scheduled runs.
    engine_logging::initialize_file_only();

    let exit_code = match harvester_batch::run(args) {
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
