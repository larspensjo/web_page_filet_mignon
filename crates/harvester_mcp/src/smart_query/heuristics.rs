use super::types::{QueryExpansion, QueryKnowledgeBaseInput, MAX_EXPANSION_ENTITIES};

pub(super) fn heuristic_query_expansion(input: &QueryKnowledgeBaseInput) -> QueryExpansion {
    let mut entity_names = super::expansion::normalize_terms(input.scope_entities.clone());
    entity_names.truncate(MAX_EXPANSION_ENTITIES);
    QueryExpansion {
        regex_patterns: heuristic_patterns(&input.question),
        entity_names,
        focus_terms: heuristic_focus_terms(&input.question),
        focus_phrases: heuristic_focus_phrases(&input.question),
        date_from: input.scope_date_from.clone(),
        date_to: input.scope_date_to.clone(),
    }
}

pub(super) fn heuristic_focus_terms(question: &str) -> Vec<String> {
    significant_terms(question)
        .into_iter()
        .map(|term| term.to_lowercase())
        .take(6)
        .collect()
}

pub(super) fn heuristic_focus_phrases(question: &str) -> Vec<String> {
    let lower = question.to_lowercase();
    let candidate_phrases = [
        "contract terms",
        "revenue sharing",
        "equity stake",
        "licensing rights",
        "cloud dependence",
        "azure reliance",
        "data center",
        "data centers",
        "data centre",
        "data centres",
        "competitive tensions",
        "competitive rivalry",
        "competition",
        "inference capacity",
        "inference",
        "capacity",
        "cloud infrastructure",
        "ai infrastructure",
        "power infrastructure",
        "gpu demand",
    ];

    let mut phrases = Vec::new();
    for phrase in candidate_phrases {
        if lower.contains(phrase) {
            super::push_unique(&mut phrases, phrase.to_string());
        }
    }
    phrases
}

pub(super) fn heuristic_patterns(question: &str) -> Vec<String> {
    use super::types::MAX_EXPANSION_PATTERNS;
    let mut patterns = Vec::new();
    for pattern in demand_growth_patterns(question) {
        super::push_unique(&mut patterns, pattern);
        if patterns.len() >= MAX_EXPANSION_PATTERNS {
            return patterns;
        }
    }
    let terms = significant_terms(question);
    if !terms.is_empty() {
        super::push_unique(&mut patterns, format!("(?i){}", terms.join("|")));
    }
    if question.trim().len() >= 4 {
        super::push_unique(
            &mut patterns,
            format!("(?i){}", regex::escape(question.trim())),
        );
    }
    for term in terms {
        super::push_unique(&mut patterns, format!("(?i){}", regex::escape(&term)));
        if patterns.len() >= MAX_EXPANSION_PATTERNS {
            break;
        }
    }
    patterns
}

fn demand_growth_patterns(question: &str) -> Vec<String> {
    let lower = question.to_lowercase();
    let mentions_ai = lower.contains(" ai")
        || lower.starts_with("ai ")
        || lower.contains("artificial intelligence");
    let mentions_demand = ["demand", "growth", "usage", "capacity", "scale", "prepared"]
        .iter()
        .any(|term| lower.contains(term));

    if !(mentions_ai && mentions_demand) {
        return Vec::new();
    }

    vec![
        "(?i)(capacity|compute|data\\s*-?center|infrastructure|power|grid|chips?|gpus?|tpus?|semiconductor|foundry)".to_string(),
        "(?i)(nvidia|tsmc|broadcom|amd|microsoft|alphabet|google|amazon|meta|oracle)".to_string(),
        "(?i)(demand|growth|adoption|usage|scaling|scale-up|scale up)".to_string(),
    ]
}

fn significant_terms(text: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "about",
        "after",
        "against",
        "among",
        "and",
        "are",
        "corpus",
        "does",
        "for",
        "from",
        "have",
        "into",
        "said",
        "says",
        "suppose",
        "that",
        "the",
        "their",
        "there",
        "these",
        "this",
        "those",
        "usage",
        "want",
        "what",
        "when",
        "where",
        "which",
        "who",
        "with",
        "will",
        "would",
        "your",
        "investigate",
        "best",
        "prepared",
        "meet",
        "increased",
        "companies",
    ];

    let mut terms = Vec::new();
    for term in text
        .split(|ch: char| !ch.is_alphanumeric() && ch != '-' && ch != '_')
        .filter(|item| item.len() >= 4)
    {
        let lowercase = term.to_lowercase();
        if STOPWORDS.contains(&lowercase.as_str()) {
            continue;
        }
        super::push_unique(&mut terms, term.to_string());
    }
    terms
}
