use super::batch::archive_token_estimates_from_parts;
use super::{
    domain_from_url, map_job_filter_status, AppState, JobResultKind, JobState, SessionState, Stage,
};
use crate::archive_display::ArchiveCoverage;
use crate::briefing::ArticleSummaryResult;
use crate::pre_triage_filter::PreTriagePhase;
use crate::preview::format_summary_for_preview;
use crate::signal_candidate::{
    canonical_signal_key, is_signal_key_excluded, ScoredCandidate, SelectionPolicy,
    SignalCandidateSelection, SignalCandidateState,
};
use crate::tabs::JobListMode;
use crate::triage::{ArticleTriageState, TriagePhase};
use crate::view_model::{
    AppViewModel, DesktopJobListView, IndirectLinkPhase, IndirectLinkSummary, JobFilterStatus,
    JobListRowView, JobRowView, LeftPaneHeaderView, PreviewContextView, PreviewHeaderView,
    RightPaneView, ScoreBand, SelectedJobView, SelectedJobVisibility, SignalCandidateOutcome,
    SignalCandidatePreviewView, SignalCandidateRow, SignalCandidateRowState, TriageAnnotationView,
    DESKTOP_JOB_LIST_MAX_ROWS, DESKTOP_JOB_LIST_RECENT_WINDOW_HOURS, TOKEN_LIMIT,
};
use chrono::{DateTime, Duration, Utc};
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
        self.build_view()
    }

    /// Builds the desktop-facing view.
    pub fn desktop_view(&self) -> AppViewModel {
        self.build_view()
    }

    fn build_view(&self) -> AppViewModel {
        let summary_lookup = self.build_summary_lookup();

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
        let preview_text = self.ui.preview_content().map(ToOwned::to_owned);
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
        let first_visible_job_id = desktop_job_list.rows.first().map(|row| row.job_id);
        let selected_jobs_visible_in_filter = selected_job_id
            .is_some_and(|job_id| desktop_job_list.rows.iter().any(|row| row.job_id == job_id));
        let left_pane_header = LeftPaneHeaderView {
            title: "Jobs".to_string(),
            scope_label: Some(
                match desktop_job_list.mode {
                    JobListMode::Results => "Results",
                    JobListMode::SinceCheckpoint => "Since checkpoint",
                    JobListMode::Last24Hours => "Last 24 hours",
                }
                .to_string(),
            ),
            count_label: Some(format!("{} jobs", desktop_job_list.scoped_count)),
            state_label: (desktop_job_list.scoped_count == 0)
                .then_some("no jobs in scope".to_string()),
        };
        let preview_context = preview_header.as_ref().map(build_preview_context_view);
        let preview_header_text = match self.workspace_view() {
            crate::WorkspaceView::Trends => Some(self.format_trends_preview_header()),
            crate::WorkspaceView::PollStats => Some("Poll Stats | last poll".to_string()),
            crate::WorkspaceView::Review | crate::WorkspaceView::Blacklist => None,
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
            session: self.session,
            queued_urls: self.ui.urls.clone(),
            job_count: self.jobs.len(),
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
            selected_url,
            left_pane: crate::view_model::LeftPaneView {
                jobs_search_query,
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
            right_pane: self.build_right_pane_view(),
            blacklist: crate::view_model::BlacklistTabView::from_state(
                self.blacklist(),
                self.last_observed_utc()
                    .unwrap_or(chrono::DateTime::UNIX_EPOCH),
            ),
        }
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
        let mode = self.job_list_mode();
        if mode == JobListMode::Results {
            return DesktopJobSelection::default();
        }
        let since = self.briefing_since_utc();
        let window_start = self
            .last_observed_utc()
            .map(|now| now - Duration::hours(DESKTOP_JOB_LIST_RECENT_WINDOW_HOURS));
        let time_reference_known = match mode {
            JobListMode::SinceCheckpoint => since.is_some(),
            JobListMode::Last24Hours => window_start.is_some(),
            JobListMode::Results => false,
        };
        let mut selection = DesktopJobSelection {
            hidden_without_fetch_time: if time_reference_known {
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
            let in_scope = match mode {
                JobListMode::SinceCheckpoint => is_since_checkpoint(job, since),
                JobListMode::Last24Hours => is_within_recent_window(job, window_start),
                JobListMode::Results => false,
            };
            if !in_scope {
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
                JobListMode::SinceCheckpoint | JobListMode::Last24Hours => {
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

    fn format_trends_preview_header(&self) -> String {
        "Trends | recent activity".to_string()
    }

    fn build_right_pane_view(&self) -> RightPaneView {
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
            triage_markdown: triage_markdown.or(triage_placeholder),
            summary_markdown,
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

fn is_since_checkpoint(job: &JobState, since: Option<DateTime<Utc>>) -> bool {
    match (job.fetched_utc, since) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(fetched), Some(checkpoint)) => fetched >= checkpoint,
    }
}

fn is_within_recent_window(job: &JobState, window_start: Option<DateTime<Utc>>) -> bool {
    match (job.fetched_utc, window_start) {
        (Some(fetched), Some(window_start)) => fetched >= window_start,
        _ => false,
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
