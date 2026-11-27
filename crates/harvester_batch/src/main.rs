//! Thin command-line entry point for the batch library.

use harvester_batch::{initialize_logging, parse_command_line, run_main, HostServices};
use std::process;

fn main() {
    let args = match parse_command_line() {
        Ok(args) => args,
        Err(code) => process::exit(code),
    };
    initialize_logging();
    let services = HostServices::production(args.use_stored_keys);
    process::exit(run_main(args, services));
}
