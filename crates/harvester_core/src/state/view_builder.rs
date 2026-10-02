use super::batch::archive_token_estimates_from_parts;
use super::{map_job_filter_status, AppState, JobState};
use crate::archive_display::ArchiveCoverage;
use crate::briefing::ArticleSummaryResult;
use crate::pre_triage_filter::PreTriagePhase;
use crate::preview::format_summary_for_preview;
use crate::signal_candidate::{
    canonical_signal_key, is_signal_key_excluded, ScoredCandidate, SelectionPolicy,
    SignalCandidateSelection, SignalCandidateState,
};
use crate::tabs::JobListMode;
use crate::triage::TriagePhase;
use crate::view_model::{
    AppViewModel, DesktopJobListView, JobFilterStatus, JobListRowView, JobRowView, RightPaneView,
    ScoreBand, SelectedJobView, SelectedJobVisibility, SignalCandidateOutcome, SignalCandidateRow,
    SignalCandidateRowState, TriageAnnotationView, DESKTOP_JOB_LIST_MAX_ROWS,
    DESKTOP_JOB_LIST_RECENT_WINDOW_HOURS, TOKEN_LIMIT,
};
use chrono::{DateTime, Duration, Utc};
use harvester_engine::llm::dto::SourceTier;
use harvester_engine::llm::prompt::PromptId;
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

    fn build_view(&self) -> AppViewModel {
        let summary_lookup = self.build_summary_lookup();

        let signal_candidate_rows = self.build_signal_candidate_rows();
        let desktop_job_list = self.build_desktop_job_list_view(
            &signal_candidate_rows,
            self.jobs_search_query(),
            &summary_lookup,
        );
        let ai_unavailable_message = self.ai_unavailable_message();

        let stop_finish_button = self.stop_finish_button_state();
        let archive_display = self.archive_display_counts();
        let full_filtered_count = archive_display.filtered_count();
        let archive_estimates =
            self.archive_token_estimates_for_view(archive_display.ordered_urls(), &summary_lookup);

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
        let scoring = sc.observation_counts();
        let in_progress = scoring.pending_or_in_flight;
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
                        &summary_lookup,
                    );
                    (sc_estimates.summary_tokens, selection.selected_urls.len())
                }
            } else {
                (archive_estimates.summary_tokens, full_filtered_count)
            };
        let run_state = self.run_state();
        let run_enabled = matches!(run_state, crate::RunState::Idle);
        let unfinished_work = self.unfinished_work().clone();
        let resume_disabled_reason = if !run_enabled {
            Some(match run_state {
                crate::RunState::Active => "A run is already in progress.".to_string(),
                crate::RunState::Stopping { .. } => {
                    "The current run is still stopping.".to_string()
                }
                crate::RunState::Idle => unreachable!(),
            })
        } else if !self.triage_ai_available() {
            Some(
                ai_unavailable_message
                    .clone()
                    .unwrap_or_else(|| "AI is unavailable.".to_string()),
            )
        } else {
            match &unfinished_work {
                crate::UnfinishedWork::Unknown => {
                    Some("Unfinished work is not known yet.".to_string())
                }
                crate::UnfinishedWork::Known(summary) if summary.articles_with_work == 0 => {
                    Some("There is no unfinished work to process.".to_string())
                }
                crate::UnfinishedWork::Known(_) => None,
            }
        };
        let resume_enabled = resume_disabled_reason.is_none();
        let reprocess_notice = (!matches!(run_state, crate::RunState::Idle))
            .then(|| self.reprocess_notice())
            .flatten()
            .map(
                |(articles, estimated_calls)| crate::view_model::ReprocessNoticeView {
                    articles,
                    estimated_calls,
                },
            );
        AppViewModel {
            job_count: self.jobs.len(),
            desktop_job_list,
            last_paste_stats: self.last_paste_stats.clone(),
            token_limit: TOKEN_LIMIT,
            archive_token_estimate,
            archive_filtered_count,
            archive_partial_coverage,
            raw_unprocessed_count,
            stop_finish_button,
            signal_candidate_rows,
            ai_unavailable_message,
            run_progress: self
                .run_progress
                .as_ref()
                .map_or_else(Default::default, crate::RunProgress::view),
            archive_enabled: self.export_available(),
            run_state,
            run_completion_notice: self.run_completion_notice.clone(),
            run_enabled,
            resume_enabled,
            resume_disabled_reason,
            unfinished_work,
            reprocess_notice,
            checkpoint_status_message: self
                .briefing_checkpoint_status_message
                .clone()
                .or_else(|| self.runtime_state_notice.clone()),
            llm_quota: crate::build_llm_quota_view(self.llm_quota()),
            right_pane: self.build_right_pane_view(),
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
        summary_lookup: &SummaryLookup,
    ) -> crate::ArchiveTokenEstimates {
        if urls.is_empty() {
            return crate::ArchiveTokenEstimates::default();
        }
        let url_tokens = self.archive_article_token_lookup();
        archive_token_estimates_from_parts(urls, url_tokens, |url| {
            self.content_hash_for_url(url)
                .and_then(|hash| summary_lookup.summary_for_content_hash(hash))
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
        let selected_id = self.ui.selected_job_id();
        for (job_id, job) in &self.jobs {
            let in_scope = match mode {
                JobListMode::SinceCheckpoint => is_since_checkpoint(job, since),
                JobListMode::Last24Hours => is_within_recent_window(job, window_start),
                JobListMode::Results => false,
            };
            if !in_scope {
                continue;
            }
            if selected_id == Some(*job_id) {
                selection.selected_in_scope = true;
            }
            if !query_lower.is_empty()
                && !job_matches_search_query(
                    &job.url,
                    summary_lookup
                        .summary_for_job(self, job)
                        .map(|summary| summary.title.as_str()),
                    query_lower,
                )
            {
                continue;
            }
            if selected_id == Some(*job_id) {
                selection.selected_matches_query = true;
            }
            selection.emitted.push(DesktopJobSelectionRow {
                job_id: *job_id,
                fetched_utc: job.fetched_utc,
            });
        }
        selection.searched_count = selection.emitted.len();
        if selection.emitted.len() > DESKTOP_JOB_LIST_MAX_ROWS {
            let order = |left: &DesktopJobSelectionRow, right: &DesktopJobSelectionRow| {
                fetched_descending(left.fetched_utc, right.fetched_utc)
                    .then_with(|| right.job_id.cmp(&left.job_id))
            };
            selection
                .emitted
                .select_nth_unstable_by(DESKTOP_JOB_LIST_MAX_ROWS, order);
            selection.emitted.truncate(DESKTOP_JOB_LIST_MAX_ROWS);
            selection.emitted.sort_unstable_by_key(|row| row.job_id);
        }
        selection.selected_emitted = selection
            .emitted
            .iter()
            .any(|row| Some(row.job_id) == selected_id);
        selection
    }

    fn show_filter_status(&self) -> bool {
        matches!(
            self.pre_triage.phase(),
            PreTriagePhase::Reviewing | PreTriagePhase::ReadyToTriage
        )
    }

    pub fn triage_reorder_suppressed(&self) -> bool {
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
        let since = self.briefing_since_utc();
        let show_filter_status = self.show_filter_status();
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
                    since,
                    show_filter_status,
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
                    if !selection.selected_in_scope {
                        SelectedJobVisibility::OutsideScope
                    } else if !selection.selected_matches_query {
                        SelectedJobVisibility::QueryMismatch
                    } else if !selection.selected_emitted {
                        SelectedJobVisibility::Capped
                    } else {
                        SelectedJobVisibility::Visible
                    }
                }
            };
            let metadata = self.enrich_job_view_metadata(
                selected_job_id,
                job,
                since,
                show_filter_status,
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
        if self.signal_candidate.iter_states().next().is_none() {
            return Vec::new();
        }
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

    fn build_right_pane_view(&self) -> RightPaneView {
        let summary_markdown = self
            .ui
            .selected_job_id()
            .and_then(|job_id| self.jobs.get(&job_id))
            .and_then(|job| self.briefing.summary_for_url(&job.url))
            .map(format_summary_for_preview);
        RightPaneView { summary_markdown }
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
    selected_in_scope: bool,
    selected_matches_query: bool,
    selected_emitted: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ArticleSummaryResult, ArticleTriageResult, JobOrigin, SummaryCache, SummaryCacheEntry,
        SummaryCacheKey, TriageSession,
    };
    use crate::{JobResultKind, Stage};

    #[test]
    fn archive_view_estimates_track_triage_job_url_cache_and_checkpoint_inputs() {
        let url = "https://view-cost.example/article";
        let content_hash = "view-cost-content";
        let mut triage = TriageSession::new_loading(None);
        triage.set_articles(vec![crate::LoadedArticle {
            url: url.to_string(),
            source_title: Some("View cost fixture".into()),
            prepared_text: "article body".into(),
            content_hash: content_hash.into(),
            fetched_utc: None,
        }]);
        triage.transition_to_triaging();
        triage.complete_article(
            0,
            ArticleTriageResult {
                category: "news".into(),
                priority: 3,
                tags: Vec::new(),
                rationale: "fixture".into(),
                input_tokens: 1,
                output_tokens: 1,
            },
        );
        triage.complete();

        let fetched_utc = chrono::DateTime::parse_from_rfc3339("2026-09-01T12:00:00Z")
            .expect("valid fetched timestamp")
            .with_timezone(&Utc);
        let mut state = AppState::new();
        state.set_triage(triage);
        state.job_list_mode = JobListMode::SinceCheckpoint;
        state.jobs.insert(
            1,
            JobState {
                url: url.to_string(),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                tokens: Some(1_200),
                origin: JobOrigin::Direct,
                fetched_utc: Some(fetched_utc),
                ..Default::default()
            },
        );
        state.rebuild_archive_job_tokens();
        assert_archive_view_matches_full_lookup(&state);
        assert_eq!(state.view().archive_token_estimate, 1_200);

        state.jobs.get_mut(&1).expect("fixture job").tokens = Some(2_400);
        state.rebuild_archive_job_tokens();
        assert_archive_view_matches_full_lookup(&state);
        assert_eq!(state.view().archive_token_estimate, 2_400);

        state
            .jobs
            .get_mut(&1)
            .expect("fixture job")
            .set_url(format!("{url}#section"));
        state.rebuild_archive_job_tokens();
        assert_archive_view_matches_full_lookup(&state);
        assert_eq!(state.view().archive_token_estimate, 2_400);

        state.jobs.insert(
            2,
            JobState {
                url: format!("{url}#duplicate"),
                stage: Stage::Done,
                outcome: Some(JobResultKind::Success),
                tokens: Some(3_600),
                origin: JobOrigin::Direct,
                fetched_utc: Some(fetched_utc),
                ..Default::default()
            },
        );
        state.rebuild_archive_job_tokens();
        assert_archive_view_matches_full_lookup(&state);
        assert_eq!(state.view().archive_token_estimate, 3_600);

        let summary_key = SummaryCacheKey::try_new(
            content_hash,
            PromptId::ArticleSummary,
            Some(1),
            Some("summary-model"),
            &[],
        )
        .expect("complete summary cache key");
        let mut cache = SummaryCache::new();
        cache.insert(
            summary_key.clone(),
            SummaryCacheEntry {
                result: ArticleSummaryResult {
                    title: "Current summary".into(),
                    summary: "Summary body".into(),
                    key_points: Vec::new(),
                    input_tokens: 300,
                    output_tokens: 700,
                    entities: crate::SummaryEntities::default(),
                },
                created_at_utc: "2026-09-25T12:00:00Z".into(),
            },
        );
        state.set_summary_cache(cache.clone());
        assert_archive_view_matches_full_lookup(&state);
        assert_eq!(state.view().archive_token_estimate, 700);

        cache.insert(
            summary_key,
            SummaryCacheEntry {
                result: ArticleSummaryResult {
                    title: "Updated summary".into(),
                    summary: "Updated summary body".into(),
                    key_points: Vec::new(),
                    input_tokens: 320,
                    output_tokens: 880,
                    entities: crate::SummaryEntities::default(),
                },
                created_at_utc: "2026-09-25T12:01:00Z".into(),
            },
        );
        state.set_summary_cache(cache);
        assert_archive_view_matches_full_lookup(&state);
        assert_eq!(state.view().archive_token_estimate, 880);

        state.briefing_since_utc = Some(
            chrono::DateTime::parse_from_rfc3339("2026-09-02T00:00:00Z")
                .expect("valid checkpoint")
                .with_timezone(&Utc),
        );
        assert!(state.view().desktop_job_list.rows.is_empty());
        state.briefing_since_utc = Some(
            chrono::DateTime::parse_from_rfc3339("2026-08-31T00:00:00Z")
                .expect("valid checkpoint")
                .with_timezone(&Utc),
        );
        let view = state.view();
        assert_eq!(view.desktop_job_list.rows.len(), 2);
        assert!(view
            .desktop_job_list
            .rows
            .iter()
            .all(|row| row.is_since_checkpoint));
        assert_archive_view_matches_full_lookup(&state);
    }

    fn assert_archive_view_matches_full_lookup(state: &AppState) {
        let corpus = state.archive_corpus();
        let expected = state.archive_token_estimates(corpus.ordered_urls());
        let view = state.view();
        assert_eq!(view.archive_filtered_count, corpus.count());
        assert_eq!(view.archive_token_estimate, expected.summary_tokens);
        assert_eq!(
            view.raw_unprocessed_count,
            corpus.count() - expected.summary_coverage
        );
        assert_eq!(view.archive_partial_coverage, None);
    }

    #[test]
    fn archive_job_token_index_matches_from_scratch_after_restore_progress_and_url_change() {
        use crate::{CompletedJobSnapshot, Msg};
        let url = "https://archive-index.example/article";
        let snapshots = [
            (url.to_string(), Some(100)),
            (format!("{url}#later"), Some(200)),
            ("https://archive-index.example/other".to_string(), None),
        ]
        .into_iter()
        .map(|(url, tokens)| CompletedJobSnapshot {
            url,
            tokens,
            bytes: None,
            links: Vec::new(),
            fetched_utc: None,
        })
        .collect();
        let (mut state, _) = crate::update(AppState::new(), Msg::RestoreCompletedJobs(snapshots));
        assert_archive_job_tokens_match_scan(&state);
        assert_eq!(
            state.archive_article_token_lookup().tokens_for_url(url),
            200
        );

        state = crate::update(
            state,
            Msg::JobProgress {
                job_id: 3,
                stage: Stage::Downloading,
                tokens: Some(300),
                bytes: None,
            },
        )
        .0;
        assert_archive_job_tokens_match_scan(&state);

        state
            .jobs
            .get_mut(&2)
            .unwrap()
            .set_url("https://archive-index.example/replaced".into());
        state.rebuild_archive_job_tokens();
        assert_archive_job_tokens_match_scan(&state);
        assert_eq!(
            state.archive_article_token_lookup().tokens_for_url(url),
            100
        );

        state.set_summary_cache(SummaryCache::new());
        assert_archive_job_tokens_match_scan(&state);
    }

    fn assert_archive_job_tokens_match_scan(state: &AppState) {
        let mut expected = HashMap::new();
        for job in state.jobs.values() {
            if let Some(tokens) = job.tokens {
                expected.insert(harvester_engine::archive_url_key(&job.url), tokens as u64);
            }
        }
        for job in state.jobs.values() {
            let key = harvester_engine::archive_url_key(&job.url);
            assert_eq!(
                state
                    .archive_article_token_lookup()
                    .tokens_for_url(&job.url),
                expected.get(&key).copied().unwrap_or(0)
            );
        }
    }
}
