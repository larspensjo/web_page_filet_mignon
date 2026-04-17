use super::{
    domain_from_url, format_lab_briefing_markdown, format_lab_summary_markdown,
    format_lab_triage_markdown, map_job_filter_status, AppState, JobResultKind, PreviewMode,
    SessionState, Stage,
};
use crate::briefing::BriefingPhase;
use crate::pre_triage_filter::PreTriagePhase;
use crate::tabs::{AppTab, JobListScope, LeftTab};
use crate::triage::{ArticleTriageState, TriagePhase};
use crate::view_model::{
    AppViewModel, IndirectLinkPhase, IndirectLinkSummary, JobFilterStatus, JobRowView,
    LayoutViewModel, LeftPaneHeaderView, OperationProgress, PreviewContextView, PreviewHeaderView,
    RightPaneView, TriageAnnotationView, TOKEN_LIMIT,
};
use harvester_engine::normalize_url_for_dedupe;

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
            if let Some(summary) = self.briefing.summary_for_url(&job_view.url) {
                job_view.has_summary = true;
                job_view.summary_title = Some(summary.title.clone());
            } else {
                job_view.has_summary = false;
                job_view.summary_title = None;
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
        let left_pane_header = build_left_pane_header_view(
            self.left_tab,
            self.job_list_scope,
            &jobs,
            self.ai_unavailable_message().as_deref(),
        );
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
        let operation_progress =
            if let Some((completed, total)) = self.source_states.poll_progress() {
                Some(OperationProgress {
                    label: "Polling".to_string(),
                    completed: completed as u32,
                    total: total as u32,
                })
            } else if matches!(self.triage.phase(), TriagePhase::Triaging) {
                let completed = self.triage.completed_count() + self.triage.failed_count();
                Some(OperationProgress {
                    label: "Triaging".to_string(),
                    completed: completed as u32,
                    total: self.triage.total() as u32,
                })
            } else if matches!(self.briefing.phase(), BriefingPhase::Summarizing) {
                let completed =
                    self.briefing.completed_summary_count() + self.briefing.failed_summary_count();
                Some(OperationProgress {
                    label: "Summarizing".to_string(),
                    completed: completed as u32,
                    total: self.briefing.total() as u32,
                })
            } else if matches!(self.pre_triage.phase(), PreTriagePhase::LoadingArticles) {
                let (completed, total) = self
                    .pre_triage_load_progress()
                    .and_then(|(files_scanned, files_total, _)| {
                        (files_total > 0).then_some((files_scanned as u32, files_total as u32))
                    })
                    .unwrap_or((0, 1));
                Some(OperationProgress {
                    label: self.pre_triage_loading_operation_label(),
                    completed,
                    total,
                })
            } else {
                None
            };
        let ai_warning_banner = self.ai_warning_banner();
        let ai_unavailable_message = self.ai_unavailable_message();
        let triage_blocked_reason = self.triage_blocked_reason();
        let briefing_blocked_reason = self.briefing_blocked_reason();
        let stop_finish_button = self.stop_finish_button_state();
        AppViewModel {
            session: self.session,
            queued_urls: self.ui.urls.clone(),
            job_count: self.jobs.len(),
            jobs,
            last_paste_stats: self.last_paste_stats.clone(),
            dirty: self.dirty,
            total_tokens: self.metrics.total_tokens,
            token_limit: TOKEN_LIMIT,
            preview_text,
            selected_job_id,
            left_pane_header,
            preview_header,
            preview_context,
            ai_warning_banner,
            preview_header_text,
            preview_source,
            briefing_can_start: self.briefing.can_start() && self.briefing_ai_available(),
            briefing_progress: self.briefing.progress_text(),
            briefing_preview,
            stop_finish_button,
            triage_can_start: self.triage_ai_available()
                && (!self.briefing_orchestration.is_requested())
                && self.triage.can_start()
                && self.can_start_triage_from_pre_triage(),
            triage_progress: self
                .triage
                .progress_text()
                .or_else(|| self.pre_triage_progress_text()),
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
            right_pane: self.build_right_pane_view(selected_triage_article_available),
        }
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
            BriefingPhase::WaitingForTriage => "Waiting for triage".to_string(),
            BriefingPhase::LoadingArticles => "Loading articles".to_string(),
            BriefingPhase::Summarizing => {
                let settled =
                    self.briefing.completed_summary_count() + self.briefing.failed_summary_count();
                format!("Summaries {settled}/{total}")
            }
            BriefingPhase::GeneratingBriefing => "Generating briefing".to_string(),
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
            operation_progress_visible: self.source_states.poll_progress().is_some()
                || matches!(self.triage.phase(), TriagePhase::Triaging)
                || matches!(self.briefing.phase(), BriefingPhase::Summarizing)
                || matches!(self.pre_triage.phase(), PreTriagePhase::LoadingArticles),
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

        let summary_markdown = selected_url.and_then(|url| {
            self.briefing.summary_for_url(url).map(|summary| {
                let kp_lines: String = summary
                    .key_points
                    .iter()
                    .map(|kp| format!("- {kp}\n"))
                    .collect();
                format!(
                    "# {}\n\n{}\n\n**Key Points:**\n\n{}\n",
                    summary.title, summary.summary, kp_lines
                )
            })
        });

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
                Some(crate::poll_stats_fmt::format_poll_stats(stats))
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

fn build_left_pane_header_view(
    left_tab: LeftTab,
    job_list_scope: JobListScope,
    jobs: &[JobRowView],
    ai_unavailable_message: Option<&str>,
) -> LeftPaneHeaderView {
    let scoped_jobs: Vec<&JobRowView> = if job_list_scope == JobListScope::SinceCheckpoint {
        jobs.iter().filter(|job| job.is_since_checkpoint).collect()
    } else {
        jobs.iter().collect()
    };
    let scope_label = if job_list_scope == JobListScope::SinceCheckpoint {
        Some("Since checkpoint".to_string())
    } else {
        None
    };

    match left_tab {
        LeftTab::Jobs => {
            let count = scoped_jobs.len();
            LeftPaneHeaderView {
                title: "Jobs".to_string(),
                scope_label,
                count_label: Some(format!("{count} jobs")),
                state_label: if count == 0 {
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
                title: "Triage Results".to_string(),
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
    }
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
