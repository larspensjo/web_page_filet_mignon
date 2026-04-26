use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;

use commanductui::types::{
    FormButtons, FormDialogDescriptor, FormField, FormFieldValue, FormFileExistsWarning, FormRow,
    FormTextValidation, MessageSeverity, TreeItemMarkerKind,
};
use commanductui::{
    AppEvent, PlatformCommand, PlatformEventHandler, PlatformInterface, UiStateProvider,
    WindowConfig, WindowId,
};
use harvester_core::{
    update, AiAvailability, AiUnavailableReason, AppState, AppTab, AppViewModel,
    ArchiveTokenEstimates, Effect, JobFilterStatus, JobListScope, JobResultKind, LayoutViewModel,
    LeftTab, LinkDownloadState, ManualDecision, Msg, PromptLabStage, TrendCategory,
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
    load_window_size, EffectRunner, PersistenceSnapshot, PersistenceWorker, RuntimePaths,
};

use super::effects;
use super::logging::{self, LogDestination};
use super::ui;
use super::ui::tree_item_ids::{decode_tree_item_id, TreeItemKind};
use super::Win32PlatformHandler;

const ARCHIVE_DIALOG_CONTEXT_PREFIX: &str = "archive:";
const ARCHIVE_DIALOG_FILENAME_FIELD_ID: &str = "archive.basename";
const ARCHIVE_DIALOG_USE_SUMMARIES_FIELD_ID: &str = "archive.use_summaries";
const ARCHIVE_DIALOG_SET_CHECKPOINT_FIELD_ID: &str = "archive.set_checkpoint";
const DEFAULT_LLM_MAX_CONCURRENT_REQUESTS: usize = 3;
const LLM_MAX_CONCURRENT_REQUESTS_ENV: &str = "LLM_MAX_CONCURRENT_REQUESTS";
const MAX_LLM_CONCURRENT_REQUESTS: usize = 10;

fn apply_startup_msg(state: AppState, msg: Msg, startup_effects: &mut Vec<Effect>) -> AppState {
    let (next_state, effects) = update(state, msg);
    startup_effects.extend(effects);
    next_state
}

fn prepare_startup_state(
    mut state: AppState,
    paths: &RuntimePaths,
    initial_width: i32,
    llm_max_concurrent_requests: usize,
    startup_ai_availability: Option<AiAvailability>,
) -> (AppState, Vec<Effect>) {
    let mut startup_effects = Vec::new();

    // Synchronous startup preparation: seed all cheap, local facts before the
    // first view snapshot so the first visible frame is already correct.
    let (mut next_state, _) = update(
        state,
        Msg::WindowResized {
            window_width: initial_width,
        },
    );
    next_state.set_triage_max_in_flight(llm_max_concurrent_requests);
    next_state.set_summary_max_in_flight(llm_max_concurrent_requests);
    state = next_state;

    if let Some(availability) = startup_ai_availability {
        state = apply_startup_msg(
            state,
            Msg::AiAvailabilityDetected { availability },
            &mut startup_effects,
        );
    }

    // Asynchronous startup hydration begins here. Reducer-owned startup
    // scheduling stays adjacent to the state transition that emits those effects.
    state = apply_startup_msg(state, Msg::StartupHydrationRequested, &mut startup_effects);

    let completed = load_completed_jobs(&paths.state_path);
    if !completed.is_empty() {
        state = apply_startup_msg(
            state,
            Msg::RestoreCompletedJobs(completed),
            &mut startup_effects,
        );
    }

    let summary_cache = load_summary_cache(&paths.summary_cache_path);
    if !summary_cache.is_empty() {
        state = apply_startup_msg(
            state,
            Msg::SummaryCacheHydrated {
                cache: summary_cache,
            },
            &mut startup_effects,
        );
    }

    let triage_cache = load_triage_cache(&paths.triage_cache_path);
    if !triage_cache.is_empty() {
        state = apply_startup_msg(
            state,
            Msg::TriageCacheHydrated {
                cache: triage_cache,
            },
            &mut startup_effects,
        );
    }

    let overrides = load_pre_triage_overrides(&paths.state_path);
    if !overrides.is_empty() {
        state = apply_startup_msg(
            state,
            Msg::PreTriageOverridesHydrated { overrides },
            &mut startup_effects,
        );
    }

    (state, startup_effects)
}

fn assemble_startup_commands(
    window_id: WindowId,
    initial_view: &AppViewModel,
    tree_render_state: &mut ui::render::TreeRenderState,
) -> Vec<PlatformCommand> {
    let mut initial_commands = ui::layout::initial_commands(window_id);
    initial_commands.extend(ui::render::render(
        window_id,
        initial_view,
        tree_render_state,
    ));

    // Reveal ownership stays at the app layer so first render and first reveal
    // remain one explicit, testable contract.
    initial_commands.push(PlatformCommand::SignalMainWindowUISetupComplete { window_id });
    initial_commands.push(PlatformCommand::ShowWindow { window_id });
    initial_commands
}

pub fn run_app() -> commanductui::PlatformResult<()> {
    logging::initialize(LogDestination::Both);
    engine_info!("Logger initialized. Starting harvester_app...");

    const DEFAULT_WINDOW_WIDTH: i32 = 960;
    const DEFAULT_WINDOW_HEIGHT: i32 = 720;

    let output_dir = effects::default_output_dir();
    let paths = RuntimePaths::new(
        output_dir.clone(),
        effects::default_source_config_path(),
        effects::contexts_directory(),
        effects::prompts_directory(),
    );

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
    effect_runner.enqueue(vec![Effect::LoadPromptTemplateFiles]);
    {
        let mut guard = shared_state.lock().expect("lock shared state");
        let state = std::mem::take(&mut guard.state);
        let (prepared_state, startup_effects) = prepare_startup_state(
            state,
            &paths,
            initial_width,
            llm_max_concurrent_requests,
            startup_ai_availability,
        );
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

fn triage_marker_for_priority(priority: u8) -> TreeItemMarkerKind {
    match priority {
        6..=u8::MAX => TreeItemMarkerKind::Red,
        5 => TreeItemMarkerKind::Yellow,
        4 => TreeItemMarkerKind::Purple,
        3 => TreeItemMarkerKind::Gray,
        _ => TreeItemMarkerKind::None,
    }
}

fn pre_triage_toggle_message(state: &AppState) -> Option<Msg> {
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
    matches!(
        msg,
        Msg::SplitterMoved { .. } | Msg::WindowResized { .. } | Msg::WindowResizeCompleted { .. }
    )
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

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.0}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

#[allow(clippy::too_many_arguments)]
fn build_archive_form_descriptor(
    request_id: u64,
    article_count: usize,
    since_utc: Option<chrono::DateTime<Utc>>,
    default_basename: String,
    _default_file_exists: bool,
    export_dir: PathBuf,
    pending_pre_triage_count: usize,
    token_estimates: ArchiveTokenEstimates,
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
    rows.push(FormRow::ReadOnlyText {
        label: "Full archive".to_string(),
        value: format!(
            "~{} tokens ({} articles)",
            format_tokens(token_estimates.full_tokens),
            article_count,
        ),
    });
    rows.push(FormRow::ReadOnlyText {
        label: "Summary archive".to_string(),
        value: format!(
            "~{} tokens ({}/{} with summaries)",
            format_tokens(token_estimates.summary_tokens),
            token_estimates.summary_coverage,
            article_count,
        ),
    });
    if article_count == 0 {
        rows.push(FormRow::Note {
            text: "No articles match the current filter.".to_string(),
            severity: MessageSeverity::Warning,
        });
    }
    if pending_pre_triage_count > 0 {
        rows.push(FormRow::Note {
            text: format!(
                "{} article{} await triage and are not included in this export.",
                pending_pre_triage_count,
                if pending_pre_triage_count == 1 {
                    ""
                } else {
                    "s"
                }
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
                field_id: ARCHIVE_DIALOG_USE_SUMMARIES_FIELD_ID.to_string(),
                label: "Use summaries (recommended)".to_string(),
                checked: true,
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
                    basename, reason, ..
                } = msg_for_flags
                {
                    archive_failure_notice =
                        Some((format!("Archive export failed: {basename}"), reason));
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

            let persistence_snapshot = if persist_completed_needed || persist_overrides_needed {
                Some(PersistenceSnapshot::capture(&state))
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

            if let Some(snapshot) = persistence_snapshot {
                persistence_enqueued = true;
                self.persistence_worker.enqueue(snapshot);
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
                    token_estimates,
                } => {
                    let form = build_archive_form_descriptor(
                        request_id,
                        article_count,
                        since_utc,
                        default_basename,
                        default_file_exists,
                        export_dir,
                        pending_pre_triage_count,
                        token_estimates,
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
}

impl Drop for AppEventHandler {
    fn drop(&mut self) {
        self.persistence_worker.shutdown();
    }
}

impl PlatformEventHandler for AppEventHandler {
    fn handle_event(&mut self, event: AppEvent) {
        if let AppEvent::ButtonClicked { control_id, .. } = &event {
            if let Some(msg) = ui::groups::bottom_buttons::msg_for_control(*control_id) {
                let _ = self.msg_tx.send(msg);
                return;
            }
            if let Some(msg) = ui::groups::prompt_lab_actions::msg_for_button(*control_id) {
                let _ = self.msg_tx.send(msg);
                return;
            }
        }
        if let AppEvent::CheckBoxToggled { control_id, .. } = &event {
            if let Some(msg) = ui::groups::prompt_lab_actions::msg_for_checkbox(*control_id) {
                let _ = self.msg_tx.send(msg);
                return;
            }
            if let Some(msg) = ui::groups::prompt_lab_sections::msg_for_checkbox(*control_id) {
                let _ = self.msg_tx.send(msg);
                return;
            }
        }

        match event {
            AppEvent::MainWindowUISetupComplete { .. } => {
                let _ = self.msg_tx.send(Msg::Tick);
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
            AppEvent::MenuActionClicked { action_id }
                if action_id == ui::constants::MENU_ACTION_ADD_URL =>
            {
                let _ = self.msg_tx.send(Msg::ToggleInputPanel);
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
                let set_checkpoint =
                    archive_field_checked(&field_values, ARCHIVE_DIALOG_SET_CHECKPOINT_FIELD_ID)
                        .unwrap_or(true);
                let use_summaries =
                    archive_field_checked(&field_values, ARCHIVE_DIALOG_USE_SUMMARIES_FIELD_ID)
                        .unwrap_or(true);
                engine_info!(
                    "[archive-dialog] submitted request_id={} basename={} set_checkpoint={} use_summaries={}",
                    request_id,
                    basename,
                    set_checkpoint,
                    use_summaries
                );
                let _ = self.msg_tx.send(Msg::ArchiveDialogSubmitted {
                    request_id,
                    basename,
                    set_checkpoint,
                    submitted_at: Utc::now(),
                    use_summaries,
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
            AppEvent::ListBoxItemSelectionChanged {
                window_id, item_id, ..
            } if window_id == self.window_id => {
                let _ = self.msg_tx.send(Msg::JobSelected {
                    job_id: item_id.raw(),
                });
            }
            AppEvent::ListBoxItemKeyDown {
                window_id,
                control_id,
                key_code,
            } if window_id == self.window_id && control_id == ui::constants::TREE_JOBS => {
                if key_code == b'X' as u16 {
                    let maybe_msg = {
                        let guard = self.shared.lock().unwrap();
                        if guard.state.left_tab() == LeftTab::TriageReview
                            && guard.state.is_pre_triage_reviewing()
                        {
                            pre_triage_toggle_message(&guard.state)
                        } else {
                            None
                        }
                    };
                    if let Some(msg) = maybe_msg {
                        let _ = self.msg_tx.send(msg);
                    }
                }
            }
            AppEvent::ListBoxScrolled { .. } => {}
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
            AppEvent::WindowResizeCompleted {
                window_id,
                outer_width,
                outer_height,
            } if window_id == self.window_id => {
                let _ = self.msg_tx.send(Msg::WindowResizeCompleted {
                    outer_width,
                    outer_height,
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
        match decode_tree_item_id(item_id) {
            TreeItemKind::Job { job_id } => {
                let guard = self.shared.lock().unwrap();
                if guard.state.left_tab() != LeftTab::TriageResults {
                    return TreeItemMarkerKind::None;
                }
                if let Some(result) = guard.state.triage_result_for_job(job_id) {
                    return triage_marker_for_priority(result.priority);
                }
                TreeItemMarkerKind::None
            }
            TreeItemKind::Link { job_id, link_index } => {
                let guard = self.shared.lock().unwrap();
                if let Some((download_state, age_suspect)) =
                    guard.state.link_state(job_id, link_index)
                {
                    return match download_state {
                        LinkDownloadState::Downloaded { .. } => TreeItemMarkerKind::Green,
                        LinkDownloadState::Downloading => TreeItemMarkerKind::Purple,
                        LinkDownloadState::Failed { .. } => TreeItemMarkerKind::Red,
                        LinkDownloadState::NotDownloaded if age_suspect => {
                            TreeItemMarkerKind::Yellow
                        }
                        _ => TreeItemMarkerKind::None,
                    };
                }
                TreeItemMarkerKind::None
            }
            _ => TreeItemMarkerKind::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ui::tree_item_ids::{job_tree_item_id, link_tree_item_id};
    use super::*;
    use commanductui::types::{TreeItemMarkerKind, WindowId};
    use commanductui::AppEvent;
    use harvester_core::{
        AppState, CompletedJobSnapshot, Effect, JobResultKind, LeftTab, LlmResultKind,
        LoadedArticle, Msg,
    };
    use harvester_engine::llm::prompt::PromptVersion;
    use harvester_engine::{ExtractedLink, LinkKind};
    use std::path::PathBuf;
    use std::sync::{mpsc, Arc, Mutex};

    fn completed_job_snapshot(url: &str) -> CompletedJobSnapshot {
        CompletedJobSnapshot {
            url: url.to_string(),
            tokens: Some(123),
            bytes: Some(456),
            links: vec![],
            fetched_utc: None,
        }
    }

    fn loaded_article(url: &str) -> LoadedArticle {
        LoadedArticle {
            url: url.to_string(),
            source_title: Some("Example".to_string()),
            prepared_text: std::iter::repeat_n("triage-content", 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: "triage-hash".to_string(),
            fetched_utc: None,
        }
    }

    fn triage_json(priority: u8) -> String {
        format!(
            r#"{{"category":"security","priority":{},"tags":["tag"],"rationale":"reason"}}"#,
            priority
        )
    }

    fn triage_success_result(priority: u8) -> LlmResultKind {
        let prompt_version: PromptVersion = 1;
        LlmResultKind::Success {
            output_json: triage_json(priority),
            input_tokens: 42,
            output_tokens: 7,
            prompt_version,
            model_id: "test-model".to_string(),
        }
    }

    fn extract_load_articles_request_id(effects: &[Effect]) -> u64 {
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            })
            .expect("expected LoadArticlesForTriage effect")
    }

    fn extract_llm_request_id(effects: &[Effect]) -> u64 {
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::RequestLlmCompletion { request_id, .. } => Some(*request_id),
                _ => None,
            })
            .expect("expected RequestLlmCompletion effect")
    }

    fn advance_to_triage_article_load(mut state: AppState) -> (AppState, u64) {
        for _ in 0..8 {
            let (next_state, effects) = update(state, Msg::Tick);
            state = next_state;
            if effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadArticlesForTriage { .. }))
            {
                return (state, extract_load_articles_request_id(&effects));
            }
        }
        panic!("expected LoadArticlesForTriage effect after ticking");
    }

    fn shared_state_with_triage_priority(priority: u8) -> Arc<Mutex<SharedState>> {
        let url = "https://example.com/triage";
        let (mut state, _) = update(
            AppState::new(),
            Msg::RestoreCompletedJobs(vec![completed_job_snapshot(url)]),
        );
        if let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request() {
            let ordered_urls = state.ordered_completed_job_urls_snapshot();
            let (next_state, _) = update(
                state,
                Msg::EvaluatePreTriageRefresh {
                    ordered_urls,
                    triggered_by_job_done,
                },
            );
            state = next_state;
        }
        let (state, load_request_id) = advance_to_triage_article_load(state);
        let (state, _) = update(
            state,
            Msg::TriageArticlesLoaded {
                request_id: load_request_id,
                articles: vec![loaded_article(url)],
            },
        );
        let (state, _) = update(
            state,
            Msg::LlmMetadataLoaded {
                active_versions: {
                    let mut versions = std::collections::HashMap::new();
                    versions.insert(PromptId::ArticleTriage, 1);
                    versions
                },
                effective_models: {
                    let mut models = std::collections::HashMap::new();
                    models.insert(PromptId::ArticleTriage, "test-model".to_string());
                    models
                },
                templates: std::collections::HashMap::<
                    PromptId,
                    harvester_core::PromptLabTemplateSnapshot,
                >::new(),
            },
        );
        let (state, _) = update(
            state,
            Msg::PromptContextsLoaded {
                contexts: std::collections::HashMap::new(),
            },
        );
        let (state, effects) = update(state, Msg::TriageClicked);
        let triage_request_id = extract_llm_request_id(&effects);
        let (state, _) = update(
            state,
            Msg::LlmCompleted {
                request_id: triage_request_id,
                result: triage_success_result(priority),
                metadata: None,
            },
        );
        let mut state = state;
        if let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request() {
            let ordered_urls = vec![url.to_string()];
            let (next_state, _) = update(
                state,
                Msg::EvaluatePreTriageRefresh {
                    ordered_urls,
                    triggered_by_job_done,
                },
            );
            state = next_state;
        }
        Arc::new(Mutex::new(SharedState { state }))
    }

    fn shared_state_with_ready_pre_triage_review() -> Arc<Mutex<SharedState>> {
        let url = "https://example.com/pre-triage";
        let (mut state, _) = update(
            AppState::new(),
            Msg::RestoreCompletedJobs(vec![completed_job_snapshot(url)]),
        );
        if let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request() {
            let (mut state, _) = update(
                state,
                Msg::EvaluatePreTriageRefresh {
                    ordered_urls: vec![url.to_string()],
                    triggered_by_job_done,
                },
            );
            for _ in 0..8 {
                let (next_state, effects) = update(state, Msg::Tick);
                if let Some(request_id) = effects.iter().find_map(|effect| match effect {
                    Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                    _ => None,
                }) {
                    let articles = vec![harvester_core::LoadedArticle {
                        url: url.to_string(),
                        source_title: None,
                        prepared_text: std::iter::repeat_n("pre-triage-content", 220)
                            .collect::<Vec<_>>()
                            .join(" "),
                        content_hash: "hash-pre-triage".to_string(),
                        fetched_utc: None,
                    }];
                    let (next_state, _) = update(
                        next_state,
                        Msg::TriageArticlesLoaded {
                            request_id,
                            articles,
                        },
                    );
                    return Arc::new(Mutex::new(SharedState { state: next_state }));
                }
                state = next_state;
            }
        }
        panic!("expected pre-triage refresh dispatch for test fixture");
    }

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

    fn test_handler_with_shared(
        shared: Arc<Mutex<SharedState>>,
    ) -> (AppEventHandler, mpsc::Receiver<Msg>) {
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

    fn test_handler_with_shared_and_temp_state(
        shared: Arc<Mutex<SharedState>>,
    ) -> (
        AppEventHandler,
        mpsc::Sender<Msg>,
        tempfile::TempDir,
        PathBuf,
    ) {
        let temp = tempfile::tempdir().expect("tempdir");
        let (in_tx, in_rx) = mpsc::channel();
        let (out_tx, _out_rx) = mpsc::channel();
        let output_dir = temp.path().to_path_buf();
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
        (handler, in_tx, temp, paths.state_path)
    }

    fn test_handler_with_outbound() -> (AppEventHandler, mpsc::Receiver<Msg>) {
        test_handler_with_shared(Arc::new(Mutex::new(SharedState::default())))
    }

    fn startup_test_paths() -> (tempfile::TempDir, RuntimePaths) {
        let temp = tempfile::tempdir().expect("tempdir");
        let output_dir = temp.path().to_path_buf();
        let paths = RuntimePaths::new(
            output_dir.clone(),
            output_dir.join("sources.ron"),
            output_dir.join("contexts"),
            output_dir.join("prompts"),
        );
        (temp, paths)
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
    fn pre_triage_override_persistence_preserves_completed_jobs() {
        let shared = shared_state_with_ready_pre_triage_review();
        let key = {
            let guard = shared.lock().unwrap();
            guard
                .state
                .pre_triage_key_for_job(1)
                .expect("pre-triage key for restored job")
        };
        let (mut handler, in_tx, _temp, state_path) =
            test_handler_with_shared_and_temp_state(shared);

        in_tx
            .send(Msg::PreTriageDecisionSet {
                key,
                decision: ManualDecision::Exclude,
            })
            .expect("send pre-triage decision");
        handler.process_pending_messages();
        handler.persistence_worker.shutdown();

        let completed = harvester_io::load_completed_jobs(&state_path);
        let overrides = harvester_io::load_pre_triage_overrides(&state_path);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].url, "https://example.com/pre-triage");
        assert_eq!(overrides.len(), 1);
    }

    #[test]
    fn job_tree_items_show_priority_markers_only_in_triage_results() {
        let shared = shared_state_with_triage_priority(5);
        let provider = AppUiStateProvider::new(shared.clone());
        let item_id = job_tree_item_id(1);

        assert_eq!(
            provider.tree_item_marker(WindowId::new(1), item_id),
            TreeItemMarkerKind::Yellow
        );

        {
            let mut guard = shared.lock().unwrap();
            let (state, _) = update(
                std::mem::take(&mut guard.state),
                Msg::LeftTabSelected { tab: LeftTab::Jobs },
            );
            guard.state = state;
        }

        assert_eq!(
            provider.tree_item_marker(WindowId::new(1), item_id),
            TreeItemMarkerKind::None
        );
    }

    #[test]
    fn job_tree_items_do_not_show_priority_markers_in_triage_review() {
        let shared = shared_state_with_triage_priority(4);
        let provider = AppUiStateProvider::new(shared.clone());
        let item_id = job_tree_item_id(1);

        {
            let mut guard = shared.lock().unwrap();
            let (state, _) = update(
                std::mem::take(&mut guard.state),
                Msg::LeftTabSelected {
                    tab: LeftTab::TriageReview,
                },
            );
            guard.state = state;
        }

        assert_eq!(
            provider.tree_item_marker(WindowId::new(1), item_id),
            TreeItemMarkerKind::None
        );
    }

    #[test]
    fn triage_priority_marker_mapping_is_stable() {
        assert_eq!(triage_marker_for_priority(7), TreeItemMarkerKind::Red);
        assert_eq!(triage_marker_for_priority(5), TreeItemMarkerKind::Yellow);
        assert_eq!(triage_marker_for_priority(4), TreeItemMarkerKind::Purple);
        assert_eq!(triage_marker_for_priority(3), TreeItemMarkerKind::Gray);
        assert_eq!(triage_marker_for_priority(2), TreeItemMarkerKind::None);
    }

    #[test]
    fn tree_item_marker_reads_triage_result_without_view_model_rebuild() {
        let shared = shared_state_with_triage_priority(4);
        let provider = AppUiStateProvider::new(shared.clone());
        let item_id = job_tree_item_id(1);

        assert_eq!(
            provider.tree_item_marker(WindowId::new(1), item_id),
            TreeItemMarkerKind::Purple
        );
    }

    #[test]
    fn tree_item_marker_uses_gray_for_priority_three() {
        let shared = shared_state_with_triage_priority(3);
        let provider = AppUiStateProvider::new(shared.clone());
        let item_id = job_tree_item_id(1);

        assert_eq!(
            provider.tree_item_marker(WindowId::new(1), item_id),
            TreeItemMarkerKind::Gray
        );
    }

    #[test]
    fn tree_item_marker_suppresses_low_priority_jobs() {
        let shared = shared_state_with_triage_priority(2);
        let provider = AppUiStateProvider::new(shared.clone());
        let item_id = job_tree_item_id(1);

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
    fn prepare_startup_state_schedules_metadata_load_once() {
        let (_temp, paths) = startup_test_paths();
        let (_state, effects) = prepare_startup_state(
            AppState::new(),
            &paths,
            1200,
            4,
            Some(AiAvailability::Unavailable {
                reason: AiUnavailableReason::MissingApiKey,
            }),
        );

        assert_eq!(
            effects
                .iter()
                .filter(|effect| matches!(effect, Effect::LoadLlmMetadata))
                .count(),
            1
        );
        assert_eq!(
            effects
                .iter()
                .filter(|effect| matches!(effect, Effect::LoadPromptContexts))
                .count(),
            1
        );
    }

    #[test]
    fn assembled_startup_commands_render_before_reveal() {
        let (_temp, paths) = startup_test_paths();
        let (state, _effects) = prepare_startup_state(AppState::new(), &paths, 1200, 4, None);
        let view = state.view();
        let window_id = WindowId::new(1);
        let mut tree_render_state = ui::render::TreeRenderState::new();

        let layout_commands = ui::layout::initial_commands(window_id);
        let render_commands = ui::render::render(window_id, &view, &mut tree_render_state);
        let commands =
            assemble_startup_commands(window_id, &view, &mut ui::render::TreeRenderState::new());

        let render_end = layout_commands.len() + render_commands.len();
        let show_window_indexes = commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| match command {
                PlatformCommand::ShowWindow { .. } => Some(index),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(show_window_indexes, vec![render_end + 1]);
        assert!(matches!(
            commands.get(render_end),
            Some(PlatformCommand::SignalMainWindowUISetupComplete { .. })
        ));
        assert!(matches!(
            commands.get(render_end + 1),
            Some(PlatformCommand::ShowWindow { .. })
        ));
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
    fn archive_footer_button_emits_archive_clicked() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BUTTON_ARCHIVE,
        });
        let msg = rx.recv_timeout(Duration::from_millis(250)).expect("msg");
        assert_eq!(msg, Msg::ArchiveClicked);
    }

    #[test]
    fn summarize_footer_button_emits_prepare_summaries_clicked() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BUTTON_SUMMARIZE,
        });
        let msg = rx.recv_timeout(Duration::from_millis(250)).expect("msg");
        assert_eq!(msg, Msg::PrepareSummariesClicked);
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
    fn listbox_x_key_toggles_pre_triage_decision_on_triage_review() {
        let shared = shared_state_with_ready_pre_triage_review();
        apply_msg(
            &shared,
            Msg::LeftTabSelected {
                tab: LeftTab::TriageReview,
            },
        );
        apply_msg(&shared, Msg::JobSelected { job_id: 1 });

        let expected_key = {
            let guard = shared.lock().unwrap();
            guard
                .state
                .pre_triage_key_for_job(1)
                .expect("pre triage key")
        };
        let expected_decision = {
            let guard = shared.lock().unwrap();
            match guard.state.job_filter_status(1) {
                Some(JobFilterStatus::HardExcluded { .. })
                | Some(JobFilterStatus::ManuallyExcluded) => ManualDecision::Include,
                _ => ManualDecision::Exclude,
            }
        };

        let (mut handler, rx) = test_handler_with_shared(shared);
        handler.handle_event(AppEvent::ListBoxItemKeyDown {
            window_id: WindowId::new(1),
            control_id: ui::constants::TREE_JOBS,
            key_code: b'X' as u16,
        });

        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250))
                .expect("pre triage decision"),
            Msg::PreTriageDecisionSet {
                key: expected_key,
                decision: expected_decision,
            }
        );
    }

    #[test]
    fn listbox_x_key_does_nothing_outside_pre_triage_review() {
        let shared = shared_state_with_triage_priority(5);
        apply_msg(&shared, Msg::LeftTabSelected { tab: LeftTab::Jobs });
        apply_msg(&shared, Msg::JobSelected { job_id: 1 });

        let (mut handler, rx) = test_handler_with_shared(shared);
        handler.handle_event(AppEvent::ListBoxItemKeyDown {
            window_id: WindowId::new(1),
            control_id: ui::constants::TREE_JOBS,
            key_code: b'X' as u16,
        });

        assert!(rx.recv_timeout(Duration::from_millis(150)).is_err());
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
}
