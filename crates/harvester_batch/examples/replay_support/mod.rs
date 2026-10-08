use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use clap::Parser;
use harvester_batch::{runner, Args};
use harvester_core::{update, AppState, CompletedJobSnapshot, Effect, JobResultKind, Msg};
use harvester_engine::llm::prompts::register_defaults;
use harvester_engine::llm::{LlmProvider, PromptRegistry};
use harvester_engine::{
    normalize_url_for_dedupe, parse_frontmatter, scan_archive_article_metadata, ArchiveArticleMeta,
    ExtractedLink, LinkKind, SourceId, SourceKind, SourceType,
};
use harvester_io::{
    host_bootstrap::{build_effect_runner_with_provider, prepare_desktop_startup_state},
    load_brave_seen_set, load_completed_jobs, load_seen_set, load_signal_candidate_cache,
    load_sources, load_summary_cache, load_triage_cache, persist_brave_seen_set,
    persist_completed_jobs, persist_seen_set, EffectRunner, FileWriteObserver, NoOpPlatformHandler,
    PersistenceWorker, RuntimePaths,
};
use harvester_ui_bridge::driver::{
    run_driver_with_observers, DriverIterationTiming, SnapshotSignal,
};
use harvester_ui_bridge::snapshot::BodyTable;
use openai_provider_kit::{FinishReason, LlmError, LlmRequest, LlmResponse, TokenUsage};
use serde::Serialize;
use serde_json::json;

const MAX_INPUT_BYTES: usize = 100_000;
const SOURCE_COUNT: usize = 29;
const BRAVE_SOURCE_COUNT: usize = 24;
const COPY_MARKER: &str = ".replay_bench_copy";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // The integration test includes this shared example support module.
pub enum BenchmarkHost {
    Batch,
    Desktop,
}

#[derive(Clone, Debug)]
pub struct HarnessOptions {
    pub source_dir: PathBuf,
    pub work_dir: PathBuf,
    pub hold_back: usize,
    pub reuse_copy: bool,
    pub host: BenchmarkHost,
    pub llm_latency_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TimingSummary {
    pub count: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub total_ms: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrivateFileSummary {
    pub writes: u64,
    pub bytes_written: u64,
    pub latency: TimingSummary,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SkippedRssSources {
    pub sources: usize,
    pub entries: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct DriverReconciliation {
    pub phase_and_idle_total_ms: f64,
    pub driver_wall_ms: f64,
    pub outside_driver_ms: f64,
    pub accounted_percent: f64,
}

type DriverPhase = (&'static str, fn(&DriverIterationTiming) -> Duration);

#[derive(Clone, Debug, Serialize)]
pub struct BenchmarkReport {
    pub completed: bool,
    pub host: String,
    pub source_dir: PathBuf,
    pub work_dir: PathBuf,
    pub report_path: PathBuf,
    pub held_back_articles: usize,
    pub llm_latency_ms: u64,
    pub model_call_path: String,
    pub wall_time_to_completion_ms: f64,
    pub reducer_time_by_message_kind: BTreeMap<String, TimingSummary>,
    pub effects_by_kind: BTreeMap<String, u64>,
    pub private_file_writes: BTreeMap<String, PrivateFileSummary>,
    pub view_builds: TimingSummary,
    pub snapshots_emitted: u64,
    pub llm_calls_by_prompt: BTreeMap<String, u64>,
    pub skipped_persisted_rss: SkippedRssSources,
    pub desktop_driver_iterations: u64,
    pub desktop_iteration_wall_time: TimingSummary,
    pub desktop_phase_totals_ms: BTreeMap<String, f64>,
    pub desktop_reconciliation: Option<DriverReconciliation>,
    pub harness_synchronous_effect_handling: BTreeMap<String, TimingSummary>,
}

#[derive(Clone)]
struct ArticleFile {
    url: String,
    source_title: Option<String>,
    title: String,
    fetched_utc: Option<String>,
    path: PathBuf,
    markdown: String,
    content_hash: String,
}

#[derive(Clone)]
struct HeldArticle {
    url: String,
    fetched_utc: Option<String>,
    path: PathBuf,
    markdown: String,
    links: Vec<String>,
}

#[derive(Default)]
struct ProviderRoutes {
    prepared_text_to_url: HashMap<String, String>,
    url_to_priority: HashMap<String, u8>,
}

#[derive(Default)]
struct Measurements {
    reducer_ns: BTreeMap<String, Vec<u128>>,
    effects: BTreeMap<String, u64>,
    private_writes: BTreeMap<String, (u64, u64, Vec<u128>)>,
    view_ns: Vec<u128>,
    snapshots: u64,
    llm_calls: BTreeMap<String, u64>,
    errors: Vec<String>,
    skipped_persisted_rss: SkippedRssSources,
    desktop_iterations: Vec<DriverIterationTiming>,
    harness_sync_effect_ns: BTreeMap<String, Vec<u128>>,
}

impl Measurements {
    fn record_reducer(&mut self, kind: &str, elapsed: Duration) {
        self.reducer_ns
            .entry(kind.to_owned())
            .or_default()
            .push(elapsed.as_nanos());
    }

    fn record_write(&mut self, path: &Path, bytes: u64, elapsed: Duration) {
        let name = if path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("llm_results")
        {
            "llm_results/*.json".to_owned()
        } else if path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some(".article_links")
        {
            ".article_links/*.json".to_owned()
        } else {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown-private-file")
                .to_owned()
        };
        let entry = self.private_writes.entry(name).or_default();
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry.1.saturating_add(bytes);
        entry.2.push(elapsed.as_nanos());
    }
}

#[allow(dead_code)] // Used by replay_bench.rs; the integration test supplies its own temp folder.
pub fn default_work_dir() -> PathBuf {
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    PathBuf::from(".local")
        .join("bench")
        .join(timestamp.to_string())
}

pub fn run_benchmark(options: HarnessOptions) -> Result<BenchmarkReport, String> {
    let source_dir = resolve_existing(&options.source_dir)?;
    if !source_dir.is_dir() {
        return Err(format!(
            "source folder is not a directory: {}",
            source_dir.display()
        ));
    }
    let work_dir = resolve_guard_path(&options.work_dir)?;
    if work_dir.starts_with(&source_dir) || source_dir.starts_with(&work_dir) {
        return Err(format!(
            "work folder overlaps the source folder: source={} work={}",
            source_dir.display(),
            work_dir.display()
        ));
    }

    if options.reuse_copy {
        if !work_dir.is_dir() {
            return Err(format!(
                "--reuse-copy requires an existing work folder: {}",
                work_dir.display()
            ));
        }
        if !work_dir.join(COPY_MARKER).is_file() {
            return Err(format!(
                "--reuse-copy requires a benchmark copy marker in {}",
                work_dir.display()
            ));
        }
    } else {
        if work_dir.exists() {
            return Err(format!(
                "work folder already exists; choose another path or pass --reuse-copy: {}",
                work_dir.display()
            ));
        }
        if let Some(parent) = work_dir.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create work-folder parent: {error}"))?;
        }
        copy_directory(&source_dir, &work_dir)?;
        fs::write(
            work_dir.join(COPY_MARKER),
            b"Harvester replay benchmark copy\n",
        )
        .map_err(|error| format!("mark benchmark copy {}: {error}", work_dir.display()))?;
    }

    let paths = RuntimePaths::new(
        work_dir.clone(),
        work_dir.join(".sources.ron"),
        workspace_root().join("contexts"),
        workspace_root().join("prompts"),
    );
    let _lock_guard = if options.host == BenchmarkHost::Batch {
        Some(harvester_io::acquire_lock(
            &paths.output_dir,
            harvester_io::COMMAND_LINE_LOCK_IDENTITY,
            false,
        )?)
    } else {
        None
    };
    let held_articles = hold_back_newest(&paths, options.hold_back)?;
    let poll_urls = if held_articles.is_empty() {
        load_completed_jobs(&paths.state_path)
            .into_iter()
            .map(|job| job.url)
            .collect::<Vec<_>>()
    } else {
        held_articles
            .iter()
            .map(|article| article.url.clone())
            .collect::<Vec<_>>()
    };
    let provider_routes = prepare_provider_routes(&held_articles)?;

    let args = Args::try_parse_from([
        "harvester_batch",
        "--output-dir",
        work_dir.to_str().ok_or("work path is not valid UTF-8")?,
        "--sources",
        paths
            .sources_path
            .to_str()
            .ok_or("source path is not valid UTF-8")?,
        "--contexts-dir",
        paths
            .contexts_dir
            .to_str()
            .ok_or("context path is not valid UTF-8")?,
        "--prompts-dir",
        paths
            .prompts_dir
            .to_str()
            .ok_or("prompt path is not valid UTF-8")?,
    ])
    .map_err(|error| error.to_string())?;

    let measurements = Arc::new(Mutex::new(Measurements::default()));
    let file_observer = make_file_observer(Arc::clone(&measurements));
    let (msg_tx, msg_rx) = mpsc::channel();
    let provider = Arc::new(RoutingCannedProvider {
        latency: Duration::from_millis(options.llm_latency_ms),
        routes: Arc::new(provider_routes),
        measurements: Arc::clone(&measurements),
    });
    let provider_trait: Arc<dyn LlmProvider> = provider;
    let (effect_runner, _) = build_effect_runner_with_provider(
        &paths,
        msg_tx.clone(),
        args.llm_concurrency,
        &harvester_io::host_bootstrap::HostLlmDefaults {
            default_model: harvester_engine::llm::ModelId::new(
                harvester_engine::llm::ProviderKind::OpenAi,
                harvester_engine::llm::OPENAI_MODEL_GPT_4O_MINI,
            ),
            session_id_prefix: "batch-",
        },
        provider_trait,
        Box::new(NoOpPlatformHandler),
        Box::new(PersistenceWorker::new_with_file_write_observer(
            paths.state_path.clone(),
            paths.blacklist_path.clone(),
            file_observer,
        )),
        Some(make_file_observer(Arc::clone(&measurements))),
    );

    let run_started = Instant::now();
    let mut effect_sink = |effects| {
        route_effects(
            effects,
            &effect_runner,
            &msg_tx,
            &paths,
            &poll_urls,
            &held_articles,
            &measurements,
        );
    };
    let mut reducer_observer = |kind: &str, elapsed| {
        measurements
            .lock()
            .expect("benchmark measurements")
            .record_reducer(kind, elapsed);
    };
    let mut state = match options.host {
        BenchmarkHost::Batch => runner::prepare_cycle_state_with_effect_sink(
            &paths,
            &args,
            &msg_rx,
            &mut effect_sink,
            &mut reducer_observer,
        )?,
        BenchmarkHost::Desktop => {
            let (state, startup_effects) = prepare_desktop_startup_state(
                AppState::new(),
                &paths,
                args.llm_concurrency,
                None,
                None,
            );
            if !startup_effects.is_empty() {
                effect_sink(startup_effects);
            }
            state
        }
    };

    let (completed, wall_time) = match options.host {
        BenchmarkHost::Batch => {
            runner::run_single_cycle_with_effect_sink(
                &mut state,
                &paths,
                &msg_tx,
                &msg_rx,
                &mut effect_sink,
                &mut reducer_observer,
                &make_file_observer(Arc::clone(&measurements)),
            )?;
            effect_runner.flush_results().map_err(|e| e.to_string())?;
            drop(effect_runner);
            runner::persist_final_cycle_state(
                &paths,
                &state,
                Some(&make_file_observer(Arc::clone(&measurements))),
            );
            (true, run_started.elapsed())
        }
        BenchmarkHost::Desktop => {
            msg_tx
                .send(Msg::PipelineRunRequested {
                    scope: harvester_core::PipelineRunScope::Full,
                })
                .map_err(|error| format!("send desktop run request: {error}"))?;
            let tick_running = Arc::new(AtomicBool::new(true));
            let tick_flag = Arc::clone(&tick_running);
            let tick_tx = msg_tx.clone();
            let tick_thread = thread::spawn(move || {
                while tick_flag.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(75));
                    if tick_flag.load(Ordering::Relaxed)
                        && tick_tx.send(Msg::tick_at(Utc::now())).is_err()
                    {
                        break;
                    }
                }
            });
            let desktop_started = Instant::now();
            let termination = run_driver_with_observers(
                state,
                msg_rx,
                update,
                effect_sink,
                |_| {},
                {
                    let measurements = Arc::clone(&measurements);
                    move |signal| {
                        if matches!(signal, SnapshotSignal::Snapshot(_)) {
                            measurements
                                .lock()
                                .expect("benchmark measurements")
                                .snapshots += 1;
                        }
                    }
                },
                Arc::new(RwLock::new(BodyTable::new())),
                move || desktop_started.elapsed(),
                {
                    let measurements = Arc::clone(&measurements);
                    move |kind, elapsed| {
                        measurements
                            .lock()
                            .expect("benchmark measurements")
                            .record_reducer(kind, elapsed);
                    }
                },
                {
                    let measurements = Arc::clone(&measurements);
                    move |elapsed| {
                        measurements
                            .lock()
                            .expect("benchmark measurements")
                            .view_ns
                            .push(elapsed.as_nanos());
                    }
                },
                Some({
                    let measurements = Arc::clone(&measurements);
                    move |iteration| {
                        measurements
                            .lock()
                            .expect("benchmark measurements")
                            .desktop_iterations
                            .push(iteration);
                    }
                }),
                |state: &AppState| {
                    state.pipeline_run_phase() == harvester_core::PipelineRunPhase::Idle
                },
            );
            tick_running.store(false, Ordering::Relaxed);
            tick_thread
                .join()
                .map_err(|_| "desktop tick thread panicked")?;
            effect_runner.flush_results().map_err(|e| e.to_string())?;
            drop(effect_runner);
            (
                termination == harvester_ui_bridge::DriverTermination::Clean,
                run_started.elapsed(),
            )
        }
    };

    drop(msg_tx);
    wait_for_cache_writes(&measurements, Duration::from_secs(30))?;

    let report_path = work_dir.join("report.json");
    let report = make_report(
        completed,
        options.host,
        &source_dir,
        &work_dir,
        &report_path,
        held_articles.len(),
        options.llm_latency_ms,
        wall_time,
        &measurements,
    );
    let report_json = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("serialize replay report: {error}"))?;
    fs::write(&report_path, &report_json)
        .map_err(|error| format!("write {}: {error}", report_path.display()))?;
    println!("{report_json}");
    if let Some(reconciliation) = &report.desktop_reconciliation {
        println!(
            "desktop driver phases + idle: {:.1} ms / {:.1} ms driver wall ({:.1}% accounted); {:.1} ms outside driver",
            reconciliation.phase_and_idle_total_ms,
            reconciliation.driver_wall_ms,
            reconciliation.accounted_percent,
            reconciliation.outside_driver_ms
        );
    }

    if !report.completed {
        return Err("replay host did not complete cleanly".into());
    }
    let errors = measurements
        .lock()
        .expect("benchmark measurements")
        .errors
        .clone();
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn make_report(
    completed: bool,
    host: BenchmarkHost,
    source_dir: &Path,
    work_dir: &Path,
    report_path: &Path,
    held_back_articles: usize,
    llm_latency_ms: u64,
    wall_time: Duration,
    measurements: &Arc<Mutex<Measurements>>,
) -> BenchmarkReport {
    let measurements = measurements.lock().expect("benchmark measurements");
    let phases: [DriverPhase; 7] = [
        ("reduce", |item: &DriverIterationTiming| item.reduce),
        ("pre_triage_pump", |item: &DriverIterationTiming| {
            item.pre_triage_pump
        }),
        ("effect_hand_off", |item: &DriverIterationTiming| {
            item.effect_hand_off
        }),
        ("view_build", |item: &DriverIterationTiming| item.view_build),
        ("view_compare", |item: &DriverIterationTiming| {
            item.view_compare
        }),
        ("snapshot_push_flush", |item: &DriverIterationTiming| {
            item.snapshot
        }),
        ("idle_recv", |item: &DriverIterationTiming| item.idle_recv),
    ];
    let desktop_phase_totals_ms = phases
        .into_iter()
        .map(|(name, phase)| {
            (
                name.to_owned(),
                millis(
                    measurements
                        .desktop_iterations
                        .iter()
                        .map(|item| phase(item).as_nanos())
                        .sum(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let phase_total = desktop_phase_totals_ms.values().sum::<f64>();
    let desktop_iteration_wall_time = timing_summary(
        &measurements
            .desktop_iterations
            .iter()
            .map(|item| item.wall.as_nanos())
            .collect::<Vec<_>>(),
    );
    let desktop_reconciliation = (host == BenchmarkHost::Desktop).then(|| DriverReconciliation {
        phase_and_idle_total_ms: phase_total,
        driver_wall_ms: desktop_iteration_wall_time.total_ms,
        outside_driver_ms: (millis(wall_time.as_nanos()) - desktop_iteration_wall_time.total_ms)
            .max(0.0),
        accounted_percent: if desktop_iteration_wall_time.total_ms == 0.0 {
            0.0
        } else {
            phase_total / desktop_iteration_wall_time.total_ms * 100.0
        },
    });
    BenchmarkReport {
        completed,
        host: match host {
            BenchmarkHost::Batch => "batch",
            BenchmarkHost::Desktop => "desktop",
        }
        .to_owned(),
        source_dir: source_dir.to_path_buf(),
        work_dir: work_dir.to_path_buf(),
        report_path: report_path.to_path_buf(),
        held_back_articles,
        llm_latency_ms,
        model_call_path: "synchronous".to_owned(),
        wall_time_to_completion_ms: millis(wall_time.as_nanos()),
        reducer_time_by_message_kind: measurements
            .reducer_ns
            .iter()
            .map(|(kind, values)| (kind.clone(), timing_summary(values)))
            .collect(),
        effects_by_kind: measurements.effects.clone(),
        private_file_writes: measurements
            .private_writes
            .iter()
            .map(|(name, (writes, bytes, values))| {
                (
                    name.clone(),
                    PrivateFileSummary {
                        writes: *writes,
                        bytes_written: *bytes,
                        latency: timing_summary(values),
                    },
                )
            })
            .collect(),
        view_builds: timing_summary(&measurements.view_ns),
        snapshots_emitted: measurements.snapshots,
        llm_calls_by_prompt: measurements.llm_calls.clone(),
        skipped_persisted_rss: measurements.skipped_persisted_rss.clone(),
        desktop_driver_iterations: measurements.desktop_iterations.len() as u64,
        desktop_iteration_wall_time,
        desktop_phase_totals_ms,
        desktop_reconciliation,
        harness_synchronous_effect_handling: measurements
            .harness_sync_effect_ns
            .iter()
            .map(|(kind, values)| (kind.clone(), timing_summary(values)))
            .collect(),
    }
}

fn timing_summary(values: &[u128]) -> TimingSummary {
    if values.is_empty() {
        return TimingSummary {
            count: 0,
            p50_ms: 0.0,
            p95_ms: 0.0,
            max_ms: 0.0,
            total_ms: 0.0,
        };
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let total = sorted.iter().copied().sum::<u128>();
    let percentile = |numerator: usize| {
        let rank = (sorted.len() * numerator).div_ceil(100).max(1) - 1;
        millis(sorted[rank])
    };
    TimingSummary {
        count: sorted.len() as u64,
        p50_ms: percentile(50),
        p95_ms: percentile(95),
        max_ms: millis(*sorted.last().unwrap_or(&0)),
        total_ms: millis(total),
    }
}

fn millis(nanos: u128) -> f64 {
    nanos as f64 / 1_000_000.0
}

fn make_file_observer(measurements: Arc<Mutex<Measurements>>) -> FileWriteObserver {
    Arc::new(move |path, bytes, elapsed| {
        measurements
            .lock()
            .expect("benchmark measurements")
            .record_write(path, bytes, elapsed);
    })
}

fn wait_for_cache_writes(
    measurements: &Arc<Mutex<Measurements>>,
    timeout: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    let mut last_write_count = 0;
    let mut quiet_since = Instant::now();
    loop {
        let write_count = measurements
            .lock()
            .expect("benchmark measurements")
            .private_writes
            .values()
            .map(|item| item.0)
            .sum::<u64>();
        if write_count != last_write_count {
            last_write_count = write_count;
            quiet_since = Instant::now();
        }
        // Allow the host persistence sink to finish its pending writes.
        if quiet_since.elapsed() >= Duration::from_millis(250) {
            return Ok(());
        }
        if started.elapsed() >= timeout {
            let values = measurements.lock().expect("benchmark measurements");
            return Err(format!(
                "timed out waiting for result-cache writes: effects={:?} writes={:?}",
                values.effects,
                values
                    .private_writes
                    .iter()
                    .map(|(name, (count, _, _))| (name, count))
                    .collect::<Vec<_>>()
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn route_effects(
    effects: Vec<Effect>,
    effect_runner: &EffectRunner,
    msg_tx: &mpsc::Sender<Msg>,
    paths: &RuntimePaths,
    poll_urls: &[String],
    held_articles: &[HeldArticle],
    measurements: &Arc<Mutex<Measurements>>,
) {
    let held_by_url: HashMap<String, &HeldArticle> = held_articles
        .iter()
        .map(|article| (normalize_url_for_dedupe(&article.url), article))
        .collect();
    for effect in effects {
        let kind = effect_kind(&effect);
        measurements
            .lock()
            .expect("benchmark measurements")
            .effects
            .entry(kind.to_owned())
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        match effect {
            Effect::PollAllSources => {
                let started = Instant::now();
                dispatch_canned_poll(paths, poll_urls, msg_tx, measurements);
                record_harness_sync(measurements, "canned_poll", started.elapsed());
            }
            Effect::EnqueueUrl { job_id, url } => {
                match held_by_url.get(&normalize_url_for_dedupe(&url)) {
                    Some(article) => {
                        let started = Instant::now();
                        deliver_held_article(article, job_id, msg_tx, measurements);
                        record_harness_sync(
                            measurements,
                            "held_article_delivery",
                            started.elapsed(),
                        );
                    }
                    None => {
                        let reason = format!("no held article is available for EnqueueUrl {url}");
                        measurements
                            .lock()
                            .expect("benchmark measurements")
                            .errors
                            .push(reason.clone());
                        let _ = msg_tx.send(Msg::JobDone {
                            job_id,
                            result: JobResultKind::Failed { reason },
                            extracted_links: Vec::new(),
                            fetched_utc: None,
                        });
                    }
                }
            }
            other => effect_runner.enqueue(vec![other]),
        }
    }
}

fn record_harness_sync(measurements: &Arc<Mutex<Measurements>>, kind: &str, elapsed: Duration) {
    measurements
        .lock()
        .expect("benchmark measurements")
        .harness_sync_effect_ns
        .entry(kind.to_owned())
        .or_default()
        .push(elapsed.as_nanos());
}

fn deliver_held_article(
    article: &HeldArticle,
    job_id: u64,
    msg_tx: &mpsc::Sender<Msg>,
    measurements: &Arc<Mutex<Measurements>>,
) {
    if let Err(error) = fs::write(&article.path, &article.markdown) {
        measurements
            .lock()
            .expect("benchmark measurements")
            .errors
            .push(format!(
                "write held article {}: {error}",
                article.path.display()
            ));
        let _ = msg_tx.send(Msg::JobDone {
            job_id,
            result: JobResultKind::Failed {
                reason: format!("synthetic article write failed: {error}"),
            },

            extracted_links: Vec::new(),
            fetched_utc: None,
        });
        return;
    }
    let bytes = article.markdown.len() as u64;
    let tokens = article.markdown.split_whitespace().count() as u32;
    let links = article
        .links
        .iter()
        .map(|url| ExtractedLink {
            url: url.clone(),
            text: None,
            kind: LinkKind::Hyperlink,
        })
        .collect();
    let _ = msg_tx.send(Msg::JobProgress {
        job_id,
        stage: harvester_core::Stage::Downloading,
        tokens: Some(tokens),
        bytes: Some(bytes),
    });
    let _ = msg_tx.send(Msg::JobDone {
        job_id,
        result: JobResultKind::Success,
        extracted_links: links,
        fetched_utc: article.fetched_utc.clone(),
    });
}

fn dispatch_canned_poll(
    paths: &RuntimePaths,
    urls: &[String],
    msg_tx: &mpsc::Sender<Msg>,
    measurements: &Arc<Mutex<Measurements>>,
) {
    let mut sources = Vec::with_capacity(SOURCE_COUNT);
    for index in 0..SOURCE_COUNT {
        let kind = if index < BRAVE_SOURCE_COUNT {
            SourceKind::Brave
        } else {
            SourceKind::Rss
        };
        let source_id = SourceId::new(format!(
            "bench-{}-{index:02}",
            match kind {
                SourceKind::Brave => "brave",
                SourceKind::Rss => "rss",
                _ => unreachable!(),
            }
        ))
        .expect("synthetic source ID is valid");
        let source_urls = urls
            .iter()
            .enumerate()
            .filter(|(url_index, _)| url_index % SOURCE_COUNT == index)
            .map(|(_, url)| url.clone())
            .collect::<Vec<_>>();
        sources.push((source_id, kind, source_urls));
    }

    let mut rss_seen = load_seen_set(&paths.seen_set_path);
    let mut brave_seen = load_brave_seen_set(&paths.brave_seen_set_path);
    // Offer persisted seen-only URLs too. The real seen-set filters decide
    // which URLs reach SourcePollCompleted, as in the network polling path.
    let brave_offered = brave_seen.entries().map(str::to_owned).collect::<Vec<_>>();
    sources[0].2.extend(brave_offered);
    let mut rss_offered = BTreeMap::<String, Vec<String>>::new();
    for (source_id, guid) in rss_seen.entries() {
        rss_offered
            .entry(source_id.to_owned())
            .or_default()
            .push(guid.to_owned());
    }
    let current_rss_ids = load_sources(&paths.sources_path)
        .sources
        .into_iter()
        .filter(|source| matches!(source.source_type, SourceType::Rss { .. }))
        .map(|source| source.id.to_string())
        .collect::<HashSet<_>>();
    let mut rss_offered = rss_offered.into_iter().collect::<Vec<_>>();
    rss_offered.sort_by(|(left_id, left_guids), (right_id, right_guids)| {
        current_rss_ids
            .contains(right_id)
            .cmp(&current_rss_ids.contains(left_id))
            .then_with(|| right_guids.len().cmp(&left_guids.len()))
            .then_with(|| left_id.cmp(right_id))
    });
    let mut skipped = SkippedRssSources::default();
    for (index, (source_id, guids)) in rss_offered.into_iter().enumerate() {
        if let Some(source) = sources.get_mut(BRAVE_SOURCE_COUNT + index) {
            source.0 = SourceId::new(source_id).expect("persisted RSS source ID is valid");
            source.2.extend(guids);
        } else {
            skipped.sources += 1;
            skipped.entries += guids.len();
        }
    }
    measurements
        .lock()
        .expect("benchmark measurements")
        .skipped_persisted_rss = skipped;
    let mut filtered_sources = Vec::with_capacity(SOURCE_COUNT);
    for (source_id, kind, offered_urls) in sources {
        let parsed = offered_urls.len();
        let urls = match kind {
            SourceKind::Brave => brave_seen.filter_unseen(offered_urls),
            SourceKind::Rss => {
                let entries = offered_urls
                    .into_iter()
                    .map(|url| harvester_engine::FeedEntry {
                        guid: url.clone(),
                        url: Some(url),
                        title: None,
                        published: None,
                    })
                    .collect();
                rss_seen
                    .filter_unseen_entries(source_id.as_str(), entries)
                    .into_iter()
                    .filter_map(|entry| entry.url)
                    .collect()
            }
            _ => unreachable!(),
        };
        filtered_sources.push((source_id, kind, parsed, urls));
    }
    let started = Instant::now();
    if let Err(error) = persist_seen_set(&rss_seen, &paths.seen_set_path) {
        measurements
            .lock()
            .expect("benchmark measurements")
            .errors
            .push(format!("persist synthetic RSS seen-set: {error}"));
    } else {
        record_write_from_path(measurements, &paths.seen_set_path, started.elapsed());
    }
    let started = Instant::now();
    if let Err(error) = persist_brave_seen_set(&brave_seen, &paths.brave_seen_set_path) {
        measurements
            .lock()
            .expect("benchmark measurements")
            .errors
            .push(format!("persist synthetic Brave seen-set: {error}"));
    } else {
        record_write_from_path(measurements, &paths.brave_seen_set_path, started.elapsed());
    }

    let _ = msg_tx.send(Msg::PollStarted {
        total: SOURCE_COUNT,
    });
    for (source_id, kind, parsed, source_urls) in filtered_sources {
        let dedup_filtered = parsed - source_urls.len();
        let _ = msg_tx.send(Msg::SourcePollCompleted {
            source_id,
            urls: source_urls,
            kind,
            parsed,
            dedup_filtered,
        });
    }
    let _ = msg_tx.send(Msg::AllSourcesPollEnded);
}

fn record_write_from_path(measurements: &Arc<Mutex<Measurements>>, path: &Path, elapsed: Duration) {
    let bytes = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    measurements
        .lock()
        .expect("benchmark measurements")
        .record_write(path, bytes, elapsed);
}

fn effect_kind(effect: &Effect) -> &'static str {
    match effect {
        Effect::EnqueueUrl { .. } => "EnqueueUrl",
        Effect::LoadArticleLinks { .. } => "LoadArticleLinks",
        Effect::StoreArticleLinks { .. } => "StoreArticleLinks",
        Effect::LoadProcessingConfiguration { .. } => "LoadProcessingConfiguration",
        Effect::ResetCorpusScanIndex => "ResetCorpusScanIndex",
        Effect::LoadArticlesForTriage { .. } => "LoadArticlesForTriage",
        Effect::LoadPromptContexts => "LoadPromptContexts",
        Effect::LoadPromptTemplateFiles => "LoadPromptTemplateFiles",
        Effect::LoadLlmMetadata => "LoadLlmMetadata",
        Effect::PollAllSources => "PollAllSources",
        Effect::RequestLlmCompletion { .. } => "RequestLlmCompletion",
        Effect::StartSession => "StartSession",
        Effect::StopFinish { .. } => "StopFinish",
        Effect::ArchiveRequested { .. } => "ArchiveRequested",
        Effect::OpenArchiveDialog { .. } => "OpenArchiveDialog",
        Effect::ShowArchiveDialog { .. } => "ShowArchiveDialog",
        Effect::SaveResults { .. } => "SaveResults",
        Effect::FlushResults => "FlushResults",
        Effect::PersistSignalCandidateOverrides { .. } => "PersistSignalCandidateOverrides",
        Effect::LoadBriefingCheckpoint => "LoadBriefingCheckpoint",
        Effect::SaveBriefingCheckpoint { .. } => "SaveBriefingCheckpoint",
        Effect::OpenUrlInBrowser { .. } => "OpenUrlInBrowser",
        Effect::ImportSavedWebpages { .. } => "ImportSavedWebpages",
        Effect::PersistDesktopWindowSize { .. } => "PersistDesktopWindowSize",
        Effect::PersistRuntimeState { .. } => "PersistRuntimeState",
    }
}

fn hold_back_newest(paths: &RuntimePaths, count: usize) -> Result<Vec<HeldArticle>, String> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut article_files = read_article_files(&paths.output_dir)?;
    article_files.sort_by(|left, right| {
        article_order(right)
            .cmp(&article_order(left))
            .then_with(|| left.path.cmp(&right.path))
    });
    article_files.truncate(count);
    if !article_files.is_empty() {
        // Admit held articles without accidentally reopening the whole historical
        // corpus. The source checkpoint and all source files remain untouched.
        if let (Some(checkpoint), Some(earliest)) = (
            harvester_io::load_briefing_checkpoint(&paths.briefing_checkpoint_path),
            article_files
                .iter()
                .filter_map(|a| a.fetched_utc.as_deref())
                .filter_map(|s| DateTime::parse_from_rfc3339(s).ok())
                .min(),
        ) {
            if DateTime::parse_from_rfc3339(&checkpoint).is_ok_and(|date| date > earliest) {
                harvester_io::save_briefing_checkpoint(
                    &paths.briefing_checkpoint_path,
                    Some(&earliest.to_rfc3339()),
                )
                .map_err(|error| format!("adjust benchmark-copy briefing checkpoint: {error}"))?;
            }
        }
    }
    let selected_urls: HashSet<_> = article_files
        .iter()
        .map(|article| normalize_url_for_dedupe(&article.url))
        .collect();
    let mut rss_seen = load_seen_set(&paths.seen_set_path);
    let mut brave_seen = load_brave_seen_set(&paths.brave_seen_set_path);
    for article in &article_files {
        rss_seen.forget_guid_everywhere(&article.url);
        brave_seen.forget_url(&article.url);
    }
    persist_seen_set(&rss_seen, &paths.seen_set_path)
        .map_err(|error| format!("reset held RSS entries in benchmark copy: {error}"))?;
    persist_brave_seen_set(&brave_seen, &paths.brave_seen_set_path)
        .map_err(|error| format!("reset held Brave entries in benchmark copy: {error}"))?;
    let snapshots = load_completed_jobs(&paths.state_path);
    let mut held_articles = article_files
        .iter()
        .map(|article| {
            let links = snapshots
                .iter()
                .find(|job| {
                    normalize_url_for_dedupe(&job.url) == normalize_url_for_dedupe(&article.url)
                })
                .map(|job| job.links.iter().map(|link| link.url.clone()).collect())
                .unwrap_or_default();
            HeldArticle {
                url: article.url.clone(),
                fetched_utc: article.fetched_utc.clone(),
                path: article.path.clone(),
                markdown: article.markdown.clone(),
                links,
            }
        })
        .collect::<Vec<_>>();
    let mut remaining_jobs: Vec<CompletedJobSnapshot> = snapshots
        .into_iter()
        .filter(|job| !selected_urls.contains(&normalize_url_for_dedupe(&job.url)))
        .collect();
    if remaining_jobs.len() != load_completed_jobs(&paths.state_path).len() {
        persist_completed_jobs(
            &paths.state_path,
            &remaining_jobs
                .iter()
                .map(|job| harvester_core::SlimJobRecord {
                    url: job.url.clone(),
                    tokens: job.tokens,
                    bytes: job.bytes,
                    fetched_utc: job.fetched_utc.clone(),
                })
                .collect::<Vec<_>>(),
        );
    }
    remaining_jobs.clear();

    remove_held_back_cache_entries(paths, &article_files)?;
    for article in &held_articles {
        fs::remove_file(&article.path)
            .map_err(|error| format!("remove held article {}: {error}", article.path.display()))?;
    }
    held_articles.shrink_to_fit();
    Ok(held_articles)
}

fn remove_held_back_cache_entries(
    paths: &RuntimePaths,
    articles: &[ArticleFile],
) -> Result<(), String> {
    if articles.is_empty() {
        return Ok(());
    }
    let content_hashes: HashSet<_> = articles
        .iter()
        .map(|article| article.content_hash.as_str())
        .collect();
    let triage = load_triage_cache(&paths.triage_cache_path).expect("load result store");
    let summary = load_summary_cache(&paths.summary_cache_path).expect("load result store");
    let mut removed_signal_inputs = HashSet::new();
    for article in articles {
        let triage_results: Vec<_> = triage
            .iter()
            .filter(|(key, _)| key.content_hash == article.content_hash)
            .map(|(key, entry)| (key.clone(), entry.result.clone()))
            .collect();
        let summaries: Vec<_> = summary
            .iter()
            .filter(|(key, _)| key.content_hash == article.content_hash)
            .map(|(key, entry)| (key.clone(), entry.result.clone()))
            .collect();
        for (triage_key, triage_result) in &triage_results {
            for (summary_key, summary_result) in &summaries {
                let mut tags = triage_result
                    .tags
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>();
                tags.sort_unstable();
                let outlet = best_effort_outlet(article.source_title.as_deref(), &article.url);
                let bundle = harvester_core::SignalCandidateInputBundle {
                    url: &article.url,
                    outlet: &outlet,
                    title: &article.title,
                    // Archive frontmatter has no source publication time. A
                    // stale signal key using that time may remain in the copy.
                    published_at: article.fetched_utc.as_deref().unwrap_or_default(),
                    triage_priority: triage_result.priority,
                    triage_tags_sorted: tags,
                    summary: &summary_result.summary,
                    key_points: &summary_result.key_points,
                    upstream_summary_cache_digest: summary_key.digest(),
                };
                removed_signal_inputs.insert(bundle.hash());
            }
            let _ = triage_key;
        }
    }

    let mut new_triage = harvester_core::TriageCache::new();
    for (key, entry) in triage.iter() {
        if !content_hashes.contains(key.content_hash.as_str()) {
            new_triage.insert_entry(key.clone(), entry.clone());
        }
    }
    write_filtered_result_copy(&paths.triage_cache_path, new_triage.iter())
        .map_err(|error| format!("filter triage cache: {error}"))?;

    let mut new_summary = harvester_core::SummaryCache::new();
    for (key, entry) in summary.iter() {
        if !content_hashes.contains(key.content_hash.as_str()) {
            new_summary.insert(key.clone(), entry.clone());
        }
    }
    write_filtered_result_copy(&paths.summary_cache_path, new_summary.iter())
        .map_err(|error| format!("filter summary cache: {error}"))?;

    let signal = load_signal_candidate_cache(&paths.signal_candidate_cache_path)
        .map_err(|error| format!("load signal-candidate cache: {error}"))?;
    write_filtered_result_copy(
        &paths.signal_candidate_cache_path,
        signal
            .entries
            .iter()
            .filter(|(key, _)| !removed_signal_inputs.contains(&key.signal_input_hash)),
    )
    .map_err(|error| format!("filter signal-candidate cache: {error}"))?;
    Ok(())
}

// This prepares synthetic input in the harness's disposable copy, before the
// runner starts. Production stores never rewrite records, and RON backups in
// both the source and the copy remain untouched.
fn write_filtered_result_copy<'a, K: serde::Serialize + 'a, E: serde::Serialize + 'a>(
    path: &Path,
    records: impl Iterator<Item = (&'a K, &'a E)>,
) -> std::io::Result<()> {
    let mut bytes = Vec::new();
    for record in records {
        serde_json::to_writer(&mut bytes, &record)?;
        bytes.push(b'\n');
    }
    fs::write(path, bytes)
}

fn read_article_files(output_dir: &Path) -> Result<Vec<ArticleFile>, String> {
    let metadata = scan_archive_article_metadata(output_dir)?;
    let by_url: HashMap<String, ArchiveArticleMeta> = metadata
        .into_iter()
        .map(|item| (normalize_url_for_dedupe(&item.url), item))
        .collect();
    let mut articles = Vec::new();
    for entry in fs::read_dir(output_dir)
        .map_err(|error| format!("list article files in {}: {error}", output_dir.display()))?
    {
        let entry = entry.map_err(|error| format!("read article file entry: {error}"))?;
        let path = entry.path();
        if !path.is_file()
            || path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.eq_ignore_ascii_case("md"))
                != Some(true)
        {
            continue;
        }
        let markdown = fs::read_to_string(&path)
            .map_err(|error| format!("read article {}: {error}", path.display()))?;
        let Some(fields) = parse_frontmatter(&markdown) else {
            continue;
        };
        let Some(url) = fields.url.filter(|url| !url.trim().is_empty()) else {
            continue;
        };
        let key = normalize_url_for_dedupe(&url);
        let Some(content_hash) = by_url.get(&key).and_then(|item| item.content_hash.clone()) else {
            continue;
        };
        let source_title = fields.title.filter(|title| !title.trim().is_empty());
        let title = best_effort_article_title(source_title.as_deref(), &url);
        articles.push(ArticleFile {
            url,
            source_title,
            title,
            fetched_utc: fields.fetched_utc,
            path,
            markdown,
            content_hash,
        });
    }
    Ok(articles)
}

fn best_effort_outlet(source_title: Option<&str>, url: &str) -> String {
    if let Some(source_title) = source_title
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return source_title.to_owned();
    }
    url_host(url)
}

fn best_effort_article_title(source_title: Option<&str>, url: &str) -> String {
    if let Some(title) = source_title
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        return title.to_owned();
    }
    let without_scheme = url
        .find("://")
        .map(|index| &url[index + 3..])
        .unwrap_or(url);
    let without_query = without_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    let slug = without_query
        .rsplit('/')
        .next()
        .unwrap_or(without_query)
        .trim();
    if slug.is_empty() || !slug.contains('-') {
        return url.to_owned();
    }
    slug.split(['-', '_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                format!(
                    "{}{}",
                    first.to_uppercase(),
                    chars.as_str().to_ascii_lowercase()
                )
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn url_host(url: &str) -> String {
    let without_scheme = url
        .find("://")
        .map(|index| &url[index + 3..])
        .unwrap_or(url);
    without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn article_order(article: &ArticleFile) -> i128 {
    article
        .fetched_utc
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| i128::from(value.timestamp_millis()))
        .unwrap_or_else(|| {
            article
                .path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as i128)
                .unwrap_or(0)
        })
}

fn prepare_provider_routes(articles: &[HeldArticle]) -> Result<ProviderRoutes, String> {
    if articles.is_empty() {
        return Ok(ProviderRoutes::default());
    }
    let scratch =
        tempfile::tempdir().map_err(|error| format!("create replay scratch folder: {error}"))?;
    for article in articles {
        let filename = article
            .path
            .file_name()
            .ok_or_else(|| format!("article path has no filename: {}", article.path.display()))?;
        fs::write(scratch.path().join(filename), &article.markdown)
            .map_err(|error| format!("write replay scratch article: {error}"))?;
    }
    let registry = prompt_registry();
    let guard = registry.read().expect("replay prompt registry");
    let urls = articles.iter().map(|a| a.url.clone()).collect::<Vec<_>>();
    let (delta, _) = harvester_engine::CorpusScanIndex::default().load_delta(
        scratch.path(),
        MAX_INPUT_BYTES,
        &guard,
        &urls,
        None,
        &[],
        |_| {},
    )?;
    let prepared = delta.articles;
    let mut routes = ProviderRoutes::default();
    for article in prepared {
        routes
            .prepared_text_to_url
            .insert(article.prepared_text, article.url);
    }
    let mut urls = articles
        .iter()
        .map(|article| article.url.as_str())
        .collect::<Vec<_>>();
    urls.sort_by_key(|url| stable_url_hash(url));
    for (index, url) in urls.into_iter().enumerate() {
        routes
            .url_to_priority
            .insert(normalize_url_for_dedupe(url), (index % 5 + 1) as u8);
    }
    Ok(routes)
}

fn prompt_registry() -> Arc<RwLock<PromptRegistry>> {
    let mut registry = PromptRegistry::new();
    register_defaults(&mut registry);
    Arc::new(RwLock::new(registry))
}

struct RoutingCannedProvider {
    latency: Duration,
    routes: Arc<ProviderRoutes>,
    measurements: Arc<Mutex<Measurements>>,
}

#[async_trait::async_trait]
impl openai_provider_kit::LlmProvider for RoutingCannedProvider {
    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        if !self.latency.is_zero() {
            tokio::time::sleep(self.latency).await;
        }
        let system = request
            .messages()
            .first()
            .map(|message| message.content().to_ascii_lowercase())
            .unwrap_or_default();
        let user = request
            .messages()
            .get(1)
            .map(|message| message.content())
            .unwrap_or_default();
        let prompt_kind = if system.contains("signallog")
            || user.to_ascii_lowercase().contains("signallog candidate")
        {
            "ArticleSignalCandidate"
        } else if system.contains("triage") {
            "ArticleTriage"
        } else {
            "ArticleSummary"
        };
        let url = extract_url(user).or_else(|| {
            self.routes
                .prepared_text_to_url
                .iter()
                .find(|(prepared, _)| user.contains(prepared.as_str()))
                .map(|(_, url)| url.as_str())
        });
        let url = url.unwrap_or("https://replay.invalid/unmapped");
        let hash = stable_url_hash(url);
        let priority = self
            .routes
            .url_to_priority
            .get(&normalize_url_for_dedupe(url))
            .copied()
            .unwrap_or_else(|| (hash % 5 + 1) as u8);
        self.measurements
            .lock()
            .expect("benchmark measurements")
            .llm_calls
            .entry(prompt_kind.to_owned())
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        let content = match prompt_kind {
            "ArticleTriage" => json!({
                "category": "technology",
                "priority": priority,
                "tags": ["replay", "business-signal"],
                "rationale": "Canned replay response distributed by stable URL hash."
            }),
            "ArticleSignalCandidate" => json!({
                "signal_score": hash % 101,
                "signal_key": format!("replay-event-{hash:08x}"),
                "themes": ["ai-infrastructure"],
                "draft_gist": "A replayed article describes a dated business development relevant to AI markets.",
                "source_tier": "Tier1",
                "confidence": "High",
                "reasoning": "Synthetic replay scoring response."
            }),
            _ => json!({
                "title": "Replay article",
                "summary": "A canned summary for deterministic replay measurement.",
                "key_points": ["A measurable business change occurred.", "The change affects an AI market participant.", "The implementation provides no live model dependency."],
                "entities": {"companies": ["OpenAI"], "technologies": ["large language model"], "products": []}
            }),
        }
        .to_string();
        Ok(LlmResponse::new(
            content,
            TokenUsage::new(1200, 128),
            request.model().clone(),
            FinishReason::Stop,
        ))
    }

    fn provider_name(&self) -> &str {
        "routing-canned"
    }
}

fn extract_url(user_message: &str) -> Option<&str> {
    user_message
        .lines()
        .find_map(|line| line.trim().strip_prefix("URL:"))
        .map(str::trim)
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
}

fn stable_url_hash(url: &str) -> u32 {
    url.as_bytes().iter().fold(2_166_136_261, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(16_777_619)
    })
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("harvester_batch is two directories below the workspace root")
        .to_path_buf()
}

fn resolve_existing(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("read current directory: {error}"))?
            .join(path)
    };
    fs::canonicalize(&absolute)
        .map_err(|error| format!("resolve source folder {}: {error}", absolute.display()))
}

fn resolve_guard_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("read current directory: {error}"))?
            .join(path)
    };
    let lexical = normalize_path(&absolute);
    if lexical.exists() {
        return fs::canonicalize(&lexical)
            .map_err(|error| format!("resolve work folder {}: {error}", lexical.display()));
    }
    let mut ancestor = lexical.as_path();
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| format!("cannot resolve work folder path {}", lexical.display()))?;
    }
    let canonical_ancestor = fs::canonicalize(ancestor)
        .map_err(|error| format!("resolve work-folder parent {}: {error}", ancestor.display()))?;
    let suffix = lexical
        .strip_prefix(ancestor)
        .map_err(|error| format!("resolve work-folder suffix: {error}"))?;
    Ok(normalize_path(&canonical_ancestor.join(suffix)))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("create work folder {}: {error}", destination.display()))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("list source folder {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("read source entry: {error}"))?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect source entry {}: {error}", from.display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "source folder contains a symbolic link: {}",
                from.display()
            ));
        } else if file_type.is_dir() {
            copy_directory(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to)
                .map_err(|error| format!("copy {} to {}: {error}", from.display(), to.display()))?;
        }
    }
    Ok(())
}
