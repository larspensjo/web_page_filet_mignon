//! Headless batch host for scheduled Harvester runs.

mod archive_result;
mod cli;
mod import_mode;
mod no_progress;
mod progress;
pub mod runner;
mod services;

pub use cli::Args;
pub use services::{HostServices, InterruptHandler};

/// Each batch process starts with its own fresh file log.
pub fn initialize_logging() {
    let _ = std::fs::File::create("engine.log");
    engine_logging::initialize_file_only();
}

fn read_morning_commit() -> Option<String> {
    std::env::var("HARVESTER_MORNING_COMMIT").ok()
}

/// Parse the process arguments with clap's normal help, version and diagnostics.
pub fn parse_command_line() -> Result<Args, i32> {
    let argv: Vec<_> = std::env::args_os().collect();
    let archive = argv
        .iter()
        .skip(1)
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--archive-out" || arg.as_encoded_bytes().starts_with(b"--archive-out="));
    Args::try_parse_cli(argv).map_err(|error| {
        let code = error.exit_code();
        let _ = error.print();
        if archive && code != 0 {
            let result = archive_result::ArchiveResult::new(
                archive_result::Status::UsageError,
                Some("invalid_arguments"),
            );
            let commit = read_morning_commit();
            println!("{}", result.line(commit.as_deref()));
        }
        code
    })
}

/// Run the same batch host for the binary and hosts with injected services.
pub fn run_main(args: Args, services: HostServices) -> i32 {
    use engine_logging::{engine_error, engine_info};
    let exit_code = match runner::run_with_services(args, services) {
        Ok(code) => code,
        Err(error) => {
            engine_error!("[batch] Fatal error: {error}");
            eprintln!("harvester_batch: {error}");
            2
        }
    };
    engine_info!("[batch] Exiting with code {exit_code}");
    exit_code
}

/// Run the batch host with already-parsed command-line arguments.
pub fn run(
    args: Args,
    stored_keys: Option<harvester_io::credentials::StoredKeyAccess>,
) -> Result<i32, String> {
    runner::run(args, stored_keys)
}
