use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use commanductui::types::TreeItemMarkerKind;
use commanductui::{
    AppEvent, CheckState, PlatformCommand, PlatformEventHandler, PlatformInterface,
    UiStateProvider, WindowConfig, WindowId,
};
use harvester_core::{
    update, AppState, AppViewModel, Effect, JobResultKind, LinkDownloadState, Msg,
};

use engine_logging::engine_info;

use super::effects::EffectRunner;
use super::logging::{self, LogDestination};
use super::ui;
use super::ui::tree_item_ids::{decode_tree_item_id, TreeItemKind};
use super::{effects, persistence};

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
    let output_dir = effects::default_output_dir();
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let effect_runner = EffectRunner::new(msg_tx.clone());
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
                let _ = self
                    .msg_tx
                    .send(Msg::SplitterMoved { desired_left_width_px });
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
                let _ = self
                    .msg_tx
                    .send(Msg::SplitterMoved { desired_left_width_px });
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
    use commanductui::types::TreeItemMarkerKind;
    use commanductui::WindowId;
    use harvester_core::{AppState, JobResultKind};
    use harvester_engine::{ExtractedLink, LinkKind};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

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
        let mut shared = SharedState::default();
        shared.state = state;
        Arc::new(Mutex::new(shared))
    }

    fn apply_msg(shared: &Arc<Mutex<SharedState>>, msg: Msg) {
        let mut guard = shared.lock().unwrap();
        let (state, _) = update(std::mem::take(&mut guard.state), msg);
        guard.state = state;
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
}
