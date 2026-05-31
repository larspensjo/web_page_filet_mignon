use super::{
    map_job_filter_status, normalize_url_for_dedupe, AppState, CompletedJobSnapshot, JobId,
    JobOrigin, JobResultKind, JobState, LinkDownloadState, LinkRecord, LinkSnapshotRecord,
    MetricsState, PreviewMode, SessionState, SourceStateIndex, Stage,
};
use crate::pre_triage_filter::PreTriagePhase;
use crate::preview::{self, PreviewContentKind};
use crate::triage::{ArticleTriageResult, TriageSession};
use crate::url_age::AgeEstimate;
use crate::view_model::JobFilterStatus;
use harvester_engine::ExtractedLink;
use harvester_engine::LinkKind;
use std::path::PathBuf;

impl AppState {
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
                        downloaded_path: match &link.download_state {
                            LinkDownloadState::Downloaded { path } => {
                                Some(path.to_string_lossy().to_string())
                            }
                            _ => None,
                        },
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
        self.seen_urls.clear();
        self.metrics = MetricsState::default();
        self.ui.urls.clear();
        self.ui.clear_preview();
        self.ui.clear_input_buffer();
        self.last_paste_stats = None;
        self.next_job_id = 1;
        self.reset_llm_requests();
        self.pre_triage = crate::pre_triage_filter::PreTriageSession::default();
        self.pre_triage_load_context = None;
        self.pre_triage_load_progress = None;
        self.pre_triage_manual_overrides.clear();

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
                    stage: Stage::Done,
                    outcome: Some(JobResultKind::Success),
                    tokens,
                    bytes,
                    content_preview: None,
                    preview_quality: None,
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
                job.apply_link_snapshots(&link_snapshots);
            }
            let normalized = normalize_url_for_dedupe(&url);
            self.seen_urls.insert(normalized);
            if let Some(tokens) = tokens {
                self.metrics.total_tokens = self.metrics.total_tokens.saturating_add(tokens as u64);
            }
        }

        self.metrics.total_urls = self.jobs.len();
        self.session = SessionState::Idle;
        self.dirty = true;
        self.briefing = crate::briefing::BriefingSession::default();
        self.triage = TriageSession::default();
        self.source_states = SourceStateIndex::default();
    }

    pub(crate) fn revert_preview_to_briefing(&mut self) {
        self.ui.set_preview_mode(PreviewMode::Briefing);
        self.dirty = true;
    }

    /// Resolve the best available preview content for a given URL.
    ///
    /// Follows strict priority order:
    /// 1. Summary (if available)
    /// 2. Triage result (if summary missing but triage completed)
    /// 3. Exclusion reasons (if filtered/excluded in pre-triage)
    /// 4. Fallback message (if nothing else available)
    ///
    /// Returns (PreviewContentKind, formatted content).
    pub(super) fn resolve_best_preview(&self, url: &str) -> (PreviewContentKind, String) {
        if let Some(summary) = self.summary_result_for_url(url) {
            return (
                PreviewContentKind::Summary,
                preview::format_summary_for_preview(summary),
            );
        }

        if let Some(triage_result) = self.triage.result_for_url(url) {
            let title =
                preview::best_effort_article_title(self.triage.source_title_for_url(url), url);
            return (
                PreviewContentKind::Triage,
                preview::format_triage_for_preview(title.as_deref(), triage_result),
            );
        }

        if let Some(entry) = self.pre_triage.entry_for_url(url) {
            use crate::pre_triage_filter::{AutoVerdict, ManualDecision};
            let is_excluded = matches!(
                (entry.auto_verdict, entry.manual_decision),
                (AutoVerdict::HardExclude, None)
                    | (AutoVerdict::Review, None)
                    | (_, Some(ManualDecision::Exclude))
            );
            if is_excluded {
                return (
                    PreviewContentKind::Exclusion,
                    preview::format_exclusion_for_preview(entry),
                );
            }
        }

        (
            PreviewContentKind::Fallback,
            preview::format_fallback_preview(),
        )
    }

    /// Refresh the preview for the currently selected job, if any.
    ///
    /// Re-runs resolve_best_preview and updates the UI state if the content changed.
    /// This is called after triage/summary completion to ensure the preview stays current.
    pub(crate) fn refresh_selected_preview(&mut self) {
        let Some(selected_job_id) = self.ui.selected_job_id() else {
            return;
        };
        let Some(job) = self.jobs.get(&selected_job_id) else {
            return;
        };

        let (kind, content) = self.resolve_best_preview(&job.url);
        let changed = self.ui.select_job(selected_job_id, Some((&content, kind)));
        if changed {
            engine_logging::engine_info!(
                "[preview] Preview upgraded for job {} (url={})",
                selected_job_id,
                job.url
            );
            self.dirty = true;
        }
    }

    pub(crate) fn select_job(&mut self, job_id: JobId) {
        let Some(job) = self.jobs.get(&job_id) else {
            return;
        };

        let (kind, content) = self.resolve_best_preview(&job.url);

        let changed = self.ui.select_job(job_id, Some((&content, kind)));
        if changed {
            self.ui.set_preview_mode(PreviewMode::SelectedJob);
            self.dirty = true;
        } else {
            self.ui.set_preview_mode(PreviewMode::SelectedJob);
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

    pub(crate) fn selected_job_has_summary(&self) -> bool {
        self.ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id))
            .and_then(|job| self.summary_result_for_url(&job.url))
            .is_some()
    }

    /// URL of the currently selected job, regardless of summarization state.
    pub(crate) fn selected_job_url(&self) -> Option<String> {
        let job_id = self.ui.selected_job_id()?;
        let job = self.jobs.get(&job_id)?;
        Some(job.url.clone())
    }

    pub fn job_url_for(&self, job_id: JobId) -> Option<&str> {
        self.jobs.get(&job_id).map(|job| job.url.as_str())
    }

    pub(crate) fn link_metadata(
        &self,
        job_id: JobId,
        link_index: u32,
    ) -> Option<(String, Option<PathBuf>)> {
        self.jobs.get(&job_id).and_then(|job| {
            job.links
                .iter()
                .find(|record| record.index == link_index)
                .map(|record| {
                    (
                        record.url.clone(),
                        match &record.download_state {
                            LinkDownloadState::Downloaded { path } => Some(path.clone()),
                            _ => None,
                        },
                    )
                })
        })
    }

    pub fn link_state(&self, job_id: JobId, link_index: u32) -> Option<(LinkDownloadState, bool)> {
        self.jobs.get(&job_id).and_then(|job| {
            job.links
                .iter()
                .find(|record| record.index == link_index)
                .map(|record| (record.download_state.clone(), record.age_estimate.is_some()))
        })
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

    pub fn set_link_age_estimate(
        &mut self,
        job_id: JobId,
        link_index: u32,
        estimate: Option<AgeEstimate>,
    ) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            if let Some(record) = job
                .links
                .iter_mut()
                .find(|record| record.index == link_index)
            {
                record.age_estimate = estimate;
                self.dirty = true;
                return true;
            }
        }
        false
    }

    pub(crate) fn mark_link_download_requested(&mut self, job_id: JobId, link_index: u32) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_download_requested(link_index);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_link_download_completed(
        &mut self,
        job_id: JobId,
        link_index: u32,
        path: PathBuf,
    ) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_download_completed(link_index, path);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_link_download_failed(
        &mut self,
        job_id: JobId,
        link_index: u32,
        error: String,
    ) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_download_failed(link_index, error);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_link_deleted(&mut self, job_id: JobId, link_index: u32) -> bool {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.mark_link_deleted(link_index);
            self.dirty = true;
            true
        } else {
            false
        }
    }
}
