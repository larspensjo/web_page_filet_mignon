//! Platform logging initialization for harvester_app.
//!
//! Writes logs to `./engine.log` in the current working directory.

use std::fs::File;
use std::path::PathBuf;

use log::LevelFilter;
use simplelog::{
    ColorChoice, CombinedLogger, Config, ConfigBuilder, SharedLogger, TermLogger, TerminalMode,
    WriteLogger,
};

/// Destination for log output.
#[allow(dead_code)]
pub enum LogDestination {
    /// Write to ./engine.log in current directory.
    File,
    /// Write to terminal (stdout).
    Terminal,
    /// Write to both file and terminal.
    Both,
}

/// Initialize the logger with the specified destination.
///
/// For `LogDestination::File` or `Both`, creates `./engine.log` in the
/// current working directory.
pub fn initialize(destination: LogDestination) {
    let level = LevelFilter::Info;

    let config = build_config();

    let loggers: Vec<Box<dyn SharedLogger>> = match destination {
        LogDestination::File => {
            if let Some(file_logger) = create_file_logger(level, config) {
                vec![file_logger]
            } else {
                return;
            }
        }
        LogDestination::Terminal => {
            vec![TermLogger::new(
                level,
                config,
                TerminalMode::Mixed,
                ColorChoice::Auto,
            )]
        }
        LogDestination::Both => {
            let mut loggers: Vec<Box<dyn SharedLogger>> = vec![TermLogger::new(
                level,
                config.clone(),
                TerminalMode::Mixed,
                ColorChoice::Auto,
            )];
            if let Some(file_logger) = create_file_logger(level, config) {
                loggers.push(file_logger);
            }
            loggers
        }
    };

    let _ = CombinedLogger::init(loggers);
}

fn build_config() -> Config {
    ConfigBuilder::new()
        .set_time_format_rfc3339()
        .set_target_level(LevelFilter::Error)
        .build()
}

fn create_file_logger(level: LevelFilter, config: Config) -> Option<Box<WriteLogger<File>>> {
    let log_path = PathBuf::from("./engine.log");
    match File::create(&log_path) {
        Ok(file) => Some(WriteLogger::new(level, config, file)),
        Err(err) => {
            eprintln!("Warning: Could not create log file at {:?}: {}", log_path, err);
            None
        }
    }
}
