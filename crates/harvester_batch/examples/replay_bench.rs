mod replay_support;

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use replay_support::{default_work_dir, run_benchmark, BenchmarkHost, HarnessOptions};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum HostArg {
    Batch,
    Desktop,
}

#[derive(Debug, Parser)]
#[command(about = "Run Harvester's keyless replay benchmark on a private output copy")]
struct Cli {
    /// Source output folder. The source is read only.
    #[arg(long, default_value = "output", value_name = "PATH")]
    source_dir: PathBuf,

    /// Work folder for the copy and report. Defaults to .local/bench/<timestamp>.
    #[arg(long, value_name = "PATH")]
    work_dir: Option<PathBuf>,

    /// Number of newest copied articles to remove before replay and restore on EnqueueUrl.
    #[arg(long, default_value_t = 40, value_name = "N")]
    hold_back: usize,

    /// Reuse an existing work copy without copying the source again.
    #[arg(long)]
    reuse_copy: bool,

    /// Host loop to measure.
    #[arg(long, value_enum, default_value_t = HostArg::Batch)]
    host: HostArg,

    /// Synthetic provider delay for each model call.
    #[arg(long, default_value_t = 0, value_name = "MS")]
    llm_latency_ms: u64,
}

fn main() {
    let cli = Cli::parse();
    // Keep replay diagnostics separate from the owner's production engine.log.
    let log_dir = cli
        .work_dir
        .as_deref()
        .and_then(std::path::Path::parent)
        .unwrap_or_else(|| std::path::Path::new(".local/bench"));
    std::fs::create_dir_all(log_dir).expect("benchmark log directory");
    let log_name = cli
        .work_dir
        .as_deref()
        .and_then(std::path::Path::file_name)
        .map(|name| format!("{}.engine.log", name.to_string_lossy()))
        .unwrap_or_else(|| "replay-engine.log".into());
    engine_logging::initialize_at(log_dir.join(log_name));
    let options = HarnessOptions {
        source_dir: cli.source_dir,
        work_dir: cli.work_dir.unwrap_or_else(default_work_dir),
        hold_back: cli.hold_back,
        reuse_copy: cli.reuse_copy,
        host: match cli.host {
            HostArg::Batch => BenchmarkHost::Batch,
            HostArg::Desktop => BenchmarkHost::Desktop,
        },
        llm_latency_ms: cli.llm_latency_ms,
    };
    if let Err(error) = run_benchmark(options) {
        eprintln!("replay_bench: {error}");
        std::process::exit(2);
    }
}
