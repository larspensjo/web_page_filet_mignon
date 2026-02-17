use super::{prompt_template_store, source_loader};
use std::path::PathBuf;

// Re-export the shared EffectRunner for use by the app
pub use harvester_io::EffectRunner;

/// Default output directory based on current working directory.
/// Used for backward compatibility with existing app behavior.
pub(crate) fn default_output_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("output")
}

/// Default contexts directory (relative to CWD).
pub(crate) fn contexts_directory() -> PathBuf {
    PathBuf::from("contexts")
}

/// Default source configuration file path.
pub(crate) fn default_source_config_path() -> PathBuf {
    source_loader::default_source_config_path()
}

/// Default prompts directory.
pub(crate) fn prompts_directory() -> PathBuf {
    prompt_template_store::prompts_directory()
}