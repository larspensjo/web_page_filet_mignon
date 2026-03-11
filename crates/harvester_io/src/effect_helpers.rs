use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::{error::Error as StdError, fmt};

use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{LlmResultKind, Msg, Stage};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::types::{ModelId, ProviderKind};
use harvester_engine::llm::{LlmCompletionError, LlmEvent, LlmRunMetadata};
use harvester_engine::{
    build_markdown_document, decode_html, deterministic_filename, ensure_output_dir,
    poll_rss_source, AtomicFileWriter, DecodeError, ExtractionPipeline, ExtractionPolicy,
    FetchSettings, RssSeenSet, SourceId, UrlPolicy, WhitespaceTokenCounter,
};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::redirect::Policy;
use url::Url;

pub(crate) const MAX_FEED_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const FEED_ACCEPT_HEADER: &str =
    "application/rss+xml, application/atom+xml, application/feed+json, application/json, application/xml, text/xml";

pub fn build_local_model_catalog(
    provider_kind: Option<ProviderKind>,
    effective_models: &HashMap<PromptId, String>,
) -> Vec<ModelId> {
    let Some(provider_kind) = provider_kind else {
        return Vec::new();
    };

    let mut names: Vec<String> = effective_models.values().cloned().collect();
    names.sort();
    names.dedup();

    names
        .into_iter()
        .map(|name| ModelId::new(provider_kind, name))
        .collect()
}

pub fn prompt_context_filename(prompt_id: PromptId) -> &'static str {
    match prompt_id {
        PromptId::ArticleTriage => "article_triage.toml",
        PromptId::ArticleSummary => "article_summary.toml",
        PromptId::AggregateBriefing => "aggregate_briefing.toml",
    }
}

pub struct RssPollContext<'a> {
    pub seen_set: &'a mut RssSeenSet,
    pub seen_set_path: &'a Path,
    pub msg_tx: &'a mpsc::Sender<Msg>,
}

pub fn handle_rss_source_poll(
    source_id: &SourceId,
    feed_url: &str,
    url_policy: &UrlPolicy,
    fetch_settings: &FetchSettings,
    context: &mut RssPollContext,
    max_urls_per_poll: Option<usize>,
) {
    let bytes = match fetch_feed(feed_url, url_policy, fetch_settings) {
        Ok(bytes) => bytes,
        Err(reason) => {
            engine_warn!("[rss-poll] fetch failed for {}: {}", source_id, reason);
            let _ = context.msg_tx.send(Msg::SourcePollFailed {
                source_id: source_id.clone(),
                error: reason,
            });
            return;
        }
    };
    let result = match poll_rss_source(
        source_id.clone(),
        &bytes,
        feed_url,
        context.seen_set,
        max_urls_per_poll,
    ) {
        Ok(result) => result,
        Err(err) => {
            engine_warn!("[rss-poll] {} failed: {}", source_id, err);
            let _ = context.msg_tx.send(Msg::SourcePollFailed {
                source_id: source_id.clone(),
                error: err.to_string(),
            });
            return;
        }
    };

    // Persist seen set after successful poll
    if let Err(err) = crate::persist_seen_set(context.seen_set, context.seen_set_path) {
        engine_warn!(
            "[rss-poll] failed to persist seen set for {}: {}",
            source_id,
            err
        );
    }

    engine_info!(
        "[rss-poll] {} => {} new URL(s)",
        source_id,
        result.urls.len()
    );
    let _ = context.msg_tx.send(Msg::SourcePollCompleted {
        source_id: source_id.clone(),
        urls: result.urls,
    });
}

pub struct PollGuard {
    msg_tx: mpsc::Sender<Msg>,
}

impl PollGuard {
    pub fn new(msg_tx: mpsc::Sender<Msg>) -> Self {
        Self { msg_tx }
    }
}

impl Drop for PollGuard {
    fn drop(&mut self) {
        let _ = self.msg_tx.send(Msg::AllSourcesPollEnded);
    }
}

#[derive(Debug)]
struct RedirectPolicyError(String);

impl fmt::Display for RedirectPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl StdError for RedirectPolicyError {}

pub fn fetch_feed(
    feed_url: &str,
    url_policy: &UrlPolicy,
    fetch_settings: &FetchSettings,
) -> Result<Vec<u8>, String> {
    let parsed =
        Url::parse(feed_url).map_err(|err| format!("invalid feed url {}: {}", feed_url, err))?;
    if let Err(violation) = url_policy.check(&parsed) {
        return Err(format!(
            "feed url {} violates policy: {}",
            feed_url, violation
        ));
    }

    let redirect_limit = fetch_settings.redirect_limit;
    let policy = Policy::custom({
        let url_policy = url_policy.clone();
        move |attempt| {
            let count = attempt.previous().len();
            if count >= redirect_limit {
                attempt.error("redirect limit exceeded")
            } else {
                let target_url = attempt.url().clone();
                if let Err(violation) = url_policy.check(&target_url) {
                    attempt.error(RedirectPolicyError(format!(
                        "redirect target {} violated policy: {}",
                        target_url, violation
                    )))
                } else {
                    attempt.follow()
                }
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
        .header(ACCEPT, FEED_ACCEPT_HEADER)
        .send()
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP error {} for {}", response.status(), feed_url));
    }

    if let Some(len) = response.content_length() {
        if len > MAX_FEED_RESPONSE_BYTES as u64 {
            return Err(format!(
                "feed {} too large: {} bytes",
                feed_url, MAX_FEED_RESPONSE_BYTES
            ));
        }
    }

    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut total = 0;
    loop {
        let read = response.read(&mut chunk).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        total += read;
        if total > MAX_FEED_RESPONSE_BYTES {
            return Err(format!(
                "feed {} exceeded {} bytes",
                feed_url, MAX_FEED_RESPONSE_BYTES
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    Ok(buffer)
}

pub fn download_link_page(
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

    let pipeline = ExtractionPipeline::new(ExtractionPolicy::default());
    let extracted_article = pipeline.extract(&decoded.html, Some(final_url.as_str()));
    let token_counter = WhitespaceTokenCounter;
    let fetched_utc = Utc::now().to_rfc3339();
    let (_tokens, doc) = build_markdown_document(
        &final_url,
        extracted_article.title.as_deref(),
        &decoded.encoding_label,
        &fetched_utc,
        &extracted_article.markdown,
        &token_counter,
    );

    let filename = deterministic_filename(extracted_article.title.as_deref(), &final_url);
    let writer = AtomicFileWriter::new(linked_dir);
    writer.write(&filename, &doc).map_err(|err| err.to_string())
}

pub fn map_stage(stage: harvester_engine::Stage) -> Stage {
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

pub fn map_llm_event(event: LlmEvent) -> Msg {
    match event {
        LlmEvent::Completed { request_id, result } => {
            let (result_kind, metadata) = match result {
                Ok(outcome) => {
                    let kind = LlmResultKind::Success {
                        output_json: outcome.output_json,
                        input_tokens: outcome.metadata.input_tokens,
                        output_tokens: outcome.metadata.output_tokens,
                        prompt_version: outcome.metadata.prompt_version,
                        model_id: outcome.metadata.resolved_model.clone(),
                    };
                    (kind, Some(outcome.metadata))
                }
                Err(LlmCompletionError::ValidationFailed {
                    reason,
                    raw_response,
                    failure_metadata,
                }) => (
                    LlmResultKind::ValidationFailed {
                        reason,
                        raw_response,
                    },
                    failure_metadata.map(LlmRunMetadata::from),
                ),
                Err(LlmCompletionError::QuotaExhausted {
                    description,
                    failure_metadata,
                }) => (
                    LlmResultKind::QuotaExhausted {
                        reason: description,
                    },
                    failure_metadata.map(LlmRunMetadata::from),
                ),
                Err(LlmCompletionError::PersistenceFailed {
                    detail,
                    failure_metadata,
                }) => (
                    LlmResultKind::Failed {
                        reason: format!("replay persistence failed: {}", detail),
                    },
                    failure_metadata.map(LlmRunMetadata::from),
                ),
                Err(error) => (
                    LlmResultKind::Failed {
                        reason: llm_error_reason(error),
                    },
                    None,
                ),
            };
            Msg::LlmCompleted {
                request_id,
                result: result_kind,
                metadata,
            }
        }
    }
}

fn llm_error_reason(error: LlmCompletionError) -> String {
    match error {
        LlmCompletionError::ProviderError(err) => err.to_string(),
        LlmCompletionError::QuotaExhausted { description, .. } => description,
        LlmCompletionError::PromptNotFound { prompt_id } => {
            format!("prompt {:?} not found", prompt_id)
        }
        LlmCompletionError::PersistenceFailed { detail, .. } => {
            format!("replay persistence failed: {}", detail)
        }
        LlmCompletionError::InputTooLarge { size, limit } => {
            format!("input too large ({} > {})", size, limit)
        }
        LlmCompletionError::TemplateRenderFailed { detail } => {
            format!("template rendering failed: {}", detail)
        }
        LlmCompletionError::UnsupportedModel { model, reason } => {
            format!(
                "unsupported model {:?}/{}: {}",
                model.provider(),
                model.model_name(),
                reason
            )
        }
        LlmCompletionError::ValidationFailed { .. } => unreachable!(),
    }
}
