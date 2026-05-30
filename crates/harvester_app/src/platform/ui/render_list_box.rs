use std::collections::HashMap;

use commanductui::{
    BadgeDescriptor, ListBoxItemDescriptor, ListBoxItemId, ListBoxRowDensity, PlatformCommand,
    StyleId, WindowId,
};
use harvester_core::{
    AppViewModel, JobFilterStatus, JobListScope, JobOrigin, JobResultKind, JobRowView, LeftTab,
    SignalCandidateOutcome, Stage,
};

use super::constants::*;
use super::render_text::{
    compact_url_label, domain_from_url, format_compact_bytes, format_compact_tokens,
    title_case_label, truncate_with_ellipsis,
};

const LIST_BOX_DEFAULT_BADGE_COLUMN_WIDTH: i32 = 44;

const LIST_BOX_BADGE_PAD_X: i32 = 6;

const LIST_BOX_BADGE_GAP: i32 = 4;

pub(super) fn append_list_box_commands(
    window_id: WindowId,
    list_box: ListBoxRenderModel,
    cmds: &mut Vec<PlatformCommand>,
) {
    let ListBoxRenderModel {
        row_density,
        items,
        selected_item_id,
    } = list_box;
    let badge_column_width = compute_list_box_badge_column_width(&items);
    let badge_column_width = badge_column_width.clamp(0, u16::MAX as i32) as u16;
    cmds.push(PlatformCommand::SetListBoxRowDensity {
        window_id,
        control_id: TREE_JOBS,
        density: row_density,
    });
    cmds.push(PlatformCommand::PopulateListBox {
        window_id,
        control_id: TREE_JOBS,
        items,
        badge_column_width,
    });
    if let Some(item_id) = selected_item_id {
        cmds.push(PlatformCommand::SetListBoxSelection {
            window_id,
            control_id: TREE_JOBS,
            item_id,
        });
    }
}

pub(super) struct ListBoxRenderModel {
    row_density: ListBoxRowDensity,
    items: Vec<ListBoxItemDescriptor>,
    selected_item_id: Option<ListBoxItemId>,
}

impl ListBoxRenderModel {
    pub(super) fn from_view(view: &AppViewModel) -> Self {
        let row_density = match view.left_pane.left_tab {
            LeftTab::TriageResults => ListBoxRowDensity::Compact,
            _ => ListBoxRowDensity::Expanded,
        };
        let items = build_list_box_items(view);
        let selected_item_id = view
            .selected_job_id
            .map(ListBoxItemId::new)
            .filter(|selected_item_id| items.iter().any(|item| item.id == *selected_item_id));
        Self {
            row_density,
            items,
            selected_item_id,
        }
    }
}

pub(super) fn compute_list_box_badge_column_width(items: &[ListBoxItemDescriptor]) -> i32 {
    let mut badge_widths_by_index = Vec::<i32>::new();
    for item in items {
        for (index, badge) in item.badges.iter().enumerate() {
            let text_width = (badge.text.chars().count() as i32 * 8).max(24);
            let badge_width = text_width + LIST_BOX_BADGE_PAD_X * 2;
            if index == badge_widths_by_index.len() {
                badge_widths_by_index.push(badge_width);
            } else {
                badge_widths_by_index[index] = badge_widths_by_index[index].max(badge_width);
            }
        }
    }
    let badge_gaps = badge_widths_by_index
        .len()
        .saturating_sub(1)
        .try_into()
        .unwrap_or(i32::MAX);
    let badge_columns = badge_widths_by_index
        .iter()
        .sum::<i32>()
        .saturating_add(badge_gaps * LIST_BOX_BADGE_GAP);

    badge_columns
        .saturating_add(LIST_BOX_BADGE_GAP * 2)
        .clamp(LIST_BOX_DEFAULT_BADGE_COLUMN_WIDTH, 280)
}

pub(super) fn build_list_box_items(view: &AppViewModel) -> Vec<ListBoxItemDescriptor> {
    let tab = view.left_pane.left_tab;
    let scope_filtered: Vec<&JobRowView> = match tab {
        LeftTab::Jobs => {
            let jobs_by_id: HashMap<_, _> = view.jobs.iter().map(|job| (job.job_id, job)).collect();
            view.left_pane
                .visible_jobs_after_filter
                .iter()
                .filter_map(|job_id| jobs_by_id.get(job_id).copied())
                .collect()
        }
        _ if view.left_pane.job_list_scope == JobListScope::SinceCheckpoint => {
            view.jobs.iter().filter(|j| j.is_since_checkpoint).collect()
        }
        _ => view.jobs.iter().collect(),
    };
    let mut sorted_buf: Vec<&JobRowView>;
    let jobs_iter: &[&JobRowView] =
        if matches!(tab, LeftTab::TriageResults) && !view.triage_results_reorder_suppressed {
            sorted_buf = scope_filtered;
            sorted_buf.sort_by(|a, b| {
                let p_a = a
                    .triage_annotation
                    .as_ref()
                    .map(|t| t.priority)
                    .unwrap_or(0);
                let p_b = b
                    .triage_annotation
                    .as_ref()
                    .map(|t| t.priority)
                    .unwrap_or(0);
                p_b.cmp(&p_a).then(a.job_id.cmp(&b.job_id))
            });
            &sorted_buf
        } else {
            sorted_buf = scope_filtered;
            &sorted_buf
        };

    let signal_outcome_by_job_id: HashMap<_, _> = view
        .signal_candidate_rows
        .iter()
        .filter_map(|row| row.outcome.as_ref().map(|outcome| (row.job_id, outcome)))
        .collect();

    jobs_iter
        .iter()
        .map(|job| build_list_box_item_with_signal_outcome(tab, job, &signal_outcome_by_job_id))
        .collect()
}

fn build_list_box_item_with_signal_outcome(
    tab: LeftTab,
    job: &JobRowView,
    signal_outcome_by_job_id: &HashMap<u64, &SignalCandidateOutcome>,
) -> ListBoxItemDescriptor {
    let signal_outcome = if matches!(tab, LeftTab::TriageResults) {
        signal_outcome_by_job_id.get(&job.job_id).copied()
    } else {
        None
    };
    let mut item = build_list_box_item(tab, job);
    if let Some((text, style)) = signal_candidate_outcome_badge(signal_outcome) {
        item.badges.insert(
            0,
            BadgeDescriptor {
                text: text.to_string(),
                style,
            },
        );
    }
    item
}

pub(super) fn build_list_box_item(tab: LeftTab, job: &JobRowView) -> ListBoxItemDescriptor {
    let mut badges = match tab {
        LeftTab::Jobs => vec![BadgeDescriptor {
            text: job_status_label(job).to_string(),
            style: job_status_style(job),
        }],
        LeftTab::TriageReview => vec![BadgeDescriptor {
            text: triage_review_status_label(job).to_string(),
            style: triage_review_status_style(job),
        }],
        LeftTab::TriageResults => {
            let mut badges = vec![BadgeDescriptor {
                text: triage_priority_label(job),
                style: triage_priority_style(job),
            }];
            if let Some(annotation) = job.triage_annotation.as_ref() {
                badges.push(BadgeDescriptor {
                    text: title_case_label(&annotation.category),
                    style: StyleId::BadgeCategory,
                });
            }
            badges
        }
        LeftTab::PromptLab => vec![BadgeDescriptor {
            text: job_status_label(job).to_string(),
            style: job_status_style(job),
        }],
    };
    if matches!(tab, LeftTab::TriageReview) && matches!(job.origin, JobOrigin::Indirect { .. }) {
        badges.push(BadgeDescriptor {
            text: "Indirect".to_string(),
            style: StyleId::BadgeIndirect,
        });
    }

    let title = job
        .summary_title
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| compact_url_label(&job.url, 80));
    let metadata = match tab {
        LeftTab::Jobs => format!(
            "{} · {} · {}",
            job_source_label(job),
            job.tokens
                .map(|tokens| format_compact_tokens(tokens as u64))
                .unwrap_or_else(|| "—".to_string()),
            job.bytes
                .map(format_compact_bytes)
                .unwrap_or_else(|| "—".to_string())
        ),
        LeftTab::TriageReview => job
            .triage_annotation
            .as_ref()
            .map(|triage| title_case_label(&triage.category))
            .unwrap_or_else(|| "Untriaged".to_string()),
        LeftTab::TriageResults => String::new(),
        LeftTab::PromptLab => format!("{} · {}", job_source_label(job), job_status_label(job)),
    };
    let enabled = !matches!(
        tab,
        LeftTab::TriageReview
            if matches!(
                job.filter_status,
                Some(JobFilterStatus::HardExcluded { .. })
                    | Some(JobFilterStatus::ManuallyExcluded)
            )
    );

    ListBoxItemDescriptor {
        id: ListBoxItemId::new(job.job_id),
        badges,
        title,
        metadata,
        enabled,
    }
}

#[cfg(test)]
fn build_signal_candidate_item(row: &harvester_core::SignalCandidateRow) -> ListBoxItemDescriptor {
    let mut badges = Vec::new();
    if let Some((text, style)) = signal_candidate_outcome_badge(row.outcome.as_ref()) {
        badges.push(BadgeDescriptor {
            text: text.to_string(),
            style,
        });
    }
    badges.extend([
        BadgeDescriptor {
            text: row.score.to_string(),
            style: signal_candidate_score_style(row.score_band),
        },
        BadgeDescriptor {
            text: format!("{:?}", row.source_tier),
            style: StyleId::BadgeCategory,
        },
        BadgeDescriptor {
            text: format!("{} dupes", row.dupes_count),
            style: StyleId::BadgeStatusMuted,
        },
        BadgeDescriptor {
            text: signal_candidate_state_label(&row.state_label).to_string(),
            style: signal_candidate_state_style(&row.state_label),
        },
    ]);

    let title = if row.gist_truncated.is_empty() {
        compact_url_label(&row.url, 80)
    } else {
        row.gist_truncated.clone()
    };
    let mut metadata = if row.themes.is_empty() {
        String::new()
    } else {
        row.themes.join(" · ")
    };
    if let Some(SignalCandidateOutcome::Deduplicated { kept_gist }) = row.outcome.as_ref() {
        if !kept_gist.is_empty() {
            if metadata.is_empty() {
                metadata = format!("→ kept: {kept_gist}");
            } else {
                metadata.push_str(&format!(" · → kept: {kept_gist}"));
            }
        }
    }

    ListBoxItemDescriptor {
        id: ListBoxItemId::new(row.job_id),
        badges,
        title,
        metadata,
        enabled: !matches!(
            row.outcome.as_ref(),
            Some(SignalCandidateOutcome::Deduplicated { .. })
                | Some(SignalCandidateOutcome::BelowThreshold)
                | Some(SignalCandidateOutcome::Excluded)
        ),
    }
}

fn signal_candidate_outcome_badge(
    outcome: Option<&SignalCandidateOutcome>,
) -> Option<(&'static str, StyleId)> {
    match outcome? {
        SignalCandidateOutcome::Selected => Some(("ARCH", StyleId::BadgeStatusDone)),
        SignalCandidateOutcome::Deduplicated { .. } => Some(("DUP", StyleId::BadgeStatusMuted)),
        SignalCandidateOutcome::BelowThreshold => Some(("LOW", StyleId::BadgeStatusMuted)),
        SignalCandidateOutcome::Excluded => Some(("EXCL", StyleId::BadgeStatusMuted)),
    }
}

pub(super) fn job_status_style(job: &JobRowView) -> StyleId {
    match job_status_label(job) {
        "OK" | "Done" => StyleId::BadgeStatusDone,
        "ERR" => StyleId::BadgeStatusError,
        "Fetch" | "Queued" => StyleId::BadgeStatusActive,
        _ => StyleId::BadgeStatusMuted,
    }
}

pub(super) fn triage_review_status_label(job: &JobRowView) -> &'static str {
    match job.filter_status.as_ref() {
        Some(JobFilterStatus::HardExcluded { .. }) => "Auto Excluded",
        Some(JobFilterStatus::ReviewNeeded { .. }) => "Review",
        Some(JobFilterStatus::ManuallyExcluded) => "Excluded",
        Some(JobFilterStatus::ManuallyIncluded) => "Included",
        Some(JobFilterStatus::AutoIncluded) => "Included",
        None => "Review",
    }
}

pub(super) fn triage_review_status_style(job: &JobRowView) -> StyleId {
    match job.filter_status.as_ref() {
        Some(JobFilterStatus::HardExcluded { .. }) | Some(JobFilterStatus::ManuallyExcluded) => {
            StyleId::BadgeStatusMuted
        }
        Some(JobFilterStatus::ReviewNeeded { .. }) => StyleId::BadgeStatusActive,
        Some(JobFilterStatus::ManuallyIncluded) | Some(JobFilterStatus::AutoIncluded) => {
            StyleId::BadgeStatusDone
        }
        None => StyleId::BadgeStatusMuted,
    }
}

pub(super) fn triage_priority_style(job: &JobRowView) -> StyleId {
    // Triage prompt (crates/harvester_engine/src/llm/prompts/triage.rs) asks the
    // model for priority 1 (lowest) through 5 (highest/most urgent). We have four
    // badge styles, so P1 and P2 share the muted "Low" pill and P3..P5 each get
    // their own color to accelerate scan speed on the high-urgency tail.
    match job
        .triage_annotation
        .as_ref()
        .map(|triage| triage.priority)
        .unwrap_or_default()
    {
        0..=2 => StyleId::BadgePriorityLow,
        3 => StyleId::BadgePriorityMedium,
        4 => StyleId::BadgePriorityHigh,
        _ => StyleId::BadgePriorityCritical,
    }
}

pub(super) fn triage_priority_label(job: &JobRowView) -> String {
    let priority = job
        .triage_annotation
        .as_ref()
        .map(|triage| triage.priority)
        .unwrap_or_default();
    format!("P{priority}")
}

pub(super) fn job_source_label(job: &JobRowView) -> String {
    let domain = domain_from_url(&job.url);
    if domain.is_empty() {
        compact_url_label(&job.url, 32)
    } else {
        truncate_with_ellipsis(&domain, 32)
    }
}

pub(super) fn job_status_label(job: &JobRowView) -> &'static str {
    match &job.outcome {
        Some(JobResultKind::Success) => "OK",
        Some(JobResultKind::Failed { .. }) => "ERR",
        None => match job.stage {
            Stage::Queued => "Queued",
            Stage::Downloading => "Fetch",
            Stage::Sanitizing => "Clean",
            Stage::Converting => "Convert",
            Stage::Tokenizing => "Tokens",
            Stage::Writing => "Write",
            Stage::Done => "Done",
        },
    }
}

#[cfg(test)]
fn signal_candidate_score_style(score_band: harvester_core::ScoreBand) -> StyleId {
    match score_band {
        harvester_core::ScoreBand::High => StyleId::BadgeStatusDone,
        harvester_core::ScoreBand::Mid => StyleId::BadgeStatusActive,
        harvester_core::ScoreBand::Low => StyleId::BadgeStatusMuted,
    }
}

#[cfg(test)]
fn signal_candidate_state_label(state: &harvester_core::SignalCandidateRowState) -> &'static str {
    match state {
        harvester_core::SignalCandidateRowState::Scoring => "Scoring",
        harvester_core::SignalCandidateRowState::Scored => "Scored",
        harvester_core::SignalCandidateRowState::Failed { .. } => "Failed",
    }
}

#[cfg(test)]
fn signal_candidate_state_style(state: &harvester_core::SignalCandidateRowState) -> StyleId {
    match state {
        harvester_core::SignalCandidateRowState::Scoring => StyleId::BadgeStatusActive,
        harvester_core::SignalCandidateRowState::Scored => StyleId::BadgeStatusDone,
        harvester_core::SignalCandidateRowState::Failed { .. } => StyleId::BadgeStatusError,
    }
}

#[cfg(test)]
mod signal_candidate_item_tests {
    use super::*;
    use harvester_core::{
        ScoreBand, SignalCandidateOutcome, SignalCandidateRow, SignalCandidateRowState,
    };
    use harvester_engine::llm::dto::SourceTier;

    fn row(outcome: Option<SignalCandidateOutcome>) -> SignalCandidateRow {
        SignalCandidateRow {
            job_id: 1,
            url: "https://example.com/x".to_string(),
            score: 80,
            score_band: ScoreBand::High,
            source_tier: SourceTier::Tier1,
            themes: vec!["silicon".to_string()],
            gist_truncated: "Some gist".to_string(),
            dupes_count: 0,
            state_label: SignalCandidateRowState::Scored,
            signal_key: "k".to_string(),
            outcome,
        }
    }

    #[test]
    fn selected_row_is_enabled_with_arch_badge() {
        let item = build_signal_candidate_item(&row(Some(SignalCandidateOutcome::Selected)));
        assert!(item.enabled);
        assert_eq!(item.badges.first().map(|b| b.text.as_str()), Some("ARCH"));
    }

    #[test]
    fn deduped_row_is_dimmed_and_shows_kept_article() {
        let item = build_signal_candidate_item(&row(Some(SignalCandidateOutcome::Deduplicated {
            kept_gist: "Apple unveils M5".to_string(),
        })));
        assert!(
            !item.enabled,
            "cut rows are disabled (dimmed) but still selectable"
        );
        assert_eq!(item.badges.first().map(|b| b.text.as_str()), Some("DUP"));
        assert!(item.metadata.contains("→ kept: Apple unveils M5"));
    }

    #[test]
    fn below_threshold_and_excluded_rows_are_dimmed() {
        let low = build_signal_candidate_item(&row(Some(SignalCandidateOutcome::BelowThreshold)));
        assert!(!low.enabled);
        assert_eq!(low.badges.first().map(|b| b.text.as_str()), Some("LOW"));

        let excl = build_signal_candidate_item(&row(Some(SignalCandidateOutcome::Excluded)));
        assert!(!excl.enabled);
        assert_eq!(excl.badges.first().map(|b| b.text.as_str()), Some("EXCL"));
    }

    #[test]
    fn scoring_and_failed_rows_stay_enabled_without_outcome_badge() {
        let mut scoring = row(None);
        scoring.state_label = SignalCandidateRowState::Scoring;
        let item = build_signal_candidate_item(&scoring);
        assert!(item.enabled, "in-progress rows are not dimmed");
        assert_eq!(item.badges.first().map(|b| b.text.as_str()), Some("80"));

        let mut failed = row(None);
        failed.state_label = SignalCandidateRowState::Failed {
            reason: "oops".to_string(),
        };
        let item = build_signal_candidate_item(&failed);
        assert!(item.enabled, "failed rows are not cut outcomes");
        assert_eq!(item.badges.first().map(|b| b.text.as_str()), Some("80"));
    }
}
