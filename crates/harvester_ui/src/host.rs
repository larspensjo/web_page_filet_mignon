use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use engine_logging::{engine_error, engine_info, engine_warn};
use harvester_core::{
    update, AiAvailability, AiUnavailableReason, AppState, IntentContext, IntentEffect, Msg,
    DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH,
};
use harvester_engine::llm::{ModelId, ProviderKind, OPENAI_MODEL_GPT_5_4_NANO};
use harvester_io::{
    acquire_lock,
    host_bootstrap::{
        build_effect_runner, llm_max_concurrency_requests_from_env, prepare_desktop_startup_state,
        HostLlmDefaults,
    },
    load_desktop_window_size, EffectRunner, PersistenceWorker, PlatformEffectHandler, RuntimePaths,
    GUI_LOCK_IDENTITY,
};
use harvester_ui_bridge::{
    decode_intent, fetch_body as bridge_fetch_body, run_driver, BodyKey, BodyResponse, BodyTable,
    DriverTermination, SnapshotEnvelope, SnapshotSignal, CSP,
};
use tauri::Emitter;

use crate::probe::report;

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(350);

#[derive(Default)]
struct ResizeDebounceState {
    deadline_ms: AtomicU64,
    logical_width: AtomicI32,
    logical_height: AtomicI32,
    gate: Mutex<()>,
    changed: Condvar,
}

fn install_resize_persistence(window: &tauri::WebviewWindow, sender: mpsc::Sender<Msg>) {
    let state = Arc::new(ResizeDebounceState::default());
    let clock = Instant::now();
    let worker_state = Arc::clone(&state);
    thread::spawn(move || {
        let mut guard = worker_state.gate.lock().expect("resize debounce lock");
        loop {
            let deadline_ms = worker_state.deadline_ms.load(Ordering::Acquire);
            if deadline_ms == 0 {
                guard = worker_state
                    .changed
                    .wait(guard)
                    .expect("resize debounce wait");
                continue;
            }
            let now_ms = clock.elapsed().as_millis() as u64;
            if now_ms < deadline_ms {
                let (next, _) = worker_state
                    .changed
                    .wait_timeout(guard, Duration::from_millis(deadline_ms - now_ms))
                    .expect("resize debounce wait timeout");
                guard = next;
                continue;
            }
            if worker_state
                .deadline_ms
                .compare_exchange(deadline_ms, 0, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let _ = sender.send(Msg::DesktopWindowResizeCompleted {
                    inner_width: worker_state.logical_width.load(Ordering::Relaxed),
                    inner_height: worker_state.logical_height.load(Ordering::Relaxed),
                });
            }
        }
    });

    let resize_window = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Resized(size) = event {
            let scale_factor = match resize_window.scale_factor() {
                Ok(scale_factor) => scale_factor,
                Err(error) => {
                    engine_warn!("[desktop-window-size] failed to read scale factor: {error}");
                    return;
                }
            };
            let Some((width, height)) = harvester_ui_bridge::window::logical_inner_size(
                size.width,
                size.height,
                scale_factor,
            ) else {
                engine_warn!(
                    "[desktop-window-size] rejected physical size {}x{} scale_factor={}",
                    size.width,
                    size.height,
                    scale_factor
                );
                return;
            };
            state.logical_width.store(width, Ordering::Relaxed);
            state.logical_height.store(height, Ordering::Relaxed);
            let deadline_ms = (clock.elapsed() + RESIZE_DEBOUNCE).as_millis() as u64;
            state
                .deadline_ms
                .store(deadline_ms.max(1), Ordering::Release);
            state.changed.notify_one();
        }
    });
}

struct PlatformHandler;
impl PlatformEffectHandler for PlatformHandler {
    fn open_url(&self, url: &str) {
        if let Err(error) = open::that(url) {
            engine_warn!("[ui-host] open_url failed url={url} error={error}");
        }
    }
}

#[derive(Clone)]
struct HostState {
    sender: mpsc::Sender<Msg>,
    snapshot: Arc<RwLock<Option<SnapshotEnvelope>>>,
    bodies: Arc<RwLock<BodyTable>>,
    highest_emitted: Arc<AtomicU64>,
    highest_acked: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    probe_data: Option<Arc<ProbeData>>,
}

#[derive(Default)]
struct ProbeData {
    measurements: Mutex<ProbeMeasurements>,
    changed: Condvar,
}

#[derive(Default)]
struct ProbeMeasurements {
    emitted_at: HashMap<u64, Instant>,
    latency_ms: Vec<u64>,
    backlog: Vec<u64>,
    bytes: Vec<u64>,
    page: Option<harvester_ui_bridge::probe::ProbePageReport>,
}

#[tauri::command]
fn get_snapshot(state: tauri::State<'_, HostState>) -> Option<SnapshotEnvelope> {
    state.snapshot.read().expect("snapshot lock").clone()
}

#[tauri::command]
fn dispatch_intent(
    payload: serde_json::Value,
    state: tauri::State<'_, HostState>,
) -> Result<(), String> {
    let raw = serde_json::to_vec(&payload).map_err(|error| error.to_string())?;
    let intent = decode_intent("dispatch_intent", &raw).map_err(|error| error.to_string())?;
    match intent.into_effect(&IntentContext { now: Utc::now() }) {
        IntentEffect::Dispatch(message) => state
            .sender
            .send(message)
            .map_err(|error| error.to_string()),
        IntentEffect::Host(harvester_core::HostAction::CancelArchiveDialog) => Ok(()),
    }
}

#[tauri::command]
fn fetch_body(key: BodyKey, state: tauri::State<'_, HostState>) -> Option<BodyResponse> {
    bridge_fetch_body(&state.bodies.read().expect("body table lock"), key)
}

#[tauri::command]
fn probe_ack(generation: u64, state: tauri::State<'_, HostState>) {
    if let Some(probe) = &state.probe_data {
        state.highest_acked.fetch_max(generation, Ordering::Relaxed);
        let mut measurements = probe.measurements.lock().expect("probe measurements lock");
        if let Some(emitted_at) = measurements.emitted_at.remove(&generation) {
            measurements
                .latency_ms
                .push(emitted_at.elapsed().as_millis() as u64);
        }
        probe.changed.notify_all();
    }
}

#[tauri::command]
fn probe_report(
    report: harvester_ui_bridge::probe::ProbePageReport,
    state: tauri::State<'_, HostState>,
) {
    if let Some(probe) = &state.probe_data {
        probe
            .measurements
            .lock()
            .expect("probe measurements lock")
            .page = Some(report);
        probe.changed.notify_all();
    }
}

pub fn run(probe: bool) -> Result<(), String> {
    let root = repository_root();
    initialize_logging(&root);
    if probe {
        return run_probe(root);
    }
    let paths = runtime_paths(&root);
    let _lock =
        acquire_lock(&paths.output_dir, GUI_LOCK_IDENTITY, false).inspect_err(|message| {
            rfd::MessageDialog::new()
                .set_title("Harvester already running")
                .set_description(message)
                .set_level(rfd::MessageLevel::Error)
                .show();
        })?;

    let (width, height) = load_desktop_window_size(&paths.state_path)
        .filter(|(width, height)| {
            *width >= DEFAULT_WINDOW_WIDTH && *height >= DEFAULT_WINDOW_HEIGHT
        })
        .unwrap_or((DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT));
    let (sender, receiver) = mpsc::channel();
    let state = HostState {
        sender: sender.clone(),
        snapshot: Arc::new(RwLock::new(None)),
        bodies: Arc::new(RwLock::new(BodyTable::new())),
        highest_emitted: Arc::new(AtomicU64::new(0)),
        highest_acked: Arc::new(AtomicU64::new(0)),
        running: Arc::new(AtomicBool::new(true)),
        probe_data: None,
    };
    let (app_state, effect_runner) = prepare_state(&paths, width, &sender)?;
    let run_state = state.clone();
    let builder = tauri::Builder::default()
        .manage(state.clone())
        .register_uri_scheme_protocol("harvester", move |_context, request| {
            serve_asset(&root.join("frontend").join("dist"), request.uri().path())
        })
        .setup(move |app| {
            let url = "harvester://localhost/index.html"
                .parse()
                .map_err(|error| format!("invalid harvester URL: {error}"))?;
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::CustomProtocol(url),
            )
            .title("Harvester")
            .inner_size(f64::from(width), f64::from(height))
            .build()?;
            install_resize_persistence(&window, state.sender.clone());
            start_driver(
                app.handle().clone(),
                app_state,
                receiver,
                state.clone(),
                paths.clone(),
                effect_runner,
            );
            Ok(())
        });
    let context = tauri::generate_context!();
    let result = builder
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            dispatch_intent,
            fetch_body
        ])
        .run(context)
        .map_err(|error| error.to_string());
    if !run_state.running.load(Ordering::Relaxed) {
        return Err("Harvester stopped responding — see engine.log".into());
    }
    result
}

fn run_probe(root: PathBuf) -> Result<(), String> {
    let (sender, _receiver) = mpsc::channel();
    let state = HostState {
        sender,
        snapshot: Arc::new(RwLock::new(None)),
        bodies: Arc::new(RwLock::new(BodyTable::new())),
        highest_emitted: Arc::new(AtomicU64::new(0)),
        highest_acked: Arc::new(AtomicU64::new(0)),
        running: Arc::new(AtomicBool::new(true)),
        probe_data: Some(Arc::new(ProbeData::default())),
    };
    let asset_root = root.join("frontend").join("dist");
    let builder = tauri::Builder::default()
        .manage(state.clone())
        .register_uri_scheme_protocol("harvester", move |_context, request| {
            serve_asset(&asset_root, request.uri().path())
        })
        .setup(move |app| {
            let url = format!(
                "harvester://localhost/index.html?probe=1&durationMs={}",
                harvester_ui_bridge::probe::PROBE_DURATION.as_millis()
            )
            .parse()
            .map_err(|error| format!("invalid harvester URL: {error}"))?;
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::CustomProtocol(url))
                .title("Harvester IPC probe")
                .inner_size(
                    f64::from(DEFAULT_WINDOW_WIDTH),
                    f64::from(DEFAULT_WINDOW_HEIGHT),
                )
                .build()?;
            emit_probe_snapshots(app.handle().clone(), state.clone());
            Ok(())
        });
    builder
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            dispatch_intent,
            fetch_body,
            probe_ack,
            probe_report
        ])
        .run(tauri::generate_context!())
        .map_err(|error| error.to_string())
}

fn emit_probe_snapshots(app: tauri::AppHandle, host: HostState) {
    thread::spawn(move || {
        let interval = Duration::from_millis(1_000 / harvester_ui_bridge::probe::PROBE_RATE_HZ);
        let readiness = harvester_ui_bridge::probe::synthetic_snapshot(1).with_generation(1);
        host.highest_emitted.store(1, Ordering::Relaxed);
        *host.snapshot.write().expect("snapshot lock") = Some(readiness.clone());
        let _ = app.emit("harvester://snapshot", readiness);

        let probe = host.probe_data.as_ref().expect("probe state");
        let readiness_deadline = Instant::now() + harvester_ui_bridge::probe::PROBE_HOST_WATCHDOG;
        let mut measurements = probe.measurements.lock().expect("probe measurements lock");
        while host.highest_acked.load(Ordering::Relaxed) < 1 {
            let now = Instant::now();
            if now >= readiness_deadline {
                drop(measurements);
                engine_error!("[ui-probe] page readiness acknowledgement timed out");
                finish_probe(&host);
                return;
            }
            let (next, _) = probe
                .changed
                .wait_timeout(
                    measurements,
                    readiness_deadline.saturating_duration_since(now),
                )
                .expect("probe readiness wait");
            measurements = next;
        }
        drop(measurements);

        let case_started = Instant::now();
        let case_timeout = harvester_ui_bridge::probe::PROBE_DURATION
            .min(harvester_ui_bridge::probe::PROBE_CASE_TIMEOUT);
        let mut next_backlog_sample = Duration::from_millis(250);
        let mut generation: u64 = 1;
        while case_started.elapsed() < case_timeout {
            generation = generation.saturating_add(1);
            let envelope = harvester_ui_bridge::probe::synthetic_snapshot(generation)
                .with_generation(generation);
            let bytes = serde_json::to_vec(&envelope)
                .expect("envelope serializes")
                .len() as u64;
            host.highest_emitted.store(generation, Ordering::Relaxed);
            if let Some(probe) = &host.probe_data {
                let mut measurements = probe.measurements.lock().expect("probe measurements lock");
                measurements.emitted_at.insert(generation, Instant::now());
                measurements.bytes.push(bytes);
            }
            *host.snapshot.write().expect("snapshot lock") = Some(envelope.clone());
            let _ = app.emit("harvester://snapshot", envelope);
            if case_started.elapsed() >= next_backlog_sample {
                probe
                    .measurements
                    .lock()
                    .expect("probe measurements lock")
                    .backlog
                    .push(
                        host.highest_emitted
                            .load(Ordering::Relaxed)
                            .saturating_sub(host.highest_acked.load(Ordering::Relaxed)),
                    );
                next_backlog_sample += Duration::from_millis(250);
            }
            thread::sleep(interval);
        }
        let _ = app.emit(
            "harvester://probe-finished",
            serde_json::json!({ "finalGeneration": generation }),
        );

        let report_deadline = Instant::now() + harvester_ui_bridge::probe::PROBE_HOST_WATCHDOG;
        let mut measurements = probe.measurements.lock().expect("probe measurements lock");
        while measurements.page.is_none() {
            let now = Instant::now();
            if now >= report_deadline {
                engine_error!(
                    "[ui-probe] page report timed out after final generation={generation}"
                );
                break;
            }
            let (next, _) = probe
                .changed
                .wait_timeout(measurements, report_deadline.saturating_duration_since(now))
                .expect("probe report wait");
            measurements = next;
        }
        drop(measurements);
        finish_probe(&host);
    });
}

fn finish_probe(host: &HostState) {
    if !host.running.swap(false, Ordering::Relaxed) {
        return;
    }
    let measurements = host
        .probe_data
        .as_ref()
        .expect("probe state")
        .measurements
        .lock()
        .expect("probe measurements lock");
    let page = measurements.page.unwrap_or_default();
    let mut report = harvester_ui_bridge::probe::evaluate(
        measurements.latency_ms.clone(),
        measurements.backlog.clone(),
        page,
        measurements.bytes.clone(),
    );
    report.invoke_succeeded =
        host.highest_acked.load(Ordering::Relaxed) > 0 && measurements.page.is_some();
    report.passed &= report.invoke_succeeded;
    report.tauri_version = Some(tauri::VERSION.into());
    report.wry_version = Some(env!("HARVESTER_UI_WRY_VERSION").into());
    report.webview2_com_version = Some(env!("HARVESTER_UI_WEBVIEW2_COM_VERSION").into());
    report.webview2_runtime_version = tauri::webview_version().ok();
    engine_info!(
        "[ui-probe] case=ipc passed={} latency_p95_ms={} latency_max_ms={} backlog_p95={} backlog_max={} slow_frame_percent={} csp_fetch_rejected={} invoke_succeeded={}",
        report.passed,
        report.latency_ms_p95,
        report.latency_ms_max,
        report.backlog_p95,
        report.backlog_max,
        report.slow_frame_percent,
        report.csp_fetch_rejected,
        report.invoke_succeeded
    );
    if let Err(error) = report::write(&repository_root(), &report) {
        engine_error!("[ui-probe] report write failed: {error}");
        std::process::exit(1);
    }
    std::process::exit(if report.passed { 0 } else { 1 });
}

fn prepare_state(
    paths: &RuntimePaths,
    width: i32,
    sender: &mpsc::Sender<Msg>,
) -> Result<(AppState, EffectRunner), String> {
    let state = AppState::new();
    let defaults = HostLlmDefaults {
        default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_5_4_NANO),
        session_id_prefix: "session-",
    };
    let llm_concurrency = llm_max_concurrency_requests_from_env();
    let (runner, limits, _) = build_effect_runner(
        paths,
        sender.clone(),
        llm_concurrency,
        &defaults,
        Box::new(PlatformHandler),
        "OPENAI_API_KEY not set; LLM features disabled",
        None,
    )?;
    let availability =
        std::env::var("OPENAI_API_KEY")
            .is_err()
            .then_some(AiAvailability::Unavailable {
                reason: AiUnavailableReason::MissingApiKey,
            });
    let (state, effects) =
        prepare_desktop_startup_state(state, paths, width, llm_concurrency, availability, limits);
    if !effects.is_empty() {
        runner.enqueue(effects);
    }
    Ok((state, runner))
}

fn start_driver(
    app: tauri::AppHandle,
    state: AppState,
    receiver: mpsc::Receiver<Msg>,
    host: HostState,
    paths: RuntimePaths,
    effect_runner: EffectRunner,
) {
    let sender = host.sender.clone();
    let running = Arc::clone(&host.running);
    thread::spawn(move || {
        while running.load(Ordering::Relaxed) && sender.send(Msg::tick_at(Utc::now())).is_ok() {
            thread::sleep(Duration::from_millis(75));
        }
    });
    let persistence = Arc::new(Mutex::new(PersistenceWorker::new(
        paths.state_path.clone(),
        paths.blacklist_path.clone(),
    )));
    let runner = Arc::new(Mutex::new(effect_runner));
    let driver_host = host.clone();
    let command_app = app.clone();
    let signal_app = app.clone();
    thread::spawn(move || {
        let snapshot = Arc::clone(&driver_host.snapshot);
        let emitted = Arc::clone(&driver_host.highest_emitted);
        let bodies = Arc::clone(&driver_host.bodies);
        let effect_runner = Arc::clone(&runner);
        let result = run_driver(
            state,
            receiver,
            update,
            move |effects| {
                effect_runner
                    .lock()
                    .expect("effect runner lock")
                    .enqueue(effects)
            },
            move |command| {
                let _ = command_app.emit("harvester://ui-command", command);
            },
            move |signal| match signal {
                SnapshotSignal::Snapshot(envelope) => {
                    emitted.store(envelope.generation, Ordering::Relaxed);
                    *snapshot.write().expect("snapshot lock") = Some(envelope.clone());
                    let _ = signal_app.emit("harvester://snapshot", envelope);
                }
                SnapshotSignal::Fatal { message } => {
                    engine_error!("[ui-host] {message}");
                    driver_host.running.store(false, Ordering::Relaxed);
                    let envelope = SnapshotEnvelope::fatal(
                        emitted.load(Ordering::Relaxed).saturating_add(1),
                        message,
                    );
                    *snapshot.write().expect("snapshot lock") = Some(envelope.clone());
                    let _ = signal_app.emit("harvester://snapshot", envelope);
                }
            },
            move |snapshot| {
                persistence
                    .lock()
                    .expect("persistence lock")
                    .enqueue(snapshot)
            },
            bodies,
            {
                let started = std::time::Instant::now();
                move || started.elapsed()
            },
        );
        if result == DriverTermination::Fatal {
            engine_error!("[ui-host] driver terminated fatally");
        }
    });
}

fn serve_asset(root: &Path, path: &str) -> tauri::http::Response<Vec<u8>> {
    match harvester_ui_bridge::resolve(root, path)
        .and_then(|asset| std::fs::read(&asset.path).ok().map(|bytes| (asset, bytes)))
    {
        Some((asset, bytes)) => tauri::http::Response::builder()
            .header("content-type", asset.mime)
            .header("content-security-policy", CSP)
            .body(bytes)
            .expect("valid response"),
        None => tauri::http::Response::builder()
            .status(404)
            .header("content-security-policy", CSP)
            .body(b"not found".to_vec())
            .expect("valid response"),
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}
fn runtime_paths(root: &Path) -> RuntimePaths {
    let output = root.join("output");
    RuntimePaths::new(
        output.clone(),
        output.join(".sources.ron"),
        root.join("contexts"),
        root.join("prompts"),
    )
}
fn initialize_logging(root: &Path) {
    let path = root.join("engine.log");
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        engine_logging::initialize_at(path);
    } else {
        engine_logging::initialize_file_only_at(path);
    }
    engine_info!("[ui-host] logging initialized");
}
