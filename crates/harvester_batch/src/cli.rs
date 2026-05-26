use clap::Parser;
use std::path::PathBuf;

const DEFAULT_POLL_INTERVAL_MINUTES: u32 = 15;
const DEFAULT_LLM_CONCURRENCY: usize = 12;
const MAX_LLM_CONCURRENCY: usize = 12;

fn parse_signal_candidate_cap(value: &str) -> Result<usize, String> {
    let cap = value
        .parse::<usize>()
        .map_err(|_| format!("invalid usize value: {value}"))?;
    if cap == 0 {
        return Err("value must be greater than or equal to 1".to_string());
    }
    Ok(cap)
}

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

    /// Maximum concurrent LLM requests (1-12)
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

    /// Single-shot mode: run one full cycle (poll + triage + persist) and exit
    #[arg(long, conflicts_with = "dry_run")]
    pub single_shot: bool,

    /// Wait time in minutes between poll cycles (1-1440)
    #[arg(long, default_value_t = DEFAULT_POLL_INTERVAL_MINUTES)]
    pub poll_interval: u32,

    /// Set the briefing time-filter checkpoint to the given RFC3339 timestamp
    #[arg(long, value_name = "RFC3339")]
    pub set_briefing_since: Option<String>,

    /// Set the briefing time-filter checkpoint to the current UTC time
    #[arg(long)]
    pub set_briefing_since_now: bool,

    /// Clear the briefing time-filter checkpoint (include all articles)
    #[arg(long)]
    pub clear_briefing_since: bool,

    /// Print the current briefing time-filter checkpoint and exit
    #[arg(long)]
    pub show_briefing_since: bool,

    /// Import browser-saved .htm/.html files from this directory (conflicts with --dry-run and --single-shot)
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with = "dry_run",
        conflicts_with = "single_shot"
    )]
    pub import_saved_web_dir: Option<PathBuf>,

    /// Refresh up to N article summaries that are missing the current summary prompt/model/context cache key
    #[arg(
        long,
        value_name = "N",
        conflicts_with = "dry_run",
        conflicts_with = "single_shot",
        conflicts_with = "import_saved_web_dir",
        conflicts_with = "set_briefing_since",
        conflicts_with = "set_briefing_since_now",
        conflicts_with = "clear_briefing_since",
        conflicts_with = "show_briefing_since"
    )]
    pub refresh_stale_summaries_limit: Option<usize>,

    /// Minimum signal_score (0..=100) for inclusion. Default 60.
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100), value_name = "0..=100")]
    pub signal_candidate_threshold: Option<u8>,

    /// Hard cap on selected candidate count. Default 25.
    #[arg(long, value_parser = parse_signal_candidate_cap, value_name = "N")]
    pub signal_candidate_cap: Option<usize>,
}

/// A resolved checkpoint management command.
#[derive(Debug)]
pub enum CheckpointCommand {
    Set(String),
    SetNow,
    Clear,
    Show,
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
        self.llm_concurrency = self.llm_concurrency.clamp(1, MAX_LLM_CONCURRENCY);

        // Clamp poll_interval to valid range (1 minute to 24 hours)
        self.poll_interval = self.poll_interval.clamp(1, 1440);
    }

    /// Resolve the checkpoint flags into a single command, or `None` if no flags are set.
    /// Returns `Err` if more than one checkpoint flag is set simultaneously.
    pub fn checkpoint_command(&self) -> Result<Option<CheckpointCommand>, String> {
        let count = self.set_briefing_since.is_some() as usize
            + self.set_briefing_since_now as usize
            + self.clear_briefing_since as usize
            + self.show_briefing_since as usize;
        if count > 1 {
            return Err(
                "--set-briefing-since, --set-briefing-since-now, --clear-briefing-since, \
                 and --show-briefing-since are mutually exclusive"
                    .to_string(),
            );
        }
        if self.show_briefing_since {
            return Ok(Some(CheckpointCommand::Show));
        }
        if self.set_briefing_since_now {
            return Ok(Some(CheckpointCommand::SetNow));
        }
        if self.clear_briefing_since {
            return Ok(Some(CheckpointCommand::Clear));
        }
        if let Some(ts) = &self.set_briefing_since {
            chrono::DateTime::parse_from_rfc3339(ts).map_err(|_| {
                format!(
                    "Invalid timestamp format. Expected RFC3339, e.g. 2025-01-01T12:00:00Z\nGot: {}",
                    ts
                )
            })?;
            return Ok(Some(CheckpointCommand::Set(ts.clone())));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_command_none_when_no_flags() {
        let args = Args::parse_from(&["harvester_batch"]);
        assert!(matches!(args.checkpoint_command(), Ok(None)));
    }

    #[test]
    fn checkpoint_command_show() {
        let args = Args::parse_from(&["harvester_batch", "--show-briefing-since"]);
        assert!(matches!(
            args.checkpoint_command(),
            Ok(Some(CheckpointCommand::Show))
        ));
    }

    #[test]
    fn checkpoint_command_set_now() {
        let args = Args::parse_from(&["harvester_batch", "--set-briefing-since-now"]);
        assert!(matches!(
            args.checkpoint_command(),
            Ok(Some(CheckpointCommand::SetNow))
        ));
    }

    #[test]
    fn checkpoint_command_clear() {
        let args = Args::parse_from(&["harvester_batch", "--clear-briefing-since"]);
        assert!(matches!(
            args.checkpoint_command(),
            Ok(Some(CheckpointCommand::Clear))
        ));
    }

    #[test]
    fn checkpoint_command_valid_set_since() {
        let args = Args::parse_from(&[
            "harvester_batch",
            "--set-briefing-since",
            "2025-01-01T12:00:00Z",
        ]);
        match args.checkpoint_command() {
            Ok(Some(CheckpointCommand::Set(ts))) => {
                assert_eq!(ts, "2025-01-01T12:00:00Z");
            }
            other => panic!("unexpected result: {:?}", other.map(|_| "Ok")),
        }
    }

    #[test]
    fn checkpoint_command_invalid_timestamp_returns_err() {
        let args =
            Args::parse_from(&["harvester_batch", "--set-briefing-since", "not-a-timestamp"]);
        let err = args.checkpoint_command().unwrap_err();
        assert!(err.contains("Invalid timestamp format"));
        assert!(err.contains("RFC3339"));
    }

    #[test]
    fn checkpoint_command_rejects_multiple_flags() {
        let args = Args::parse_from(&[
            "harvester_batch",
            "--set-briefing-since-now",
            "--show-briefing-since",
        ]);
        assert!(args.checkpoint_command().is_err());
    }

    #[test]
    fn explicit_values_are_parsed_and_applied() {
        let args = Args::parse_from(&[
            "harvester_batch",
            "--sources",
            "custom_sources.ron",
            "--output-dir",
            "custom_output",
            "--contexts-dir",
            "custom_contexts",
            "--prompts-dir",
            "custom_prompts",
            "--llm-concurrency",
            "4",
            "--poll-interval",
            "30",
            "--dry-run",
            "--force-unlock",
            "--allow-unsupported-sources",
        ]);
        assert_eq!(args.sources, PathBuf::from("custom_sources.ron"));
        assert_eq!(args.output_dir, PathBuf::from("custom_output"));
        assert_eq!(args.contexts_dir, PathBuf::from("custom_contexts"));
        assert_eq!(args.prompts_dir, PathBuf::from("custom_prompts"));
        assert_eq!(args.llm_concurrency, 4);
        assert_eq!(args.poll_interval, 30);
        assert!(args.dry_run);
        assert!(!args.single_shot);
        assert!(args.force_unlock);
        assert!(args.allow_unsupported_sources);
    }

    #[test]
    fn single_shot_flag_is_parsed() {
        let args = Args::parse_from(&["harvester_batch", "--single-shot"]);
        assert!(args.single_shot);
    }

    #[test]
    fn single_shot_conflicts_with_dry_run() {
        let result =
            <Args as Parser>::try_parse_from(["harvester_batch", "--single-shot", "--dry-run"]);
        assert!(result.is_err());
    }

    #[test]
    fn llm_concurrency_defaults_to_doubled_summary_parallelism() {
        let args = Args::parse_from(&["harvester_batch"]);
        assert_eq!(args.llm_concurrency, DEFAULT_LLM_CONCURRENCY);
        assert_eq!(args.llm_concurrency, 12);
    }

    #[test]
    fn llm_concurrency_is_clamped() {
        let args = Args::parse_from(&["harvester_batch", "--llm-concurrency", "999"]);
        assert_eq!(args.llm_concurrency, MAX_LLM_CONCURRENCY);

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

    #[test]
    fn refresh_stale_summaries_limit_is_parsed() {
        let args = Args::parse_from(&["harvester_batch", "--refresh-stale-summaries-limit", "100"]);
        assert_eq!(args.refresh_stale_summaries_limit, Some(100));
    }

    #[test]
    fn refresh_stale_summaries_limit_conflicts_with_dry_run() {
        let result = <Args as Parser>::try_parse_from([
            "harvester_batch",
            "--refresh-stale-summaries-limit",
            "100",
            "--dry-run",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn signal_candidate_threshold_parses() {
        let args = Args::try_parse_from(["harvester_batch", "--signal-candidate-threshold", "75"])
            .unwrap();
        assert_eq!(args.signal_candidate_threshold, Some(75));
    }

    #[test]
    fn signal_candidate_cap_parses() {
        let args =
            Args::try_parse_from(["harvester_batch", "--signal-candidate-cap", "15"]).unwrap();
        assert_eq!(args.signal_candidate_cap, Some(15));
    }

    #[test]
    fn signal_candidate_cap_rejects_zero() {
        let result = Args::try_parse_from(["harvester_batch", "--signal-candidate-cap", "0"]);
        assert!(result.is_err());
    }

    #[test]
    fn signal_candidate_threshold_rejects_over_100() {
        let result =
            Args::try_parse_from(["harvester_batch", "--signal-candidate-threshold", "101"]);
        assert!(result.is_err());
    }
}
