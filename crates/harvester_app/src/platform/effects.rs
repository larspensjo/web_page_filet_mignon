use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{Effect, JobResultKind, Msg, Stage, StopPolicy};
use harvester_engine::{EngineConfig, EngineEvent, EngineHandle};

pub struct EffectRunner {
    engine: EngineHandle,
}

impl EffectRunner {
    pub fn new(msg_tx: mpsc::Sender<Msg>) -> Self {
        let output_dir = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("output");

        let mut config = EngineConfig::default_with_output(output_dir);
        config.fetched_utc = std::sync::Arc::new(|| Utc::now().to_rfc3339());

        let engine = EngineHandle::new(config);
        let runner = Self { engine };
        runner.spawn_event_loop(msg_tx);
        runner
    }

    pub fn enqueue(&self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::EnqueueUrl { job_id, url } => {
                    engine_info!(
                        "EnqueueUrl job_id={} url_len={} url={}",
                        job_id,
                        url.len(),
                        url
                    );
                    self.engine.enqueue(job_id, url);
                }
                Effect::StartSession => {
                    // no-op; engine starts on first enqueue
                }
                Effect::StopFinish { policy } => {
                    let immediate = matches!(policy, StopPolicy::Immediate);
                    self.engine.stop(immediate);
                }
            }
        }
    }

    fn spawn_event_loop(&self, msg_tx: mpsc::Sender<Msg>) {
        let engine = self.engine.clone();
        thread::spawn(move || loop {
            if let Some(event) = engine.try_recv() {
                match event {
                    EngineEvent::Progress(progress) => {
                        let _ = msg_tx.send(Msg::JobProgress {
                            job_id: progress.job_id,
                            stage: map_stage(progress.stage),
                            tokens: progress.tokens,
                            bytes: progress.bytes,
                        });
                    }
                    EngineEvent::JobCompleted { job_id, result } => {
                        let msg = Msg::JobDone {
                            job_id,
                            result: match &result {
                                Ok(_) => JobResultKind::Success,
                                Err(failure_kind) => {
                                    engine_warn!("Job {} failed: {}", job_id, failure_kind);
                                    JobResultKind::Failed
                                }
                            },
                        };
                        let _ = msg_tx.send(msg);
                    }
                }
            } else {
                thread::sleep(Duration::from_millis(20));
            }
        });
    }
}

fn map_stage(stage: harvester_engine::Stage) -> Stage {
    match stage {
        harvester_engine::Stage::Queued => Stage::Queued,
        harvester_engine::Stage::Downloading => Stage::Downloading,
        harvester_engine::Stage::Sanitizing => Stage::Sanitizing,
        harvester_engine::Stage::Converting => Stage::Converting,
        harvester_engine::Stage::Tokenizing => Stage::Tokenizing,
        harvester_engine::Stage::Writing => Stage::Writing,
        harvester_engine::Stage::Done => Stage::Done,
    }
}
