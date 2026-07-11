use super::prompt_template_store;
use harvester_io::default_sources_path;
use std::path::PathBuf;

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
pub(crate) fn default_source_config_path(output_dir: &std::path::Path) -> PathBuf {
    default_sources_path(output_dir)
}

/// Default prompts directory.
pub(crate) fn prompts_directory() -> PathBuf {
    prompt_template_store::prompts_directory()
}
