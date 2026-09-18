//! Thin command-line parsing and dispatch for the offline evaluator.

use std::fs;
use std::path::PathBuf;

use anyhow::bail;
use clap::{Args, Parser, Subcommand};

use crate::config::{load_config, resolve_config, ConfigOverrides, DatasetSelection, Transport};
use crate::freeze::{freeze, FreezeOptions};
use crate::manifest::manifest_path;
use crate::report::baseline::write_baseline_report;
use crate::report::compare::write_comparison_report;
use crate::review::{
    build::write_review_files_with_split,
    diagnose::write_diagnosis,
    score::{render_markdown, score_review},
};
use crate::runner::load_run_data;

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
    Run(RunArgs),
    ReviewFile(ReviewFileArgs),
    Score(ScoreArgs),
    Diagnose(DiagnoseArgs),
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
    #[arg(long)]
    run: Option<String>,
}

#[derive(Debug, Args)]
struct ReviewFileArgs {
    #[arg(long)]
    run: String,
    #[arg(long, default_value_t = 0)]
    blinding_seed: u64,
    #[arg(long, default_value_t = 1200)]
    excerpt_bytes: usize,
    #[arg(long, value_enum, default_value = "heldout")]
    split: DatasetSelection,
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ScoreArgs {
    #[arg(long)]
    run: String,
    #[arg(long)]
    review: Option<PathBuf>,
    #[arg(long)]
    key: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct DiagnoseArgs {
    #[arg(long)]
    run: String,
    #[arg(long)]
    review: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct RunArgs {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    split: Option<DatasetSelection>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long)]
    repeat: Option<u32>,
    #[arg(long)]
    run_id: Option<String>,
    #[arg(long)]
    transport: Option<Transport>,
    #[arg(long)]
    fake_dir: Option<String>,
    #[arg(long)]
    retry_failed: bool,
    #[arg(long)]
    concurrency: Option<u32>,
    #[arg(long)]
    dry_run: bool,
}

/// Dispatches a parsed command.
pub fn run(cli: Cli) -> anyhow::Result<()> {
    if tokio::runtime::Handle::try_current().is_ok() {
        bail!("cli::run cannot execute inside a Tokio runtime; use run_async");
    }
    tokio::runtime::Runtime::new()?.block_on(run_async(cli))
}

/// Async dispatch used by the Tokio binary; the synchronous wrapper remains
/// available for the existing keyless command tests.
pub async fn run_async(cli: Cli) -> anyhow::Result<()> {
    fs::create_dir_all(&cli.experiment_dir)?;
    engine_logging::initialize_at(cli.experiment_dir.join("harvester_eval.log"));
    let (operation, run_id) = command_log_context(&cli.command);
    let result = dispatch(cli).await;
    if result.is_err() {
        engine_logging::engine_error!("run_id={} operation={} status=failed", run_id, operation);
    }
    result
}

async fn dispatch(cli: Cli) -> anyhow::Result<()> {
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
            if args.baseline_only || args.run.is_none() {
                let manifest = args
                    .manifest
                    .unwrap_or_else(|| manifest_path(&cli.experiment_dir));
                let directory = write_baseline_report(&manifest, args.out.as_deref())?;
                println!("baseline report written to {}", directory.display());
            } else {
                let directory = write_comparison_report(
                    &cli.experiment_dir,
                    args.run.as_deref().expect("checked above"),
                    args.out.as_deref(),
                )?;
                println!("comparison report written to {}", directory.display());
            }
        }
        Command::Run(args) => {
            let config = load_config(&args.config)?;
            let overrides = ConfigOverrides {
                split: args.split,
                limit: args.limit,
                repeat: args.repeat,
                run_id: args.run_id,
                transport: args.transport,
                fake_dir: args.fake_dir,
                concurrency: args.concurrency,
                retry_failed: args.retry_failed.then_some(true),
                ..ConfigOverrides::default()
            };
            let resolved = resolve_config(Some(&config), &overrides)?;
            crate::runner::run(crate::runner::RunOptions {
                experiment_dir: cli.experiment_dir,
                config: resolved,
                dry_run: args.dry_run,
            })
            .await?;
        }
        Command::ReviewFile(args) => {
            let directory = write_review_files_with_split(
                &cli.experiment_dir,
                &args.run,
                args.blinding_seed,
                args.excerpt_bytes,
                Some(args.split),
                args.out.as_deref(),
            )?;
            println!("review files written to {}. Selection uses each article's lowest repetition; leave unreviewed rows in place with a blank your_priority.", directory.display());
        }
        Command::Score(args) => {
            let paths = run_paths(
                &cli.experiment_dir,
                &args.run,
                args.review.as_deref(),
                args.key.as_deref(),
            );
            let data = load_run_data(&cli.experiment_dir, &args.run)?;
            let csv = fs::read_to_string(paths.0)?;
            let key: crate::review::build::ReviewKey = serde_json::from_slice(&fs::read(paths.1)?)?;
            if key.run_id != args.run {
                bail!(
                    "review key run mismatch: expected {}, found {}",
                    args.run,
                    key.run_id
                );
            }
            let report = score_review(
                &csv,
                &key,
                &data.manifest,
                &data.records,
                &data.file.resolved_config,
            )?;
            let path = args.out.unwrap_or_else(|| {
                cli.experiment_dir
                    .join("reports")
                    .join(&args.run)
                    .join("scores.md")
            });
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, render_markdown(&report))?;
            engine_logging::engine_info!(
                "run_id={} operation=score artifact={}",
                args.run,
                path.display()
            );
            println!("scores written to {}", path.display());
        }
        Command::Diagnose(args) => {
            let path = write_diagnosis(
                &cli.experiment_dir,
                &args.run,
                args.review.as_deref(),
                args.out.as_deref(),
            )?;
            println!("diagnosis written to {}", path.display());
        }
    }
    Ok(())
}

fn command_log_context(command: &Command) -> (&'static str, String) {
    match command {
        Command::Freeze(_) => ("freeze", "-".into()),
        Command::Report(args) => ("report", args.run.clone().unwrap_or_else(|| "-".into())),
        Command::Run(args) => ("run", args.run_id.clone().unwrap_or_else(|| "-".into())),
        Command::ReviewFile(args) => ("review-file", args.run.clone()),
        Command::Score(args) => ("score", args.run.clone()),
        Command::Diagnose(args) => ("diagnose", args.run.clone()),
    }
}

fn run_paths(
    experiment_dir: &std::path::Path,
    run_id: &str,
    review: Option<&std::path::Path>,
    key: Option<&std::path::Path>,
) -> (PathBuf, PathBuf) {
    let directory = experiment_dir.join("reports").join(run_id);
    (
        review
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.join("review.csv")),
        key.map(PathBuf::from)
            .unwrap_or_else(|| directory.join("review.key.json")),
    )
}
