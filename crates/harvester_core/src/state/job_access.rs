use super::{
    map_job_filter_status, normalize_url_for_dedupe, AppState, CompletedJobSnapshot, JobId,
    JobOrigin, JobResultKind, JobState, LinkRecord, LinkSnapshotRecord, MetricsState, SessionState,
    SourceStateIndex, Stage,
};
use crate::pre_triage_filter::PreTriagePhase;
use crate::triage::{ArticleTriageResult, TriageSession};
use crate::view_model::JobFilterStatus;
use harvester_engine::ExtractedLink;
use harvester_engine::LinkKind;

impl AppState {
    pub(crate) fn set_runtime_state_notice(&mut self, message: String) {
        self.runtime_state_notice = Some(message);
        self.dirty = true;
    }

    pub(crate) fn article_links_load_url(&self, job_id: JobId) -> Option<&str> {
        self.jobs
            .get(&job_id)
            .filter(|job| job.outcome == Some(JobResultKind::Success) && job.links.is_empty())
            .map(|job| job.url.as_str())
    }
    pub fn ordered_completed_job_urls_snapshot(&self) -> Vec<String> {
        self.jobs
            .values()
            .filter_map(|job| {
                if job.stage == Stage::Done && job.outcome == Some(JobResultKind::Success) {
                    Some(job.url.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn slim_completed_jobs_snapshot(&self) -> Vec<super::SlimJobRecord> {
        self.jobs
            .values()
            .filter(|job| job.outcome == Some(JobResultKind::Success))
            .map(|job| super::SlimJobRecord {
                url: job.url.clone(),
                tokens: job.tokens,
                bytes: job.bytes,
                fetched_utc: job.fetched_utc.map(|dt| dt.to_rfc3339()),
            })
            .collect()
    }

    pub(crate) fn article_links_loaded(
        &mut self,
        job_id: JobId,
        url: &str,
        links: Vec<ExtractedLink>,
    ) {
        if self.selected_job_id() != Some(job_id) {
            return;
        }
        if let Some(job) = self.jobs.get_mut(&job_id).filter(|job| {
            job.url == url
                && job.outcome == Some(JobResultKind::Success)
                && (job.links.is_empty() || !links.is_empty())
        }) {
            job.attach_extracted_links(links);
            self.dirty = true;
        }
    }

    pub fn completed_jobs_snapshot(&self) -> Vec<CompletedJobSnapshot> {
        self.jobs
            .values()
            .filter(|job| job.outcome == Some(JobResultKind::Success))
            .map(|job| CompletedJobSnapshot {
                url: job.url.clone(),
                tokens: job.tokens,
                bytes: job.bytes,
                links: job
                    .links
                    .iter()
                    .map(|link| LinkSnapshotRecord {
                        url: link.url.clone(),
                        downloaded_path: None,
                    })
                    .collect(),
                fetched_utc: job.fetched_utc.map(|dt| dt.to_rfc3339()),
            })
            .collect()
    }

    #[allow(dead_code)]
    pub fn job_links(&self, job_id: JobId) -> Option<&[LinkRecord]> {
        self.jobs.get(&job_id).map(|job| job.links())
    }

    /// Returns the completed triage result for a job, if that job has one.
    pub fn triage_result_for_job(&self, job_id: JobId) -> Option<&ArticleTriageResult> {
        self.jobs
            .get(&job_id)
            .and_then(|job| self.triage.result_for_url(&job.url))
    }

    pub(crate) fn restore_completed_jobs(&mut self, entries: Vec<CompletedJobSnapshot>) {
        if entries.is_empty() {
            return;
        }

        self.jobs.clear();
        self.archive_article_tokens = Default::default();
        self.seen_urls.clear();
        self.metrics = MetricsState::default();
        self.ui.urls.clear();
        self.ui.clear_selection();
        self.ui.clear_input_buffer();
        self.last_paste_stats = None;
        self.next_job_id = 1;
        self.reset_llm_requests();
        self.set_pre_triage(crate::pre_triage_filter::PreTriageSession::default());

        for entry in entries {
            let CompletedJobSnapshot {
                url,
                tokens,
                bytes,
                links: link_snapshots,
                fetched_utc: snapshot_fetched_utc,
            } = entry;
            let restored_fetched_utc = snapshot_fetched_utc
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));
            let job_id = self.next_job_id;
            self.next_job_id += 1;
            self.jobs.insert(
                job_id,
                JobState {
                    url: url.clone(),
                    archive_url_key: harvester_engine::archive_url_key(&url),
                    stage: Stage::Done,
                    outcome: Some(JobResultKind::Success),
                    tokens,
                    bytes,
                    links: Vec::new(),
                    origin: JobOrigin::Direct,
                    fetched_utc: restored_fetched_utc,
                },
            );
            let extracted_links: Vec<ExtractedLink> = link_snapshots
                .iter()
                .map(|record| ExtractedLink {
                    url: record.url.clone(),
                    text: None,
                    kind: LinkKind::Hyperlink,
                })
                .collect();
            if let Some(job) = self.jobs.get_mut(&job_id) {
                job.attach_extracted_links(extracted_links);
            }
            let normalized = normalize_url_for_dedupe(&url);
            self.seen_urls.insert(normalized);
        }

        self.rebuild_archive_job_tokens();

        self.metrics.total_urls = self.jobs.len();
        self.session = SessionState::Idle;
        self.dirty = true;
        self.set_briefing(crate::briefing::BriefingSession::default());
        self.set_triage(TriageSession::default());
        self.source_states = SourceStateIndex::default();
    }

    pub(crate) fn select_job(&mut self, job_id: JobId) {
        let Some(_) = self.jobs.get(&job_id) else {
            return;
        };

        let changed = self.ui.select_job(job_id);
        if changed {
            self.dirty = true;
        }
    }

    /// URL of the currently selected and summarized article.
    /// Returns None if no job is selected or if the selected job has no summary.
    pub fn selected_article_url(&self) -> Option<String> {
        let job_id = self.ui.selected_job_id()?;
        let job = self.jobs.get(&job_id)?;
        self.summary_result_for_url(&job.url)?;
        Some(job.url.clone())
    }

    pub fn selected_job_id(&self) -> Option<JobId> {
        self.ui.selected_job_id()
    }

    pub fn job_url_for(&self, job_id: JobId) -> Option<&str> {
        self.jobs.get(&job_id).map(|job| job.url.as_str())
    }

    pub(crate) fn job_url_pairs(&self) -> Vec<(JobId, String)> {
        self.jobs
            .iter()
            .map(|(job_id, job)| (*job_id, job.url.clone()))
            .collect()
    }

    pub(crate) fn job_extracted_link_url(&self, job_id: JobId, link_index: u32) -> Option<String> {
        self.jobs
            .get(&job_id)?
            .links
            .iter()
            .find(|link| link.index == link_index)
            .map(|link| link.url.clone())
    }

    pub fn job_filter_status(&self, job_id: JobId) -> Option<JobFilterStatus> {
        if !matches!(
            self.pre_triage.phase(),
            PreTriagePhase::Reviewing | PreTriagePhase::ReadyToTriage
        ) {
            return None;
        }

        let job = self.jobs.get(&job_id)?;
        self.pre_triage
            .entry_for_url(&job.url)
            .map(map_job_filter_status)
    }
}
