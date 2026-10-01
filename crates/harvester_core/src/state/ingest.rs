use super::{
    normalize_url_for_dedupe, AppState, Effect, IngestResult, JobId, JobOrigin, JobResultKind,
    JobState, SessionState, Stage,
};
use chrono::{DateTime, Utc};
use engine_logging::engine_info;
use harvester_engine::{ExtractedLink, ImportedArchiveRef};

impl AppState {
    pub(crate) fn pending_intake_urls(&self) -> &[String] {
        &self.pending_intake
    }

    pub(crate) fn restore_pending_intake(&mut self, urls: Vec<String>) {
        self.pending_intake.clear();
        self.add_pending_intake_urls(urls);
    }

    pub(crate) fn add_pending_intake_urls(
        &mut self,
        urls: impl IntoIterator<Item = String>,
    ) -> usize {
        let mut known: std::collections::HashSet<String> = self
            .pending_intake
            .iter()
            .map(|url| normalize_url_for_dedupe(url))
            .collect();
        let mut added = 0;
        for url in urls {
            let normalized = normalize_url_for_dedupe(&url);
            if normalized.is_empty() || !known.insert(normalized) {
                continue;
            }
            self.pending_intake.push(url);
            added += 1;
        }
        if added > 0 {
            self.dirty = true;
        }
        added
    }

    pub(crate) fn take_pending_intake_urls(&mut self) -> Vec<String> {
        let urls = std::mem::take(&mut self.pending_intake);
        if !urls.is_empty() {
            self.dirty = true;
        }
        urls
    }

    pub(crate) fn remove_pending_intake_url(&mut self, url: &str) {
        let normalized = normalize_url_for_dedupe(url);
        let before = self.pending_intake.len();
        self.pending_intake
            .retain(|pending| normalize_url_for_dedupe(pending) != normalized);
        if self.pending_intake.len() != before {
            self.dirty = true;
        }
    }

    pub(crate) fn job_url(&self, job_id: JobId) -> Option<&str> {
        self.jobs.get(&job_id).map(|job| job.url.as_str())
    }

    pub(crate) fn pending_intake_for_reingest(&mut self, urls: Vec<String>) -> Vec<String> {
        urls.into_iter()
            .filter(|url| {
                let normalized = normalize_url_for_dedupe(url);
                let jobs = self
                    .jobs
                    .values()
                    .filter(|job| normalize_url_for_dedupe(&job.url) == normalized);
                let mut has_job = false;
                let mut all_cancelled = true;
                let cancelled_reason = harvester_engine::FailureKind::Cancelled.to_string();
                for job in jobs {
                    has_job = true;
                    if !matches!(job.outcome.as_ref(),
                        Some(JobResultKind::Failed { reason }) if reason == &cancelled_reason)
                    {
                        all_cancelled = false;
                    }
                }
                if !has_job || all_cancelled {
                    self.seen_urls.remove(&normalized);
                    true
                } else {
                    false
                }
            })
            .collect()
    }

    pub(crate) fn preserve_unstarted_downloads_for_retry(&mut self) -> usize {
        let urls = self
            .jobs
            .values()
            .filter(|job| job.stage == Stage::Queued && job.outcome.is_none())
            .map(|job| job.url.clone())
            .collect::<Vec<_>>();
        self.add_pending_intake_urls(urls)
    }

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
                    archive_url_key: harvester_engine::archive_url_key(&entry.canonical_url),
                    stage: Stage::Done,
                    outcome: Some(JobResultKind::Success),
                    tokens: None,
                    bytes: None,
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
        let archive_url_key = harvester_engine::archive_url_key(&url);
        JobState {
            url,
            archive_url_key,
            stage: Stage::Queued,
            outcome: None,
            tokens: None,
            bytes: None,
            links: Vec::new(),
            origin,
            fetched_utc: None,
        }
    }

    fn has_seen_url(&self, normalized_url: &str) -> bool {
        self.seen_urls.contains(normalized_url)
    }

    pub(crate) fn apply_progress(
        &mut self,
        job_id: JobId,
        stage: Stage,
        tokens: Option<u32>,
        bytes: Option<u64>,
    ) {
        let mut token_changed = false;
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.stage = stage;
            if let Some(t) = tokens {
                if job.tokens != Some(t) {
                    job.tokens = Some(t);
                    token_changed = true;
                }
            }
            if let Some(b) = bytes {
                job.bytes = Some(b);
            }
            self.dirty = true;
        }
        if token_changed {
            self.record_archive_job_tokens(job_id);
        }
    }

    pub(crate) fn apply_done(
        &mut self,
        job_id: JobId,
        result: JobResultKind,
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
                job.attach_extracted_links(extracted_links);
            } else {
                job.clear_links();
            }
            true
        } else {
            false
        };
        if job_updated {
            self.clear_settled_poll_pipeline_if_complete();
            self.dirty = true;
        }
    }
}
