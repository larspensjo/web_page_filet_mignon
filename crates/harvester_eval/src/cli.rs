//! Thin command-line parsing and dispatch for the offline evaluator.

use std::fs;
use std::path::PathBuf;

use anyhow::bail;
use clap::{Args, Parser, Subcommand};

use crate::freeze::{freeze, FreezeOptions};
use crate::manifest::manifest_path;
use crate::report::baseline::write_baseline_report;

/// Offline evaluator commands.
#[derive(Debug, Parser)]
#[command(name = "harvester_eval")]
pub struct Cli {
    /// Directory where frozen inputs and reports are written.
    #[arg(long, global = true, default_value = ".local/experiments/jev-triage")]
    pub experiment_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Freeze(FreezeArgs),
    Report(ReportArgs),
}

#[derive(Debug, Args)]
struct FreezeArgs {
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long)]
    linked_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 800)]
    limit: usize,
    #[arg(long, default_value_t = 4)]
    prompt_version: u32,
    #[arg(long, default_value = "contexts/article_triage.toml")]
    context_file: PathBuf,
    #[arg(long, default_value_t = 0.5)]
    dev_share: f64,
    #[arg(long, default_value_t = 0)]
    split_seed: u64,
    #[arg(long, default_value_t = 16)]
    min_priority_5: u64,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    precondition_report: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ReportArgs {
    #[arg(long)]
    baseline_only: bool,
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
}

/// Dispatches a parsed command.
pub fn run(cli: Cli) -> anyhow::Result<()> {
    fs::create_dir_all(&cli.experiment_dir)?;
    engine_logging::initialize_at(cli.experiment_dir.join("harvester_eval.log"));
    match cli.command {
        Command::Freeze(args) => {
            let result = freeze(&FreezeOptions {
                output_dir: args.output_dir,
                linked_dir: args.linked_dir,
                experiment_dir: cli.experiment_dir,
                limit: args.limit,
                prompt_version: args.prompt_version,
                context_file: args.context_file,
                dev_share: args.dev_share,
                split_seed: args.split_seed,
                min_priority_5: args.min_priority_5,
                dry_run: args.dry_run,
            })?;
            let report = result.preconditions.render();
            print!("{report}");
            if let Some(path) = args.precondition_report {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, serde_json::to_vec_pretty(&result.preconditions)?)?;
            }
        }
        Command::Report(args) => {
            if !args.baseline_only {
                bail!("Phase 1 supports report only with --baseline-only");
            }
            let manifest = args
                .manifest
                .unwrap_or_else(|| manifest_path(&cli.experiment_dir));
            let directory = write_baseline_report(&manifest, args.out.as_deref())?;
            println!("baseline report written to {}", directory.display());
        }
    }
    Ok(())
}
