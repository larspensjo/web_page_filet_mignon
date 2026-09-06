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
use tauri::{Emitter, Manager};

use crate::probe::report;

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(350);
const PROBE_WINDOW_TITLE: &str = "Harvester IPC probe";

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
    snapshot_reads: Arc<AtomicU64>,
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

impl ProbeMeasurements {
    fn reset_for_case(&mut self) {
        self.emitted_at.clear();
        self.latency_ms.clear();
        self.backlog.clear();
        self.bytes.clear();
        self.page = None;
    }
}

#[tauri::command]
fn get_snapshot(state: tauri::State<'_, HostState>) -> Option<SnapshotEnvelope> {
    if let Some(probe) = &state.probe_data {
        let _measurements = probe.measurements.lock().expect("probe measurements lock");
        // This counter is the probe's predicate for detecting a newly created page's first read.
        state.snapshot_reads.fetch_add(1, Ordering::Relaxed);
        probe.changed.notify_all();
    }
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
        snapshot_reads: Arc::new(AtomicU64::new(0)),
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
        snapshot_reads: Arc::new(AtomicU64::new(0)),
        running: Arc::new(AtomicBool::new(true)),
        probe_data: Some(Arc::new(ProbeData::default())),
    };
    let asset_root = root.join("frontend").join("dist");
    let builder = tauri::Builder::default()
        .manage(state.clone())
        .register_uri_scheme_protocol("harvester", move |_context, request| {
            engine_info!("[ui-probe] asset request uri={}", request.uri());
            serve_asset(&asset_root, request.uri().path())
        })
        .setup(move |app| {
            let case = harvester_ui_bridge::probe::ProbeCase::TypicalScope;
            let label = probe_window_label(case);
            let url = probe_url(case)?;
            engine_info!(
                "[ui-probe] case={} initial window create target label={label} url={url}",
                case.slug()
            );
            let result = build_probe_window(app, &label, url);
            engine_info!(
                "[ui-probe] case={} initial window create returned result={:?}",
                case.slug(),
                result.as_ref().map(|_| ())
            );
            result?;
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
        let mut generation = 0;
        let mut samples = Vec::new();
        let watchdog_deadline = Instant::now() + harvester_ui_bridge::probe::PROBE_HOST_WATCHDOG;
        for (index, case) in harvester_ui_bridge::probe::ProbeCase::ALL
            .into_iter()
            .enumerate()
        {
            if Instant::now() >= watchdog_deadline {
                engine_error!(
                    "[ui-probe] overall watchdog expired before case={}",
                    case.slug()
                );
                break;
            }
            engine_info!(
                "[ui-probe] case={} starting gated={} duration_ms={}",
                case.slug(),
                case.is_gated(),
                case.duration().as_millis()
            );
            if index > 0 {
                let snapshot_reads_before = host.snapshot_reads.load(Ordering::Relaxed);
                *host.snapshot.write().expect("snapshot lock") = None;
                engine_info!(
                    "[ui-probe] case={} creating fresh page window snapshot_reads_before={snapshot_reads_before}",
                    case.slug()
                );
                let previous_case = harvester_ui_bridge::probe::ProbeCase::ALL[index - 1];
                if let Err(error) =
                    replace_probe_window(&app, previous_case, case, watchdog_deadline)
                {
                    engine_error!(
                        "[ui-probe] case={} window replacement failed: {error}",
                        case.slug()
                    );
                    break;
                }
                if !wait_for_probe_page_mount(&host, case, snapshot_reads_before, watchdog_deadline)
                {
                    break;
                }
                engine_info!(
                    "[ui-probe] case={} page mounted snapshot_reads={}",
                    case.slug(),
                    host.snapshot_reads.load(Ordering::Relaxed)
                );
            }
            let (case_samples, final_generation, page_report_received) =
                run_probe_case(&app, &host, case, generation, interval, watchdog_deadline);
            generation = final_generation;
            samples.push(case_samples);
            if !page_report_received {
                engine_error!(
                    "[ui-probe] case={} aborting remaining cases because no page report arrived",
                    case.slug()
                );
                break;
            }
        }
        finish_probe(&host, samples);
    });
}

fn wait_for_probe_page_mount(
    host: &HostState,
    case: harvester_ui_bridge::probe::ProbeCase,
    snapshot_reads_before: u64,
    watchdog_deadline: Instant,
) -> bool {
    let probe = host.probe_data.as_ref().expect("probe state");
    let mount_deadline = probe_page_deadline(Instant::now(), watchdog_deadline);
    let mut measurements = probe.measurements.lock().expect("probe measurements lock");
    while host.snapshot_reads.load(Ordering::Relaxed) <= snapshot_reads_before {
        let now = Instant::now();
        if now >= mount_deadline {
            engine_error!(
                "[ui-probe] case={} new page did not request its initial snapshot after window creation",
                case.slug()
            );
            return false;
        }
        let (next, _) = probe
            .changed
            .wait_timeout(measurements, mount_deadline.saturating_duration_since(now))
            .expect("probe page-mount wait");
        measurements = next;
    }
    true
}

fn run_probe_case(
    app: &tauri::AppHandle,
    host: &HostState,
    case: harvester_ui_bridge::probe::ProbeCase,
    previous_generation: u64,
    interval: Duration,
    watchdog_deadline: Instant,
) -> (harvester_ui_bridge::probe::ProbeCaseSamples, u64, bool) {
    let probe = host.probe_data.as_ref().expect("probe state");
    {
        let mut measurements = probe.measurements.lock().expect("probe measurements lock");
        measurements.reset_for_case();
    }
    let readiness_generation = previous_generation.saturating_add(1);
    let readiness = harvester_ui_bridge::probe::synthetic_snapshot(case, readiness_generation)
        .with_generation(readiness_generation);
    host.highest_emitted
        .store(readiness_generation, Ordering::Relaxed);
    *host.snapshot.write().expect("snapshot lock") = Some(readiness.clone());
    let _ = app.emit("harvester://snapshot", readiness);

    let readiness_deadline = probe_page_deadline(Instant::now(), watchdog_deadline);
    let mut measurements = probe.measurements.lock().expect("probe measurements lock");
    while host.highest_acked.load(Ordering::Relaxed) < readiness_generation {
        let now = Instant::now();
        if now >= readiness_deadline {
            engine_error!(
                "[ui-probe] case={} page readiness acknowledgement timed out generation={readiness_generation}",
                case.slug()
            );
            let page_report_received = measurements.page.is_some();
            return (
                samples_for_case(case, &measurements, false),
                readiness_generation,
                page_report_received,
            );
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
    engine_info!(
        "[ui-probe] case={} readiness acknowledged generation={readiness_generation}",
        case.slug()
    );

    let case_started = Instant::now();
    let mut next_backlog_sample = Duration::from_millis(250);
    let mut generation = readiness_generation;
    while case_started.elapsed() < case.duration() {
        generation = generation.saturating_add(1);
        let envelope = harvester_ui_bridge::probe::synthetic_snapshot(case, generation)
            .with_generation(generation);
        let bytes = serde_json::to_vec(&envelope)
            .expect("envelope serializes")
            .len() as u64;
        host.highest_emitted.store(generation, Ordering::Relaxed);
        {
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
    engine_info!(
        "[ui-probe] case={} emission complete final_generation={generation} highest_acked={} elapsed_ms={}",
        case.slug(),
        host.highest_acked.load(Ordering::Relaxed),
        case_started.elapsed().as_millis()
    );
    let _ = app.emit(
        "harvester://probe-finished",
        serde_json::json!({ "finalGeneration": generation }),
    );

    let report_deadline = probe_page_deadline(Instant::now(), watchdog_deadline);
    let mut measurements = probe.measurements.lock().expect("probe measurements lock");
    while measurements.page.is_none() {
        let now = Instant::now();
        if now >= report_deadline {
            engine_error!(
                "[ui-probe] case={} page report timed out after final generation={generation} highest_acked={}",
                case.slug(),
                host.highest_acked.load(Ordering::Relaxed)
            );
            let page_report_received = measurements.page.is_some();
            return (
                samples_for_case(case, &measurements, false),
                generation,
                page_report_received,
            );
        }
        let (next, _) = probe
            .changed
            .wait_timeout(measurements, report_deadline.saturating_duration_since(now))
            .expect("probe report wait");
        measurements = next;
    }
    engine_info!(
        "[ui-probe] case={} page report received frames={} slow_frames={}",
        case.slug(),
        measurements
            .page
            .map(|page| page.frames)
            .unwrap_or_default(),
        measurements
            .page
            .map(|page| page.slow_frames)
            .unwrap_or_default()
    );
    let invoke_succeeded = host.highest_acked.load(Ordering::Relaxed) >= generation;
    let page_report_received = measurements.page.is_some();
    (
        samples_for_case(case, &measurements, invoke_succeeded),
        generation,
        page_report_received,
    )
}

fn samples_for_case(
    case: harvester_ui_bridge::probe::ProbeCase,
    measurements: &ProbeMeasurements,
    invoke_succeeded: bool,
) -> harvester_ui_bridge::probe::ProbeCaseSamples {
    harvester_ui_bridge::probe::ProbeCaseSamples {
        case,
        latency_ms: measurements.latency_ms.clone(),
        backlog: measurements.backlog.clone(),
        envelope_bytes: measurements.bytes.clone(),
        page: measurements.page.unwrap_or_default(),
        invoke_succeeded,
    }
}

fn probe_url(case: harvester_ui_bridge::probe::ProbeCase) -> Result<tauri::Url, String> {
    format!(
        "harvester://localhost/index.html?probe=1&case={}&durationMs={}",
        case.slug(),
        case.duration().as_millis()
    )
    .parse()
    .map_err(|error| format!("invalid harvester probe URL: {error}"))
}

fn probe_window_label(case: harvester_ui_bridge::probe::ProbeCase) -> String {
    format!("probe-{}", case.slug())
}

fn probe_page_deadline(now: Instant, watchdog_deadline: Instant) -> Instant {
    watchdog_deadline.min(now + harvester_ui_bridge::probe::PROBE_PAGE_RESPONSE_TIMEOUT)
}

fn build_probe_window<R: tauri::Runtime, M: tauri::Manager<R>>(
    manager: &M,
    label: &str,
    url: tauri::Url,
) -> Result<tauri::WebviewWindow<R>, String> {
    tauri::WebviewWindowBuilder::new(manager, label, tauri::WebviewUrl::CustomProtocol(url))
        .title(PROBE_WINDOW_TITLE)
        .inner_size(
            f64::from(DEFAULT_WINDOW_WIDTH),
            f64::from(DEFAULT_WINDOW_HEIGHT),
        )
        .build()
        .map_err(|error| error.to_string())
}

fn replace_probe_window(
    app: &tauri::AppHandle,
    previous_case: harvester_ui_bridge::probe::ProbeCase,
    next_case: harvester_ui_bridge::probe::ProbeCase,
    watchdog_deadline: Instant,
) -> Result<(), String> {
    let previous_label = probe_window_label(previous_case);
    let next_label = probe_window_label(next_case);
    let url = probe_url(next_case)?;
    engine_info!(
        "[ui-probe] case={} window replacement target previous_label={previous_label} next_label={next_label} url={url}",
        next_case.slug()
    );
    let window_app = app.clone();
    let (result_sender, result_receiver) = mpsc::sync_channel(1);
    let queued = app
        .run_on_main_thread(move || {
            engine_info!(
                "[ui-probe] case={} issuing window creation on main thread label={next_label} url={url}",
                next_case.slug()
            );
            let create_result = build_probe_window(&window_app, &next_label, url);
            engine_info!(
                "[ui-probe] case={} window creation returned result={:?}",
                next_case.slug(),
                create_result.as_ref().map(|_| ())
            );
            let result = create_result.and_then(|_| {
                let previous_window = window_app
                    .get_webview_window(&previous_label)
                    .ok_or_else(|| format!("previous probe window label={previous_label} is unavailable"))?;
                engine_info!(
                    "[ui-probe] case={} issuing previous window close on main thread label={previous_label}",
                    next_case.slug()
                );
                let close_result = previous_window.close().map_err(|error| {
                    format!("failed to close previous probe window label={previous_label}: {error}")
                });
                engine_info!(
                    "[ui-probe] case={} previous window close returned result={close_result:?}",
                    next_case.slug()
                );
                close_result
            });
            engine_info!(
                "[ui-probe] case={} window replacement returned result={result:?}",
                next_case.slug()
            );
            let _ = result_sender.send(result);
        })
        .map_err(|error| format!("failed to queue window replacement on the main thread: {error}"));
    await_main_thread_result(
        queued,
        result_receiver,
        next_case,
        "window replacement",
        watchdog_deadline,
    )
}

fn await_main_thread_result<T>(
    queued: Result<(), String>,
    result_receiver: mpsc::Receiver<Result<T, String>>,
    case: harvester_ui_bridge::probe::ProbeCase,
    operation: &str,
    watchdog_deadline: Instant,
) -> Result<T, String> {
    queued?;
    let now = Instant::now();
    let timeout = probe_page_deadline(now, watchdog_deadline).saturating_duration_since(now);
    match result_receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            engine_error!(
                "[ui-probe] case={} main-thread step={operation} timed out after {} ms",
                case.slug(),
                timeout.as_millis()
            );
            Err(format!(
                "main-thread {operation} timed out after {} ms",
                timeout.as_millis()
            ))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(format!(
            "main-thread {operation} did not return a result: channel disconnected"
        )),
    }
}

fn finish_probe(host: &HostState, samples: Vec<harvester_ui_bridge::probe::ProbeCaseSamples>) {
    if !host.running.swap(false, Ordering::Relaxed) {
        return;
    }
    let mut report = harvester_ui_bridge::probe::evaluate(samples);
    report.tauri_version = Some(tauri::VERSION.into());
    report.wry_version = Some(env!("HARVESTER_UI_WRY_VERSION").into());
    report.webview2_com_version = Some(env!("HARVESTER_UI_WEBVIEW2_COM_VERSION").into());
    report.webview2_runtime_version = tauri::webview_version().ok();
    for required in harvester_ui_bridge::probe::ProbeCase::ALL
        .into_iter()
        .filter(|case| case.is_gated())
    {
        if !report.cases.iter().any(|case| case.case == required.slug()) {
            engine_error!(
                "[ui-probe] incomplete run missing gated case={}",
                required.slug()
            );
        }
    }
    for case in &report.cases {
        engine_info!(
            "[ui-probe] case={} gated={} passed={} envelope_bytes_p95={} latency_p95_ms={} latency_max_ms={} backlog_p95={} backlog_max={} slow_frame_percent={} csp_fetch_rejected={} invoke_succeeded={}",
            case.case,
            case.gated,
            case.passed,
            case.envelope_bytes_p95,
            case.latency_ms_p95,
            case.latency_ms_max,
            case.backlog_p95,
            case.backlog_max,
            case.slow_frame_percent,
            case.csp_fetch_rejected,
            case.invoke_succeeded
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn main_thread_result_deadline() -> Instant {
        Instant::now() + Duration::from_secs(1)
    }

    #[test]
    fn main_thread_result_returns_the_inner_operation_result() {
        let (sender, receiver) = mpsc::sync_channel::<Result<(), String>>(1);
        sender
            .send(Err("webview operation failed".to_string()))
            .unwrap();

        assert_eq!(
            await_main_thread_result(
                Ok(()),
                receiver,
                harvester_ui_bridge::probe::ProbeCase::TypicalScope,
                "window replacement",
                main_thread_result_deadline(),
            ),
            Err("webview operation failed".to_string())
        );
    }

    #[test]
    fn main_thread_result_returns_successful_operation_value() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.send(Ok(42)).unwrap();

        assert_eq!(
            await_main_thread_result(
                Ok(()),
                receiver,
                harvester_ui_bridge::probe::ProbeCase::TypicalScope,
                "window replacement",
                main_thread_result_deadline(),
            ),
            Ok(42)
        );
    }

    #[test]
    fn main_thread_result_propagates_queue_failure() {
        let (_sender, receiver) = mpsc::sync_channel::<Result<(), String>>(1);

        assert_eq!(
            await_main_thread_result(
                Err("main-thread queue unavailable".to_string()),
                receiver,
                harvester_ui_bridge::probe::ProbeCase::TypicalScope,
                "window replacement",
                main_thread_result_deadline(),
            ),
            Err("main-thread queue unavailable".to_string())
        );
    }

    #[test]
    fn main_thread_result_reports_a_disconnected_result_channel() {
        let (sender, receiver) = mpsc::sync_channel::<Result<(), String>>(1);
        drop(sender);

        let error = await_main_thread_result(
            Ok(()),
            receiver,
            harvester_ui_bridge::probe::ProbeCase::TypicalScope,
            "window replacement",
            main_thread_result_deadline(),
        )
        .expect_err("a disconnected main-thread result channel must fail");

        assert!(error.starts_with("main-thread window replacement did not return a result:"));
    }

    #[test]
    fn main_thread_result_times_out_at_the_page_deadline() {
        let (_sender, receiver) = mpsc::sync_channel::<Result<(), String>>(1);

        let error = await_main_thread_result(
            Ok(()),
            receiver,
            harvester_ui_bridge::probe::ProbeCase::TypicalScope,
            "window replacement",
            Instant::now(),
        )
        .expect_err("a stalled main-thread operation must time out");

        assert!(error.starts_with("main-thread window replacement timed out after"));
    }

    #[test]
    fn probe_window_labels_are_distinct_and_derived_from_case_slugs() {
        let labels = harvester_ui_bridge::probe::ProbeCase::ALL.map(probe_window_label);

        for (case, label) in harvester_ui_bridge::probe::ProbeCase::ALL
            .into_iter()
            .zip(&labels)
        {
            assert_eq!(label, &format!("probe-{}", case.slug()));
        }
        for (index, label) in labels.iter().enumerate() {
            assert!(!labels[..index].contains(label));
        }
    }

    #[test]
    fn probe_page_deadline_uses_the_per_step_timeout_when_it_expires_first() {
        let now = Instant::now();
        let watchdog_deadline = now + harvester_ui_bridge::probe::PROBE_HOST_WATCHDOG;

        assert_eq!(
            probe_page_deadline(now, watchdog_deadline),
            now + harvester_ui_bridge::probe::PROBE_PAGE_RESPONSE_TIMEOUT
        );
    }

    #[test]
    fn probe_page_deadline_uses_the_overall_watchdog_when_it_expires_first() {
        let now = Instant::now();
        let watchdog_deadline = now + Duration::from_secs(1);

        assert_eq!(
            probe_page_deadline(now, watchdog_deadline),
            watchdog_deadline
        );
    }
}
