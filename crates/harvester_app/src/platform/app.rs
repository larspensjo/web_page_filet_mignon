use std::collections::{HashMap, VecDeque};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::Utc;

use commanductui::types::{MenuActionId, TreeItemMarkerKind};
use commanductui::{
    AppEvent, CheckState, PlatformCommand, PlatformEventHandler, PlatformInterface,
    UiStateProvider, WindowConfig, WindowId,
};
use harvester_core::{
    update, AppState, AppViewModel, Effect, JobResultKind, LinkDownloadState, Msg,
    PromptLabInputSource, PromptLabStage,
};

use engine_logging::{engine_info, engine_warn};

use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::prompts::register_defaults;
use harvester_engine::llm::{
    LlmConfig, LlmHandle, LlmQuotas, ModelId, OpenAiProvider, PricingRegistry, PromptRegistry,
    ProviderKind,
};

use super::effects::EffectRunner;
use super::logging::{self, LogDestination};
use super::ui;
use super::ui::tree_item_ids::{decode_tree_item_id, TreeItemKind};
use super::{effects, persistence};

const MENU_ACTION_ADD_URL: MenuActionId = MenuActionId(1);
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
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let effect_runner = if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
        let provider = Arc::new(OpenAiProvider::new(api_key));
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        let config = LlmConfig {
            provider,
            default_model: ModelId::new(ProviderKind::OpenAi, "gpt-4o-mini"),
            triage_model: None,
            summary_model: None,
            briefing_model: None,
            registry: registry.clone(),
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
        EffectRunner::new_with_llm(msg_tx.clone(), handle, 100_000, registry, model_map)
    } else {
        engine_warn!("OPENAI_API_KEY not set; LLM features disabled");
        EffectRunner::new(msg_tx.clone())
    };
    {
        let completed = persistence::load_completed_jobs(&output_dir);
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
        use super::summary_cache_store::{default_summary_cache_path, load_summary_cache};

        let path = default_summary_cache_path();
        let cache = load_summary_cache(&path);
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
        use super::triage_cache_store::{default_triage_cache_path, load_triage_cache};

        let path = default_triage_cache_path();
        let cache = load_triage_cache(&path);
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
            tree_render_state,
            output_dir,
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
    tree_render_state: ui::render::TreeRenderState,
    output_dir: std::path::PathBuf,
}

fn job_id_for_item(item_id: commanductui::TreeItemId) -> Option<harvester_core::JobId> {
    match decode_tree_item_id(item_id) {
        TreeItemKind::Job { job_id } => Some(job_id),
        TreeItemKind::LinksFolder { job_id }
        | TreeItemKind::LinksShowMore { job_id }
        | TreeItemKind::Link { job_id, .. } => Some(job_id),
    }
}

impl AppEventHandler {
    fn new(
        window_id: WindowId,
        shared: Arc<Mutex<SharedState>>,
        msg_rx: mpsc::Receiver<Msg>,
        msg_tx: mpsc::Sender<Msg>,
        effect_runner: EffectRunner,
        tree_render_state: ui::render::TreeRenderState,
        output_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            window_id,
            shared,
            commands: VecDeque::new(),
            msg_rx: Mutex::new(msg_rx),
            msg_tx,
            effect_runner,
            tree_render_state,
            output_dir,
        }
    }

    fn process_pending_messages(&mut self) {
        let mut inbox = Vec::new();
        if let Ok(rx) = self.msg_rx.lock() {
            while let Ok(msg) = rx.try_recv() {
                inbox.push(msg);
            }
        }
        for msg in inbox {
            self.dispatch_msg(msg);
        }
    }

    fn dispatch_msg(&mut self, msg: Msg) {
        let (maybe_view, clear_input) = {
            let msg_for_log = msg.clone();
            let mut guard = self.shared.lock().expect("lock shared state");
            let state = std::mem::take(&mut guard.state);
            let (state, effects) = update(state, msg);
            let should_persist = matches!(
                msg_for_log,
                Msg::JobDone {
                    result: JobResultKind::Success,
                    ..
                }
            );
            let clear_input = effects
                .iter()
                .any(|effect| matches!(effect, Effect::EnqueueUrl { .. }));
            let view = state.view();
            let mut state = state;
            let completed_snapshot = if should_persist {
                Some(state.completed_jobs_snapshot())
            } else {
                None
            };
            let was_dirty = state.consume_dirty();
            guard.state = state;
            self.effect_runner.enqueue(effects);
            if let Some(snapshot) = completed_snapshot {
                persistence::save_completed_jobs(&self.output_dir, &snapshot);
            }
            if was_dirty {
                (Some(view), clear_input)
            } else {
                (None, clear_input)
            }
        };

        if clear_input {
            self.commands.push_back(PlatformCommand::SetInputText {
                window_id: self.window_id,
                control_id: ui::constants::INPUT_URLS,
                text: String::new(),
            });
        }

        if let Some(view) = maybe_view {
            self.enqueue_render(&view);
        }
    }

    fn enqueue_render(&mut self, view: &AppViewModel) {
        self.commands.extend(ui::render::render(
            self.window_id,
            view,
            &mut self.tree_render_state,
        ));
    }

    fn prompt_lab_toggle_msg(&self) -> Msg {
        let visible = self
            .shared
            .lock()
            .map(|guard| guard.state.view().prompt_lab.visible)
            .unwrap_or(false);
        if visible {
            Msg::PromptLabCloseRequested
        } else {
            Msg::PromptLabOpenRequested
        }
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
                if control_id == ui::constants::BUTTON_ARCHIVE =>
            {
                let _ = self.msg_tx.send(Msg::ArchiveClicked);
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
                if control_id == ui::constants::BUTTON_ADD_URL =>
            {
                let _ = self.msg_tx.send(Msg::ToggleInputPanel);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BUTTON_OPEN_BROWSER =>
            {
                let _ = self.msg_tx.send(Msg::OpenInBrowserClicked);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_TOGGLE =>
            {
                let msg = self.prompt_lab_toggle_msg();
                let _ = self.msg_tx.send(msg.clone());
                if matches!(msg, Msg::PromptLabOpenRequested) {
                    let _ = self.msg_tx.send(Msg::PromptLabContextEditorOpened);
                }
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_STAGE_TRIAGE =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabStageSelected {
                    stage: PromptLabStage::Triage,
                });
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_STAGE_SUMMARY =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabStageSelected {
                    stage: PromptLabStage::Summary,
                });
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_STAGE_BRIEFING =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabStageSelected {
                    stage: PromptLabStage::Briefing,
                });
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_SOURCE_FROM_TRIAGE =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabInputSourceSelected {
                    source: PromptLabInputSource::FromTriageArticles,
                });
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_SOURCE_TYPE_URL =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabInputSourceSelected {
                    source: PromptLabInputSource::TypeUrl,
                });
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
                if control_id == ui::constants::BTN_PROMPT_LAB_RERUN =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabRerunRequested);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BTN_PROMPT_LAB_CLEAR =>
            {
                let _ = self.msg_tx.send(Msg::PromptLabHistoryCleared);
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
            AppEvent::MenuActionClicked { action_id } if action_id == MENU_ACTION_ADD_URL => {
                let _ = self.msg_tx.send(Msg::ToggleInputPanel);
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
        let effect_runner = EffectRunner::new(out_tx.clone());
        let output_dir = std::env::temp_dir();
        let handler = AppEventHandler::new(
            WindowId::new(1),
            shared,
            in_rx,
            out_tx,
            effect_runner,
            ui::render::TreeRenderState::new(),
            output_dir,
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
    fn prompt_lab_stage_button_emits_stage_selected_msg() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::ButtonClicked {
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
    fn prompt_lab_source_button_emits_source_selected_msg() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_SOURCE_TYPE_URL,
        });
        let msg = rx.recv_timeout(Duration::from_millis(250)).expect("msg");
        assert_eq!(
            msg,
            Msg::PromptLabInputSourceSelected {
                source: PromptLabInputSource::TypeUrl
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
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_PROMPT_LAB_RERUN,
        });
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_PROMPT_LAB_CLEAR,
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
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250)).expect("rerun"),
            Msg::PromptLabRerunRequested
        );
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250)).expect("clear"),
            Msg::PromptLabHistoryCleared
        );
    }

    #[test]
    fn prompt_lab_toggle_emits_open_then_close() {
        let (mut handler, rx) = test_handler_with_outbound();
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_PROMPT_LAB_TOGGLE,
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250)).expect("open"),
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
        handler.handle_event(AppEvent::ButtonClicked {
            window_id: WindowId::new(1),
            control_id: ui::constants::BTN_PROMPT_LAB_TOGGLE,
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250)).expect("close"),
            Msg::PromptLabCloseRequested
        );
    }
}
