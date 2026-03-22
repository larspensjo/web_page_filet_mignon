use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;

use commanductui::types::{
    FormButtons, FormDialogDescriptor, FormField, FormFieldValue, FormFileExistsWarning, FormRow,
    FormTextValidation, MenuActionId, MessageSeverity, TreeItemMarkerKind,
};
use commanductui::{
    AppEvent, CheckState, PlatformCommand, PlatformEventHandler, PlatformInterface,
    UiStateProvider, WindowConfig, WindowId,
};
use harvester_core::{
    update, AppState, AppTab, AppViewModel, Effect, JobFilterStatus, JobListScope, JobResultKind,
    LayoutViewModel, LeftTab, LinkDownloadState, ManualDecision, Msg, PromptLabStage,
    TrendCategory,
};

use engine_logging::{engine_info, engine_warn};

use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::prompts::register_defaults;
use harvester_engine::llm::{
    LlmConfig, LlmHandle, LlmQuotas, ModelId, OpenAiProvider, PricingRegistry, PromptRegistry,
    ProviderKind, DEFAULT_BRIEFING_MODEL, DEFAULT_SUMMARY_MODEL, DEFAULT_TRIAGE_MODEL,
    OPENAI_MODEL_GPT_5_4_NANO,
};
use harvester_io::{
    load_completed_jobs, load_pre_triage_overrides, load_summary_cache, load_triage_cache,
    EffectRunner, PersistenceSnapshot, PersistenceWorker, RuntimePaths,
};

use super::effects;
use super::logging::{self, LogDestination};
use super::ui;
use super::ui::tree_item_ids::{decode_tree_item_id, TreeItemKind};
use super::Win32PlatformHandler;

const MENU_ACTION_ADD_URL: MenuActionId = MenuActionId(1);
const MENU_ACTION_ARCHIVE: MenuActionId = MenuActionId(2);
const MENU_ACTION_PROMPT_LAB: MenuActionId = MenuActionId(3);
const ARCHIVE_DIALOG_CONTEXT_PREFIX: &str = "archive:";
const ARCHIVE_DIALOG_FILENAME_FIELD_ID: &str = "archive.basename";
const ARCHIVE_DIALOG_SET_CHECKPOINT_FIELD_ID: &str = "archive.set_checkpoint";
const DEFAULT_LLM_MAX_CONCURRENT_REQUESTS: usize = 3;
const LLM_MAX_CONCURRENT_REQUESTS_ENV: &str = "LLM_MAX_CONCURRENT_REQUESTS";
const MAX_LLM_CONCURRENT_REQUESTS: usize = 10;

pub fn run_app() -> commanductui::PlatformResult<()> {
    logging::initialize(LogDestination::Both);
    engine_info!("Logger initialized. Starting harvester_app...");

    let platform = PlatformInterface::new("harvester_app".to_string())?;
    let window_id = platform.create_window(WindowConfig {
        title: "Harvester",
        width: 960,
        height: 720,
    })?;

    let shared_state = Arc::new(Mutex::new(SharedState::default()));
    let llm_max_concurrent_requests = llm_max_concurrency_requests_from_env();
    {
        let mut guard = shared_state.lock().expect("lock shared state");
        guard
            .state
            .set_triage_max_in_flight(llm_max_concurrent_requests);
        guard
            .state
            .set_summary_max_in_flight(llm_max_concurrent_requests);
    }
    engine_info!(
        "[llm-concurrency] configured max_concurrent_requests={}",
        llm_max_concurrent_requests
    );

    let output_dir = effects::default_output_dir();
    let paths = RuntimePaths::new(
        output_dir.clone(),
        effects::default_source_config_path(),
        effects::contexts_directory(),
        effects::prompts_directory(),
    );
    let platform_handler = Box::new(Win32PlatformHandler);
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let effect_runner = if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        let provider: Arc<dyn harvester_engine::llm::provider::LlmProvider> =
            Arc::new(OpenAiProvider::new(api_key));
        let provider_clone = Arc::clone(&provider);
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        let registry = Arc::new(RwLock::new(registry));
        let config = LlmConfig {
            provider,
            default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_5_4_NANO),
            triage_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_TRIAGE_MODEL)),
            summary_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_SUMMARY_MODEL)),
            briefing_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_BRIEFING_MODEL)),
            registry: Arc::clone(&registry),
            quotas: LlmQuotas::default(),
            output_dir: output_dir.clone(),
            pricing: PricingRegistry::with_defaults(),
            max_input_bytes: 100_000,
            #[allow(deprecated)]
            max_input_chars: 0,
            timestamp_utc: Arc::new(|| Utc::now().to_rfc3339()),
            session_id: format!("session-{}", Utc::now().format("%Y%m%d-%H%M%S")),
            replay_cache: None,
            max_concurrent_requests: llm_max_concurrent_requests,
        };
        let model_map = effective_model_map(&config);
        let handle = LlmHandle::new(config);
        EffectRunner::new_with_llm(
            paths.clone(),
            msg_tx.clone(),
            handle,
            100_000,
            Arc::clone(&registry),
            model_map,
            provider_clone,
            ProviderKind::OpenAi,
            platform_handler,
        )
    } else {
        engine_warn!("OPENAI_API_KEY not set; LLM features disabled");
        EffectRunner::new(paths.clone(), msg_tx.clone(), platform_handler)
    };
    effect_runner.enqueue(vec![
        Effect::LoadPromptTemplateFiles,
        Effect::LoadLlmMetadata,
    ]);
    {
        let mut guard = shared_state.lock().unwrap();
        let state = std::mem::take(&mut guard.state);
        let (state, effects) = update(state, Msg::StartupHydrationRequested);
        if !effects.is_empty() {
            effect_runner.enqueue(effects);
        }
        guard.state = state;
    }
    {
        let completed = load_completed_jobs(&paths.state_path);
        if !completed.is_empty() {
            let mut guard = shared_state.lock().unwrap();
            let state = std::mem::take(&mut guard.state);
            let (state, effects) = update(state, Msg::RestoreCompletedJobs(completed));
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
            guard.state = state;
        }
    }

    // Hydrate summary cache from persistent store
    {
        let cache = load_summary_cache(&paths.summary_cache_path);
        if !cache.is_empty() {
            let mut guard = shared_state.lock().unwrap();
            let state = std::mem::take(&mut guard.state);
            let (state, effects) = update(state, Msg::SummaryCacheHydrated { cache });
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
            guard.state = state;
        }
    }

    {
        let cache = load_triage_cache(&paths.triage_cache_path);
        if !cache.is_empty() {
            let mut guard = shared_state.lock().unwrap();
            let state = std::mem::take(&mut guard.state);
            let (state, effects) = update(state, Msg::TriageCacheHydrated { cache });
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
            guard.state = state;
        }
    }
    {
        let overrides = load_pre_triage_overrides(&paths.state_path);
        if !overrides.is_empty() {
            let mut guard = shared_state.lock().unwrap();
            let state = std::mem::take(&mut guard.state);
            let (state, effects) = update(state, Msg::PreTriageOverridesHydrated { overrides });
            if !effects.is_empty() {
                effect_runner.enqueue(effects);
            }
            guard.state = state;
        }
    }

    let initial_view = shared_state.lock().unwrap().state.view();
    let mut tree_render_state = ui::render::TreeRenderState::new();
    let mut initial_commands = ui::layout::initial_commands(window_id);
    initial_commands.extend(ui::render::render(
        window_id,
        &initial_view,
        &mut tree_render_state,
    ));

    let event_handler: Arc<Mutex<dyn PlatformEventHandler>> =
        Arc::new(Mutex::new(AppEventHandler::new(
            window_id,
            shared_state.clone(),
            msg_rx,
            msg_tx.clone(),
            effect_runner,
            PersistenceWorker::new(paths.state_path.clone()),
            tree_render_state,
        )));
    let ui_state_provider: Arc<Mutex<dyn UiStateProvider>> =
        Arc::new(Mutex::new(AppUiStateProvider::new(shared_state)));

    // Background tick to throttle rendering and UI updates.
    thread::spawn(move || {
        let interval = Duration::from_millis(75);
        while msg_tx.send(Msg::Tick).is_ok() {
            thread::sleep(interval);
        }
    });

    platform.main_event_loop(event_handler, ui_state_provider, initial_commands)
}

fn parse_llm_max_concurrency_requests(raw: Option<&str>) -> usize {
    match raw {
        None => DEFAULT_LLM_MAX_CONCURRENT_REQUESTS,
        Some(value) => match value.trim().parse::<usize>() {
            Ok(parsed) => parsed.clamp(1, MAX_LLM_CONCURRENT_REQUESTS),
            Err(_) => DEFAULT_LLM_MAX_CONCURRENT_REQUESTS,
        },
    }
}

fn llm_max_concurrency_requests_from_env() -> usize {
    let raw = std::env::var(LLM_MAX_CONCURRENT_REQUESTS_ENV).ok();
    let parsed = parse_llm_max_concurrency_requests(raw.as_deref());
    if let Some(value) = raw {
        engine_info!(
            "[llm-concurrency] {}='{}' -> {}",
            LLM_MAX_CONCURRENT_REQUESTS_ENV,
            value,
            parsed
        );
    }
    parsed
}

fn effective_model_map(config: &LlmConfig) -> HashMap<PromptId, String> {
    let mut map = HashMap::new();

    let triage_model = config
        .triage_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleTriage, triage_model);

    let summary_model = config
        .summary_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleSummary, summary_model);

    let briefing_model = config
        .briefing_model
        .as_ref()
        .unwrap_or(&config.default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::AggregateBriefing, briefing_model);

    map
}

#[derive(Default)]
struct SharedState {
    state: AppState,
}

struct AppEventHandler {
    window_id: WindowId,
    shared: Arc<Mutex<SharedState>>,
    commands: VecDeque<PlatformCommand>,
    msg_rx: Mutex<mpsc::Receiver<Msg>>,
    msg_tx: mpsc::Sender<Msg>,
    effect_runner: EffectRunner,
    persistence_worker: PersistenceWorker,
    tree_render_state: ui::render::TreeRenderState,
}

fn job_id_for_item(item_id: commanductui::TreeItemId) -> Option<harvester_core::JobId> {
    match decode_tree_item_id(item_id) {
        TreeItemKind::Job { job_id } => Some(job_id),
        TreeItemKind::LinksFolder { job_id }
        | TreeItemKind::LinksShowMore { job_id }
        | TreeItemKind::Link { job_id, .. } => Some(job_id),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    Full,
    LayoutOnly,
}

enum PendingRender {
    Full(Box<AppViewModel>),
    LayoutOnly(LayoutViewModel),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct GeometryBatchStats {
    splitter_moves: usize,
    window_resizes: usize,
    last_splitter_width: Option<i32>,
    last_window_width: Option<i32>,
}

impl GeometryBatchStats {
    fn observe(&mut self, msg: &Msg) {
        match msg {
            Msg::SplitterMoved {
                desired_left_width_px,
            } => {
                self.splitter_moves += 1;
                self.last_splitter_width = Some(*desired_left_width_px);
            }
            Msg::WindowResized { window_width } => {
                self.window_resizes += 1;
                self.last_window_width = Some(*window_width);
            }
            _ => {}
        }
    }

    fn is_empty(self) -> bool {
        self.splitter_moves == 0 && self.window_resizes == 0
    }
}

fn is_geometry_only_message(msg: &Msg) -> bool {
    matches!(msg, Msg::SplitterMoved { .. } | Msg::WindowResized { .. })
}

fn select_render_mode(
    any_dirty: bool,
    geometry_only_batch: bool,
    refresh_evaluation_dispatched: bool,
    clear_input_needed: bool,
    effect_count: usize,
) -> Option<RenderMode> {
    if !any_dirty {
        return None;
    }
    if geometry_only_batch
        && !refresh_evaluation_dispatched
        && !clear_input_needed
        && effect_count == 0
    {
        return Some(RenderMode::LayoutOnly);
    }
    Some(RenderMode::Full)
}

fn archive_dialog_context_tag(request_id: u64) -> String {
    format!("{ARCHIVE_DIALOG_CONTEXT_PREFIX}{request_id}")
}

fn parse_archive_dialog_request_id(context_tag: &str) -> Option<u64> {
    context_tag
        .strip_prefix(ARCHIVE_DIALOG_CONTEXT_PREFIX)
        .and_then(|raw| raw.parse::<u64>().ok())
}

fn archive_field_text(field_values: &[FormFieldValue], field_id: &str) -> Option<String> {
    field_values.iter().find_map(|value| match value {
        FormFieldValue::Text {
            field_id: value_field_id,
            value,
        } if value_field_id == field_id => Some(value.clone()),
        _ => None,
    })
}

fn archive_field_checked(field_values: &[FormFieldValue], field_id: &str) -> Option<bool> {
    field_values.iter().find_map(|value| match value {
        FormFieldValue::CheckBox {
            field_id: value_field_id,
            checked,
        } if value_field_id == field_id => Some(*checked),
        _ => None,
    })
}

fn format_archive_since_label(since_utc: Option<chrono::DateTime<Utc>>) -> Option<String> {
    since_utc.map(|since| {
        let now = Utc::now();
        let days = (now - since).num_days().max(0);
        format!("{} ({} days ago)", since.format("%Y-%m-%d"), days)
    })
}

fn build_archive_form_descriptor(
    request_id: u64,
    article_count: usize,
    since_utc: Option<chrono::DateTime<Utc>>,
    default_basename: String,
    default_file_exists: bool,
    export_dir: PathBuf,
    pending_pre_triage_count: usize,
) -> FormDialogDescriptor {
    let mut rows = Vec::new();
    let articles_label = if since_utc.is_some() {
        format!("{article_count} URLs (since checkpoint)")
    } else {
        format!("{article_count} URLs (all)")
    };
    rows.push(FormRow::ReadOnlyText {
        label: "Articles".to_string(),
        value: articles_label,
    });
    if let Some(checkpoint) = format_archive_since_label(since_utc) {
        rows.push(FormRow::ReadOnlyText {
            label: "Checkpoint".to_string(),
            value: checkpoint,
        });
    }
    rows.push(FormRow::ReadOnlyText {
        label: "Up to".to_string(),
        value: Utc::now().format("%Y-%m-%d %H:%M UTC").to_string(),
    });
    if article_count == 0 {
        rows.push(FormRow::Note {
            text: "No articles match the current filter.".to_string(),
            severity: MessageSeverity::Warning,
        });
    } else if default_file_exists {
        rows.push(FormRow::Note {
            text: "file already exists - will be overwritten".to_string(),
            severity: MessageSeverity::Warning,
        });
    }
    if pending_pre_triage_count > 0 {
        rows.push(FormRow::Note {
            text: format!(
                "{} article{} await triage and are not included in this export.",
                pending_pre_triage_count,
                if pending_pre_triage_count == 1 { "" } else { "s" }
            ),
            severity: MessageSeverity::Warning,
        });
    }

    FormDialogDescriptor {
        title: "Archive Export".to_string(),
        context_tag: archive_dialog_context_tag(request_id),
        rows,
        fields: vec![
            FormField::TextInput {
                field_id: ARCHIVE_DIALOG_FILENAME_FIELD_ID.to_string(),
                label: "Output file".to_string(),
                value: default_basename,
                validation: FormTextValidation::PathSegment,
                live_warning: Some(FormFileExistsWarning {
                    base_dir: export_dir,
                    message: "file already exists - will be overwritten".to_string(),
                }),
            },
            FormField::CheckBox {
                field_id: ARCHIVE_DIALOG_SET_CHECKPOINT_FIELD_ID.to_string(),
                label: "Set checkpoint to now after export".to_string(),
                checked: true,
            },
        ],
        buttons: FormButtons {
            confirm_label: "Export".to_string(),
            cancel_label: "Cancel".to_string(),
            confirm_enabled: article_count > 0,
        },
    }
}

impl AppEventHandler {
    fn new(
        window_id: WindowId,
        shared: Arc<Mutex<SharedState>>,
        msg_rx: mpsc::Receiver<Msg>,
        msg_tx: mpsc::Sender<Msg>,
        effect_runner: EffectRunner,
        persistence_worker: PersistenceWorker,
        tree_render_state: ui::render::TreeRenderState,
    ) -> Self {
        Self {
            window_id,
            shared,
            commands: VecDeque::new(),
            msg_rx: Mutex::new(msg_rx),
            msg_tx,
            effect_runner,
            persistence_worker,
            tree_render_state,
        }
    }

    fn process_pending_messages(&mut self) {
        let mut inbox = Vec::new();
        if let Ok(rx) = self.msg_rx.lock() {
            while let Ok(msg) = rx.try_recv() {
                inbox.push(msg);
            }
        }
        if inbox.is_empty() {
            return;
        }
        let batch_size = inbox.len();
        let mut geometry_stats = GeometryBatchStats::default();
        for msg in &inbox {
            geometry_stats.observe(msg);
        }
        let geometry_only_batch = inbox.iter().all(is_geometry_only_message);

        let mut clear_input_needed = false;
        let mut persist_completed_needed = false;
        let mut persist_overrides_needed = false;
        let mut archive_failure_notice: Option<(String, String)> = None;
        let mut refresh_evaluation_dispatched = false;
        let mut persistence_enqueued = false;
        let mut queued_effects = Vec::new();
        let (maybe_render, render_mode, render_snapshot_ms, rendered_job_count) = {
            let mut guard = self.shared.lock().expect("lock shared state");
            let mut state = std::mem::take(&mut guard.state);
            let mut any_dirty = false;

            for msg in inbox {
                let msg_for_flags = msg.clone();
                let (next_state, effects) = update(state, msg);
                state = next_state;
                persist_completed_needed |= matches!(
                    msg_for_flags,
                    Msg::JobDone {
                        result: JobResultKind::Success,
                        ..
                    }
                );
                persist_overrides_needed |= matches!(
                    msg_for_flags,
                    Msg::PreTriageDecisionSet { .. } | Msg::PreTriageResetClicked
                );
                if let Msg::ArchiveExportFailed {
                    basename,
                    reason,
                    ..
                } = msg_for_flags
                {
                    archive_failure_notice = Some((
                        format!("Archive export failed: {basename}"),
                        reason,
                    ));
                }
                clear_input_needed |= effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::EnqueueUrl { .. }));
                queued_effects.extend(effects);
                any_dirty |= state.consume_dirty();
            }

            if let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request()
            {
                refresh_evaluation_dispatched = true;
                let ordered_urls = state.ordered_completed_job_urls_snapshot();
                let (next_state, effects) = update(
                    state,
                    Msg::EvaluatePreTriageRefresh {
                        ordered_urls,
                        triggered_by_job_done,
                    },
                );
                state = next_state;
                queued_effects.extend(effects);
                any_dirty |= state.consume_dirty();
            }

            let completed_snapshot = if persist_completed_needed {
                Some(state.completed_jobs_snapshot())
            } else {
                None
            };
            let overrides_snapshot = if persist_completed_needed || persist_overrides_needed {
                Some(state.pre_triage_manual_overrides().clone())
            } else {
                None
            };
            let render_mode = select_render_mode(
                any_dirty,
                geometry_only_batch,
                refresh_evaluation_dispatched,
                clear_input_needed,
                queued_effects.len(),
            );
            let snapshot_start = Instant::now();
            let maybe_render = match render_mode {
                Some(RenderMode::Full) => {
                    let view = state.view();
                    let job_count = view.job_count;
                    (
                        Some(PendingRender::Full(Box::new(view))),
                        Some(RenderMode::Full),
                        job_count,
                    )
                }
                Some(RenderMode::LayoutOnly) => (
                    Some(PendingRender::LayoutOnly(state.layout_view())),
                    Some(RenderMode::LayoutOnly),
                    0,
                ),
                None => (None, None, 0),
            };
            let render_snapshot_ms = snapshot_start.elapsed().as_millis();
            guard.state = state;

            if completed_snapshot.is_some() || overrides_snapshot.is_some() {
                persistence_enqueued = true;
                self.persistence_worker.enqueue(PersistenceSnapshot {
                    completed: completed_snapshot.unwrap_or_default(),
                    pre_triage_overrides: overrides_snapshot.unwrap_or_default(),
                });
            }
            (
                maybe_render.0,
                maybe_render.1,
                render_snapshot_ms,
                maybe_render.2,
            )
        };

        let mut effect_runner_effects = Vec::new();
        for effect in queued_effects {
            match effect {
                Effect::ShowArchiveDialog {
                    request_id,
                    article_count,
                    since_utc,
                    default_basename,
                    default_file_exists,
                    export_dir,
                    pending_pre_triage_count,
                } => {
                    let form = build_archive_form_descriptor(
                        request_id,
                        article_count,
                        since_utc,
                        default_basename,
                        default_file_exists,
                        export_dir,
                        pending_pre_triage_count,
                    );
                    self.commands.push_back(PlatformCommand::ShowFormDialog {
                        window_id: self.window_id,
                        form,
                    });
                }
                other => effect_runner_effects.push(other),
            }
        }

        let effect_count = effect_runner_effects.len();
        self.effect_runner.enqueue(effect_runner_effects);
        let did_work = effect_count > 0
            || maybe_render.is_some()
            || clear_input_needed
            || persistence_enqueued
            || refresh_evaluation_dispatched;
        if did_work {
            engine_info!("[msg-loop] batch_size={} queue_lag_ms={}", batch_size, 0);
        }

        if clear_input_needed {
            self.commands.push_back(PlatformCommand::SetInputText {
                window_id: self.window_id,
                control_id: ui::constants::INPUT_URLS,
                text: String::new(),
            });
        }

        if let Some((title, message)) = archive_failure_notice {
            self.commands.push_back(PlatformCommand::ShowMessageBox {
                window_id: self.window_id,
                title,
                message,
                severity: MessageSeverity::Error,
            });
        }

        let mut render_enqueue_ms = 0;
        match maybe_render {
            Some(PendingRender::Full(view)) => {
                let render_start = Instant::now();
                self.enqueue_render(&view);
                render_enqueue_ms = render_start.elapsed().as_millis();
            }
            Some(PendingRender::LayoutOnly(layout)) => {
                let render_start = Instant::now();
                self.enqueue_layout_render(&layout);
                render_enqueue_ms = render_start.elapsed().as_millis();
            }
            None => {}
        }

        if !geometry_stats.is_empty() {
            engine_info!(
                "[ui-geometry] splitter_moves={} window_resizes={} last_splitter_width={:?} last_window_width={:?} render_mode={:?} snapshot_ms={} enqueue_ms={} job_count={}",
                geometry_stats.splitter_moves,
                geometry_stats.window_resizes,
                geometry_stats.last_splitter_width,
                geometry_stats.last_window_width,
                render_mode,
                render_snapshot_ms,
                render_enqueue_ms,
                rendered_job_count,
            );
        }
    }

    fn enqueue_render(&mut self, view: &AppViewModel) {
        self.commands.extend(ui::render::render(
            self.window_id,
            view,
            &mut self.tree_render_state,
        ));
    }

    fn enqueue_layout_render(&mut self, layout: &LayoutViewModel) {
        self.commands.extend(ui::render::render_layout_only(
            self.window_id,
            layout,
            &mut self.tree_render_state,
        ));
    }

    fn toggle_prompt_lab_from_menu(&self) {
        let (input_panel_visible, prompt_lab_visible) = self
            .shared
            .lock()
            .map(|guard| {
                let view = guard.state.view();
                (view.input_panel_visible, view.left_pane.prompt_lab.visible)
            })
            .unwrap_or((false, false));
        if !prompt_lab_visible && !input_panel_visible {
            let _ = self.msg_tx.send(Msg::ToggleInputPanel);
        }
        let msg = if prompt_lab_visible {
            Msg::PromptLabCloseRequested
        } else {
            Msg::PromptLabOpenRequested
        };
        let _ = self.msg_tx.send(msg.clone());
        if matches!(msg, Msg::PromptLabOpenRequested) {
            let _ = self.msg_tx.send(Msg::PromptLabContextEditorOpened);
        }
    }
}

impl Drop for AppEventHandler {
    fn drop(&mut self) {
        self.persistence_worker.shutdown();
    }
}

impl PlatformEventHandler for AppEventHandler {
    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::MainWindowUISetupComplete { .. } => {
                let _ = self.msg_tx.send(Msg::Tick);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BUTTON_STOP =>
            {
                let _ = self.msg_tx.send(Msg::StopFinishClicked);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BUTTON_BRIEFING =>
            {
                let _ = self.msg_tx.send(Msg::GenerateBriefingClicked);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BUTTON_TRIAGE =>
            {
                let _ = self.msg_tx.send(Msg::TriageClicked);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BUTTON_POLL_SOURCES =>
            {
                let _ = self.msg_tx.send(Msg::PollSourcesClicked);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BUTTON_OPEN_BROWSER =>
            {
                let _ = self.msg_tx.send(Msg::OpenInBrowserClicked);
            }
            AppEvent::TabBarSelectionChanged {
                control_id,
                selected_index,
                ..
            } if control_id == ui::constants::TAB_BAR_RIGHT => {
                if let Some(tab) = AppTab::from_index(selected_index) {
                    let _ = self.msg_tx.send(Msg::TabSelected { tab });
                }
            }
            AppEvent::TabBarSelectionChanged {
                control_id,
                selected_index,
                ..
            } if control_id == ui::constants::TAB_BAR_LEFT => {
                if let Some(tab) = LeftTab::from_index(selected_index) {
                    let _ = self.msg_tx.send(Msg::LeftTabSelected { tab });
                }
            }
            AppEvent::TabBarSelectionChanged {
                control_id,
                selected_index,
                ..
            } if control_id == ui::constants::TAB_BAR_TRENDS => {
                if let Some(category) = TrendCategory::from_index(selected_index) {
                    let _ = self.msg_tx.send(Msg::TrendCategorySelected { category });
                }
            }
            AppEvent::RadioButtonSelected { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_MODE_BASIC =>
            {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabAdvancedModeSet { enabled: false });
            }
            AppEvent::RadioButtonSelected { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_MODE_ADVANCED =>
            {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabAdvancedModeSet { enabled: true });
            }
            AppEvent::CheckBoxToggled { control_id, .. }
                if control_id == ui::constants::CHK_PROMPT_LAB_SECTION_COMPARE =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabCompareSectionToggled);
            }
            AppEvent::CheckBoxToggled { control_id, .. }
                if control_id == ui::constants::CHK_PROMPT_LAB_SECTION_CONTEXT =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabContextSectionToggled);
            }
            AppEvent::CheckBoxToggled { control_id, .. }
                if control_id == ui::constants::CHK_PROMPT_LAB_SECTION_TEMPLATE =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabTemplateSectionToggled);
            }
            AppEvent::CheckBoxToggled { control_id, .. }
                if control_id == ui::constants::CHK_PROMPT_LAB_SECTION_RUN_DETAILS =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabRunDetailsSectionToggled);
            }
            AppEvent::ToggleSwitchToggled {
                control_id,
                checked,
                ..
            } if control_id == ui::constants::TS_JOBS_SCOPE => {
                let scope = if checked {
                    JobListScope::SinceCheckpoint
                } else {
                    JobListScope::All
                };
                let _ = self.msg_tx.send(Msg::JobListScopeSet { scope });
            }
            AppEvent::RadioButtonSelected { control_id, .. }
                if control_id == ui::constants::BTN_STAGE_TRIAGE =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabStageSelected {
                    stage: PromptLabStage::Triage,
                });
            }
            AppEvent::RadioButtonSelected { control_id, .. }
                if control_id == ui::constants::BTN_STAGE_SUMMARY =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabStageSelected {
                    stage: PromptLabStage::Summary,
                });
            }
            AppEvent::RadioButtonSelected { control_id, .. }
                if control_id == ui::constants::BTN_STAGE_BRIEFING =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabStageSelected {
                    stage: PromptLabStage::Briefing,
                });
            }
            AppEvent::ComboBoxSelectionChanged {
                control_id,
                selected_index: Some(index),
                window_id: _,
            } if control_id == ui::constants::COMBO_PROMPT_LAB_MODEL_SELECTOR => {
                let guard = self.shared.lock().unwrap();
                let view = guard.state.view();
                let model = ui::render::combo_index_to_model(
                    index,
                    &view.left_pane.prompt_lab.model_catalog,
                );
                let _ = self.msg_tx.send(Msg::PromptLabModelOverrideSet { model });
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_RESOLVE =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabResolveRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_RUN =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabRunRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_COMPARE_ADD_CURRENT =>
            {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabCompareCurrentSettingsCaptured);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_COMPARE_ADD_BASELINE =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabCompareBaselineCaptured);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_COMPARE_RESET_DRAFT =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabCompareDraftReset);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_COMPARE_START =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabCompareBatchStartRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_COMPARE_CANCEL =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabCompareBatchCancelRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_COMPARE_AUTO_SELECT =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabCompareAutoSelectRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_COMPARE_WINNER_CLEAR =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabCompareWinnerCleared);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_CONTEXT_APPLY =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabContextApplyRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_CONTEXT_APPLY_RERUN =>
            {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabContextApplyAndRerunRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_CONTEXT_REVERT =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabContextRevertRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_CONTEXT_SAVE =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabContextSaveRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_CONTEXT_RELOAD =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabContextReloadRequested);
            }
            AppEvent::CheckBoxToggled { control_id, .. }
                if control_id == ui::constants::CHK_PROMPT_LAB_TEMPLATE_OPEN =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabTemplateEditorToggled);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_TEMPLATE_APPLY =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabTemplateApplyRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN =>
            {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabTemplateApplyAndRerunRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_TEMPLATE_REVERT =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabTemplateRevertRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_TEMPLATE_SAVE =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabTemplateSaveRequested);
            }
            AppEvent::MenuActionClicked { action_id } if action_id == MENU_ACTION_ADD_URL => {
                let _ = self.msg_tx.send(Msg::ToggleInputPanel);
            }
            AppEvent::MenuActionClicked { action_id } if action_id == MENU_ACTION_ARCHIVE => {
                let _ = self.msg_tx.send(Msg::ArchiveClicked);
            }
            AppEvent::MenuActionClicked { action_id } if action_id == MENU_ACTION_PROMPT_LAB => {
                self.toggle_prompt_lab_from_menu();
            }
            AppEvent::FormDialogCompleted {
                window_id,
                context_tag,
                confirmed,
                field_values,
            } if window_id == self.window_id => {
                if !confirmed {
                    return;
                }
                let Some(request_id) = parse_archive_dialog_request_id(&context_tag) else {
                    engine_warn!(
                        "[archive-dialog] ignoring form dialog result with unrecognized context_tag={}",
                        context_tag
                    );
                    return;
                };
                let basename = archive_field_text(&field_values, ARCHIVE_DIALOG_FILENAME_FIELD_ID)
                    .unwrap_or_default();
                let set_checkpoint = archive_field_checked(
                    &field_values,
                    ARCHIVE_DIALOG_SET_CHECKPOINT_FIELD_ID,
                )
                .unwrap_or(true);
                engine_info!(
                    "[archive-dialog] submitted request_id={} basename={} set_checkpoint={}",
                    request_id,
                    basename,
                    set_checkpoint
                );
                let _ = self.msg_tx.send(Msg::ArchiveDialogSubmitted {
                    request_id,
                    basename,
                    set_checkpoint,
                    submitted_at: Utc::now(),
                });
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_URLS => {
                engine_info!(
                    "InputTextChanged: {} chars, preview=\"{}\"",
                    text.len(),
                    text.chars().take(120).collect::<String>()
                );
                let _ = self.msg_tx.send(Msg::InputChanged(text));
                let _ = self.msg_tx.send(Msg::UrlsSubmitted);
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_PROMPT_LAB_URL => {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabUrlInputChanged { url: text });
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_PROMPT_LAB_CONTEXT => {
                let _ = self.msg_tx.send(Msg::PromptLabContextDraftChanged { text });
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_PROMPT_LAB_TEMPLATE_SYSTEM => {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabTemplateSystemDraftChanged { text });
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_PROMPT_LAB_TEMPLATE_USER => {
                let _ = self
                    .msg_tx
                    .send(Msg::PromptLabTemplateUserDraftChanged { text });
            }
            AppEvent::TreeViewItemSelectionChanged { window_id, item_id }
                if window_id == self.window_id =>
            {
                if let Some(job_id) = job_id_for_item(item_id) {
                    let _ = self.msg_tx.send(Msg::JobSelected { job_id });
                }
            }
            AppEvent::TreeViewItemToggledByUser {
                window_id,
                item_id,
                new_state,
            } if window_id == self.window_id => {
                if let TreeItemKind::Link { job_id, link_index } = decode_tree_item_id(item_id) {
                    let checked = matches!(new_state, CheckState::Checked);
                    let _ = self.msg_tx.send(Msg::LinkToggleRequested {
                        job_id,
                        link_index,
                        checked,
                    });
                } else if let TreeItemKind::Job { job_id } = decode_tree_item_id(item_id) {
                    let guard = self.shared.lock().unwrap();
                    if guard.state.is_pre_triage_reviewing() {
                        if let Some(key) = guard.state.pre_triage_key_for_job(job_id) {
                            let decision = if matches!(new_state, CheckState::Checked) {
                                ManualDecision::Include
                            } else {
                                ManualDecision::Exclude
                            };
                            let _ = self
                                .msg_tx
                                .send(Msg::PreTriageDecisionSet { key, decision });
                        }
                    }
                }
            }
            AppEvent::WindowCloseRequestedByUser { .. } => {
                self.commands.push_back(PlatformCommand::QuitApplication);
            }
            AppEvent::SplitterDragging {
                desired_left_width_px,
                ..
            } => {
                // Continuous dragging - update layout in real-time
                let _ = self.msg_tx.send(Msg::SplitterMoved {
                    desired_left_width_px,
                });
            }
            AppEvent::SplitterDragEnded {
                desired_left_width_px,
                ..
            } => {
                // Log boundary event: drag completed
                engine_info!(
                    "Splitter drag ended: left_panel_width={}",
                    desired_left_width_px
                );
                let _ = self.msg_tx.send(Msg::SplitterMoved {
                    desired_left_width_px,
                });
            }
            AppEvent::WindowResized {
                window_id, width, ..
            } if window_id == self.window_id => {
                let _ = self.msg_tx.send(Msg::WindowResized {
                    window_width: width,
                });
            }
            _ => {}
        }
    }

    fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
        self.process_pending_messages();
        self.commands.pop_front()
    }
}

struct AppUiStateProvider {
    shared: Arc<Mutex<SharedState>>,
}

impl AppUiStateProvider {
    fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self { shared }
    }
}

impl UiStateProvider for AppUiStateProvider {
    fn is_tree_item_new(&self, _window_id: WindowId, _item_id: commanductui::TreeItemId) -> bool {
        false
    }

    fn tree_item_marker(
        &self,
        _window_id: WindowId,
        item_id: commanductui::TreeItemId,
    ) -> TreeItemMarkerKind {
        if let TreeItemKind::Job { job_id } = decode_tree_item_id(item_id) {
            let guard = self.shared.lock().unwrap();
            return match guard.state.job_filter_status(job_id) {
                Some(JobFilterStatus::HardExcluded { .. }) => TreeItemMarkerKind::Red,
                Some(JobFilterStatus::ReviewNeeded { .. }) => TreeItemMarkerKind::Yellow,
                Some(JobFilterStatus::ManuallyExcluded) => TreeItemMarkerKind::Gray,
                Some(JobFilterStatus::ManuallyIncluded) => TreeItemMarkerKind::Blue,
                _ => TreeItemMarkerKind::None,
            };
        }
        if let TreeItemKind::Link { job_id, link_index } = decode_tree_item_id(item_id) {
            let guard = self.shared.lock().unwrap();
            if let Some((download_state, age_suspect)) = guard.state.link_state(job_id, link_index)
            {
                return match download_state {
                    LinkDownloadState::Downloaded { .. } => TreeItemMarkerKind::Green,
                    LinkDownloadState::Downloading => TreeItemMarkerKind::Purple,
                    LinkDownloadState::Failed { .. } => TreeItemMarkerKind::Red,
                    LinkDownloadState::NotDownloaded if age_suspect => TreeItemMarkerKind::Yellow,
                    _ => TreeItemMarkerKind::None,
                };
            }
        }

        TreeItemMarkerKind::None
    }
}

#[cfg(test)]
mod tests {
    use super::ui::tree_item_ids::link_tree_item_id;
    use super::*;
    use commanductui::types::{TreeItemMarkerKind, WindowId};
    use commanductui::AppEvent;
    use harvester_core::{AppState, JobResultKind};
    use harvester_engine::{ExtractedLink, LinkKind};
    use std::path::PathBuf;
    use std::sync::{mpsc, Arc, Mutex};

    fn shared_state_with_single_link() -> Arc<Mutex<SharedState>> {
        let state = AppState::new();
        let (state, _) = update(state, Msg::InputChanged("https://example.com".to_string()));
        let (state, _) = update(state, Msg::UrlsSubmitted);
        let (state, _) = update(
            state,
            Msg::JobDone {
                job_id: 1,
                result: JobResultKind::Success,
                content_preview: None,
                extracted_links: vec![ExtractedLink {
                    url: "http://example.com/".to_string(),
                    text: Some("Example".to_string()),
                    kind: LinkKind::Hyperlink,
                }],
                fetched_utc: None,
            },
        );
        let shared = SharedState { state };
        Arc::new(Mutex::new(shared))
    }

    fn apply_msg(shared: &Arc<Mutex<SharedState>>, msg: Msg) {
        let mut guard = shared.lock().unwrap();
        let (state, _) = update(std::mem::take(&mut guard.state), msg);
        guard.state = state;
    }

    fn test_handler_with_outbound() -> (AppEventHandler, mpsc::Receiver<Msg>) {
        let shared = Arc::new(Mutex::new(SharedState::default()));
        let (in_tx, in_rx) = mpsc::channel();
        let _ = in_tx;
        let (out_tx, out_rx) = mpsc::channel();
        let output_dir = std::env::temp_dir();
        let paths = RuntimePaths::new(
            output_dir.clone(),
            output_dir.join("sources.ron"),
            output_dir.join("contexts"),
            output_dir.join("prompts"),
        );
        let platform_handler = Box::new(Win32PlatformHandler);
        let effect_runner = EffectRunner::new(paths.clone(), out_tx.clone(), platform_handler);
        let handler = AppEventHandler::new(
            WindowId::new(1),
            shared,
            in_rx,
            out_tx,
            effect_runner,
            PersistenceWorker::new(paths.state_path.clone()),
            ui::render::TreeRenderState::new(),
        );
        (handler, out_rx)
    }

    #[test]
    fn tree_item_marker_updates_with_link_state() {
        let shared = shared_state_with_single_link();
        let provider = AppUiStateProvider::new(shared.clone());
        let item_id = link_tree_item_id(1, 0);

        assert_eq!(
            provider.tree_item_marker(WindowId::new(1), item_id),
            TreeItemMarkerKind::None
        );

        apply_msg(
            &shared,
            Msg::LinkToggleRequested {
                job_id: 1,
                link_index: 0,
                checked: true,
            },
        );
        assert_eq!(
            provider.tree_item_marker(WindowId::new(1), item_id),
            TreeItemMarkerKind::Purple
        );

        apply_msg(
            &shared,
            Msg::LinkDownloadCompleted {
                job_id: 1,
                link_index: 0,
                path: PathBuf::from("linked/example.md"),
            },
        );
        assert_eq!(
            provider.tree_item_marker(WindowId::new(1), item_id),
            TreeItemMarkerKind::Green
        );

        apply_msg(
            &shared,
            Msg::LinkDownloadFailed {
                job_id: 1,
                link_index: 0,
                error: "boom".to_string(),
            },
        );
        assert_eq!(
            provider.tree_item_marker(WindowId::new(1), item_id),
            TreeItemMarkerKind::Red
        );

        apply_msg(
            &shared,
            Msg::LinkDeleted {
                job_id: 1,
                link_index: 0,
            },
        );
        assert_eq!(
            provider.tree_item_marker(WindowId::new(1), item_id),
            TreeItemMarkerKind::None
        );
    }

    #[test]
    fn parse_llm_max_concurrency_uses_default_when_missing_or_invalid() {
        assert_eq!(
            parse_llm_max_concurrency_requests(None),
            DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(
            parse_llm_max_concurrency_requests(Some("not-a-number")),
            DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(
            parse_llm_max_concurrency_requests(Some("")),
            DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
        );
    }

    #[test]
    fn parse_llm_max_concurrency_clamps_to_valid_range() {
        assert_eq!(parse_llm_max_concurrency_requests(Some("1")), 1);
        assert_eq!(parse_llm_max_concurrency_requests(Some("3")), 3);
        assert_eq!(
            parse_llm_max_concurrency_requests(Some("999")),
            MAX_LLM_CONCURRENT_REQUESTS
        );
        assert_eq!(parse_llm_max_concurrency_requests(Some(" 2 ")), 2);
    }

    #[test]
    fn geometry_only_batches_use_layout_only_render_mode() {
        assert_eq!(
            select_render_mode(true, true, false, false, 0),
            Some(RenderMode::LayoutOnly)
        );
        assert_eq!(
            select_render_mode(true, true, false, false, 1),
            Some(RenderMode::Full)
        );
        assert_eq!(
            select_render_mode(true, false, false, false, 0),
            Some(RenderMode::Full)
        );
    }

    #[test]
    fn prompt_lab_stage_button_emits_stage_selected_msg() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::RadioButtonSelected {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_STAGE_SUMMARY,
        });
        let msg = rx.recv_timeout(Duration::from_millis(250)).expect("msg");
        assert_eq!(
            msg,
            Msg::PromptLabStageSelected {
                stage: PromptLabStage::Summary
            }
        );
    }

    #[test]
    fn prompt_lab_url_input_emits_url_changed_msg() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::InputTextChanged {
            window_id: WindowId::new(1),
            control_id: ui::constants::INPUT_PROMPT_LAB_URL,
            text: "https://example.com".to_string(),
        });
        let msg = rx.recv_timeout(Duration::from_millis(250)).expect("msg");
        assert_eq!(
            msg,
            Msg::PromptLabUrlInputChanged {
                url: "https://example.com".to_string()
            }
        );
    }

    #[test]
    fn prompt_lab_action_buttons_emit_expected_msgs() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_PROMPT_LAB_RESOLVE,
        });
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_PROMPT_LAB_RUN,
        });

        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("resolve"),
            Msg::PromptLabResolveRequested
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250)).expect("run"),
            Msg::PromptLabRunRequested
        );
    }

    #[test]
    fn prompt_lab_template_buttons_emit_expected_msgs() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::CheckBoxToggled {
            window_id: WindowId::new(1),
            control_id: ui::constants::CHK_PROMPT_LAB_TEMPLATE_OPEN,
            checked: true,
        });
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_PROMPT_LAB_TEMPLATE_APPLY,
        });
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_PROMPT_LAB_TEMPLATE_APPLY_RERUN,
        });
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_PROMPT_LAB_TEMPLATE_REVERT,
        });
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_PROMPT_LAB_TEMPLATE_SAVE,
        });

        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("template open"),
            Msg::PromptLabTemplateEditorToggled
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("template apply"),
            Msg::PromptLabTemplateApplyRequested
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("template apply rerun"),
            Msg::PromptLabTemplateApplyAndRerunRequested
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("template revert"),
            Msg::PromptLabTemplateRevertRequested
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("template save"),
            Msg::PromptLabTemplateSaveRequested
        );
    }

    #[test]
    fn prompt_lab_compare_buttons_emit_expected_msgs() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_COMPARE_ADD_CURRENT,
        });
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_COMPARE_ADD_BASELINE,
        });
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_COMPARE_RESET_DRAFT,
        });
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_COMPARE_START,
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("add current"),
            Msg::PromptLabCompareCurrentSettingsCaptured
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("add baseline"),
            Msg::PromptLabCompareBaselineCaptured
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250)).expect("reset"),
            Msg::PromptLabCompareDraftReset
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250)).expect("start"),
            Msg::PromptLabCompareBatchStartRequested
        );
    }

    #[test]
    fn prompt_lab_mode_and_section_buttons_emit_expected_msgs() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::RadioButtonSelected {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_PROMPT_LAB_MODE_ADVANCED,
        });
        handler.handle_event(AppEvent::CheckBoxToggled {
            window_id: WindowId::new(1),
            control_id: ui::constants::CHK_PROMPT_LAB_SECTION_COMPARE,
            checked: true,
        });
        handler.handle_event(AppEvent::CheckBoxToggled {
            window_id: WindowId::new(1),
            control_id: ui::constants::CHK_PROMPT_LAB_SECTION_CONTEXT,
            checked: true,
        });
        handler.handle_event(AppEvent::CheckBoxToggled {
            window_id: WindowId::new(1),
            control_id: ui::constants::CHK_PROMPT_LAB_SECTION_TEMPLATE,
            checked: true,
        });
        handler.handle_event(AppEvent::CheckBoxToggled {
            window_id: WindowId::new(1),
            control_id: ui::constants::CHK_PROMPT_LAB_SECTION_RUN_DETAILS,
            checked: true,
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("advanced"),
            Msg::PromptLabAdvancedModeSet { enabled: true }
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("toggle compare"),
            Msg::PromptLabCompareSectionToggled
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("toggle context"),
            Msg::PromptLabContextSectionToggled
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("toggle template"),
            Msg::PromptLabTemplateSectionToggled
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("toggle run details"),
            Msg::PromptLabRunDetailsSectionToggled
        );
    }

    #[test]
    fn left_tab_bar_selection_emits_new_left_tab_variants() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::TabBarSelectionChanged {
            window_id: WindowId::new(1),
            control_id: ui::constants::TAB_BAR_LEFT,
            selected_index: LeftTab::TriageReview.to_index(),
        });
        handler.handle_event(AppEvent::TabBarSelectionChanged {
            window_id: WindowId::new(1),
            control_id: ui::constants::TAB_BAR_LEFT,
            selected_index: LeftTab::TriageResults.to_index(),
        });

        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("triage review tab"),
            Msg::LeftTabSelected {
                tab: LeftTab::TriageReview
            }
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("triage results tab"),
            Msg::LeftTabSelected {
                tab: LeftTab::TriageResults
            }
        );
    }

    #[test]
    fn jobs_scope_toggle_emits_typed_scope_message() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::ToggleSwitchToggled {
            window_id: WindowId::new(1),
            control_id: ui::constants::TS_JOBS_SCOPE,
            checked: true,
        });
        handler.handle_event(AppEvent::ToggleSwitchToggled {
            window_id: WindowId::new(1),
            control_id: ui::constants::TS_JOBS_SCOPE,
            checked: false,
        });

        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("scope on"),
            Msg::JobListScopeSet {
                scope: JobListScope::SinceCheckpoint
            }
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("scope off"),
            Msg::JobListScopeSet {
                scope: JobListScope::All
            }
        );
    }

    #[test]
    fn prompt_lab_menu_action_emits_open_then_close() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::MenuActionClicked {
            action_id: MENU_ACTION_PROMPT_LAB,
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("input panel open"),
            Msg::ToggleInputPanel
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("lab open"),
            Msg::PromptLabOpenRequested
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("context open"),
            Msg::PromptLabContextEditorOpened
        );

        {
            let mut guard = handler.shared.lock().expect("shared lock");
            let (state, _) = update(
                std::mem::take(&mut guard.state),
                Msg::PromptLabOpenRequested,
            );
            guard.state = state;
        }
        handler.handle_event(AppEvent::MenuActionClicked {
            action_id: MENU_ACTION_PROMPT_LAB,
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250)).expect("close"),
            Msg::PromptLabCloseRequested
        );
    }
}
