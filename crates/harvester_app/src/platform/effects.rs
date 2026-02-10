use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{Effect, JobResultKind, LlmResultKind, LoadedArticle, Msg, Stage, StopPolicy};
use harvester_engine::{
    build_markdown_document, decode_html, deterministic_filename, ensure_output_dir,
    is_confined_to,
    llm::{LlmCommand, LlmCompletionError, LlmEvent, LlmHandle, PromptRegistry},
    load_and_prepare_articles, AtomicFileWriter, Converter, DecodeError, EngineConfig, EngineEvent,
    EngineHandle, Extractor, FetchSettings, LinkExtractingConverter, ReadabilityLikeExtractor,
    UrlPolicy, WhitespaceTokenCounter,
};
use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;
use reqwest::redirect::Policy;

pub(crate) fn default_output_dir() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("output")
}

pub struct EffectRunner {
    engine: EngineHandle,
    msg_tx: mpsc::Sender<Msg>,
    output_dir: PathBuf,
    url_policy: UrlPolicy,
    fetch_settings: FetchSettings,
    llm_handle: Option<LlmHandle>,
    llm_max_input_chars: Option<usize>,
    prompt_registry: PromptRegistry,
}

impl EffectRunner {
    pub fn new(msg_tx: mpsc::Sender<Msg>) -> Self {
        let registry = PromptRegistry::with_defaults();
        Self::with_optional_llm(msg_tx, None, None, registry)
    }

    pub fn new_with_llm(
        msg_tx: mpsc::Sender<Msg>,
        llm_handle: LlmHandle,
        llm_max_input_chars: usize,
        prompt_registry: PromptRegistry,
    ) -> Self {
        Self::with_optional_llm(
            msg_tx,
            Some(llm_handle),
            Some(llm_max_input_chars),
            prompt_registry,
        )
    }

    fn with_optional_llm(
        msg_tx: mpsc::Sender<Msg>,
        llm_handle: Option<LlmHandle>,
        llm_max_input_chars: Option<usize>,
        prompt_registry: PromptRegistry,
    ) -> Self {
        let output_dir = default_output_dir();

        let mut config = EngineConfig::default_with_output(output_dir.clone());
        config.fetched_utc = std::sync::Arc::new(|| Utc::now().to_rfc3339());
        let url_policy = config.url_policy.clone();
        let fetch_settings = config.fetch_settings.clone();

        let engine = EngineHandle::new(config);
        let runner = Self {
            engine,
            msg_tx: msg_tx.clone(),
            output_dir,
            url_policy,
            fetch_settings,
            llm_handle,
            llm_max_input_chars,
            prompt_registry,
        };
        runner.spawn_event_loop(msg_tx);
        runner
    }

    pub fn enqueue(&self, effects: Vec<Effect>) {
        for effect in effects {
            if let Err(reason) = self.validate_effect(&effect) {
                self.reject_effect(effect, reason);
                continue;
            }
            self.execute_effect(effect);
        }
    }

    fn execute_effect(&self, effect: Effect) {
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
                let fetch_settings = self.fetch_settings.clone();
                thread::spawn(move || {
                    let _ = msg_tx.send(Msg::LinkDownloadStarted { job_id, link_index });
                    match download_link_page(&url, &output_dir, &url_policy, &fetch_settings) {
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
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                prompt_version,
                input_content,
                context,
            } => {
                if let Some(handle) = &self.llm_handle {
                    let cmd = LlmCommand::Complete {
                        request_id,
                        prompt_id,
                        prompt_version,
                        input_content,
                        context,
                    };
                    if let Err(err) = handle.send(cmd) {
                        engine_warn!(
                            "LLM completion request failed to dispatch: request_id={} error={:?}",
                            request_id,
                            err
                        );
                        let _ = self.msg_tx.send(Msg::LlmCompleted {
                            request_id,
                            result: LlmResultKind::Failed {
                                reason: "LLM worker unavailable".to_string(),
                            },
                        });
                    } else {
                        engine_info!(
                            "[llm-dispatch] request_id={} prompt_id={:?}",
                            request_id,
                            prompt_id
                        );
                    }
                } else {
                    engine_warn!(
                        "LLM completion requested without handle: request_id={}",
                        request_id
                    );
                    let _ = self.msg_tx.send(Msg::LlmCompleted {
                        request_id,
                        result: LlmResultKind::Failed {
                            reason: "LLM not configured".to_string(),
                        },
                    });
                }
            }
            Effect::LoadArticlesForBriefing => {
                let msg_tx = self.msg_tx.clone();
                let output_dir = self.output_dir.clone();
                let max_input_bytes = self.llm_max_input_chars.unwrap_or(100_000);
                let registry = self.prompt_registry.clone();
                thread::spawn(move || {
                    match load_and_prepare_articles(&output_dir, max_input_bytes, &registry) {
                        Ok((articles, collection_text)) => {
                            let loaded_articles: Vec<LoadedArticle> = articles
                                .into_iter()
                                .map(|article| LoadedArticle {
                                    url: article.url,
                                    source_title: article.source_title,
                                    prepared_text: article.prepared_text,
                                    content_hash: article.content_hash,
                                })
                                .collect();
                            engine_info!(
                                "[briefing-loader] prepared {} article(s)",
                                loaded_articles.len()
                            );
                            let _ = msg_tx.send(Msg::ArticlesLoaded {
                                articles: loaded_articles,
                                collection_text,
                            });
                        }
                        Err(reason) => {
                            engine_warn!("[briefing-loader] load failed: {}", reason);
                            let _ = msg_tx.send(Msg::ArticlesLoadFailed { reason });
                        }
                    }
                });
            }
            Effect::LoadArticlesForTriage => {
                engine_warn!("[triage-loader] TODO: effect not implemented yet");
            }
        }
    }

    fn spawn_event_loop(&self, msg_tx: mpsc::Sender<Msg>) {
        let engine = self.engine.clone();
        let engine_tx = msg_tx.clone();
        thread::spawn(move || loop {
            if let Some(event) = engine.try_recv() {
                match event {
                    EngineEvent::Progress(progress) => {
                        let _ = engine_tx.send(Msg::JobProgress {
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
                                let reason = failure_kind.to_string();
                                engine_warn!("Job {} failed: {}", job_id, reason);
                                Msg::JobDone {
                                    job_id,
                                    result: JobResultKind::Failed { reason },
                                    content_preview: None,
                                    extracted_links: Vec::new(),
                                }
                            }
                        };
                        let _ = engine_tx.send(msg);
                    }
                }
            } else {
                thread::sleep(Duration::from_millis(20));
            }
        });

        if let Some(handle) = self.llm_handle.clone() {
            let llm_tx = msg_tx.clone();
            let receiver = handle.event_receiver();
            thread::spawn(move || loop {
                let event = {
                    let guard = receiver.lock().expect("LLM event receiver lock");
                    guard.recv()
                };
                match event {
                    Ok(llm_event) => {
                        let msg = map_llm_event(llm_event);
                        if llm_tx.send(msg).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            });
        }
    }
}

impl EffectRunner {
    fn validate_effect(&self, effect: &Effect) -> Result<(), String> {
        match effect {
            Effect::EnqueueUrl { url, .. } | Effect::DownloadLinkedPage { url, .. } => {
                let parsed = reqwest::Url::parse(url)
                    .map_err(|err| format!("url parsing failed for {}: {}", url, err))?;
                if let Err(violation) = self.url_policy.check(&parsed) {
                    Err(format!(
                        "url policy violation for url '{}': {}",
                        url, violation
                    ))
                } else {
                    Ok(())
                }
            }
            Effect::DeleteLinkedPage { path, .. } => {
                if is_confined_to(path, &self.output_dir) {
                    Ok(())
                } else {
                    Err(format!(
                        "path policy violation for linked delete path '{}'",
                        path.display()
                    ))
                }
            }
            Effect::RequestLlmCompletion { input_content, .. } => {
                if let Some(limit) = self.llm_max_input_chars {
                    if input_content.len() > limit {
                        return Err(format!(
                            "LLM input too large ({} > {} characters)",
                            input_content.len(),
                            limit
                        ));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn reject_effect(&self, effect: Effect, reason: String) {
        match effect {
            Effect::EnqueueUrl { job_id, .. } => {
                engine_warn!("EnqueueUrl rejected job_id={} reason={}", job_id, reason);
                if let Err(err) = self.msg_tx.send(Msg::JobDone {
                    job_id,
                    result: JobResultKind::Failed {
                        reason: reason.clone(),
                    },
                    content_preview: None,
                    extracted_links: Vec::new(),
                }) {
                    engine_warn!(
                        "Failed to notify job failure for job_id={}: {}",
                        job_id,
                        err
                    );
                }
            }
            Effect::DownloadLinkedPage {
                job_id, link_index, ..
            } => {
                engine_warn!(
                    "DownloadLinkedPage rejected job_id={} link_index={} reason={}",
                    job_id,
                    link_index,
                    reason
                );
                if let Err(err) = self.msg_tx.send(Msg::LinkDownloadFailed {
                    job_id,
                    link_index,
                    error: reason,
                }) {
                    engine_warn!(
                        "Failed to report download failure for job_id={} link_index={}: {}",
                        job_id,
                        link_index,
                        err
                    );
                }
            }
            Effect::DeleteLinkedPage {
                job_id,
                link_index,
                path,
            } => {
                engine_warn!(
                    "DeleteLinkedPage rejected job_id={} link_index={} path={} reason={}",
                    job_id,
                    link_index,
                    path.display(),
                    reason
                );
                let _ = self.msg_tx.send(Msg::LinkDeleted { job_id, link_index });
            }
            other => {
                engine_warn!("Effect rejected but no handler for {:?}: {}", other, reason);
            }
        }
    }
}

fn map_llm_event(event: LlmEvent) -> Msg {
    match event {
        LlmEvent::Completed { request_id, result } => {
            let result_kind = match result {
                Ok(outcome) => LlmResultKind::Success {
                    output_json: outcome.output_json,
                    input_tokens: outcome.usage.input_tokens,
                    output_tokens: outcome.usage.output_tokens,
                },
                Err(LlmCompletionError::ValidationFailed {
                    reason,
                    raw_response,
                }) => LlmResultKind::ValidationFailed {
                    reason,
                    raw_response,
                },
                Err(LlmCompletionError::QuotaExhausted { description }) => {
                    LlmResultKind::QuotaExhausted {
                        reason: description,
                    }
                }
                Err(error) => LlmResultKind::Failed {
                    reason: llm_error_reason(error),
                },
            };
            Msg::LlmCompleted {
                request_id,
                result: result_kind,
            }
        }
    }
}

fn llm_error_reason(error: LlmCompletionError) -> String {
    match error {
        LlmCompletionError::ProviderError(err) => err.to_string(),
        LlmCompletionError::QuotaExhausted { description } => description,
        LlmCompletionError::PromptNotFound { prompt_id } => {
            format!("prompt {:?} not found", prompt_id)
        }
        LlmCompletionError::PersistenceFailed { detail } => {
            format!("replay persistence failed: {}", detail)
        }
        LlmCompletionError::InputTooLarge { size, limit } => {
            format!("input too large ({} > {})", size, limit)
        }
        LlmCompletionError::ValidationFailed { .. } => unreachable!(),
    }
}

fn download_link_page(
    url: &str,
    output_dir: &Path,
    url_policy: &UrlPolicy,
    fetch_settings: &FetchSettings,
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

    let redirect_limit = fetch_settings.redirect_limit;
    let redirect_counter = Arc::new(AtomicUsize::new(0));
    let policy = Policy::custom({
        let counter = redirect_counter.clone();
        move |attempt| {
            let count = attempt.previous().len();
            counter.store(count, Ordering::Relaxed);
            if count >= redirect_limit {
                attempt.error("redirect limit exceeded")
            } else {
                attempt.follow()
            }
        }
    });

    let client = Client::builder()
        .connect_timeout(fetch_settings.connect_timeout)
        .timeout(fetch_settings.request_timeout)
        .redirect(policy)
        .user_agent(fetch_settings.user_agent.clone())
        .build()
        .map_err(|err| err.to_string())?;
    let mut response = client
        .get(parsed.clone())
        .send()
        .map_err(|err| err.to_string())?;
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
        if !fetch_settings
            .allowed_content_types
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(ct))
        {
            return Err(format!("unsupported content type '{}'", ct));
        }
    }

    let final_url = response.url().to_string();
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = response.read(&mut buffer).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        let next_len = bytes.len() + read;
        if next_len as u64 > fetch_settings.max_bytes {
            return Err(format!(
                "response too large for linked page {} (limit {} bytes)",
                url, fetch_settings.max_bytes
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn download_link_page_rejects_disallowed_scheme_before_request() {
        let temp = tempdir().expect("tempdir");
        let fetch_settings = FetchSettings::default();
        let policy = UrlPolicy::default();
        let err = download_link_page("file:///etc/passwd", temp.path(), &policy, &fetch_settings)
            .unwrap_err();

        assert!(
            err.contains("url policy violation"),
            "expected url policy error, got '{}'",
            err
        );
    }

    fn runner_with_receiver() -> (EffectRunner, mpsc::Receiver<Msg>) {
        let (tx, rx) = mpsc::channel();
        (EffectRunner::new(tx), rx)
    }

    #[test]
    fn enqueue_url_effect_is_rejected_by_url_policy() {
        let (runner, rx) = runner_with_receiver();
        let job_id = 42;
        runner.enqueue(vec![Effect::EnqueueUrl {
            job_id,
            url: "file:///etc/passwd".to_string(),
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected job done msg");

        match msg {
            Msg::JobDone {
                job_id: received,
                result: JobResultKind::Failed { reason },
                ..
            } => {
                assert_eq!(received, job_id);
                assert!(reason.contains("url policy"));
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn download_link_page_effect_is_rejected_by_authorization() {
        let (runner, rx) = runner_with_receiver();
        let job_id = 7;
        let link_index = 1;
        runner.enqueue(vec![Effect::DownloadLinkedPage {
            job_id,
            link_index,
            url: "file:///tmp/secret".to_string(),
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected link download failed msg");

        match msg {
            Msg::LinkDownloadFailed {
                job_id: received,
                link_index: received_index,
                error,
            } => {
                assert_eq!(received, job_id);
                assert_eq!(received_index, link_index);
                assert!(error.contains("url policy"));
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[test]
    fn delete_linked_page_effect_is_rejected_on_unsafe_path() {
        let (runner, rx) = runner_with_receiver();
        let job_id = 11;
        let link_index = 3;
        runner.enqueue(vec![Effect::DeleteLinkedPage {
            job_id,
            link_index,
            path: PathBuf::from("../outside.md"),
        }]);

        let msg = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("expected link deleted msg");

        match msg {
            Msg::LinkDeleted {
                job_id: received,
                link_index: received_index,
            } => {
                assert_eq!(received, job_id);
                assert_eq!(received_index, link_index);
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }
}
