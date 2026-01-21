use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use commanductui::{
    AppEvent, PlatformCommand, PlatformEventHandler, PlatformInterface, UiStateProvider,
    WindowConfig, WindowId,
};
use harvester_core::{update, AppState, AppViewModel, Msg};

use super::ui;

pub fn run_app() -> commanductui::PlatformResult<()> {
    env_logger::init();

    let platform = PlatformInterface::new("harvester_app".to_string())?;
    let window_id = platform.create_window(WindowConfig {
        title: "Harvester",
        width: 960,
        height: 720,
    })?;

    let shared_state = Arc::new(Mutex::new(SharedState::default()));
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();

    let initial_view = shared_state.lock().unwrap().state.view();
    let mut initial_commands = ui::layout::initial_commands(window_id);
    initial_commands.extend(ui::render::render(window_id, &initial_view));

    let event_handler: Arc<Mutex<dyn PlatformEventHandler>> = Arc::new(Mutex::new(
        AppEventHandler::new(window_id, shared_state.clone(), msg_rx, msg_tx.clone()),
    ));
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
}

impl AppEventHandler {
    fn new(
        window_id: WindowId,
        shared: Arc<Mutex<SharedState>>,
        msg_rx: mpsc::Receiver<Msg>,
        msg_tx: mpsc::Sender<Msg>,
    ) -> Self {
        Self {
            window_id,
            shared,
            commands: VecDeque::new(),
            msg_rx: Mutex::new(msg_rx),
            msg_tx,
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
        let maybe_view = {
            let mut guard = self.shared.lock().expect("lock shared state");
            let state = std::mem::take(&mut guard.state);
            let (state, _effects) = update(state, msg);
            let view = state.view();
            let mut state = state;
            let was_dirty = state.consume_dirty();
            guard.state = state;
            if was_dirty {
                Some(view)
            } else {
                None
            }
        };

        if let Some(view) = maybe_view {
            self.enqueue_render(&view);
        }
    }

    fn enqueue_render(&mut self, view: &AppViewModel) {
        self.commands
            .extend(ui::render::render(self.window_id, view));
    }
}

impl PlatformEventHandler for AppEventHandler {
    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::MainWindowUISetupComplete { .. } => {
                let _ = self.msg_tx.send(Msg::Tick);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BUTTON_START =>
            {
                let _ = self.msg_tx.send(Msg::StartClicked);
            }
            AppEvent::ButtonClicked { control_id, .. }
                if control_id == ui::constants::BUTTON_STOP =>
            {
                let _ = self.msg_tx.send(Msg::StopFinishClicked);
            }
            AppEvent::InputTextChanged {
                control_id, text, ..
            } if control_id == ui::constants::INPUT_URLS => {
                let _ = self.msg_tx.send(Msg::UrlsPasted(text));
            }
            AppEvent::WindowCloseRequestedByUser { .. } => {
                self.commands.push_back(PlatformCommand::QuitApplication);
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
    _shared: Arc<Mutex<SharedState>>,
}

impl AppUiStateProvider {
    fn new(shared: Arc<Mutex<SharedState>>) -> Self {
        Self { _shared: shared }
    }
}

impl UiStateProvider for AppUiStateProvider {
    fn is_tree_item_new(&self, _window_id: WindowId, _item_id: commanductui::TreeItemId) -> bool {
        // No tree view yet; always false.
        false
    }
}
