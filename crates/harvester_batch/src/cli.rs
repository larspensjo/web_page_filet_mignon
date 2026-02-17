use clap::Parser;
use std::path::PathBuf;

const DEFAULT_POLL_INTERVAL_MINUTES: u32 = 15;
const DEFAULT_LLM_CONCURRENCY: usize = 3;

/// Harvester batch runner - headless mode for scheduled execution
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to sources configuration file
    #[arg(long, default_value = "sources.ron")]
    pub sources: PathBuf,

    /// Output directory for downloaded content and state
    #[arg(long, default_value = "output")]
    pub output_dir: PathBuf,

    /// Contexts directory for prompt context files
    #[arg(long, default_value = "contexts")]
    pub contexts_dir: PathBuf,

    /// Prompts directory for prompt template files
    #[arg(long, default_value = "prompts")]
    pub prompts_dir: PathBuf,

    /// Maximum concurrent LLM requests (1-10)
    #[arg(long, default_value_t = DEFAULT_LLM_CONCURRENCY)]
    pub llm_concurrency: usize,

    /// Force unlock if lock file exists (use with caution)
    #[arg(long)]
    pub force_unlock: bool,

    /// Allow running with unsupported source types (downgrades errors to warnings)
    #[arg(long)]
    pub allow_unsupported_sources: bool,

    /// Dry-run mode: poll sources and show what would be processed, but don't download or triage
    #[arg(long)]
    pub dry_run: bool,

    /// Wait time in minutes between poll cycles (1-1440)
    #[arg(long, default_value_t = DEFAULT_POLL_INTERVAL_MINUTES)]
    pub poll_interval: u32,
}

impl Args {
    /// Parse command-line arguments with automatic clamping of values.
    pub fn parse() -> Self {
        let mut args = <Args as Parser>::parse();
        args.clamp_values();
        args
    }

    /// Parse arguments from a slice (used for testing).
    #[cfg(test)]
    pub fn parse_from(args: &[&str]) -> Self {
        let mut parsed = <Args as Parser>::parse_from(args);
        parsed.clamp_values();
        parsed
    }

    /// Clamp configuration values to valid ranges.
    fn clamp_values(&mut self) {
        // Clamp llm_concurrency to valid range
        self.llm_concurrency = self.llm_concurrency.clamp(1, 10);

        // Clamp poll_interval to valid range (1 minute to 24 hours)
        self.poll_interval = self.poll_interval.clamp(1, 1440);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_are_reasonable() {
        let args = Args::parse_from(&["harvester_batch"]);
        assert_eq!(args.sources, PathBuf::from("sources.ron"));
        assert_eq!(args.output_dir, PathBuf::from("output"));
        assert_eq!(args.llm_concurrency, 3);
        assert_eq!(args.poll_interval, 15);
        assert!(!args.dry_run);
        assert!(!args.force_unlock);
    }

    #[test]
    fn llm_concurrency_is_clamped() {
        let args = Args::parse_from(&["harvester_batch", "--llm-concurrency", "999"]);
        assert_eq!(args.llm_concurrency, 10);

        let args = Args::parse_from(&["harvester_batch", "--llm-concurrency", "0"]);
        assert_eq!(args.llm_concurrency, 1);
    }

    #[test]
    fn poll_interval_is_clamped() {
        let args = Args::parse_from(&["harvester_batch", "--poll-interval", "9999"]);
        assert_eq!(args.poll_interval, 1440);

        let args = Args::parse_from(&["harvester_batch", "--poll-interval", "0"]);
        assert_eq!(args.poll_interval, 1);
    }
}
