use super::batch::archive_token_estimates_from_parts;
use super::{
    domain_from_url, format_lab_briefing_markdown, format_lab_summary_markdown,
    format_lab_triage_markdown, map_job_filter_status, AppState, JobResultKind, JobState,
    PreviewMode, SessionState, Stage,
};
use crate::archive_display::ArchiveCoverage;
use crate::briefing::{ArticleSummaryResult, BriefingPhase};
use crate::pre_triage_filter::PreTriagePhase;
use crate::preview::format_summary_for_preview;
use crate::signal_candidate::{
    canonical_signal_key, is_signal_key_excluded, ScoredCandidate, SelectionPolicy,
    SignalCandidateSelection, SignalCandidateState,
};
use crate::tabs::{AppTab, JobListMode, JobListScope, LeftTab};
use crate::triage::{ArticleTriageState, TriagePhase};
use crate::view_model::{
    AppViewModel, DesktopJobListView, IndirectLinkPhase, IndirectLinkSummary, JobFilterStatus,
    JobListRowView, JobRowView, LayoutViewModel, LeftPaneHeaderView, OperationProgress,
    PreviewContextView, PreviewHeaderView, RightPaneView, ScoreBand, SelectedJobView,
    SelectedJobVisibility, SignalCandidateOutcome, SignalCandidatePreviewView, SignalCandidateRow,
    SignalCandidateRowState, TriageAnnotationView, DESKTOP_JOB_LIST_MAX_ROWS, TOKEN_LIMIT,
};
use harvester_engine::llm::dto::SourceTier;
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::normalize_url_for_dedupe;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

fn compare_desktop_job_rows(left: &JobListRowView, right: &JobListRowView) -> Ordering {
    match (&left.triage_annotation, &right.triage_annotation) {
        (Some(left_annotation), Some(right_annotation)) => right_annotation
            .priority
            .cmp(&left_annotation.priority)
            .then_with(|| left.job_id.cmp(&right.job_id)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.job_id.cmp(&right.job_id),
    }
}

impl AppState {
    pub fn view(&self) -> AppViewModel {
        self.build_view(true)
    }

    /// Builds the desktop-facing view without materializing frozen-renderer arrays.
    pub fn desktop_view(&self) -> AppViewModel {
        self.build_view(false)
    }

    fn build_view(&self, materialize_frozen_jobs: bool) -> AppViewModel {
        let since = self.briefing_since_utc();
        let summary_lookup = self.build_summary_lookup();
        let job_metadata = if materialize_frozen_jobs {
            self.build_job_view_metadata(since, &summary_lookup)
        } else {
            Vec::new()
        };
        let jobs = if materialize_frozen_jobs {
            job_metadata
                .iter()
                .map(|metadata| self.materialize_job_row(metadata))
                .collect()
        } else {
            Vec::new()
        };

        let selected_job_id = self.ui.selected_job_id();
        let selected_url = selected_job_id
            .and_then(|job_id| self.jobs.get(&job_id))
            .map(|job| job.url.clone());
        let signal_candidate_rows = self.build_signal_candidate_rows();
        let desktop_job_list = self.build_desktop_job_list_view(
            &signal_candidate_rows,
            self.jobs_search_query(),
            &summary_lookup,
        );
        let signal_candidate_preview =
            self.signal_candidate_preview_for_selected_job(selected_job_id);
        let briefing_preview = self.briefing.format_preview();
        let preview_text = match self.ui.preview_mode() {
            PreviewMode::SelectedJob => self.ui.preview_content().map(ToOwned::to_owned),
            PreviewMode::Briefing => briefing_preview
                .clone()
                .or_else(|| self.ui.preview_content().map(ToOwned::to_owned)),
        };
        let preview_header = self
            .ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id))
            .map(|job| {
                let quality = job.preview_quality.unwrap_or_default();
                PreviewHeaderView {
                    domain: domain_from_url(&job.url),
                    tokens: job.tokens,
                    bytes: job.bytes,
                    stage: job.stage,
                    outcome: job.outcome.clone(),
                    heading_count: quality.heading_count,
                    link_density: quality.link_density,
                    nav_heavy: quality.nav_heavy(),
                }
            });
        let jobs_search_query = self.jobs_search_query().to_string();
        let legacy_job_list = self.build_legacy_job_list_metrics(
            &jobs_search_query,
            &summary_lookup,
            selected_job_id,
            materialize_frozen_jobs,
        );
        let visible_jobs_after_filter = if materialize_frozen_jobs {
            legacy_job_list.visible_job_ids.clone()
        } else {
            Vec::new()
        };
        let first_visible_job_id = legacy_job_list.first_visible_job_id;
        let selected_jobs_visible_in_filter = legacy_job_list.selected_job_visible;
        let left_pane_header = build_left_pane_header_view(LeftPaneHeaderInputs {
            left_tab: self.left_tab,
            job_list_scope: self.job_list_scope,
            scoped_count: legacy_job_list.scoped_count,
            review_needed_count: legacy_job_list.review_needed_count,
            triage_result_count: legacy_job_list.triage_result_count,
            visible_job_count: legacy_job_list.visible_job_count,
            jobs_search_query: &jobs_search_query,
            ai_unavailable_message: self.ai_unavailable_message().as_deref(),
        });
        let preview_context = preview_header.as_ref().map(build_preview_context_view);
        let preview_header_text = match self.active_tab() {
            AppTab::Briefing => Some(self.format_briefing_preview_header()),
            AppTab::Trends => Some(self.format_trends_preview_header()),
            AppTab::PollStats => Some("Poll Stats | last poll".to_string()),
            AppTab::Triage | AppTab::Summary => None,
        };
        let selected_triage_article_available = self
            .ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id))
            .and_then(|job| {
                let selected_norm = normalize_url_for_dedupe(&job.url);
                self.triage()
                    .articles()
                    .iter()
                    .find(|article| {
                        normalize_url_for_dedupe(&article.url) == selected_norm
                            && matches!(article.triage_state, ArticleTriageState::Completed { .. })
                    })
                    .map(|_| ())
            })
            .is_some();
        let preview_source = self.ui.preview.content_kind();
        let operation_progress = self.build_operation_progress();
        let ai_warning_banner = self
            .ai_warning_banner()
            .or_else(|| self.provider_alert_banner());
        let ai_unavailable_message = self.ai_unavailable_message();
        let triage_blocked_reason = self.triage_blocked_reason();
        let briefing_blocked_reason = self.briefing_blocked_reason();
        let stop_finish_button = self.stop_finish_button_state();
        let archive_display = self.archive_display_counts();
        let full_filtered_count = archive_display.filtered_count();
        let mut archive_url_tokens = None;
        let archive_estimates = self.archive_token_estimates_for_view(
            archive_display.ordered_urls(),
            &mut archive_url_tokens,
            &summary_lookup,
        );

        let archive_partial_coverage = match archive_display.coverage() {
            ArchiveCoverage::CacheDerived {
                triaged,
                actionable_total,
            } if *triaged > 0 => Some(crate::ArchivePartialCoverageView {
                triaged: *triaged,
                actionable_total: *actionable_total,
            }),
            ArchiveCoverage::LiveComplete | ArchiveCoverage::CacheDerived { .. } => None,
        };

        // "raw" is a backlog indicator over the whole archive corpus: triaged articles
        // that do not yet have a cached summary. It is independent of which subset the
        // token meter is currently estimating.
        let raw_unprocessed_count = full_filtered_count - archive_estimates.summary_coverage;

        // The token meter bar and its "filtered" count reflect the archive export target.
        // When all signal candidates are settled and the selection is non-empty, the export
        // defaults to that subset — show those numbers on the bar instead of the full corpus.
        let sc = self.signal_candidate();
        let settled = sc.completed_count();
        let in_progress = sc
            .enqueued_count()
            .saturating_sub(settled)
            .saturating_sub(sc.failed_count());
        let (archive_token_estimate, archive_filtered_count) =
            if matches!(archive_display.coverage(), ArchiveCoverage::LiveComplete)
                && settled > 0
                && in_progress == 0
            {
                let scored: Vec<ScoredCandidate> = sc
                    .iter_completed()
                    .map(|(url, result)| ScoredCandidate {
                        url: url.to_string(),
                        result: result.clone(),
                    })
                    .collect();
                let policy = SelectionPolicy {
                    threshold: self.signal_candidate_threshold(),
                    active_prompt_version: self
                        .active_version_for(
                            harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate,
                        )
                        .unwrap_or_default(),
                    excluded: sc.excluded().clone(),
                };
                let selection = SignalCandidateSelection::compute(&scored, policy);
                if selection.selected_urls.is_empty() {
                    (archive_estimates.summary_tokens, full_filtered_count)
                } else {
                    let sc_estimates = self.archive_token_estimates_for_view(
                        &selection.selected_urls,
                        &mut archive_url_tokens,
                        &summary_lookup,
                    );
                    (sc_estimates.summary_tokens, selection.selected_urls.len())
                }
            } else {
                (archive_estimates.summary_tokens, full_filtered_count)
            };
        AppViewModel {
            workspace_view: self.workspace_view(),
            job_list_mode: self.job_list_mode(),
            reading_pane_mode: self.reading_pane_mode(),
            session: self.session,
            queued_urls: self.ui.urls.clone(),
            job_count: self.jobs.len(),
            jobs,
            desktop_job_list,
            last_paste_stats: self.last_paste_stats.clone(),
            dirty: self.dirty,
            total_tokens: self.metrics.total_tokens,
            token_limit: TOKEN_LIMIT,
            archive_token_estimate,
            archive_filtered_count,
            archive_partial_coverage,
            raw_unprocessed_count,
            preview_text,
            selected_job_id,
            left_pane_header,
            preview_header,
            preview_context,
            ai_warning_banner,
            preview_header_text,
            preview_source,
            briefing_generate_enabled: matches!(
                self.briefing_generate_readiness(),
                crate::state::BriefingGenerateReadiness::Ready { .. }
            ) && self.briefing.can_generate()
                && self.briefing_ai_available(),
            next_item_enabled: self.briefing.next_item_enabled() && self.briefing_ai_available(),
            summaries_can_start: self.summaries_can_start() && self.briefing_ai_available(),
            briefing_preview,
            stop_finish_button,
            triage_can_start: self.triage_ai_available()
                && self.triage.can_start()
                && self.can_start_triage_from_pre_triage(),
            triage_results_reorder_suppressed: self.triage_reorder_suppressed(),
            signal_candidate_rows,
            signal_candidate_preview,
            ai_unavailable_message,
            triage_blocked_reason,
            briefing_blocked_reason,
            operation_progress_visible: operation_progress.is_some(),
            operation_progress,
            run_progress: self
                .run_progress
                .as_ref()
                .map_or_else(Default::default, crate::RunProgress::view),
            run_completion_notice: self.run_completion_notice.clone(),
            poll_sources_enabled: matches!(
                self.session,
                SessionState::Idle | SessionState::Running
            ) && !self.source_states.is_poll_in_progress(),
            poll_indirect_links_enabled: !self.indirect_link_pool.is_empty()
                && !self.indirect_poll_in_progress(),
            checkpoint_status_message: self.briefing_checkpoint_status_message.clone(),
            left_panel_width: self.ui.left_panel_width(),
            input_panel_visible: self.ui.input_panel_visible(),
            window_width: self.ui.window_width(),
            selected_url,
            left_pane: crate::view_model::LeftPaneView {
                left_tab: self.left_tab,
                job_list_scope: self.job_list_scope,
                jobs_search_query,
                visible_jobs_after_filter,
                first_visible_job_id,
                selected_jobs_visible_in_filter,
                prompt_lab: crate::view_model::PromptLabView::from_state(
                    &self.prompt_lab,
                    &self.prompt_contexts,
                    &self.prompt_lab_templates,
                    selected_triage_article_available,
                ),
            },
            is_pre_triage_reviewing: self.pre_triage.is_interactive(),
            indirect_link_summary: self.build_indirect_link_summary(),
            llm_usage_by_model: self.llm_usage_rows(),
            llm_quota: crate::build_llm_quota_view(self.llm_quota()),
            right_pane: self.build_right_pane_view(selected_triage_article_available),
            blacklist: crate::view_model::BlacklistTabView::from_state(
                self.blacklist(),
                self.last_observed_utc()
                    .unwrap_or(chrono::DateTime::UNIX_EPOCH),
            ),
        }
    }

    fn build_job_view_metadata(
        &self,
        since: Option<chrono::DateTime<chrono::Utc>>,
        summary_lookup: &SummaryLookup,
    ) -> Vec<JobViewMetadata> {
        let show_filter_status = self.show_filter_status();
        self.jobs
            .iter()
            .map(|(job_id, job)| {
                self.enrich_job_view_metadata(
                    *job_id,
                    job,
                    since,
                    show_filter_status,
                    summary_lookup,
                )
            })
            .collect()
    }

    fn enrich_job_view_metadata(
        &self,
        job_id: crate::JobId,
        job: &JobState,
        since: Option<chrono::DateTime<chrono::Utc>>,
        show_filter_status: bool,
        summary_lookup: &SummaryLookup,
    ) -> JobViewMetadata {
        let is_since_checkpoint = is_since_checkpoint(job, since);
        let triage_annotation =
            self.triage
                .result_for_url(&job.url)
                .map(|result| TriageAnnotationView {
                    priority: result.priority,
                    category: result.category.clone(),
                    tags: result.tags.clone(),
                });
        let (has_summary, summary_title, summary_tokens) = summary_lookup
            .summary_for_job(self, job)
            .map(|summary| {
                (
                    true,
                    Some(summary.title.clone()),
                    Some(summary.output_tokens),
                )
            })
            .unwrap_or((false, None, None));
        let filter_status = show_filter_status
            .then(|| {
                self.pre_triage
                    .entry_for_url(&job.url)
                    .map(map_job_filter_status)
            })
            .flatten();
        let has_analysis = has_summary
            || triage_annotation.is_some()
            || matches!(
                filter_status,
                Some(JobFilterStatus::HardExcluded { .. })
                    | Some(JobFilterStatus::ReviewNeeded { .. })
                    | Some(JobFilterStatus::ManuallyExcluded)
            );
        JobViewMetadata {
            job_id,
            is_since_checkpoint,
            triage_annotation,
            has_summary,
            summary_title,
            summary_tokens,
            filter_status,
            has_analysis,
        }
    }

    fn materialize_job_row(&self, metadata: &JobViewMetadata) -> JobRowView {
        let job = self
            .jobs
            .get(&metadata.job_id)
            .expect("job metadata derives from the current state");
        let mut row = job.to_view(metadata.job_id, metadata.is_since_checkpoint);
        metadata.apply_to(&mut row);
        row
    }

    fn build_summary_lookup(&self) -> SummaryLookup<'_> {
        let mut lookup = SummaryLookup::default();
        for (url, summary) in self.briefing.completed_summaries() {
            lookup.briefing_by_url.entry(url).or_insert(summary);
        }
        for (key, entry) in self.summary_cache().iter() {
            if key.prompt_id != PromptId::ArticleSummary {
                continue;
            }
            let tie_break_key = (
                key.prompt_version,
                key.model_id.as_str(),
                key.context_hash.as_str(),
            );
            let replace = lookup
                .cache_by_content_hash
                .get(key.content_hash.as_str())
                .is_none_or(|cached| {
                    (entry.created_at_utc.as_str(), tie_break_key)
                        > (cached.created_at_utc, cached.tie_break_key)
                });
            if replace {
                lookup.cache_by_content_hash.insert(
                    key.content_hash.as_str(),
                    CachedSummary {
                        created_at_utc: entry.created_at_utc.as_str(),
                        tie_break_key,
                        summary: &entry.result,
                    },
                );
            }
        }
        lookup
    }

    fn archive_token_estimates_for_view(
        &self,
        urls: &[String],
        archive_url_tokens: &mut Option<HashMap<String, u64>>,
        summary_lookup: &SummaryLookup,
    ) -> crate::ArchiveTokenEstimates {
        if urls.is_empty() {
            return crate::ArchiveTokenEstimates::default();
        }
        let archive_url_tokens =
            archive_url_tokens.get_or_insert_with(|| self.archive_article_token_lookup());
        archive_token_estimates_from_parts(urls, archive_url_tokens, |url| {
            self.content_hash_for_url(url)
                .and_then(|content_hash| summary_lookup.summary_for_content_hash(content_hash))
                .map(|summary| summary.output_tokens)
        })
    }

    fn select_desktop_job_rows(
        &self,
        query_lower: &str,
        summary_lookup: &SummaryLookup,
    ) -> DesktopJobSelection {
        if self.job_list_mode() == JobListMode::Results {
            return DesktopJobSelection::default();
        }
        let since = self.briefing_since_utc();
        let mut selection = DesktopJobSelection {
            hidden_without_fetch_time: if since.is_some() {
                self.jobs
                    .values()
                    .filter(|job| job.fetched_utc.is_none())
                    .count()
            } else {
                0
            },
            ..Default::default()
        };
        for (job_id, job) in &self.jobs {
            let is_since_checkpoint = is_since_checkpoint(job, since);
            if !is_since_checkpoint {
                continue;
            }
            selection.scoped_ids.insert(*job_id);
            if !job_matches_search_query(
                &job.url,
                summary_lookup
                    .summary_for_job(self, job)
                    .map(|summary| summary.title.as_str()),
                query_lower,
            ) {
                continue;
            }
            selection.searched_ids.insert(*job_id);
            selection.emitted.push(DesktopJobSelectionRow {
                job_id: *job_id,
                fetched_utc: job.fetched_utc,
            });
        }
        selection.searched_count = selection.emitted.len();
        if selection.emitted.len() > DESKTOP_JOB_LIST_MAX_ROWS {
            selection.emitted.sort_unstable_by(|left, right| {
                fetched_descending(left.fetched_utc, right.fetched_utc)
                    .then_with(|| right.job_id.cmp(&left.job_id))
            });
            selection.emitted.truncate(DESKTOP_JOB_LIST_MAX_ROWS);
            selection.emitted.sort_unstable_by_key(|row| row.job_id);
        }
        selection.emitted_ids = selection.emitted.iter().map(|row| row.job_id).collect();
        selection
    }

    fn build_legacy_job_list_metrics(
        &self,
        query: &str,
        summary_lookup: &SummaryLookup,
        selected_job_id: Option<crate::JobId>,
        materialize_visible_job_ids: bool,
    ) -> LegacyJobListMetrics {
        let since = self.briefing_since_utc();
        let show_filter_status = self.show_filter_status();
        let query_lower = query.to_lowercase();
        let mut metrics = LegacyJobListMetrics::default();
        for (job_id, job) in &self.jobs {
            if self.job_list_scope == JobListScope::SinceCheckpoint
                && !is_since_checkpoint(job, since)
            {
                continue;
            }
            metrics.scoped_count += 1;
            if show_filter_status
                && matches!(
                    self.pre_triage
                        .entry_for_url(&job.url)
                        .map(map_job_filter_status),
                    Some(JobFilterStatus::ReviewNeeded { .. })
                )
            {
                metrics.review_needed_count += 1;
            }
            if self.triage.result_for_url(&job.url).is_some() {
                metrics.triage_result_count += 1;
            }
            if self.left_tab == LeftTab::Jobs
                && job_matches_search_query(
                    &job.url,
                    summary_lookup
                        .summary_for_job(self, job)
                        .map(|summary| summary.title.as_str()),
                    &query_lower,
                )
            {
                metrics.visible_job_count += 1;
                metrics.first_visible_job_id.get_or_insert(*job_id);
                metrics.selected_job_visible |= Some(*job_id) == selected_job_id;
                if materialize_visible_job_ids {
                    metrics.visible_job_ids.push(*job_id);
                }
            }
        }
        metrics
    }

    fn show_filter_status(&self) -> bool {
        matches!(
            self.pre_triage.phase(),
            PreTriagePhase::Reviewing | PreTriagePhase::ReadyToTriage
        )
    }

    fn triage_reorder_suppressed(&self) -> bool {
        matches!(self.triage.phase(), TriagePhase::Triaging)
    }

    fn build_desktop_job_list_view(
        &self,
        signal_candidate_rows: &[SignalCandidateRow],
        query: &str,
        summary_lookup: &SummaryLookup,
    ) -> DesktopJobListView {
        let mode = self.job_list_mode();
        let query = query.to_string();
        let query_lower = query.to_lowercase();
        let selection = self.select_desktop_job_rows(&query_lower, summary_lookup);
        let mut rows = selection
            .emitted
            .iter()
            .map(|selected| {
                let job = self
                    .jobs
                    .get(&selected.job_id)
                    .expect("selection derives from the current state");
                let metadata = self.enrich_job_view_metadata(
                    selected.job_id,
                    job,
                    self.briefing_since_utc(),
                    self.show_filter_status(),
                    summary_lookup,
                );
                JobListRowView::from_row(&self.materialize_job_row(&metadata), selected.fetched_utc)
            })
            .collect::<Vec<_>>();
        if !self.triage_reorder_suppressed() {
            rows.sort_unstable_by(compare_desktop_job_rows);
        }
        let scoped_count = selection.searched_count;
        let visible_count = rows.len();
        let selected_job = self.ui.selected_job_id().and_then(|selected_job_id| {
            let job = self.jobs.get(&selected_job_id)?;
            let list_visibility = match mode {
                JobListMode::Results => {
                    if signal_candidate_rows
                        .iter()
                        .any(|candidate| candidate.job_id == selected_job_id)
                    {
                        SelectedJobVisibility::Visible
                    } else {
                        SelectedJobVisibility::OutsideScope
                    }
                }
                JobListMode::SinceCheckpoint => {
                    if !selection.scoped_ids.contains(&selected_job_id) {
                        SelectedJobVisibility::OutsideScope
                    } else if !selection.searched_ids.contains(&selected_job_id) {
                        SelectedJobVisibility::QueryMismatch
                    } else if !selection.emitted_ids.contains(&selected_job_id) {
                        SelectedJobVisibility::Capped
                    } else {
                        SelectedJobVisibility::Visible
                    }
                }
            };
            let metadata = self.enrich_job_view_metadata(
                selected_job_id,
                job,
                self.briefing_since_utc(),
                self.show_filter_status(),
                summary_lookup,
            );
            Some(SelectedJobView::from_row(
                &self.materialize_job_row(&metadata),
                job.fetched_utc,
                list_visibility,
            ))
        });

        DesktopJobListView {
            mode,
            query,
            rows,
            selected_job,
            scoped_count,
            visible_count,
            truncated: scoped_count > visible_count,
            hidden_without_fetch_time: selection.hidden_without_fetch_time,
        }
    }

    fn build_operation_progress(&self) -> Option<OperationProgress> {
        if let Some((completed, total)) = self.source_states.poll_progress() {
            return Some(OperationProgress {
                label: "Scanning sources".to_string(),
                completed: completed as u32,
                total: total as u32,
            });
        }

        if matches!(self.triage.phase(), TriagePhase::Triaging) {
            let completed = self.triage.completed_count() + self.triage.failed_count();
            return Some(OperationProgress {
                label: "Triaging".to_string(),
                completed: completed as u32,
                total: self.triage.total() as u32,
            });
        }

        if matches!(self.briefing.phase(), BriefingPhase::Summarizing) {
            let completed =
                self.briefing.completed_summary_count() + self.briefing.failed_summary_count();
            return Some(OperationProgress {
                label: "Summarizing".to_string(),
                completed: completed as u32,
                total: self.briefing.total() as u32,
            });
        }

        {
            let session = &self.signal_candidate;
            let completed = session.completed_count() + session.failed_count();
            let total = session.enqueued_count();
            if total > completed {
                return Some(OperationProgress {
                    label: "Scoring signals".to_string(),
                    completed,
                    total,
                });
            }
        }

        if let Some((completed, total)) = self.poll_pipeline_article_progress() {
            return Some(OperationProgress {
                label: "Downloading articles".to_string(),
                completed: completed as u32,
                total: total as u32,
            });
        }

        if matches!(self.pre_triage.phase(), PreTriagePhase::LoadingArticles) {
            let (completed, total) = self
                .pre_triage_load_progress()
                .and_then(|(files_scanned, files_total, _)| {
                    (files_total > 0).then_some((files_scanned as u32, files_total as u32))
                })
                .unwrap_or((0, 1));
            return Some(OperationProgress {
                label: self.pre_triage_loading_operation_label(),
                completed,
                total,
            });
        }

        None
    }

    pub fn build_signal_candidate_rows(&self) -> Vec<SignalCandidateRow> {
        let completed_candidates: Vec<ScoredCandidate> = self
            .signal_candidate
            .iter_completed()
            .map(|(url, result)| ScoredCandidate {
                url: url.to_string(),
                result: result.clone(),
            })
            .collect();
        let active_prompt_version = self
            .active_version_for(harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate)
            .unwrap_or_default();
        let threshold = self.signal_candidate_threshold();
        let selection = SignalCandidateSelection::compute(
            &completed_candidates,
            SelectionPolicy {
                threshold,
                active_prompt_version,
                excluded: self.signal_candidate.excluded().clone(),
            },
        );

        let selected_urls: HashSet<&str> =
            selection.selected_urls.iter().map(String::as_str).collect();
        // signal_key -> representative gist (the kept article shown on deduped rows).
        let mut kept_gist_by_key: HashMap<String, String> = HashMap::new();
        for url in &selection.selected_urls {
            if let Some(SignalCandidateState::Completed { result }) =
                self.signal_candidate.state_for(url)
            {
                let cluster_key = canonical_signal_key(&result.signal_key);
                kept_gist_by_key
                    .entry(cluster_key)
                    .or_insert_with(|| truncate_signal_candidate_gist(&result.draft_gist));
            }
        }
        let job_id_by_url: HashMap<&str, crate::JobId> = self
            .jobs
            .iter()
            .map(|(job_id, job)| (job.url.as_str(), *job_id))
            .collect();
        let mut rows = Vec::new();
        for (url, state) in self.signal_candidate.iter_states() {
            let Some(job_id) = job_id_by_url.get(url).copied() else {
                continue;
            };
            match state {
                SignalCandidateState::Pending => continue,
                SignalCandidateState::Deferred => continue,
                SignalCandidateState::Scoring { .. } => {
                    rows.push(SignalCandidateRow {
                        job_id,
                        url: url.to_string(),
                        score: 0,
                        score_band: ScoreBand::Low,
                        source_tier: SourceTier::Tier3,
                        themes: Vec::new(),
                        gist_truncated: String::new(),
                        dupes_count: 0,
                        state_label: SignalCandidateRowState::Scoring,
                        signal_key: String::new(),
                        outcome: None,
                    });
                }
                SignalCandidateState::Failed { reason } => {
                    rows.push(SignalCandidateRow {
                        job_id,
                        url: url.to_string(),
                        score: 0,
                        score_band: ScoreBand::Low,
                        source_tier: SourceTier::Tier3,
                        themes: Vec::new(),
                        gist_truncated: String::new(),
                        dupes_count: 0,
                        state_label: SignalCandidateRowState::Failed {
                            reason: reason.clone(),
                        },
                        signal_key: String::new(),
                        outcome: None,
                    });
                }
                SignalCandidateState::Completed { result } => {
                    let score_band = match result.signal_score {
                        80..=u8::MAX => ScoreBand::High,
                        60..=79 => ScoreBand::Mid,
                        _ => ScoreBand::Low,
                    };
                    let dupes_count = selection
                        .cluster_size_for_signal_key(&result.signal_key)
                        .saturating_sub(1);
                    let is_excluded = is_signal_key_excluded(
                        self.signal_candidate.excluded(),
                        &result.signal_key,
                        active_prompt_version,
                    );
                    let outcome = if is_excluded {
                        SignalCandidateOutcome::Excluded
                    } else if result.signal_score < threshold {
                        SignalCandidateOutcome::BelowThreshold
                    } else if selected_urls.contains(url) {
                        SignalCandidateOutcome::Selected
                    } else {
                        SignalCandidateOutcome::Deduplicated {
                            kept_gist: kept_gist_by_key
                                .get(&canonical_signal_key(&result.signal_key))
                                .cloned()
                                .unwrap_or_default(),
                        }
                    };
                    rows.push(SignalCandidateRow {
                        job_id,
                        url: url.to_string(),
                        score: result.signal_score,
                        score_band,
                        source_tier: result.source_tier,
                        themes: result.themes.clone(),
                        gist_truncated: truncate_signal_candidate_gist(&result.draft_gist),
                        dupes_count,
                        state_label: SignalCandidateRowState::Scored,
                        signal_key: result.signal_key.clone(),
                        outcome: Some(outcome),
                    });
                }
            }
        }

        rows.sort_by(|a, b| {
            signal_candidate_sort_rank(a)
                .cmp(&signal_candidate_sort_rank(b))
                .then(b.score.cmp(&a.score))
                .then(a.url.cmp(&b.url))
        });
        rows
    }

    fn signal_candidate_preview_for_selected_job(
        &self,
        selected_job_id: Option<crate::JobId>,
    ) -> Option<SignalCandidatePreviewView> {
        let job_id = selected_job_id?;
        let job = self.jobs.get(&job_id)?;
        let state = self.signal_candidate.state_for(&job.url)?;
        let SignalCandidateState::Completed { result } = state else {
            return None;
        };
        let signal_key = result.signal_key.clone();
        let cluster_key = canonical_signal_key(&signal_key);
        let mut duplicate_urls: Vec<String> = self
            .signal_candidate
            .iter_completed()
            .filter(|(url, candidate)| {
                *url != job.url && canonical_signal_key(&candidate.signal_key) == cluster_key
            })
            .map(|(url, _)| url.to_string())
            .collect();
        duplicate_urls.insert(0, job.url.clone());
        duplicate_urls[1..].sort();
        let exclude_checked = self
            .active_version_for(harvester_engine::llm::prompt::PromptId::ArticleSignalCandidate)
            .map(|prompt_version| {
                is_signal_key_excluded(
                    self.signal_candidate.excluded(),
                    &signal_key,
                    prompt_version,
                )
            })
            .unwrap_or(false);
        Some(SignalCandidatePreviewView {
            signal_key,
            duplicate_urls,
            exclude_checked,
            state_label: String::from("Scored"),
        })
    }

    fn format_briefing_preview_header(&self) -> String {
        let total = self.briefing.articles().len();
        let scope = if self.briefing_since_utc().is_some() {
            "Since checkpoint"
        } else {
            "All articles"
        };
        let status = match self.briefing.phase() {
            BriefingPhase::Idle => "Idle".to_string(),
            BriefingPhase::LoadingArticles => "Loading articles".to_string(),
            BriefingPhase::Summarizing => {
                let settled =
                    self.briefing.completed_summary_count() + self.briefing.failed_summary_count();
                format!("Summaries {settled}/{total}")
            }
            BriefingPhase::AwaitingBatch => "Awaiting batch".to_string(),
            BriefingPhase::GeneratingBriefing => "Generating briefing".to_string(),
            BriefingPhase::Streaming => {
                if self.briefing.next_item_in_flight() {
                    "Fetching next item".to_string()
                } else {
                    "Streaming".to_string()
                }
            }
            BriefingPhase::Complete => "Done".to_string(),
            BriefingPhase::Failed { .. } => "Failed".to_string(),
        };

        if total == 0 {
            format!("Executive Briefing | {scope} | {status}")
        } else {
            format!("Executive Briefing | {total} articles | {scope} | {status}")
        }
    }

    fn format_trends_preview_header(&self) -> String {
        "Trends | recent activity".to_string()
    }

    pub fn layout_view(&self) -> LayoutViewModel {
        let selected_job = self
            .ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id));
        let preview_header_override_visible = matches!(
            self.active_tab(),
            AppTab::Briefing | AppTab::Trends | AppTab::PollStats
        );
        LayoutViewModel {
            left_panel_width: self.ui.left_panel_width(),
            input_panel_visible: self.ui.input_panel_visible(),
            operation_progress_visible: self.build_operation_progress().is_some(),
            active_tab: self.active_tab(),
            left_tab: self.left_tab(),
            left_header_meta_visible: matches!(
                self.left_tab(),
                LeftTab::Jobs | LeftTab::TriageReview | LeftTab::TriageResults
            ),
            ai_warning_banner_visible: self.ai_warning_banner().is_some()
                || self.provider_alert_banner().is_some(),
            preview_header_override_visible,
            preview_context_visible: selected_job.is_some() && !preview_header_override_visible,
            preview_attention_visible: selected_job
                .and_then(|job| job.preview_quality.as_ref())
                .map(|quality| quality.nav_heavy())
                .unwrap_or(false)
                && !preview_header_override_visible,
            signal_candidate_preview_visible: selected_job
                .and_then(|job| self.signal_candidate.state_for(&job.url))
                .is_some_and(|state| matches!(state, SignalCandidateState::Completed { .. })),
            prompt_lab_advanced_mode: self.prompt_lab.advanced_mode(),
            prompt_lab_compare_section_open: self.prompt_lab.compare_section_open(),
            prompt_lab_context_section_open: self.prompt_lab.context_section_open(),
            prompt_lab_template_section_open: self.prompt_lab.template_section_open(),
            prompt_lab_run_details_section_open: self.prompt_lab.run_details_section_open(),
            prompt_lab_template_editor_open: self.prompt_lab.template_editor_open(),
        }
    }

    fn build_right_pane_view(&self, selected_triage_article_available: bool) -> RightPaneView {
        let selected_url = self
            .ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id))
            .map(|job| job.url.as_str());

        let triage_markdown = selected_url.and_then(|url| {
            let title = crate::preview::best_effort_article_title(
                self.triage.source_title_for_url(url),
                url,
            );
            self.triage
                .result_for_url(url)
                .map(|result| crate::preview::format_triage_for_preview(title.as_deref(), result))
        });

        let summary_markdown = selected_url
            .and_then(|url| self.briefing.summary_for_url(url))
            .map(format_summary_for_preview);

        let briefing_markdown = self.briefing.format_preview();
        let triage_placeholder = if triage_markdown.is_none() {
            match self.ai_unavailable_reason() {
                Some(crate::AiUnavailableReason::MissingApiKey) => Some(
                    "AI setup required\n\nTriage is disabled because `OPENAI_API_KEY` is not set.\n\nSet `OPENAI_API_KEY` in the launch environment and restart the app to enable article triage.".to_string(),
                ),
                _ => self
                    .triage_blocked_reason()
                    .map(|reason| format!("Article triage is unavailable because {reason}.")),
            }
        } else {
            None
        };
        let briefing_placeholder = if briefing_markdown.is_none() {
            match self.ai_unavailable_reason() {
                Some(crate::AiUnavailableReason::MissingApiKey) => Some(
                    "AI setup required\n\nBriefing is disabled because `OPENAI_API_KEY` is not set.\n\nSet `OPENAI_API_KEY` in the launch environment and restart the app to enable briefing generation.".to_string(),
                ),
                _ => self
                    .briefing_blocked_reason()
                    .map(|reason| format!("Briefing is unavailable because {reason}.")),
            }
        } else {
            None
        };

        let prompt_lab = crate::view_model::PromptLabView::from_state(
            &self.prompt_lab,
            &self.prompt_contexts,
            &self.prompt_lab_templates,
            selected_triage_article_available,
        );

        let (effective_triage_markdown, effective_summary_markdown, effective_briefing_markdown) =
            if self.left_tab == LeftTab::PromptLab {
                let lab_triage = prompt_lab.latest_run.as_ref().and_then(|run| {
                    if run.stage == crate::prompt_lab::PromptLabStage::Triage {
                        run.output_json.as_deref().map(format_lab_triage_markdown)
                    } else {
                        None
                    }
                });
                let lab_summary = prompt_lab.latest_run.as_ref().and_then(|run| {
                    if run.stage == crate::prompt_lab::PromptLabStage::Summary {
                        run.output_json.as_deref().map(format_lab_summary_markdown)
                    } else {
                        None
                    }
                });
                let lab_briefing = prompt_lab.latest_run.as_ref().and_then(|run| {
                    if run.stage == crate::prompt_lab::PromptLabStage::Briefing {
                        run.output_json.as_deref().map(format_lab_briefing_markdown)
                    } else {
                        None
                    }
                });
                (
                    lab_triage.or(triage_markdown).or(triage_placeholder),
                    lab_summary.or(summary_markdown),
                    lab_briefing.or(briefing_markdown).or(briefing_placeholder),
                )
            } else {
                (
                    triage_markdown.or(triage_placeholder),
                    summary_markdown,
                    briefing_markdown.or(briefing_placeholder),
                )
            };

        let _ = prompt_lab;

        let trends = crate::view_model::build_trends_tab_view(
            self.entity_trend_data.as_ref(),
            self.active_trend_category,
        );

        let poll_stats_markdown = {
            let stats = self.source_states.last_completed_poll_stats();
            if stats.is_empty() {
                None
            } else {
                let warning = crate::build_poll_quota_warning(stats, self.llm_quota());
                Some(crate::poll_stats_fmt::format_poll_stats_with_warning(
                    stats,
                    warning.as_ref(),
                ))
            }
        };

        RightPaneView {
            active_tab: self.active_tab,
            triage_markdown: effective_triage_markdown,
            summary_markdown: effective_summary_markdown,
            briefing_markdown: effective_briefing_markdown,
            trends,
            poll_stats_markdown,
        }
    }

    fn build_indirect_link_summary(&self) -> Option<IndirectLinkSummary> {
        let count = self.indirect_link_pool.len();
        if count == 0 && self.indirect_link_pool.generation() == 0 && !self.is_poll_in_progress() {
            return None;
        }
        let phase = if self.is_poll_in_progress() {
            IndirectLinkPhase::Collecting
        } else {
            IndirectLinkPhase::Ready
        };
        Some(IndirectLinkSummary { count, phase })
    }
}

#[derive(Clone)]
struct JobViewMetadata {
    job_id: crate::JobId,
    is_since_checkpoint: bool,
    triage_annotation: Option<TriageAnnotationView>,
    has_summary: bool,
    summary_title: Option<String>,
    summary_tokens: Option<u32>,
    filter_status: Option<JobFilterStatus>,
    has_analysis: bool,
}

struct CachedSummary<'a> {
    created_at_utc: &'a str,
    tie_break_key: (u32, &'a str, &'a str),
    summary: &'a ArticleSummaryResult,
}

/// Per-view summary index with deterministic duplicate resolution.
///
/// Briefing summaries preserve `BriefingSession::summary_for_url`: the first completed article in
/// session order wins. Cache summaries prefer the newest timestamp, then the lexicographically
/// greatest `(prompt_version, model_id, context_hash)` tuple when timestamps tie so `HashMap`
/// iteration order cannot affect views.
#[derive(Default)]
struct SummaryLookup<'a> {
    briefing_by_url: HashMap<&'a str, &'a ArticleSummaryResult>,
    cache_by_content_hash: HashMap<&'a str, CachedSummary<'a>>,
}

impl<'a> SummaryLookup<'a> {
    fn summary_for_job(
        &self,
        state: &AppState,
        job: &JobState,
    ) -> Option<&'a ArticleSummaryResult> {
        self.briefing_by_url
            .get(job.url.as_str())
            .copied()
            .or_else(|| {
                state
                    .content_hash_for_url(&job.url)
                    .and_then(|hash| self.cache_by_content_hash.get(hash))
                    .map(|cached| cached.summary)
            })
    }

    fn summary_for_content_hash(&self, content_hash: &str) -> Option<&'a ArticleSummaryResult> {
        self.cache_by_content_hash
            .get(content_hash)
            .map(|cached| cached.summary)
    }
}

#[derive(Default)]
struct DesktopJobSelection {
    scoped_ids: HashSet<crate::JobId>,
    searched_ids: HashSet<crate::JobId>,
    emitted_ids: HashSet<crate::JobId>,
    emitted: Vec<DesktopJobSelectionRow>,
    searched_count: usize,
    hidden_without_fetch_time: usize,
}

struct DesktopJobSelectionRow {
    job_id: crate::JobId,
    fetched_utc: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Default)]
struct LegacyJobListMetrics {
    scoped_count: usize,
    visible_job_ids: Vec<crate::JobId>,
    visible_job_count: usize,
    first_visible_job_id: Option<crate::JobId>,
    selected_job_visible: bool,
    review_needed_count: usize,
    triage_result_count: usize,
}

fn is_since_checkpoint(job: &JobState, since: Option<chrono::DateTime<chrono::Utc>>) -> bool {
    match (job.fetched_utc, since) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(fetched), Some(checkpoint)) => fetched >= checkpoint,
    }
}

fn fetched_descending(
    left: Option<chrono::DateTime<chrono::Utc>>,
    right: Option<chrono::DateTime<chrono::Utc>>,
) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => right.cmp(&left),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

impl JobViewMetadata {
    fn apply_to(&self, row: &mut JobRowView) {
        row.triage_annotation = self.triage_annotation.clone();
        row.has_summary = self.has_summary;
        row.summary_title = self.summary_title.clone();
        row.summary_tokens = self.summary_tokens;
        row.filter_status = self.filter_status.clone();
        row.has_analysis = self.has_analysis;
    }
}

struct LeftPaneHeaderInputs<'a> {
    left_tab: LeftTab,
    job_list_scope: JobListScope,
    scoped_count: usize,
    review_needed_count: usize,
    triage_result_count: usize,
    visible_job_count: usize,
    jobs_search_query: &'a str,
    ai_unavailable_message: Option<&'a str>,
}

fn build_left_pane_header_view(inputs: LeftPaneHeaderInputs<'_>) -> LeftPaneHeaderView {
    let LeftPaneHeaderInputs {
        left_tab,
        job_list_scope,
        scoped_count,
        review_needed_count,
        triage_result_count,
        visible_job_count,
        jobs_search_query,
        ai_unavailable_message,
    } = inputs;
    let scope_label = if job_list_scope == JobListScope::SinceCheckpoint {
        Some("Since checkpoint".to_string())
    } else {
        None
    };

    match left_tab {
        LeftTab::Jobs => {
            let scope_count = scoped_count;
            let visible_count = visible_job_count;
            let search_active = !jobs_search_query.is_empty();
            LeftPaneHeaderView {
                title: "Jobs".to_string(),
                scope_label,
                count_label: Some(if search_active {
                    format!("{visible_count} of {scope_count} jobs")
                } else {
                    format!("{scope_count} jobs")
                }),
                state_label: if scope_count == 0 {
                    Some("no jobs in scope".to_string())
                } else {
                    None
                },
            }
        }
        LeftTab::TriageReview => LeftPaneHeaderView {
            title: "Triage Review".to_string(),
            scope_label,
            count_label: Some(if review_needed_count == 0 {
                "no review-needed items".to_string()
            } else {
                format!("{review_needed_count} review-needed")
            }),
            state_label: None,
        },
        LeftTab::TriageResults => LeftPaneHeaderView {
            title: "Results".to_string(),
            scope_label,
            count_label: Some(if triage_result_count == 0 {
                "no triage results yet".to_string()
            } else {
                format!("{triage_result_count} with triage")
            }),
            state_label: ai_unavailable_message.map(|_| "AI unavailable".to_string()),
        },
        LeftTab::PromptLab => LeftPaneHeaderView {
            title: "Job List".to_string(),
            scope_label: None,
            count_label: None,
            state_label: None,
        },
        LeftTab::Blacklist => LeftPaneHeaderView {
            title: "Blacklist".to_string(),
            scope_label: None,
            count_label: None,
            state_label: None,
        },
    }
}

fn signal_candidate_sort_rank(row: &SignalCandidateRow) -> u8 {
    match &row.outcome {
        Some(SignalCandidateOutcome::Selected) => 0,
        Some(SignalCandidateOutcome::Deduplicated { .. }) => 1,
        Some(SignalCandidateOutcome::BelowThreshold) => 2,
        Some(SignalCandidateOutcome::Excluded) => 3,
        None => match row.state_label {
            SignalCandidateRowState::Failed { .. } => 5,
            // Scoring (and the unreachable Scored-without-outcome) sort just above Failed.
            _ => 4,
        },
    }
}

fn truncate_signal_candidate_gist(text: &str) -> String {
    const MAX_CHARS: usize = 180;
    let total_chars = text.chars().count();
    if total_chars <= MAX_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX_CHARS).collect();
    out.push('…');
    out
}

fn job_matches_search_query(url: &str, summary_title: Option<&str>, query_lower: &str) -> bool {
    query_lower.is_empty()
        || summary_title.is_some_and(|title| title.to_lowercase().contains(query_lower))
        || url.to_lowercase().contains(query_lower)
}

fn build_preview_context_view(header: &PreviewHeaderView) -> PreviewContextView {
    let source_label = if header.domain.is_empty() {
        "(unknown source)".to_string()
    } else {
        header.domain.clone()
    };
    let status_label = match &header.outcome {
        Some(JobResultKind::Failed { reason }) => format!("Failed ({reason})"),
        Some(JobResultKind::Success) => "Done".to_string(),
        None => match header.stage {
            Stage::Queued => "Queued",
            Stage::Downloading => "Downloading",
            Stage::Sanitizing => "Sanitizing",
            Stage::Converting => "Converting",
            Stage::Tokenizing => "Tokenizing",
            Stage::Writing => "Writing",
            Stage::Done => "Done",
        }
        .to_string(),
    };
    let attention_label = if header.nav_heavy {
        Some("navigation-heavy".to_string())
    } else {
        None
    };

    PreviewContextView {
        source_label,
        status_label,
        attention_label,
    }
}
