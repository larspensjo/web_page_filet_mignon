use std::sync::{mpsc, Arc};
use std::thread;

use crate::fetch::{ChannelProgressSink, FetchSettings, Fetcher, ReqwestFetcher};
use crate::{EngineEvent, FetchError, FetchOutput, JobId};

enum EngineCommand {
    Enqueue { job_id: JobId, url: String },
}

pub struct EngineHandle {
    cmd_tx: mpsc::Sender<EngineCommand>,
    event_rx: mpsc::Receiver<EngineEvent>,
}

impl EngineHandle {
    pub fn new(settings: FetchSettings) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let fetcher = Arc::new(ReqwestFetcher::new(settings));

        thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
            while let Ok(command) = cmd_rx.recv() {
                let fetcher = fetcher.clone();
                let event_tx = event_tx.clone();
                runtime.spawn(async move {
                    handle_command(fetcher.as_ref(), command, event_tx).await;
                });
            }
        });

        Self { cmd_tx, event_rx }
    }

    pub fn enqueue(&self, job_id: JobId, url: impl Into<String>) {
        let _ = self.cmd_tx.send(EngineCommand::Enqueue {
            job_id,
            url: url.into(),
        });
    }

    pub fn try_recv(&self) -> Option<EngineEvent> {
        self.event_rx.try_recv().ok()
    }
}

async fn handle_command(
    fetcher: &dyn Fetcher,
    command: EngineCommand,
    event_tx: mpsc::Sender<EngineEvent>,
) {
    match command {
        EngineCommand::Enqueue { job_id, url } => {
            let sink = ChannelProgressSink::new(event_tx.clone());
            let result: Result<FetchOutput, FetchError> =
                fetcher.fetch(job_id, &url, &sink).await;
            let _ = event_tx.send(EngineEvent::FetchCompleted { job_id, result });
        }
    }
}
