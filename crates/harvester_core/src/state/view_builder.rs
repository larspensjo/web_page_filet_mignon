use super::{
    domain_from_url, format_lab_briefing_markdown, format_lab_summary_markdown,
    format_lab_triage_markdown, map_job_filter_status, AppState, JobResultKind, PreviewMode,
    SessionState, Stage,
};
use crate::archive_display::ArchiveCoverage;
use crate::briefing::BriefingPhase;
use crate::pre_triage_filter::PreTriagePhase;
use crate::preview::format_summary_for_preview;
use crate::signal_candidate::{
    canonical_signal_key, is_signal_key_excluded, ScoredCandidate, SelectionPolicy,
    SignalCandidateSelection, SignalCandidateState,
};
use crate::tabs::{AppTab, JobListScope, LeftTab};
use crate::triage::{ArticleTriageState, TriagePhase};
use crate::view_model::{
    AppViewModel, IndirectLinkPhase, IndirectLinkSummary, JobFilterStatus, JobRowView,
    LayoutViewModel, LeftPaneHeaderView, OperationProgress, PreviewContextView, PreviewHeaderView,
    RightPaneView, ScoreBand, SignalCandidateOutcome, SignalCandidatePreviewView,
    SignalCandidateRow, SignalCandidateRowState, TriageAnnotationView, TOKEN_LIMIT,
};
use harvester_engine::llm::dto::SourceTier;
use harvester_engine::normalize_url_for_dedupe;
use std::collections::{HashMap, HashSet};

impl AppState {
    pub fn view(&self) -> AppViewModel {
        let since = self.briefing_since_utc();
        let mut jobs: Vec<JobRowView> = self
            .jobs
            .iter()
            .map(|(id, job)| {
                let is_since = match (job.fetched_utc, since) {
                    (_, None) => true,
                    (None, Some(_)) => false,
                    (Some(t), Some(s)) => t >= s,
                };
                job.to_view(*id, is_since)
            })
            .collect();
        for job_view in &mut jobs {
            if let Some(result) = self.triage.result_for_url(&job_view.url) {
                job_view.triage_annotation = Some(TriageAnnotationView {
                    priority: result.priority,
                    category: result.category.clone(),
                    tags: result.tags.clone(),
                });
            }
        }
        for job_view in &mut jobs {
            let cached_summary_tokens = self.summary_output_tokens_for_url(&job_view.url);
            if let Some(summary) = self.summary_result_for_url(&job_view.url) {
                job_view.has_summary = true;
                job_view.summary_title = Some(summary.title.clone());
                job_view.summary_tokens = Some(summary.output_tokens);
            } else if let Some(tokens) = cached_summary_tokens {
                job_view.has_summary = true;
                job_view.summary_title = None;
                job_view.summary_tokens = Some(tokens);
            } else {
                job_view.has_summary = false;
                job_view.summary_title = None;
                job_view.summary_tokens = None;
            }
        }
        if matches!(
            self.pre_triage.phase(),
            PreTriagePhase::Reviewing | PreTriagePhase::ReadyToTriage
        ) {
            for job_view in &mut jobs {
                job_view.filter_status = self
                    .pre_triage
                    .entry_for_url(&job_view.url)
                    .map(map_job_filter_status);
            }
        }

        for job_view in &mut jobs {
            job_view.has_analysis = job_view.has_summary
                || job_view.triage_annotation.is_some()
                || matches!(
                    job_view.filter_status,
                    Some(JobFilterStatus::HardExcluded { .. })
                        | Some(JobFilterStatus::ReviewNeeded { .. })
                        | Some(JobFilterStatus::ManuallyExcluded)
                );
        }

        let selected_job_id = self.ui.selected_job_id();
        let selected_url = selected_job_id
            .and_then(|job_id| self.jobs.get(&job_id))
            .map(|job| job.url.clone());
        let signal_candidate_rows = self.build_signal_candidate_rows();
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
        let scoped_jobs = scoped_jobs_for_job_list(&jobs, self.job_list_scope);
        let jobs_search_query = self.jobs_search_query().to_string();
        let visible_jobs_after_filter = if self.left_tab == LeftTab::Jobs {
            compute_visible_jobs_for_jobs_tab(&scoped_jobs, &jobs_search_query)
        } else {
            Vec::new()
        };
        let first_visible_job_id = visible_jobs_after_filter.first().copied();
        let selected_jobs_visible_in_filter =
            selected_job_id.is_some_and(|job_id| visible_jobs_after_filter.contains(&job_id));
        let left_pane_header = build_left_pane_header_view(LeftPaneHeaderInputs {
            left_tab: self.left_tab,
            job_list_scope: self.job_list_scope,
            scoped_jobs: &scoped_jobs,
            visible_jobs_after_filter: &visible_jobs_after_filter,
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
        let ai_warning_banner = self.ai_warning_banner();
        let ai_unavailable_message = self.ai_unavailable_message();
        let triage_blocked_reason = self.triage_blocked_reason();
        let briefing_blocked_reason = self.briefing_blocked_reason();
        let stop_finish_button = self.stop_finish_button_state();
        let archive_display = self.archive_display_counts();
        let full_filtered_count = archive_display.filtered_count();
        let archive_estimates = self.archive_token_estimates(archive_display.ordered_urls());

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
                    let sc_estimates = self.archive_token_estimates(&selection.selected_urls);
                    (sc_estimates.summary_tokens, selection.selected_urls.len())
                }
            } else {
                (archive_estimates.summary_tokens, full_filtered_count)
            };
        AppViewModel {
            session: self.session,
            queued_urls: self.ui.urls.clone(),
            job_count: self.jobs.len(),
            jobs,
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
            triage_results_reorder_suppressed: matches!(self.triage.phase(), TriagePhase::Triaging),
            signal_candidate_rows,
            signal_candidate_preview,
            ai_unavailable_message,
            triage_blocked_reason,
            briefing_blocked_reason,
            operation_progress_visible: operation_progress.is_some(),
            operation_progress,
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
                chrono::Utc::now(),
            ),
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
            ai_warning_banner_visible: self.ai_warning_banner().is_some(),
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

struct LeftPaneHeaderInputs<'a> {
    left_tab: LeftTab,
    job_list_scope: JobListScope,
    scoped_jobs: &'a [&'a JobRowView],
    visible_jobs_after_filter: &'a [crate::JobId],
    jobs_search_query: &'a str,
    ai_unavailable_message: Option<&'a str>,
}

fn build_left_pane_header_view(inputs: LeftPaneHeaderInputs<'_>) -> LeftPaneHeaderView {
    let LeftPaneHeaderInputs {
        left_tab,
        job_list_scope,
        scoped_jobs,
        visible_jobs_after_filter,
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
            let scope_count = scoped_jobs.len();
            let visible_count = visible_jobs_after_filter.len();
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
        LeftTab::TriageReview => {
            let review_needed_count = scoped_jobs
                .iter()
                .filter(|job| {
                    matches!(
                        job.filter_status,
                        Some(JobFilterStatus::ReviewNeeded { .. })
                    )
                })
                .count();
            LeftPaneHeaderView {
                title: "Triage Review".to_string(),
                scope_label,
                count_label: Some(if review_needed_count == 0 {
                    "no review-needed items".to_string()
                } else {
                    format!("{review_needed_count} review-needed")
                }),
                state_label: None,
            }
        }
        LeftTab::TriageResults => {
            let triage_result_count = scoped_jobs
                .iter()
                .filter(|job| job.triage_annotation.is_some())
                .count();
            LeftPaneHeaderView {
                title: "Results".to_string(),
                scope_label,
                count_label: Some(if triage_result_count == 0 {
                    "no triage results yet".to_string()
                } else {
                    format!("{triage_result_count} with triage")
                }),
                state_label: ai_unavailable_message.map(|_| "AI unavailable".to_string()),
            }
        }
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

fn scoped_jobs_for_job_list(jobs: &[JobRowView], job_list_scope: JobListScope) -> Vec<&JobRowView> {
    if job_list_scope == JobListScope::SinceCheckpoint {
        jobs.iter().filter(|job| job.is_since_checkpoint).collect()
    } else {
        jobs.iter().collect()
    }
}

fn compute_visible_jobs_for_jobs_tab(
    scoped_jobs: &[&JobRowView],
    query: &str,
) -> Vec<crate::JobId> {
    let query_lower = query.to_lowercase();
    scoped_jobs
        .iter()
        .filter(|job| {
            query_lower.is_empty()
                || job
                    .summary_title
                    .as_ref()
                    .is_some_and(|title| title.to_lowercase().contains(&query_lower))
                || job.url.to_lowercase().contains(&query_lower)
        })
        .map(|job| job.job_id)
        .collect()
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
