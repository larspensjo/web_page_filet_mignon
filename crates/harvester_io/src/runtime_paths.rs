use std::path::PathBuf;

/// Runtime paths for all IO operations.
/// All paths are derived from explicit CLI or default values, never from process CWD.
#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub output_dir: PathBuf,
    pub sources_path: PathBuf,
    pub contexts_dir: PathBuf,
    pub prompts_dir: PathBuf,
    pub summary_cache_path: PathBuf,
    pub triage_cache_path: PathBuf,
    pub seen_set_path: PathBuf,
    pub state_path: PathBuf,
}

impl RuntimePaths {
    /// Build RuntimePaths from explicit base directories.
    pub fn new(
        output_dir: PathBuf,
        sources_path: PathBuf,
        contexts_dir: PathBuf,
        prompts_dir: PathBuf,
    ) -> Self {
        let summary_cache_path = output_dir.join(".summary_cache.json");
        let triage_cache_path = output_dir.join(".triage_cache.json");
        let seen_set_path = output_dir.join(".seen_set.json");
        let state_path = output_dir.join(".harvester_state.json");

        Self {
            output_dir,
            sources_path,
            contexts_dir,
            prompts_dir,
            summary_cache_path,
            triage_cache_path,
            seen_set_path,
            state_path,
        }
    }

    /// Build RuntimePaths with default values for a given output directory.
    pub fn with_defaults(output_dir: PathBuf) -> Self {
        Self::new(
            output_dir,
            PathBuf::from("sources.ron"),
            PathBuf::from("contexts"),
            PathBuf::from("prompts"),
        )
    }
}
