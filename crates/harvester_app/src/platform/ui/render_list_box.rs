use commanductui::{
    BadgeDescriptor, ListBoxItemDescriptor, ListBoxItemId, ListBoxRowDensity, PlatformCommand,
    StyleId, WindowId,
};
use harvester_core::{
    AppViewModel, JobFilterStatus, JobListScope, JobOrigin, JobResultKind, JobRowView, LeftTab,
    Stage,
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
    let widest_row = items
        .iter()
        .map(|item| {
            item.badges
                .iter()
                .enumerate()
                .map(|(index, badge)| {
                    let text_width = (badge.text.chars().count() as i32 * 8).max(24);
                    text_width
                        + LIST_BOX_BADGE_PAD_X * 2
                        + if index > 0 { LIST_BOX_BADGE_GAP } else { 0 }
                })
                .sum::<i32>()
        })
        .max()
        .unwrap_or(LIST_BOX_DEFAULT_BADGE_COLUMN_WIDTH);

    widest_row
        .saturating_add(LIST_BOX_BADGE_GAP * 2)
        .clamp(LIST_BOX_DEFAULT_BADGE_COLUMN_WIDTH, 280)
}

pub(super) fn build_list_box_items(view: &AppViewModel) -> Vec<ListBoxItemDescriptor> {
    let tab = view.left_pane.left_tab;
    let scope_filtered: Vec<&JobRowView> =
        if view.left_pane.job_list_scope == JobListScope::SinceCheckpoint {
            view.jobs.iter().filter(|j| j.is_since_checkpoint).collect()
        } else {
            view.jobs.iter().collect()
        };

    let mut sorted_buf: Vec<&JobRowView>;
    let jobs_iter: &[&JobRowView] =
        if matches!(tab, LeftTab::TriageResults) && view.triage_progress.is_none() {
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

    jobs_iter
        .iter()
        .map(|job| build_list_box_item(tab, job))
        .collect()
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
