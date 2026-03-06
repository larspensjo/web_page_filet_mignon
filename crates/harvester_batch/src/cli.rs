use clap::Parser;
use std::path::PathBuf;

const DEFAULT_POLL_INTERVAL_MINUTES: u32 = 15;
const DEFAULT_LLM_CONCURRENCY: usize = 6;

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
        self.llm_concurrency = self.llm_concurrency.clamp(1, 10);

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
        assert!(args.force_unlock);
        assert!(args.allow_unsupported_sources);
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
