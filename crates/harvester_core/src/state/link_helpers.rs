use super::LinkRecord;
use crate::view_model::{JobFilterStatus, LinkRowView};
use harvester_engine::truncate_to_char_boundary;
use url::Url;

const LINK_ROW_LIMIT: usize = 200;
const LINK_LABEL_MAX: usize = 80;
const LINK_LABEL_TRUNCATE_MARKER: &str = "…";

pub(super) fn normalize_extracted_link(link: &str) -> String {
    let trimmed = link.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if let Ok(mut parsed) = Url::parse(trimmed) {
        parsed.set_fragment(None);
        if let Some(port) = parsed.port() {
            let normalized_port = match parsed.scheme() {
                "http" if port == 80 => None,
                "https" if port == 443 => None,
                _ => Some(port),
            };
            let _ = parsed.set_port(normalized_port);
        }
        parsed.into()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn format_lab_triage_markdown(output_json: &str) -> String {
    use harvester_engine::llm::validation::validate_triage;

    match validate_triage(output_json) {
        Ok(result) => {
            let triage = crate::triage::ArticleTriageResult {
                category: result.category,
                priority: result.priority.value(),
                tags: result.tags,
                rationale: result.rationale,
                input_tokens: 0,
                output_tokens: 0,
            };
            let formatted = crate::preview::format_triage_for_preview(None, &triage);
            format!("*Prompt Lab preview*\n\n{formatted}")
        }
        Err(_) => format!("**\\[Lab Triage\\]**\n\n```json\n{output_json}\n```\n"),
    }
}

pub(super) fn format_lab_summary_markdown(output_json: &str) -> String {
    use harvester_engine::llm::validation::validate_summary;

    match validate_summary(output_json) {
        Ok(result) => {
            let kp_lines: String = result
                .key_points
                .iter()
                .map(|kp| format!("- {kp}\n"))
                .collect();
            format!(
                "# \\[Lab\\] {}\n\n{}\n\n**Key Points:**\n\n{}\n",
                result.title, result.summary, kp_lines
            )
        }
        Err(_) => format!("**\\[Lab Summary\\]**\n\n```json\n{output_json}\n```\n"),
    }
}

pub(super) fn format_lab_briefing_markdown(output_json: &str) -> String {
    format!("**\\[Lab Briefing\\]**\n\n```json\n{output_json}\n```\n")
}

pub(super) fn domain_from_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .find("://")
        .map(|pos| &trimmed[pos + 3..])
        .unwrap_or(trimmed);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    if host.is_empty() {
        trimmed.to_string()
    } else {
        host.to_string()
    }
}

pub(super) fn map_job_filter_status(entry: &crate::ArticleFilterEntry) -> JobFilterStatus {
    match entry.manual_decision {
        Some(crate::ManualDecision::Exclude) => JobFilterStatus::ManuallyExcluded,
        Some(crate::ManualDecision::Include) => JobFilterStatus::ManuallyIncluded,
        None => match entry.auto_verdict {
            crate::AutoVerdict::HardExclude => JobFilterStatus::HardExcluded {
                reasons: entry.reasons.clone(),
            },
            crate::AutoVerdict::Review => JobFilterStatus::ReviewNeeded {
                reasons: entry.reasons.clone(),
            },
            crate::AutoVerdict::Include => JobFilterStatus::AutoIncluded,
        },
    }
}

pub(super) fn build_link_rows(records: &[LinkRecord]) -> Vec<LinkRowView> {
    records
        .iter()
        .take(LINK_ROW_LIMIT)
        .map(|record| LinkRowView {
            index: record.index,
            url: record.url.clone(),
            label: link_label_for_record(record),
            kind: record.kind.clone(),
            download_state: record.download_state.clone(),
            age_suspect: record.age_estimate.is_some(),
        })
        .collect()
}

fn link_label_for_record(record: &LinkRecord) -> String {
    if let Some(text) = record
        .anchor_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        text.to_string()
    } else {
        truncate_link_url(&record.url)
    }
}

fn truncate_link_url(url: &str) -> String {
    if url.chars().count() <= LINK_LABEL_MAX {
        url.to_string()
    } else {
        let max_chars = LINK_LABEL_MAX
            .saturating_sub(LINK_LABEL_TRUNCATE_MARKER.len())
            .max(1);
        let truncated = truncate_to_char_boundary(url, max_chars);
        format!("{truncated}{LINK_LABEL_TRUNCATE_MARKER}")
    }
}