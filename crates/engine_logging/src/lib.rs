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

const IGNORED_LOG_TARGETS: &[&str] = &["rustls_platform_verifier"];

fn logger_config() -> simplelog::Config {
    let mut builder = simplelog::ConfigBuilder::new();
    for target in IGNORED_LOG_TARGETS {
        builder.add_filter_ignore_str(target);
    }
    builder.build()
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
    use simplelog::{ColorChoice, CombinedLogger, TermLogger, TerminalMode};

    // Use debug level in debug builds, info in release builds.
    let level = if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    // Ignore the error if a logger was already set by another test.
    let _ = CombinedLogger::init(vec![TermLogger::new(
        level,
        logger_config(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )]);
}

/// Initializes logging for production use: both terminal and file output.
///
/// Logs to both stderr and `./engine.log` in the current working directory.
/// This safely no-ops if another logger has already been initialized.
pub fn initialize() {
    use simplelog::{ColorChoice, CombinedLogger, TermLogger, TerminalMode, WriteLogger};
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
            logger_config(),
            TerminalMode::Stderr,
            ColorChoice::Auto,
        ),
        WriteLogger::new(level, logger_config(), log_file),
    ]);
}

/// Initializes file-only logging (no console output) for dry-run mode.
///
/// Logs only to `./engine.log` in the current working directory.
/// This safely no-ops if another logger has already been initialized.
pub fn initialize_file_only() {
    use simplelog::{CombinedLogger, WriteLogger};
    use std::fs::OpenOptions;

    let level = log::LevelFilter::Info;

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("engine.log")
        .expect("Failed to open engine.log");

    // Ignore the error if a logger was already set.
    let _ = CombinedLogger::init(vec![WriteLogger::new(level, logger_config(), log_file)]);
}
