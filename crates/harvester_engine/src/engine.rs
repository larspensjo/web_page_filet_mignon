use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use engine_logging::{engine_debug, engine_info, engine_warn};
use tokio::runtime::Runtime;
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;

use crate::convert::Converter;
use crate::decode::decode_html;
use crate::extract::Extractor;
use crate::fetch::{ChannelProgressSink, FetchSettings, Fetcher, ReqwestFetcher};
use crate::frontmatter::build_markdown_document;
use crate::persist::AtomicFileWriter;
use crate::preview::prepare_preview_content;
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
            extractor: Arc::new(crate::ReadabilityLikeExtractor),
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
    Stop,
    Export,
}

#[derive(Clone)]
pub struct EngineHandle {
    cmd_tx: mpsc::Sender<EngineCommand>,
    event_rx: Arc<Mutex<mpsc::Receiver<EngineEvent>>>,
}

impl EngineHandle {
    pub fn new(config: EngineConfig) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx_raw) = mpsc::channel();
        let event_rx = Arc::new(Mutex::new(event_rx_raw));
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

    pub fn stop(&self, _immediate: bool) {
        let _ = self.cmd_tx.send(EngineCommand::Stop);
    }

    pub fn request_export(&self) {
        let _ = self.cmd_tx.send(EngineCommand::Export);
    }

    pub fn try_recv(&self) -> Option<EngineEvent> {
        if let Ok(rx) = self.event_rx.lock() {
            rx.try_recv().ok()
        } else {
            None
        }
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
                EngineCommand::Stop => {
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
                EngineCommand::Export => {
                    // Export happens when queue is empty / idle; stash command for later processing.
                    queue.push_front((0, "__EXPORT__".to_string()));
                }
            }
        }

        if let Some((job_id, url)) = queue.pop_front() {
            if url == "__EXPORT__" {
                if queue.is_empty() {
                    // Only export when no active jobs; run synchronously.
                    if let Err(_err) = crate::export::build_concatenated_export(
                        &config.output_dir,
                        crate::export::ExportOptions::default(),
                    ) {
                        let _ = event_tx.send(EngineEvent::JobCompleted {
                            job_id: 0,
                            result: Err(FailureKind::ProcessingError),
                        });
                    }
                } else {
                    // Re-enqueue to try later.
                    queue.push_back((job_id, url));
                }
                continue;
            }
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
                        EngineCommand::Stop => {
                            accept_new = false;
                            cancel_token.cancel();
                            for (job_id, _) in queue.drain(..) {
                                let _ = event_tx.send(EngineEvent::JobCompleted {
                                    job_id,
                                    result: Err(FailureKind::Cancelled),
                                });
                            }
                        }
                        EngineCommand::Export => {
                            queue.push_front((0, "__EXPORT__".to_string()));
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
    engine_info!("Job {} starting: {}", job_id, url);
    let sink = ChannelProgressSink::new(event_tx.clone());

    let fetch_result = fetcher.fetch(job_id, &url, &sink).await;
    let fetch_output = match fetch_result {
        Ok(out) => {
            engine_debug!(
                "Job {} fetched {} bytes from {}",
                job_id,
                out.metadata.byte_len,
                out.metadata.final_url
            );
            out
        }
        Err(err) => {
            // Error already logged in fetch.rs
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

    let decoded = match timeout(config.extract_timeout, async {
        decode_html(
            &fetch_output.bytes,
            fetch_output.metadata.content_type.as_deref(),
        )
    })
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

    let conversion = match timeout(config.convert_timeout, async {
        config.converter.to_markdown(
            &extracted.content_html,
            Some(fetch_output.metadata.final_url.as_str()),
        )
    })
    .await
    {
        Ok(output) => output,
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

    let markdown = conversion.markdown;
    let preview_content = prepare_preview_content(&markdown);

    let _ = event_tx.send(EngineEvent::Progress(JobProgress {
        job_id,
        stage: Stage::Converting,
        bytes: None,
        tokens: None,
        content_preview: Some(preview_content.clone()),
    }));

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
        content_preview: None,
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
            engine_info!(
                "Job {} completed: {} tokens, {} bytes written",
                job_id,
                token_count,
                doc_for_write.len()
            );
            let _ = event_tx.send(EngineEvent::JobCompleted {
                job_id,
                result: Ok(JobOutcome {
                    final_url: fetch_output.metadata.final_url,
                    tokens: Some(token_count),
                    bytes_written: Some(doc_for_write.len() as u64),
                    content_preview: Some(preview_content),
                    extracted_links: conversion.links,
                }),
            });
        }
        _ => {
            engine_warn!("Job {} failed: write error", job_id);
            let _ = event_tx.send(EngineEvent::JobCompleted {
                job_id,
                result: Err(FailureKind::ProcessingError),
            });
        }
    }
}
