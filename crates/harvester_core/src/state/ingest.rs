use super::indirect_links::{should_collect_indirect_link, IndirectLink};
use super::{
    normalize_url_for_dedupe, AppState, Effect, IngestResult, JobId, JobOrigin, JobResultKind,
    JobState, LinkKind, PreviewState, SessionState, Stage,
};
use chrono::{DateTime, Utc};
use engine_logging::engine_info;
use harvester_engine::{ExtractedLink, ImportedArchiveRef};

impl AppState {
    pub(crate) fn apply_imported_archive_entries(&mut self, entries: &[ImportedArchiveRef]) {
        if entries.is_empty() {
            return;
        }

        for entry in entries {
            let restored_fetched_utc = chrono::DateTime::parse_from_rfc3339(&entry.fetched_utc)
                .ok()
                .map(|dt| dt.with_timezone(&chrono::Utc));
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            self.jobs.insert(
                job_id,
                JobState {
                    url: entry.canonical_url.clone(),
                    stage: Stage::Done,
                    outcome: Some(JobResultKind::Success),
                    tokens: None,
                    bytes: None,
                    content_preview: None,
                    preview_quality: None,
                    links: Vec::new(),
                    origin: JobOrigin::Direct,
                    fetched_utc: restored_fetched_utc,
                },
            );
            self.seen_urls
                .insert(normalize_url_for_dedupe(&entry.canonical_url));
        }

        self.dirty = true;
    }

    pub(crate) fn enqueue_jobs_from_ui(&mut self) -> Vec<(JobId, String)> {
        let mut enqueued = Vec::new();
        for url in self.ui.urls.iter() {
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            self.jobs.insert(
                job_id,
                Self::build_job_state(url.clone(), JobOrigin::Direct),
            );
            enqueued.push((job_id, url.clone()));
        }
        self.ui.urls.clear();
        self.dirty = true;
        enqueued
    }

    pub(crate) fn ingest_urls(&mut self, urls: Vec<String>, now: DateTime<Utc>) -> IngestResult {
        let mut unique = Vec::new();
        let mut skipped = 0;
        for url in urls {
            let normalized = normalize_url_for_dedupe(&url);
            if self.has_seen_url(&normalized) {
                skipped += 1;
            } else if self.blacklist.is_url_blocked(&url, now) {
                let domain = harvester_engine::registrable_domain(&url)
                    .unwrap_or_else(|| "<unknown>".to_string());
                engine_info!(
                    "[blacklist] skipping blacklisted domain={} url={}",
                    domain,
                    url
                );
                skipped += 1;
            } else {
                self.seen_urls.insert(normalized);
                unique.push(url);
            }
        }

        if unique.is_empty() {
            return IngestResult {
                effects: Vec::new(),
                enqueued: 0,
                skipped,
                enqueued_job_ids: Vec::new(),
            };
        }

        let should_start = self.session() == SessionState::Idle;
        if should_start {
            self.start_session();
        }

        self.set_urls(unique);
        let enqueued = self.enqueue_jobs_from_ui();
        let enqueued_count = enqueued.len();
        let mut effects = Vec::with_capacity(enqueued.len() + usize::from(should_start));
        if should_start {
            effects.push(Effect::StartSession);
        }
        let enqueued_job_ids = enqueued.iter().map(|(job_id, _)| *job_id).collect();
        for (job_id, url) in enqueued {
            effects.push(Effect::EnqueueUrl { job_id, url });
        }

        IngestResult {
            effects,
            enqueued: enqueued_count,
            skipped,
            enqueued_job_ids,
        }
    }

    fn build_job_state(url: String, origin: JobOrigin) -> JobState {
        JobState {
            url,
            stage: Stage::Queued,
            outcome: None,
            tokens: None,
            bytes: None,
            content_preview: None,
            preview_quality: None,
            links: Vec::new(),
            origin,
            fetched_utc: None,
        }
    }

    fn has_seen_url(&self, normalized_url: &str) -> bool {
        self.seen_urls.contains(normalized_url)
    }

    pub(super) fn collect_indirect_links_from_job(&mut self, job_id: JobId) {
        let job = match self.jobs.get(&job_id) {
            Some(job) => job,
            None => return,
        };
        if job.origin != JobOrigin::Direct {
            return;
        }
        for link in &job.links {
            if link.kind != LinkKind::Hyperlink {
                continue;
            }
            if !should_collect_indirect_link(&job.url, &link.url) {
                continue;
            }
            let normalized = normalize_url_for_dedupe(&link.url);
            if normalized.is_empty() || self.has_seen_url(&normalized) {
                continue;
            }
            self.indirect_link_pool.add_link(IndirectLink {
                url: link.url.clone(),
                source_job_id: job_id,
            });
        }
    }

    pub(crate) fn ingest_indirect_links(
        &mut self,
        links: Vec<IndirectLink>,
        now: DateTime<Utc>,
    ) -> IngestResult {
        let mut unique = Vec::new();
        let mut skipped = 0;
        for link in links {
            let normalized = normalize_url_for_dedupe(&link.url);
            if normalized.is_empty() {
                continue;
            }
            if self.has_seen_url(&normalized) {
                skipped += 1;
                continue;
            }
            if self.blacklist.is_url_blocked(&link.url, now) {
                let domain = harvester_engine::registrable_domain(&link.url)
                    .unwrap_or_else(|| "<unknown>".to_string());
                engine_info!(
                    "[blacklist] skipping blacklisted domain={} url={}",
                    domain,
                    link.url
                );
                skipped += 1;
                continue;
            }
            self.seen_urls.insert(normalized);
            unique.push(link);
        }

        if unique.is_empty() {
            return IngestResult {
                effects: Vec::new(),
                enqueued: 0,
                skipped,
                enqueued_job_ids: Vec::new(),
            };
        }

        let should_start = self.session() == SessionState::Idle;
        if should_start {
            self.start_session();
        }

        let mut effects = Vec::with_capacity(unique.len() + usize::from(should_start));
        if should_start {
            effects.push(Effect::StartSession);
        }
        let mut enqueued = 0;
        let mut enqueued_job_ids = Vec::new();
        for link in unique {
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            enqueued_job_ids.push(job_id);
            self.jobs.insert(
                job_id,
                Self::build_job_state(
                    link.url.clone(),
                    JobOrigin::Indirect {
                        source_job_id: link.source_job_id,
                    },
                ),
            );
            effects.push(Effect::EnqueueUrl {
                job_id,
                url: link.url,
            });
            enqueued += 1;
        }

        IngestResult {
            effects,
            enqueued,
            skipped,
            enqueued_job_ids,
        }
    }

    pub(crate) fn begin_indirect_link_generation(&mut self) {
        self.indirect_link_pool.begin_new_generation();
    }

    pub(crate) fn drain_indirect_links(&mut self) -> Vec<IndirectLink> {
        self.indirect_link_pool.draining_links()
    }

    pub(crate) fn set_indirect_poll_in_progress(&mut self, value: bool) {
        self.indirect_poll_in_progress = value;
    }

    pub(crate) fn indirect_poll_in_progress(&self) -> bool {
        self.indirect_poll_in_progress
    }

    pub(crate) fn has_indirect_links(&self) -> bool {
        !self.indirect_link_pool.is_empty()
    }

    pub(crate) fn apply_progress(
        &mut self,
        job_id: JobId,
        stage: Stage,
        tokens: Option<u32>,
        bytes: Option<u64>,
        content_preview: Option<String>,
    ) {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.stage = stage;
            if let Some(t) = tokens {
                if job.tokens != Some(t) {
                    let previous = job.tokens.unwrap_or(0) as u64;
                    self.metrics.total_tokens = self
                        .metrics
                        .total_tokens
                        .saturating_sub(previous)
                        .saturating_add(t as u64);
                    job.tokens = Some(t);
                }
            }
            if let Some(b) = bytes {
                job.bytes = Some(b);
            }
            if let Some(content) = content_preview {
                let selected = self.ui.selected_job_id() == Some(job_id);
                if selected {
                    self.ui.set_preview_state(PreviewState::InProgress {
                        job_id,
                        content: content.clone(),
                    });
                }
                job.set_preview_content(content);
            }
            self.dirty = true;
        }
    }

    pub(crate) fn apply_done(
        &mut self,
        job_id: JobId,
        result: JobResultKind,
        content_preview: Option<String>,
        extracted_links: Vec<ExtractedLink>,
        msg_fetched_utc: Option<String>,
    ) {
        let job_updated = if let Some(job) = self.jobs.get_mut(&job_id) {
            job.stage = Stage::Done;
            job.outcome = Some(result);
            job.fetched_utc = msg_fetched_utc
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));
            if matches!(job.outcome.as_ref(), Some(JobResultKind::Success)) {
                if let Some(content) = content_preview {
                    job.set_preview_content(content);
                }
                job.attach_extracted_links(extracted_links);
                self.collect_indirect_links_from_job(job_id);
            } else {
                job.clear_preview_content();
                job.clear_links();
            }
            true
        } else {
            false
        };
        if job_updated && self.ui.selected_job_id() == Some(job_id) {
            self.refresh_selected_preview();
        }
        if job_updated {
            self.clear_settled_poll_pipeline_if_complete();
            self.dirty = true;
        }
    }
}
