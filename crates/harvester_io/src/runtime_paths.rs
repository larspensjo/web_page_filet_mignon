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
    pub briefing_history_path: PathBuf,
    pub briefing_checkpoint_path: PathBuf,
}

impl RuntimePaths {
    /// Build RuntimePaths from explicit base directories.
    pub fn new(
        output_dir: PathBuf,
        sources_path: PathBuf,
        contexts_dir: PathBuf,
        prompts_dir: PathBuf,
    ) -> Self {
        let summary_cache_path = output_dir.join(".summary_cache.ron");
        let triage_cache_path = output_dir.join(".triage_cache.ron");
        let seen_set_path = output_dir.join(".seen_set.ron");
        let state_path = output_dir.join(".harvester_state.ron");
        let briefing_history_path = output_dir.join(".briefing_history.ron");
        let briefing_checkpoint_path = output_dir.join(".briefing_checkpoint.ron");

        Self {
            output_dir,
            sources_path,
            contexts_dir,
            prompts_dir,
            summary_cache_path,
            triage_cache_path,
            seen_set_path,
            state_path,
            briefing_history_path,
            briefing_checkpoint_path,
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

#[cfg(test)]
mod tests {
    use super::RuntimePaths;
    use crate::{load_summary_cache, persist_summary_cache};
    use chrono::Utc;
    use harvester_core::{ArticleSummaryResult, SummaryCache, SummaryCacheEntry, SummaryCacheKey};
    use harvester_engine::llm::prompt::PromptId;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn default_paths_use_ron_extensions() {
        let dir = tempdir().expect("tempdir");
        let paths = RuntimePaths::with_defaults(dir.path().to_path_buf());

        assert!(paths.summary_cache_path.ends_with(".summary_cache.ron"));
        assert!(paths.triage_cache_path.ends_with(".triage_cache.ron"));
        assert!(paths.seen_set_path.ends_with(".seen_set.ron"));
        assert!(paths.state_path.ends_with(".harvester_state.ron"));
    }

    #[test]
    fn briefing_history_path_is_in_output_dir() {
        let paths = RuntimePaths::new(
            PathBuf::from("/tmp/out"),
            PathBuf::from("/tmp/sources.ron"),
            PathBuf::from("/tmp/contexts"),
            PathBuf::from("/tmp/prompts"),
        );
        assert_eq!(
            paths.briefing_history_path,
            PathBuf::from("/tmp/out/.briefing_history.ron")
        );
    }

    #[test]
    fn summary_cache_roundtrips_using_runtime_paths() {
        let dir = tempdir().expect("tempdir");
        let paths = RuntimePaths::with_defaults(dir.path().to_path_buf());

        let mut cache = SummaryCache::new();
        let key = SummaryCacheKey::try_new(
            "hash",
            PromptId::ArticleSummary,
            Some(1),
            Some("gpt-4o-mini"),
            &[],
        )
        .expect("key");
        let entry = SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "Title".to_string(),
                summary: "Summary".to_string(),
                key_points: vec!["point".to_string()],
                input_tokens: 1,
                output_tokens: 1,
            },
            created_at_utc: Utc::now().to_rfc3339(),
        };
        cache.insert(key.clone(), entry);

        persist_summary_cache(&cache, &paths.summary_cache_path).expect("persist");
        let reloaded = load_summary_cache(&paths.summary_cache_path);

        assert!(reloaded.lookup(&key).is_some());
    }
}
