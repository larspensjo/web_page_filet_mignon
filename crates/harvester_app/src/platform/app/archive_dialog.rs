use chrono::Utc;
use commanductui::types::{
    FormButtons, FormDialogDescriptor, FormField, FormFieldValue, FormFileExistsWarning, FormRow,
    FormTextValidation, MessageSeverity,
};
use harvester_core::{ArchiveTokenEstimates, SignalCandidateDialogDefault};
use std::path::PathBuf;

pub(super) const ARCHIVE_DIALOG_CONTEXT_PREFIX: &str = "archive:";
pub(super) const ARCHIVE_DIALOG_FILENAME_FIELD_ID: &str = "archive.basename";
pub(super) const ARCHIVE_DIALOG_USE_SUMMARIES_FIELD_ID: &str = "archive.use_summaries";
pub(super) const ARCHIVE_DIALOG_USE_SIGNAL_CANDIDATES_FIELD_ID: &str =
    "archive.use_signal_candidates";
pub(super) const ARCHIVE_DIALOG_SET_CHECKPOINT_FIELD_ID: &str = "archive.set_checkpoint";

fn archive_dialog_context_tag(request_id: u64) -> String {
    format!("{ARCHIVE_DIALOG_CONTEXT_PREFIX}{request_id}")
}

pub(super) fn parse_archive_dialog_request_id(context_tag: &str) -> Option<u64> {
    context_tag
        .strip_prefix(ARCHIVE_DIALOG_CONTEXT_PREFIX)
        .and_then(|raw| raw.parse::<u64>().ok())
}

pub(super) fn archive_field_text(
    field_values: &[FormFieldValue],
    field_id: &str,
) -> Option<String> {
    field_values.iter().find_map(|value| match value {
        FormFieldValue::Text {
            field_id: value_field_id,
            value,
        } if value_field_id == field_id => Some(value.clone()),
        _ => None,
    })
}

pub(super) fn archive_field_checked(
    field_values: &[FormFieldValue],
    field_id: &str,
) -> Option<bool> {
    field_values.iter().find_map(|value| match value {
        FormFieldValue::CheckBox {
            field_id: value_field_id,
            checked,
        } if value_field_id == field_id => Some(*checked),
        _ => None,
    })
}

fn format_archive_since_label(since_utc: Option<chrono::DateTime<Utc>>) -> Option<String> {
    since_utc.map(|since| {
        let now = Utc::now();
        let days = (now - since).num_days().max(0);
        format!("{} ({} days ago)", since.format("%Y-%m-%d"), days)
    })
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.0}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_archive_form_descriptor(
    request_id: u64,
    article_count: usize,
    since_utc: Option<chrono::DateTime<Utc>>,
    default_basename: String,
    _default_file_exists: bool,
    export_dir: PathBuf,
    pending_pre_triage_count: usize,
    token_estimates: ArchiveTokenEstimates,
    signal_candidate_default: SignalCandidateDialogDefault,
    signal_candidate_count: usize,
    signal_candidate_scoring_done: u32,
    signal_candidate_scoring_total: u32,
    signal_candidate_token_estimates: ArchiveTokenEstimates,
) -> FormDialogDescriptor {
    let mut rows = Vec::new();
    let articles_label = if since_utc.is_some() {
        format!("{article_count} URLs (since checkpoint)")
    } else {
        format!("{article_count} URLs (all)")
    };
    rows.push(FormRow::ReadOnlyText {
        label: "Articles".to_string(),
        value: articles_label,
    });
    if let Some(checkpoint) = format_archive_since_label(since_utc) {
        rows.push(FormRow::ReadOnlyText {
            label: "Checkpoint".to_string(),
            value: checkpoint,
        });
    }
    rows.push(FormRow::ReadOnlyText {
        label: "Up to".to_string(),
        value: Utc::now().format("%Y-%m-%d %H:%M UTC").to_string(),
    });
    rows.push(FormRow::ReadOnlyText {
        label: "Full archive".to_string(),
        value: format!(
            "~{} tokens ({} articles)",
            format_tokens(token_estimates.full_tokens),
            article_count,
        ),
    });
    rows.push(FormRow::ReadOnlyText {
        label: "Summary archive".to_string(),
        value: format!(
            "~{} tokens ({}/{} with summaries)",
            format_tokens(token_estimates.summary_tokens),
            token_estimates.summary_coverage,
            article_count,
        ),
    });
    let (signal_candidate_notice, signal_candidate_notice_severity) = match signal_candidate_default {
        SignalCandidateDialogDefault::OnAllSettled => (
            format!("{signal_candidate_count} candidates selected (threshold + dedup)"),
            MessageSeverity::Information,
        ),
        SignalCandidateDialogDefault::OffPartial => (
            format!(
                "Scoring in progress ({signal_candidate_scoring_done}/{signal_candidate_scoring_total}). Toggle ON to export only settled candidates ({signal_candidate_count} selected)."
            ),
            MessageSeverity::Warning,
        ),
        SignalCandidateDialogDefault::OffEmpty => (
            format!(
                "No candidates above threshold ({signal_candidate_scoring_total} scored). Lower threshold or toggle off to export the full triage set."
            ),
            MessageSeverity::Warning,
        ),
        SignalCandidateDialogDefault::OffDisabled => (
            "No candidates settled yet - defaulting to full triage set.".to_string(),
            MessageSeverity::Warning,
        ),
    };
    rows.push(FormRow::Note {
        text: signal_candidate_notice,
        severity: signal_candidate_notice_severity,
    });
    rows.push(FormRow::ReadOnlyText {
        label: "Signal candidate full archive".to_string(),
        value: format!(
            "~{} tokens ({} articles)",
            format_tokens(signal_candidate_token_estimates.full_tokens),
            signal_candidate_count,
        ),
    });
    rows.push(FormRow::ReadOnlyText {
        label: "Signal candidate summary archive".to_string(),
        value: format!(
            "~{} tokens ({} articles with summaries)",
            format_tokens(signal_candidate_token_estimates.summary_tokens),
            signal_candidate_token_estimates.summary_coverage,
        ),
    });
    if article_count == 0 {
        rows.push(FormRow::Note {
            text: "No articles match the current filter.".to_string(),
            severity: MessageSeverity::Warning,
        });
    }
    if pending_pre_triage_count > 0 {
        rows.push(FormRow::Note {
            text: format!(
                "{} article{} await triage and are not included in this export.",
                pending_pre_triage_count,
                if pending_pre_triage_count == 1 {
                    ""
                } else {
                    "s"
                }
            ),
            severity: MessageSeverity::Warning,
        });
    }

    FormDialogDescriptor {
        title: "Archive Export".to_string(),
        context_tag: archive_dialog_context_tag(request_id),
        rows,
        fields: vec![
            FormField::TextInput {
                field_id: ARCHIVE_DIALOG_FILENAME_FIELD_ID.to_string(),
                label: "Output file".to_string(),
                value: default_basename,
                validation: FormTextValidation::PathSegment,
                live_warning: Some(FormFileExistsWarning {
                    base_dir: export_dir,
                    message: "file already exists - will be overwritten".to_string(),
                }),
            },
            FormField::CheckBox {
                field_id: ARCHIVE_DIALOG_USE_SUMMARIES_FIELD_ID.to_string(),
                label: "Use summaries (recommended)".to_string(),
                checked: true,
            },
            FormField::CheckBox {
                field_id: ARCHIVE_DIALOG_USE_SIGNAL_CANDIDATES_FIELD_ID.to_string(),
                label: "Use signal-candidate selection".to_string(),
                checked: matches!(
                    signal_candidate_default,
                    SignalCandidateDialogDefault::OnAllSettled
                ),
            },
            FormField::CheckBox {
                field_id: ARCHIVE_DIALOG_SET_CHECKPOINT_FIELD_ID.to_string(),
                label: "Set checkpoint to now after export".to_string(),
                checked: true,
            },
        ],
        buttons: FormButtons {
            confirm_label: "Export".to_string(),
            cancel_label: "Cancel".to_string(),
            confirm_enabled: article_count > 0,
        },
    }
}
