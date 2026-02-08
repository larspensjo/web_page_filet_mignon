use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{Effect, JobResultKind, Msg, Stage, StopPolicy};
use harvester_engine::{
    build_markdown_document, decode_html, deterministic_filename, ensure_output_dir,
    is_confined_to, AtomicFileWriter, Converter, DecodeError, EngineConfig, EngineEvent,
    EngineHandle, Extractor, LinkExtractingConverter, ReadabilityLikeExtractor, UrlPolicy,
    WhitespaceTokenCounter,
};
use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;

pub(crate) fn default_output_dir() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("output")
}

const LINKED_ALLOWED_CONTENT_TYPES: [&str; 3] =
    ["text/html", "application/xhtml+xml", "text/plain"];
const LINK_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

pub struct EffectRunner {
    engine: EngineHandle,
    msg_tx: mpsc::Sender<Msg>,
    output_dir: PathBuf,
    url_policy: UrlPolicy,
}

impl EffectRunner {
    pub fn new(msg_tx: mpsc::Sender<Msg>) -> Self {
        let output_dir = default_output_dir();

        let mut config = EngineConfig::default_with_output(output_dir.clone());
        config.fetched_utc = std::sync::Arc::new(|| Utc::now().to_rfc3339());
        let url_policy = config.url_policy.clone();

        let engine = EngineHandle::new(config);
        let runner = Self {
            engine,
            msg_tx: msg_tx.clone(),
            output_dir,
            url_policy,
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
                    let output_dir = self.output_dir.clone();
                    let url_policy = self.url_policy.clone();
                    thread::spawn(move || {
                        let _ = msg_tx.send(Msg::LinkDownloadStarted { job_id, link_index });
                        match download_link_page(&url, &output_dir, &url_policy) {
                            Ok(absolute_path) => {
                                let relative_path = absolute_path
                                    .strip_prefix(&output_dir)
                                    .map(|p| p.to_path_buf())
                                    .unwrap_or_else(|_| absolute_path.clone());
                                let _ = msg_tx.send(Msg::LinkDownloadCompleted {
                                    job_id,
                                    link_index,
                                    path: relative_path,
                                });
                            }
                            Err(error) => {
                                engine_warn!("Linked page download failed: {}", error);
                                let _ = msg_tx.send(Msg::LinkDownloadFailed {
                                    job_id,
                                    link_index,
                                    error,
                                });
                            }
                        }
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
                    let output_dir = self.output_dir.clone();
                    thread::spawn(move || {
                        if is_confined_to(&path, &output_dir) {
                            let absolute_path = output_dir.join(&path);
                            let _ = fs::remove_file(&absolute_path);
                        } else {
                            engine_warn!(
                                "DeleteLinkedPage rejected unsafe path job_id={} link_index={} path={}",
                                job_id,
                                link_index,
                                path.display()
                            );
                        }
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

fn download_link_page(
    url: &str,
    output_dir: &Path,
    url_policy: &UrlPolicy,
) -> Result<PathBuf, String> {
    let linked_dir = output_dir.join("linked");
    ensure_output_dir(&linked_dir).map_err(|err| format!("linked output dir error: {}", err))?;

    let parsed = reqwest::Url::parse(url)
        .map_err(|err| format!("url parsing failed for {}: {}", url, err))?;
    if let Err(violation) = url_policy.check(&parsed) {
        return Err(format!(
            "url policy violation for linked page: {}",
            violation
        ));
    }

    let client = Client::builder()
        .timeout(LINK_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|err| err.to_string())?;
    let response = client.get(parsed).send().map_err(|err| err.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "HTTP error {} for linked page {}",
            response.status(),
            url
        ));
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    if let Some(ref content_type) = content_type {
        let ct = content_type
            .split(';')
            .next()
            .unwrap_or(content_type)
            .trim();
        if !LINKED_ALLOWED_CONTENT_TYPES
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(ct))
        {
            return Err(format!("unsupported content type '{}'", ct));
        }
    }

    let final_url = response.url().to_string();
    let bytes = response.bytes().map_err(|err| err.to_string())?;
    let decoded =
        decode_html(&bytes, content_type.as_deref()).map_err(|err: DecodeError| err.to_string())?;

    let extractor = ReadabilityLikeExtractor;
    let extracted = extractor.extract(&decoded.html);
    let converter = LinkExtractingConverter::new();
    let conversion = converter.to_markdown(&extracted.content_html, Some(final_url.as_str()));
    let token_counter = WhitespaceTokenCounter;
    let fetched_utc = Utc::now().to_rfc3339();
    let (_tokens, doc) = build_markdown_document(
        &final_url,
        extracted.title.as_deref(),
        &decoded.encoding_label,
        &fetched_utc,
        &conversion.markdown,
        &token_counter,
    );

    let filename = deterministic_filename(extracted.title.as_deref(), &final_url);
    let writer = AtomicFileWriter::new(linked_dir);
    writer.write(&filename, &doc).map_err(|err| err.to_string())
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
