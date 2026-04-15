use std::collections::HashMap;

use super::types::QueryKnowledgeBaseInput;

pub(crate) fn build_refinement_suggestions(
    input: &QueryKnowledgeBaseInput,
    top_companies: &[String],
    top_themes: &[String],
    overlap_tags: &[(String, usize)],
) -> Vec<String> {
    let mut suggestions = Vec::new();

    let question_lower = input.question.to_lowercase();
    let asks_for_company_comparison = question_lower.contains("which companies")
        || question_lower.contains("what companies")
        || question_lower.contains("best positioned")
        || question_lower.contains("winners")
        || question_lower.contains("beneficiaries");
    let relationship_query = input.scope_entities.len() >= 2
        || question_lower.contains("partnership")
        || question_lower.contains("relationship")
        || question_lower.contains("collaboration")
        || question_lower.contains("between ");
    let overlap_examples = overlap_tags
        .iter()
        .filter(|(_, count)| *count >= 2)
        .map(|(tag, _)| humanize_tag(tag))
        .take(3)
        .collect::<Vec<_>>();
    let theme_examples = top_themes
        .iter()
        .map(|theme| humanize_tag(theme))
        .take(3)
        .collect::<Vec<_>>();

    if relationship_query {
        let examples = if !overlap_examples.is_empty() {
            overlap_examples.join(", ")
        } else {
            "contract terms, compute supply, data centers".to_string()
        };
        suggestions.push(format!(
            "Keep the entities fixed and focus on one relationship dimension, for example {examples}."
        ));
    } else if asks_for_company_comparison {
        let examples = if !overlap_examples.is_empty() {
            overlap_examples.join(", ")
        } else if !theme_examples.is_empty() {
            theme_examples.join(", ")
        } else {
            "chips, cloud, data centers".to_string()
        };
        suggestions.push(format!(
            "Focus on one infrastructure layer or subtopic, for example {examples}."
        ));
        suggestions.push(
            "Compare one vendor class at a time, for example chipmakers, cloud providers, or data-center/power suppliers."
                .to_string(),
        );
    } else if !top_companies.is_empty() {
        suggestions.push(format!(
            "Narrow to a smaller company or vendor set, for example {}.",
            top_companies
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if input.scope_date_from.is_none() && input.scope_date_to.is_none() {
        suggestions.push(
            "Add a date range such as the last 30 or 90 days to reduce the candidate set."
                .to_string(),
        );
    } else {
        suggestions.push(
            "Tighten the date window further if you want a smaller, more comparable result set."
                .to_string(),
        );
    }

    if !relationship_query && input.scope_entities.is_empty() {
        suggestions.push(
            "Add explicit entities only if they materially narrow the topic; for saturated topics, prefer a date range or subtopic instead."
                .to_string(),
        );
    }

    suggestions.push(
        "Rerun with allow_broad=true only if you want a slower deep pass over a broad topic."
            .to_string(),
    );
    suggestions
}

pub(crate) fn top_terms(terms: impl IntoIterator<Item = String>, limit: usize) -> Vec<String> {
    ranked_term_counts(terms)
        .into_iter()
        .take(limit)
        .map(|(term, _)| term)
        .collect()
}

pub(crate) fn ranked_term_counts(terms: impl IntoIterator<Item = String>) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for term in terms {
        let normalized = term.trim();
        if normalized.is_empty() {
            continue;
        }
        *counts.entry(normalized.to_string()).or_insert(0) += 1;
    }

    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.to_lowercase().cmp(&right.0.to_lowercase()))
    });
    ranked
}

pub(crate) fn format_ranked_counts(counts: &[(String, usize)], limit: usize) -> String {
    counts
        .iter()
        .take(limit)
        .map(|(term, count)| format!("{term}:{count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn mid_band_tag_counts(
    counts: &[(String, usize)],
    min_count: usize,
    max_count: usize,
) -> Vec<(String, usize)> {
    counts
        .iter()
        .filter(|(_, count)| *count >= min_count && *count <= max_count)
        .cloned()
        .collect()
}

pub(crate) fn query_overlap_tag_counts(
    counts: &[(String, usize)],
    focus_terms: &[String],
    focus_phrases: &[String],
) -> Vec<(String, usize)> {
    let mut ranked = Vec::new();
    for (tag, count) in counts {
        if let Some(overlap_score) = tag_overlap_score(tag, focus_terms, focus_phrases) {
            ranked.push((tag.clone(), *count, overlap_score));
        }
    }
    ranked.sort_by(|left, right| {
        right
            .2
            .cmp(&left.2)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.0.to_lowercase().cmp(&right.0.to_lowercase()))
    });
    ranked
        .into_iter()
        .map(|(tag, count, _)| (tag, count))
        .collect()
}

fn tag_overlap_score(tag: &str, focus_terms: &[String], focus_phrases: &[String]) -> Option<usize> {
    let tag_lower = tag.to_lowercase();
    let tag_terms: Vec<String> = tag_lower
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|segment| segment.len() >= 3)
        .map(|segment| segment.to_string())
        .collect();
    let phrase_hits = focus_phrases
        .iter()
        .filter(|phrase| tag_lower.contains(phrase.as_str()))
        .count();
    let term_hits = tag_terms
        .iter()
        .filter(|tag_term| {
            focus_terms.iter().any(|focus_term| {
                focus_term.contains(tag_term.as_str()) || tag_term.contains(focus_term.as_str())
            })
        })
        .count();
    let overlap_score = phrase_hits * 100 + term_hits * 10;
    (overlap_score > 0).then_some(overlap_score)
}

fn humanize_tag(tag: &str) -> String {
    tag.replace('-', " ")
}
