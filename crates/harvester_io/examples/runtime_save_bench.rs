//! Measure repeated saves on a private copy of a runtime state file.
//! This example loads/migrates and writes the supplied file; never pass the
//! owner's real output folder. Run with `cargo run --offline -p harvester_io
//! --example runtime_save_bench -- .local/bench/<copy>/.harvester_state.ron`.
use std::{path::PathBuf, time::Instant};
fn main() {
    let path = PathBuf::from(std::env::args().nth(1).expect("private state path"));
    engine_logging::initialize_at(path.with_extension("save-bench.log"));
    let start = Instant::now();
    let jobs = harvester_io::load_completed_jobs(&path);
    println!(
        "load jobs={} elapsed_ms={}",
        jobs.len(),
        start.elapsed().as_millis()
    );
    let slim: Vec<_> = jobs
        .into_iter()
        .map(|job| harvester_core::SlimJobRecord {
            url: job.url,
            tokens: job.tokens,
            bytes: job.bytes,
            fetched_utc: job.fetched_utc,
        })
        .collect();
    for index in 0..3 {
        let start = Instant::now();
        harvester_io::try_persist_runtime_state(&path, &slim).unwrap();
        println!("save={index} elapsed_ms={}", start.elapsed().as_millis());
    }
}
