#![deny(missing_docs)]
//! Shared logging utilities for the engine workspace.
//!
//! This crate provides the `engine_*` logging macros used across the codebase
//! and a minimal test initializer for the global logger.

use std::cell::Cell;

thread_local! {
    /// Thread-local storage for the current simulation tick count.
    static SIM_TICK: Cell<u64> = const { Cell::new(0) };
}

/// Sets the simulation tick count for the current thread.
/// This should be called by the main application loop once per tick.
pub fn set_sim_tick(tick: u64) {
    SIM_TICK.with(|v| v.set(tick));
}

/// Retrieves the simulation tick count for the current thread.
/// Returns 0 if the tick has not been set.
pub fn get_sim_tick() -> u64 {
    SIM_TICK.with(|v| v.get())
}

// TODO: Replace all log:: with the macros below.

/// Logs a trace-level message using the global logging facade.
#[macro_export]
macro_rules! engine_trace {
    ($($arg:tt)*) => {{
        log::trace!($($arg)*);
    }};
}

/// Logs an info-level message using the global logging facade.
#[macro_export]
macro_rules! engine_info {
    ($($arg:tt)*) => {{
        log::info!($($arg)*);
    }};
}

/// Logs a debug-level message using the global logging facade.
#[macro_export]
macro_rules! engine_debug {
    ($($arg:tt)*) => {{
        log::debug!($($arg)*);
    }};
}

/// Logs a warn-level message using the global logging facade.
#[macro_export]
macro_rules! engine_warn {
    ($($arg:tt)*) => {{
        log::warn!($($arg)*);
    }};
}

/// Logs an error-level message using the global logging facade.
#[macro_export]
macro_rules! engine_error {
    ($($arg:tt)*) => {{
        log::error!($($arg)*);
    }};
}

/// Initializes a simple terminal logger for use in unit tests.
///
/// This safely no-ops if another logger has already been initialized.
pub fn initialize_for_tests() {
    use simplelog::{ColorChoice, CombinedLogger, Config, TermLogger, TerminalMode};

    // Use debug level in debug builds, info in release builds.
    let level = if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    // Ignore the error if a logger was already set by another test.
    let _ = CombinedLogger::init(vec![TermLogger::new(
        level,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )]);
}

/// Initializes logging for production use: both terminal and file output.
///
/// Logs to both stderr and `./engine.log` in the current working directory.
/// This safely no-ops if another logger has already been initialized.
pub fn initialize() {
    use simplelog::{ColorChoice, CombinedLogger, Config, TermLogger, TerminalMode, WriteLogger};
    use std::fs::OpenOptions;

    let level = log::LevelFilter::Info;

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("engine.log")
        .expect("Failed to open engine.log");

    // Ignore the error if a logger was already set.
    let _ = CombinedLogger::init(vec![
        TermLogger::new(
            level,
            Config::default(),
            TerminalMode::Stderr,
            ColorChoice::Auto,
        ),
        WriteLogger::new(level, Config::default(), log_file),
    ]);
}

/// Initializes file-only logging (no console output) for dry-run mode.
///
/// Logs only to `./engine.log` in the current working directory.
/// This safely no-ops if another logger has already been initialized.
pub fn initialize_file_only() {
    use simplelog::{CombinedLogger, Config, WriteLogger};
    use std::fs::OpenOptions;

    let level = log::LevelFilter::Info;

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("engine.log")
        .expect("Failed to open engine.log");

    // Ignore the error if a logger was already set.
    let _ = CombinedLogger::init(vec![WriteLogger::new(level, Config::default(), log_file)]);
}

/// Initializes file-only logging to a specified path (no console output).
///
/// Intended for processes where stdout is used as a transport (e.g. MCP stdio)
/// and stderr must not be written to by the logger.
/// Creates parent directories if they do not exist.
/// This safely no-ops if another logger has already been initialized.
pub fn initialize_to_path(log_path: &std::path::Path) {
    use simplelog::{CombinedLogger, Config, WriteLogger};
    use std::fs::OpenOptions;

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("Failed to create log directory {:?}: {}", parent, e));
    }

    let level = log::LevelFilter::Info;

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .unwrap_or_else(|e| panic!("Failed to open log file {:?}: {}", log_path, e));

    // Ignore the error if a logger was already set.
    let _ = CombinedLogger::init(vec![WriteLogger::new(level, Config::default(), log_file)]);
}
