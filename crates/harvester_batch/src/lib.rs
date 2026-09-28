//! Headless batch host for scheduled Harvester runs.

mod batch_coordinator;
mod batch_manifest;
mod cli;
mod import_mode;
mod progress;
pub mod runner;
mod summary_refresh;

pub use cli::Args;

/// Run the batch host with already-parsed command-line arguments.
pub fn run(args: Args) -> Result<i32, String> {
    runner::run(args)
}
