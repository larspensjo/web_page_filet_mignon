use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{Effect, JobResultKind, Msg, Stage, StopPolicy};
use harvester_engine::{EngineConfig, EngineEvent, EngineHandle};

pub(crate) fn default_output_dir() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("output")
}

pub struct EffectRunner {
    engine: EngineHandle,
    msg_tx: mpsc::Sender<Msg>,
}

impl EffectRunner {
    pub fn new(msg_tx: mpsc::Sender<Msg>) -> Self {
        let output_dir = default_output_dir();

        let mut config = EngineConfig::default_with_output(output_dir);
        config.fetched_utc = std::sync::Arc::new(|| Utc::now().to_rfc3339());

        let engine = EngineHandle::new(config);
        let runner = Self {
            engine,
            msg_tx: msg_tx.clone(),
        };
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
                Effect::ArchiveRequested => {
                    engine_info!("Archive requested: enqueue export job");
                    self.engine.request_export();
                }
                Effect::DownloadLinkedPage {
                    job_id,
                    link_index,
                    url,
                } => {
                    engine_info!(
                        "Download linked page job_id={} link_index={} url_len={}",
                        job_id,
                        link_index,
                        url.len()
                    );
                    let msg_tx = self.msg_tx.clone();
                    thread::spawn(move || {
                        let _ = msg_tx.send(Msg::LinkDownloadStarted { job_id, link_index });
                        let error =
                            format!("Linked page downloads are not implemented yet: {}", url);
                        engine_warn!("{}", error);
                        let _ = msg_tx.send(Msg::LinkDownloadFailed {
                            job_id,
                            link_index,
                            error,
                        });
                    });
                }
                Effect::DeleteLinkedPage {
                    job_id,
                    link_index,
                    path,
                } => {
                    engine_info!(
                        "Delete linked page job_id={} link_index={} path={}",
                        job_id,
                        link_index,
                        path.display()
                    );
                    let msg_tx = self.msg_tx.clone();
                    thread::spawn(move || {
                        let _ = msg_tx.send(Msg::LinkDeleted { job_id, link_index });
                    });
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
                            content_preview: progress.content_preview.clone(),
                        });
                    }
                    EngineEvent::JobCompleted { job_id, result } => {
                        let msg = match result {
                            Ok(outcome) => Msg::JobDone {
                                job_id,
                                result: JobResultKind::Success,
                                content_preview: outcome.content_preview,
                                extracted_links: outcome.extracted_links,
                            },
                            Err(failure_kind) => {
                                engine_warn!("Job {} failed: {}", job_id, failure_kind);
                                Msg::JobDone {
                                    job_id,
                                    result: JobResultKind::Failed,
                                    content_preview: None,
                                    extracted_links: Vec::new(),
                                }
                            }
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
