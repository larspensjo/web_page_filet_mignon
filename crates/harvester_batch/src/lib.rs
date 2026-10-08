//! Headless batch host for scheduled Harvester runs.

mod cli;
mod import_mode;
mod no_progress;
mod progress;
pub mod runner;

pub use cli::Args;

/// Run the batch host with already-parsed command-line arguments.
pub fn run(args: Args) -> Result<i32, String> {
    runner::run(args)
}
