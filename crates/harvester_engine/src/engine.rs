use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::thread;

use tokio::runtime::Runtime;
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;

use crate::convert::Converter;
use crate::decode::decode_html;
use crate::extract::Extractor;
use crate::fetch::{ChannelProgressSink, FetchSettings, Fetcher, ReqwestFetcher};
use crate::frontmatter::build_markdown_document;
use crate::persist::AtomicFileWriter;
use crate::token::TokenCounter;
use crate::{
    deterministic_filename, EngineEvent, FailureKind, JobId, JobOutcome, JobProgress, Stage,
};

#[derive(Clone)]
pub struct EngineConfig {
    pub fetch_settings: FetchSettings,
    pub output_dir: PathBuf,
    pub extractor: Arc<dyn Extractor>,
    pub converter: Arc<dyn Converter>,
    pub token_counter: Arc<dyn TokenCounter>,
    /// Returns UTC timestamp string. Tests can inject fixed value.
    pub fetched_utc: Arc<dyn Fn() -> String + Send + Sync>,
    pub extract_timeout: Duration,
    pub convert_timeout: Duration,
    pub tokenize_timeout: Duration,
    pub writing_timeout: Duration,
}

impl EngineConfig {
    pub fn default_with_output(output_dir: PathBuf) -> Self {
        Self {
            fetch_settings: FetchSettings::default(),
            output_dir,
            extractor: Arc::new(crate::ReadabilityLikeExtractor::default()),
            converter: Arc::new(crate::Html2MdConverter),
            token_counter: Arc::new(crate::WhitespaceTokenCounter),
            fetched_utc: Arc::new(|| "1970-01-01T00:00:00Z".to_string()),
            extract_timeout: Duration::from_secs(30),
            convert_timeout: Duration::from_secs(15),
            tokenize_timeout: Duration::from_secs(10),
            writing_timeout: Duration::from_secs(10),
        }
    }
}

enum EngineCommand {
    Enqueue { job_id: JobId, url: String },
    Stop { immediate: bool },
}

pub struct EngineHandle {
    cmd_tx: mpsc::Sender<EngineCommand>,
    event_rx: mpsc::Receiver<EngineEvent>,
}

impl EngineHandle {
    pub fn new(config: EngineConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let config = Arc::new(config);

        thread::spawn(move || worker_loop(cmd_rx, event_tx, config));

        Self { cmd_tx, event_rx }
    }

    pub fn enqueue(&self, job_id: JobId, url: impl Into<String>) {
        let _ = self.cmd_tx.send(EngineCommand::Enqueue {
            job_id,
            url: url.into(),
        });
    }

    pub fn stop(&self, immediate: bool) {
        let _ = self.cmd_tx.send(EngineCommand::Stop { immediate });
    }

    pub fn try_recv(&self) -> Option<EngineEvent> {
        self.event_rx.try_recv().ok()
    }
}

fn worker_loop(
    cmd_rx: mpsc::Receiver<EngineCommand>,
    event_tx: mpsc::Sender<EngineEvent>,
    config: Arc<EngineConfig>,
) {
    let runtime = Runtime::new().expect("tokio runtime");
    let fetcher = Arc::new(ReqwestFetcher::new(config.fetch_settings.clone()));
    let mut queue: VecDeque<(JobId, String)> = VecDeque::new();
    let mut accept_new = true;
    let cancel_token = CancellationToken::new();

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                EngineCommand::Enqueue { job_id, url } => {
                    if accept_new {
                        queue.push_back((job_id, url));
                    } else {
                        let _ = event_tx.send(EngineEvent::JobCompleted {
                            job_id,
                            result: Err(FailureKind::Cancelled),
                        });
                    }
                }
                EngineCommand::Stop { immediate: _ } => {
                    accept_new = false;
                    cancel_token.cancel();
                    // Cancel queued (not yet started) immediately.
                    for (job_id, _) in queue.drain(..) {
                        let _ = event_tx.send(EngineEvent::JobCompleted {
                            job_id,
                            result: Err(FailureKind::Cancelled),
                        });
                    }
                }
            }
        }

        if let Some((job_id, url)) = queue.pop_front() {
            let fetcher = fetcher.clone();
            let event_tx = event_tx.clone();
            let config = config.clone();
            let child_token = cancel_token.child_token();
            runtime.block_on(async move {
                run_job(job_id, url, fetcher.as_ref(), event_tx, config, child_token).await;
            });
        } else {
            // Block until next command arrives.
            match cmd_rx.recv() {
                Ok(cmd) => {
                    // push back into the queue / handle stop.
                    match cmd {
                        EngineCommand::Enqueue { job_id, url } => {
                            if accept_new {
                                queue.push_back((job_id, url));
                            } else {
                                let _ = event_tx.send(EngineEvent::JobCompleted {
                                    job_id,
                                    result: Err(FailureKind::Cancelled),
                                });
                            }
                        }
                        EngineCommand::Stop { immediate: _ } => {
                            accept_new = false;
                            cancel_token.cancel();
                            for (job_id, _) in queue.drain(..) {
                                let _ = event_tx.send(EngineEvent::JobCompleted {
                                    job_id,
                                    result: Err(FailureKind::Cancelled),
                                });
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
    }
}

async fn run_job(
    job_id: JobId,
    url: String,
    fetcher: &dyn Fetcher,
    event_tx: mpsc::Sender<EngineEvent>,
    config: Arc<EngineConfig>,
    cancel_token: CancellationToken,
) {
    let sink = ChannelProgressSink::new(event_tx.clone());

    let fetch_result = fetcher.fetch(job_id, &url, &sink).await;
    let fetch_output = match fetch_result {
        Ok(out) => out,
        Err(err) => {
            let _ = event_tx.send(EngineEvent::JobCompleted {
                job_id,
                result: Err(err.kind),
            });
            return;
        }
    };

    // Check cancellation after fetching stage boundary.
    if cancel_token.is_cancelled() {
        let _ = event_tx.send(EngineEvent::JobCompleted {
            job_id,
            result: Err(FailureKind::Cancelled),
        });
        return;
    }

    let decoded = match timeout(
        config.extract_timeout,
        async { decode_html(&fetch_output.bytes, fetch_output.metadata.content_type.as_deref()) },
    )
    .await
    {
        Ok(Ok(decoded)) => decoded,
        Ok(Err(_)) => {
            let _ = event_tx.send(EngineEvent::JobCompleted {
                job_id,
                result: Err(FailureKind::ProcessingError),
            });
            return;
        }
        Err(_) => {
            let _ = event_tx.send(EngineEvent::JobCompleted {
                job_id,
                result: Err(FailureKind::ProcessingTimeout {
                    stage: Stage::Sanitizing,
                }),
            });
            return;
        }
    };

    if cancel_token.is_cancelled() {
        let _ = event_tx.send(EngineEvent::JobCompleted {
            job_id,
            result: Err(FailureKind::Cancelled),
        });
        return;
    }

    let extracted = match timeout(config.extract_timeout, async {
        config.extractor.extract(&decoded.html)
    })
    .await
    {
        Ok(content) => content,
        Err(_) => {
            let _ = event_tx.send(EngineEvent::JobCompleted {
                job_id,
                result: Err(FailureKind::ProcessingTimeout {
                    stage: Stage::Converting,
                }),
            });
            return;
        }
    };

    let markdown = match timeout(config.convert_timeout, async {
        config.converter.to_markdown(&extracted.content_html)
    })
    .await
    {
        Ok(md) => md,
        Err(_) => {
            let _ = event_tx.send(EngineEvent::JobCompleted {
                job_id,
                result: Err(FailureKind::ProcessingTimeout {
                    stage: Stage::Converting,
                }),
            });
            return;
        }
    };

    if cancel_token.is_cancelled() {
        let _ = event_tx.send(EngineEvent::JobCompleted {
            job_id,
            result: Err(FailureKind::Cancelled),
        });
        return;
    }

    let tokens = match timeout(config.tokenize_timeout, async {
        config.token_counter.count(&markdown)
    })
    .await
    {
        Ok(t) => t,
        Err(_) => {
            let _ = event_tx.send(EngineEvent::JobCompleted {
                job_id,
                result: Err(FailureKind::ProcessingTimeout {
                    stage: Stage::Tokenizing,
                }),
            });
            return;
        }
    };

    let _ = event_tx.send(EngineEvent::Progress(JobProgress {
        job_id,
        stage: Stage::Tokenizing,
        bytes: None,
        tokens: Some(tokens),
    }));

    if cancel_token.is_cancelled() {
        let _ = event_tx.send(EngineEvent::JobCompleted {
            job_id,
            result: Err(FailureKind::Cancelled),
        });
        return;
    }

    let (token_count, doc) = build_markdown_document(
        fetch_output.metadata.final_url.as_str(),
        extracted.title.as_deref(),
        &decoded.encoding_label,
        &(config.fetched_utc)(),
        &markdown,
        config.token_counter.as_ref(),
    );

    let filename = deterministic_filename(extracted.title.as_deref(), &url);
    let writer = AtomicFileWriter::new(config.output_dir.clone());

    let doc_for_write = doc.clone();
    let write_result = timeout(config.writing_timeout, async move {
        tokio::task::spawn_blocking(move || writer.write(&filename, &doc)).await
    })
    .await;

    match write_result {
        Ok(Ok(Ok(_path))) => {
            let _ = event_tx.send(EngineEvent::JobCompleted {
                job_id,
                result: Ok(JobOutcome {
                    final_url: fetch_output.metadata.final_url,
                    tokens: Some(token_count),
                    bytes_written: Some(doc_for_write.len() as u64),
                }),
            });
        }
        _ => {
            let _ = event_tx.send(EngineEvent::JobCompleted {
                job_id,
                result: Err(FailureKind::ProcessingError),
            });
        }
    }
}
