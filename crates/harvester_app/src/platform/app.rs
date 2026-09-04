use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use commanductui::types::TreeItemMarkerKind;
use commanductui::{
    ControlId, PlatformCommand, PlatformEventHandler, PlatformInterface, UiStateProvider,
    WindowConfig, WindowId,
};

use harvester_core::{
    update, AiAvailability, AiUnavailableReason, AppState, JobFilterStatus, ManualDecision, Msg,
};

use engine_logging::engine_info;

use harvester_engine::llm::{ModelId, ProviderKind, OPENAI_MODEL_GPT_5_4_NANO};
use harvester_io::{
    acquire_lock,
    host_bootstrap::{build_effect_runner, HostLlmDefaults},
    load_window_size, EffectRunner, LockIdentity, PersistenceWorker, RuntimePaths,
};

use super::effects;
use super::logging::{self, LogDestination};
use super::ui;
use super::Win32PlatformHandler;

mod archive_dialog;
mod config;
mod event_handler;
mod render_batch;
mod startup;
mod ui_state;
use config::llm_max_concurrency_requests_from_env;
use startup::{assemble_startup_commands, prepare_startup_state};
use ui_state::AppUiStateProvider;

const GUI_LOCK_IDENTITY: LockIdentity = LockIdentity {
    filename: ".harvester_gui.lock",
    log_tag: "[gui-lock]",
    actor_description: "Harvester window",
    force_unlock_hint: None,
};

fn show_startup_lock_failure(message: &str) {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let title = HSTRING::from("Harvester already running");
    let message = HSTRING::from(message);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub fn run_app() -> commanductui::PlatformResult<()> {
    logging::initialize(LogDestination::Both);
    engine_info!("Logger initialized. Starting harvester_app...");

    const DEFAULT_WINDOW_WIDTH: i32 = 960;
    const DEFAULT_WINDOW_HEIGHT: i32 = 720;

    let output_dir = effects::default_output_dir();
    let paths = RuntimePaths::new(
        output_dir.clone(),
        effects::default_source_config_path(&output_dir),
        effects::contexts_directory(),
        effects::prompts_directory(),
    );

    let _lock_guard = match acquire_lock(&paths.output_dir, GUI_LOCK_IDENTITY, false) {
        Ok(guard) => guard,
        Err(message) => {
            show_startup_lock_failure(&message);
            return Err(commanductui::PlatformError::InitializationFailed(message));
        }
    };

    // Restore persisted window size, falling back to defaults.
    // Both dimensions must meet the minimum; otherwise use defaults for both.
    let (initial_width, initial_height) = load_window_size(&paths.state_path)
        .filter(|&(w, h)| w >= DEFAULT_WINDOW_WIDTH && h >= DEFAULT_WINDOW_HEIGHT)
        .unwrap_or((DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT));

    let platform = PlatformInterface::new("harvester_app".to_string())?;
    let window_id = platform.create_window(WindowConfig {
        title: "Harvester",
        width: initial_width,
        height: initial_height,
    })?;

    let shared_state = Arc::new(Mutex::new(SharedState::default()));
    let llm_max_concurrent_requests = llm_max_concurrency_requests_from_env();
    engine_info!(
        "[llm-concurrency] configured max_concurrent_requests={}",
        llm_max_concurrent_requests
    );

    let platform_handler = Box::new(Win32PlatformHandler);
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let startup_ai_availability = if std::env::var("OPENAI_API_KEY").is_ok() {
        None
    } else {
        Some(AiAvailability::Unavailable {
            reason: AiUnavailableReason::MissingApiKey,
        })
    };

    let defaults = HostLlmDefaults {
        default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_5_4_NANO),
        session_id_prefix: "session-",
    };
    let (effect_runner, startup_llm_quota_limits, _) = build_effect_runner(
        &paths,
        msg_tx.clone(),
        llm_max_concurrent_requests,
        &defaults,
        platform_handler,
        "OPENAI_API_KEY not set; LLM features disabled",
        None,
    )
    .map_err(commanductui::PlatformError::InitializationFailed)?;
    {
        let mut guard = shared_state.lock().expect("lock shared state");
        let state = std::mem::take(&mut guard.state);
        let (prepared_state, mut startup_effects) = prepare_startup_state(
            state,
            &paths,
            initial_width,
            llm_max_concurrent_requests,
            startup_ai_availability,
            startup_llm_quota_limits,
        );
        let (prepared_state, initial_tick_effects) =
            update(prepared_state, Msg::tick_at(chrono::Utc::now()));
        startup_effects.extend(initial_tick_effects);
        if !startup_effects.is_empty() {
            effect_runner.enqueue(startup_effects);
        }
        guard.state = prepared_state;
    }

    let initial_view = {
        let guard = shared_state.lock().expect("lock shared state");
        guard.state.view()
    };
    let mut tree_render_state = ui::render::TreeRenderState::new();
    let initial_commands =
        assemble_startup_commands(window_id, &initial_view, &mut tree_render_state);

    let event_handler: Arc<Mutex<dyn PlatformEventHandler>> =
        Arc::new(Mutex::new(AppEventHandler::new(
            window_id,
            shared_state.clone(),
            msg_rx,
            msg_tx.clone(),
            effect_runner,
            PersistenceWorker::new(paths.state_path.clone(), paths.blacklist_path.clone()),
            tree_render_state,
        )));
    let ui_state_provider: Arc<Mutex<dyn UiStateProvider>> =
        Arc::new(Mutex::new(AppUiStateProvider::new(shared_state)));

    // Background tick to throttle rendering and UI updates.
    thread::spawn(move || {
        let interval = Duration::from_millis(75);
        while msg_tx.send(Msg::tick_at(chrono::Utc::now())).is_ok() {
            thread::sleep(interval);
        }
    });

    platform.main_event_loop(event_handler, ui_state_provider, initial_commands)
}

#[derive(Default)]
pub(super) struct SharedState {
    pub(super) state: AppState,
}

pub(super) struct PendingFocus {
    pub(super) control_id: ControlId,
    pub(super) select_all: bool,
}

pub(super) struct AppEventHandler {
    pub(super) window_id: WindowId,
    pub(super) shared: Arc<Mutex<SharedState>>,
    pub(super) commands: VecDeque<PlatformCommand>,
    pub(super) msg_rx: Mutex<mpsc::Receiver<Msg>>,
    pub(super) msg_tx: mpsc::Sender<Msg>,
    pub(super) effect_runner: EffectRunner,
    pub(super) persistence_worker: PersistenceWorker,
    pub(super) tree_render_state: ui::render::TreeRenderState,
    pub(super) pending_focus_after_render: Vec<PendingFocus>,
}

fn triage_marker_for_priority(priority: u8) -> TreeItemMarkerKind {
    match priority {
        6..=u8::MAX => TreeItemMarkerKind::Red,
        5 => TreeItemMarkerKind::Yellow,
        4 => TreeItemMarkerKind::Purple,
        3 => TreeItemMarkerKind::Gray,
        _ => TreeItemMarkerKind::None,
    }
}

pub(super) fn pre_triage_toggle_message(state: &AppState) -> Option<Msg> {
    let job_id = state.selected_job_id()?;
    let key = state.pre_triage_key_for_job(job_id)?;
    let decision = match state.job_filter_status(job_id) {
        Some(JobFilterStatus::HardExcluded { .. }) | Some(JobFilterStatus::ManuallyExcluded) => {
            ManualDecision::Include
        }
        _ => ManualDecision::Exclude,
    };
    Some(Msg::PreTriageDecisionSet { key, decision })
}

#[cfg(test)]
mod tests;
