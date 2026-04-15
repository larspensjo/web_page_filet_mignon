use std::collections::{HashMap, HashSet};

use harvester_core::EntityIndexEntry;
use regex::Regex;

use super::refinement;
use super::types::{
    CandidateArticle, CandidateSelection, QueryExpansion, QueryKnowledgeBaseInput,
    SmartQueryEngine, DEFAULT_MIN_DETERMINISTIC_ADMISSION_SCORE,
};
use crate::article_index::ArticleEntry;
use crate::util;

const MAX_CANDIDATE_SNIPPET_CHARS: usize = 700;
const HIGH_SNIPPET_QUALITY_PENALTY: i32 = 360;
const MEDIUM_SNIPPET_QUALITY_PENALTY: i32 = 180;

impl SmartQueryEngine {
    pub(super) fn collect_candidates(
        &self,
        input: &QueryKnowledgeBaseInput,
        expansion: &QueryExpansion,
    ) -> CandidateSelection {
        let date_from = expansion
            .date_from
            .as_deref()
            .or(input.scope_date_from.as_deref());
        let date_to = expansion
            .date_to
            .as_deref()
            .or(input.scope_date_to.as_deref());
        let scope_entities = super::expansion::normalize_terms(input.scope_entities.clone());
        let admission_policy = build_admission_policy(input, expansion, &scope_entities);
        let regexes = compile_patterns(&expansion.regex_patterns);
        let mut candidates = HashMap::new();

        let regex_match_count = self.collect_regex_matches(
            &mut candidates,
            &regexes,
            &admission_policy,
            &scope_entities,
            date_from,
            date_to,
        );
        let entity_match_count = self.collect_entity_matches(
            &mut candidates,
            &expansion.entity_names,
            &admission_policy,
            &scope_entities,
            date_from,
            date_to,
        );

        let total_unique_candidates = candidates.len();
        let priority_eligible: Vec<_> = candidates
            .into_values()
            .filter(|candidate| candidate_is_priority_eligible(candidate, self.min_triage_priority))
            .collect();
        let filtered_low_priority_candidates =
            total_unique_candidates.saturating_sub(priority_eligible.len());
        let mut ranked: Vec<_> = priority_eligible
            .into_iter()
            .filter(|candidate| candidate_matches_admission_policy(candidate, &admission_policy))
            .filter(|candidate| {
                candidate_admission_score(candidate) >= DEFAULT_MIN_DETERMINISTIC_ADMISSION_SCORE
            })
            .collect();
        let filtered_admission_candidates =
            total_unique_candidates.saturating_sub(filtered_low_priority_candidates + ranked.len());
        ranked.sort_by(|left, right| {
            deterministic_match_score(right)
                .cmp(&deterministic_match_score(left))
                .then_with(|| right.fetched_utc.cmp(&left.fetched_utc))
                .then_with(|| left.filename.cmp(&right.filename))
        });
        let eligible_unique_candidates = ranked.len();
        let top_companies = refinement::top_terms(
            ranked
                .iter()
                .flat_map(|candidate| candidate.companies.iter().cloned()),
            5,
        );
        let top_themes = refinement::top_terms(
            ranked
                .iter()
                .flat_map(|candidate| candidate.themes.iter().cloned()),
            5,
        );
        let tag_counts = refinement::ranked_term_counts(
            ranked
                .iter()
                .flat_map(|candidate| candidate.triage_tags.iter().cloned()),
        );
        let sample_titles = ranked
            .iter()
            .filter_map(|candidate| candidate.title.clone())
            .take(5)
            .collect();
        let capped = ranked.len() > self.scoring_candidate_cap;
        ranked.truncate(self.scoring_candidate_cap);

        CandidateSelection {
            scoring_candidates: ranked.len(),
            candidates: ranked,
            regex_match_count,
            entity_match_count,
            total_unique_candidates,
            eligible_unique_candidates,
            filtered_low_priority_candidates,
            filtered_admission_candidates,
            capped,
            top_companies,
            top_themes,
            sample_titles,
            tag_counts,
        }
    }

    fn collect_regex_matches(
        &self,
        candidates: &mut HashMap<String, CandidateArticle>,
        regexes: &[(String, Regex)],
        admission_policy: &AdmissionPolicy,
        scope_entities: &[String],
        date_from: Option<&str>,
        date_to: Option<&str>,
    ) -> usize {
        let mut matched_articles = 0;
        for entry in &self.article_index.articles {
            if !crate::util::date_in_range(entry.fetched_utc.as_deref(), date_from, date_to) {
                continue;
            }
            let entity_entry = self.url_entity_entry(entry.url.as_deref());
            if !matches_scope_entities(entry, entity_entry, scope_entities) {
                continue;
            }

            let mut matched_patterns = Vec::new();
            let mut snippet = String::new();
            let mut snippet_quality_penalty = HIGH_SNIPPET_QUALITY_PENALTY;
            let mut title_pattern_hits = 0usize;
            let mut url_pattern_hits = 0usize;
            for (pattern, regex) in regexes {
                let content_match = regex.is_match(&entry.content);
                let title_match = entry
                    .title
                    .as_deref()
                    .map(|title| regex.is_match(title))
                    .unwrap_or(false);
                let url_match = entry
                    .url
                    .as_deref()
                    .map(|url| regex.is_match(url))
                    .unwrap_or(false);

                if content_match || title_match || url_match {
                    super::push_unique(&mut matched_patterns, pattern.clone());
                    if snippet.is_empty() {
                        let snippet_evidence = build_snippet(&entry.content, regex);
                        snippet = snippet_evidence.text;
                        snippet_quality_penalty = snippet_evidence.quality_penalty;
                    }
                    if title_match {
                        title_pattern_hits += 1;
                    }
                    if url_match {
                        url_pattern_hits += 1;
                    }
                }
            }

            if matched_patterns.is_empty() {
                continue;
            }
            matched_articles += 1;

            let candidate = candidates.entry(candidate_key(entry)).or_insert_with(|| {
                self.make_candidate(
                    entry,
                    entity_entry,
                    snippet.clone(),
                    snippet_quality_penalty,
                    admission_policy,
                )
            });
            super::merge_strings(&mut candidate.matched_patterns, matched_patterns);
            candidate.title_pattern_hits += title_pattern_hits;
            candidate.url_pattern_hits += url_pattern_hits;
            if candidate.snippet.is_empty()
                || snippet_quality_penalty < candidate.snippet_quality_penalty
            {
                candidate.snippet = snippet;
                candidate.snippet_quality_penalty = snippet_quality_penalty;
            }
        }
        matched_articles
    }

    fn collect_entity_matches(
        &self,
        candidates: &mut HashMap<String, CandidateArticle>,
        entity_names: &[String],
        admission_policy: &AdmissionPolicy,
        scope_entities: &[String],
        date_from: Option<&str>,
        date_to: Option<&str>,
    ) -> usize {
        let mut matched_articles = HashSet::new();
        for entity_name in entity_names {
            for (url, entity_entry) in &self.entity_index.entries {
                if !entity_entry_matches(entity_entry, entity_name) {
                    continue;
                }
                let Some(article) = self.article_by_url(url) else {
                    continue;
                };
                if !crate::util::date_in_range(article.fetched_utc.as_deref(), date_from, date_to) {
                    continue;
                }
                if !matches_scope_entities(article, Some(entity_entry), scope_entities) {
                    continue;
                }

                matched_articles.insert(candidate_key(article));
                let candidate = candidates.entry(candidate_key(article)).or_insert_with(|| {
                    self.make_candidate(
                        article,
                        Some(entity_entry),
                        String::new(),
                        HIGH_SNIPPET_QUALITY_PENALTY,
                        admission_policy,
                    )
                });
                super::push_unique(&mut candidate.matched_entities, entity_name.clone());
            }
        }
        matched_articles.len()
    }

    fn make_candidate(
        &self,
        entry: &ArticleEntry,
        entity_entry: Option<&EntityIndexEntry>,
        snippet: String,
        snippet_quality_penalty: i32,
        admission_policy: &AdmissionPolicy,
    ) -> CandidateArticle {
        let summary_entry = entry
            .url
            .as_ref()
            .and_then(|url| self.summary_index.get(url));
        let summary = summary_entry.map(|item| item.result.summary.clone());
        let key_points = summary_entry
            .map(|item| item.result.key_points.clone())
            .unwrap_or_default();
        let fallback_snippet = fallback_excerpt(&entry.content);
        let (default_snippet, snippet_quality_penalty) = if !snippet.is_empty() {
            (snippet, snippet_quality_penalty)
        } else {
            (fallback_snippet.text, fallback_snippet.quality_penalty)
        };
        let triage_priority = entry
            .url
            .as_ref()
            .and_then(|url| self.triage_index.get(url))
            .map(|triage| triage.priority);
        let triage_tags = entry
            .url
            .as_ref()
            .and_then(|url| self.triage_index.get(url))
            .map(|triage| triage.tags.clone())
            .unwrap_or_default();
        let match_haystack = build_match_haystack(
            entry,
            entity_entry,
            summary.as_deref(),
            &key_points,
            &triage_tags,
        );
        let query_entity_hits = count_unique_matches(&match_haystack, &admission_policy.entities);
        let focus_term_hits = count_unique_matches(&match_haystack, &admission_policy.focus_terms);
        let focus_phrase_hits =
            count_unique_matches(&match_haystack, &admission_policy.focus_phrases);
        let title_haystack = entry.title.as_deref().unwrap_or("").to_lowercase();
        let title_focus_term_hits =
            count_unique_matches(&title_haystack, &admission_policy.focus_terms);
        let title_focus_phrase_hits =
            count_unique_matches(&title_haystack, &admission_policy.focus_phrases);

        CandidateArticle {
            filename: entry.filename.clone(),
            title: entry.title.clone(),
            url: entry.url.clone(),
            fetched_utc: entry.fetched_utc.clone(),
            snippet: default_snippet,
            summary,
            key_points,
            matched_patterns: Vec::new(),
            matched_entities: Vec::new(),
            companies: entity_entry
                .map(|item| item.companies.clone())
                .unwrap_or_default(),
            themes: entity_entry
                .map(|item| item.themes.clone())
                .unwrap_or_default(),
            triage_tags,
            title_pattern_hits: 0,
            url_pattern_hits: 0,
            query_entity_hits,
            focus_term_hits,
            focus_phrase_hits,
            title_focus_term_hits,
            title_focus_phrase_hits,
            snippet_quality_penalty,
            triage_priority,
        }
    }

    pub(super) fn article_by_url(&self, url: &str) -> Option<&ArticleEntry> {
        self.article_index
            .articles
            .iter()
            .find(|entry| entry.url.as_deref() == Some(url))
    }

    pub(super) fn url_entity_entry(&self, url: Option<&str>) -> Option<&EntityIndexEntry> {
        url.and_then(|item| self.entity_index.entries.get(item))
    }
}

fn candidate_key(entry: &ArticleEntry) -> String {
    entry.url.clone().unwrap_or_else(|| entry.filename.clone())
}

fn candidate_is_priority_eligible(candidate: &CandidateArticle, min_triage_priority: u8) -> bool {
    candidate
        .triage_priority
        .map(|priority| priority >= min_triage_priority)
        .unwrap_or(false)
}

fn match_score(candidate: &CandidateArticle) -> usize {
    candidate.matched_patterns.len() + candidate.matched_entities.len()
}

fn deterministic_match_score(candidate: &CandidateArticle) -> i32 {
    let mut score = 0i32;
    score += candidate.title_pattern_hits as i32 * 500;
    score += candidate.triage_priority.unwrap_or(0) as i32 * 125;
    score += candidate.query_entity_hits as i32 * 90;
    score += candidate.title_focus_term_hits as i32 * 120;
    score += candidate.title_focus_phrase_hits as i32 * 260;
    score += candidate.focus_term_hits as i32 * 70;
    score += candidate.focus_phrase_hits as i32 * 220;
    score += candidate.matched_patterns.len() as i32 * 35;
    score += candidate.url_pattern_hits as i32 * 20;
    if candidate.summary.is_some() {
        score += 30;
    }
    if !candidate.key_points.is_empty() {
        score += 15;
    }
    score -= effective_snippet_quality_penalty(candidate);
    score += match_score(candidate) as i32;
    score
}

fn candidate_admission_score(candidate: &CandidateArticle) -> i32 {
    deterministic_match_score(candidate)
}

fn compile_patterns(patterns: &[String]) -> Vec<(String, Regex)> {
    patterns
        .iter()
        .filter_map(|pattern| {
            Regex::new(pattern)
                .or_else(|_| Regex::new(&format!("(?i){}", regex::escape(pattern))))
                .ok()
                .map(|regex| (pattern.clone(), regex))
        })
        .collect()
}

fn entity_entry_matches(entry: &EntityIndexEntry, query: &str) -> bool {
    let query_lower = query.to_lowercase();
    entry
        .companies
        .iter()
        .chain(entry.technologies.iter())
        .chain(entry.products.iter())
        .chain(entry.themes.iter())
        .any(|item| item.to_lowercase().contains(&query_lower))
}

fn matches_scope_entities(
    article: &ArticleEntry,
    entity_entry: Option<&EntityIndexEntry>,
    scope_entities: &[String],
) -> bool {
    if scope_entities.is_empty() {
        return true;
    }

    scope_entities.iter().any(|scope| {
        let scope_lower = scope.to_lowercase();
        article
            .title
            .as_deref()
            .map(|title| title.to_lowercase().contains(&scope_lower))
            .unwrap_or(false)
            || article
                .url
                .as_deref()
                .map(|url| url.to_lowercase().contains(&scope_lower))
                .unwrap_or(false)
            || article.content.to_lowercase().contains(&scope_lower)
            || entity_entry
                .map(|entry| entity_entry_matches(entry, scope))
                .unwrap_or(false)
    })
}

fn build_snippet(content: &str, regex: &Regex) -> SnippetEvidence {
    let lines: Vec<&str> = content.lines().collect();
    let mut included = HashSet::new();
    let mut ordered = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if !regex.is_match(line) {
            continue;
        }
        for idx in index.saturating_sub(1)..=((index + 1).min(lines.len().saturating_sub(1))) {
            if included.insert(idx) {
                ordered.push(idx);
            }
        }
        if ordered.len() >= 9 {
            break;
        }
    }

    ordered.sort_unstable();
    let compact = ordered
        .into_iter()
        .map(|index| util::compact_whitespace(lines[index]))
        .filter(|line| !util::is_low_signal_snippet_line(line))
        .collect::<Vec<_>>()
        .join(" ... ");
    SnippetEvidence {
        text: util::truncate_text_boundary(&compact, MAX_CANDIDATE_SNIPPET_CHARS),
        quality_penalty: assess_snippet_quality_penalty(&compact),
    }
}

fn fallback_excerpt(content: &str) -> SnippetEvidence {
    let compact = content
        .lines()
        .map(util::compact_whitespace)
        .filter(|line| !util::is_low_signal_snippet_line(line))
        .take(6)
        .collect::<Vec<_>>()
        .join(" ... ");
    SnippetEvidence {
        text: util::truncate_text_boundary(&compact, MAX_CANDIDATE_SNIPPET_CHARS),
        quality_penalty: assess_snippet_quality_penalty(&compact),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmissionMode {
    Broad,
    EntityScoped,
    Relationship,
}

#[derive(Debug, Clone)]
struct AdmissionPolicy {
    mode: AdmissionMode,
    entities: Vec<String>,
    focus_terms: Vec<String>,
    focus_phrases: Vec<String>,
}

fn build_admission_policy(
    input: &QueryKnowledgeBaseInput,
    expansion: &QueryExpansion,
    scope_entities: &[String],
) -> AdmissionPolicy {
    let relationship_query = is_relationship_query(input);
    let mut entities = if relationship_query && scope_entities.len() >= 2 {
        scope_entities.to_vec()
    } else if relationship_query {
        expansion
            .entity_names
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
    } else if !scope_entities.is_empty() {
        scope_entities.to_vec()
    } else {
        expansion
            .entity_names
            .iter()
            .take(1)
            .cloned()
            .collect::<Vec<_>>()
    };
    entities = normalize_needles(entities);

    let focus_terms = expansion
        .focus_terms
        .iter()
        .filter(|term| !overlaps_with_any_entity(term, &entities))
        .filter(|term| !relationship_query || !is_generic_relationship_dimension(term))
        .cloned()
        .collect::<Vec<_>>();
    let focus_phrases = expansion
        .focus_phrases
        .iter()
        .filter(|phrase| !overlaps_with_any_entity(phrase, &entities))
        .filter(|phrase| !relationship_query || !is_generic_relationship_dimension(phrase))
        .cloned()
        .collect::<Vec<_>>();
    let mode = if relationship_query && entities.len() >= 2 {
        AdmissionMode::Relationship
    } else if !entities.is_empty() {
        AdmissionMode::EntityScoped
    } else {
        AdmissionMode::Broad
    };

    AdmissionPolicy {
        mode,
        entities,
        focus_terms,
        focus_phrases,
    }
}

fn candidate_matches_admission_policy(
    candidate: &CandidateArticle,
    admission_policy: &AdmissionPolicy,
) -> bool {
    match admission_policy.mode {
        AdmissionMode::Broad => {
            if admission_policy.focus_terms.is_empty() && admission_policy.focus_phrases.is_empty()
            {
                true
            } else {
                candidate_has_focus_match(candidate)
            }
        }
        AdmissionMode::EntityScoped => {
            candidate.query_entity_hits >= 1
                && (!admission_policy.focus_terms.is_empty()
                    || !admission_policy.focus_phrases.is_empty())
                && candidate_has_focus_match(candidate)
        }
        AdmissionMode::Relationship => {
            candidate.query_entity_hits >= admission_policy.entities.len()
                && (!admission_policy.focus_terms.is_empty()
                    || !admission_policy.focus_phrases.is_empty())
                && candidate_has_focus_match(candidate)
        }
    }
}

fn candidate_has_focus_match(candidate: &CandidateArticle) -> bool {
    candidate.focus_term_hits > 0 || candidate.focus_phrase_hits > 0
}

fn is_relationship_query(input: &QueryKnowledgeBaseInput) -> bool {
    let question_lower = input.question.to_lowercase();
    input.scope_entities.len() >= 2
        || question_lower.contains("partnership")
        || question_lower.contains("relationship")
        || question_lower.contains("collaboration")
        || question_lower.contains("between ")
}

fn build_match_haystack(
    entry: &ArticleEntry,
    entity_entry: Option<&EntityIndexEntry>,
    summary: Option<&str>,
    key_points: &[String],
    triage_tags: &[String],
) -> String {
    let mut segments = Vec::new();
    if let Some(title) = &entry.title {
        segments.push(title.as_str());
    }
    if let Some(url) = &entry.url {
        segments.push(url.as_str());
    }
    segments.push(&entry.content);
    if let Some(summary) = summary {
        segments.push(summary);
    }
    for key_point in key_points {
        segments.push(key_point.as_str());
    }
    if let Some(entity_entry) = entity_entry {
        for item in entity_entry
            .companies
            .iter()
            .chain(entity_entry.technologies.iter())
            .chain(entity_entry.products.iter())
            .chain(entity_entry.themes.iter())
        {
            segments.push(item.as_str());
        }
    }
    for tag in triage_tags {
        segments.push(tag.as_str());
    }
    segments.join("\n").to_lowercase()
}

fn count_unique_matches(haystack: &str, needles: &[String]) -> usize {
    needles
        .iter()
        .filter(|needle| haystack.contains(needle.as_str()))
        .count()
}

fn normalize_needles(terms: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for term in terms {
        let trimmed = term.trim().to_lowercase();
        if trimmed.is_empty() {
            continue;
        }
        super::push_unique(&mut normalized, trimmed);
    }
    normalized
}

fn overlaps_with_any_entity(value: &str, entities: &[String]) -> bool {
    let value_lower = value.trim().to_lowercase();
    if value_lower.is_empty() {
        return false;
    }

    let value_terms = tokenize_for_overlap(&value_lower);
    entities.iter().any(|entity| {
        let entity_lower = entity.trim().to_lowercase();
        if entity_lower.is_empty() {
            return false;
        }
        if entity_lower.contains(&value_lower) || value_lower.contains(&entity_lower) {
            return true;
        }
        let entity_terms = tokenize_for_overlap(&entity_lower);
        !value_terms.is_empty()
            && value_terms
                .iter()
                .all(|term| entity_terms.iter().any(|entity_term| entity_term == term))
    })
}

fn tokenize_for_overlap(value: &str) -> Vec<String> {
    value
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|segment| segment.len() >= 3)
        .map(|segment| segment.to_string())
        .collect()
}

fn is_generic_relationship_dimension(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "relationship"
            | "relationships"
            | "partnership"
            | "partnerships"
            | "collaboration"
            | "collaborations"
            | "between"
            | "change"
            | "changes"
            | "changing"
            | "shift"
            | "shifts"
    )
}

#[derive(Debug, Clone)]
struct SnippetEvidence {
    text: String,
    quality_penalty: i32,
}

fn assess_snippet_quality_penalty(text: &str) -> i32 {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return HIGH_SNIPPET_QUALITY_PENALTY;
    }

    let total_chars: usize = lines.iter().map(|line| line.len()).sum();
    let noisy_chars: usize = lines
        .iter()
        .filter(|line| util::is_low_signal_snippet_line(line))
        .map(|line| line.len())
        .sum();
    let signal_lines = lines
        .iter()
        .filter(|line| !util::is_low_signal_snippet_line(line) && line.trim().len() >= 24)
        .count();

    if total_chars == 0 || signal_lines == 0 {
        return HIGH_SNIPPET_QUALITY_PENALTY;
    }
    if noisy_chars * 100 / total_chars >= 65 {
        return HIGH_SNIPPET_QUALITY_PENALTY;
    }
    if noisy_chars * 100 / total_chars >= 40 {
        return MEDIUM_SNIPPET_QUALITY_PENALTY;
    }
    0
}

fn effective_snippet_quality_penalty(candidate: &CandidateArticle) -> i32 {
    if candidate.summary.is_some() {
        candidate.snippet_quality_penalty / 2
    } else {
        candidate.snippet_quality_penalty
    }
}
