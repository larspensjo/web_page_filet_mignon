use std::collections::{HashMap, HashSet};

use harvester_core::EntityIndexEntry;
use regex::Regex;

use super::refinement;
use super::types::{
    CandidateArticle, CandidateSelection, QueryExpansion, QueryKnowledgeBaseInput, SmartQueryEngine,
};
use crate::article_index::ArticleEntry;

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
        let regexes = compile_patterns(&expansion.regex_patterns);
        let mut candidates = HashMap::new();

        let regex_match_count = self.collect_regex_matches(
            &mut candidates,
            &regexes,
            &scope_entities,
            date_from,
            date_to,
        );
        let entity_match_count = self.collect_entity_matches(
            &mut candidates,
            &expansion.entity_names,
            &scope_entities,
            date_from,
            date_to,
        );

        let total_unique_candidates = candidates.len();
        let mut ranked: Vec<_> = candidates
            .into_values()
            .filter(|candidate| candidate_is_eligible(candidate, self.min_triage_priority))
            .collect();
        ranked.sort_by(|left, right| {
            deterministic_match_score(right)
                .cmp(&deterministic_match_score(left))
                .then_with(|| right.fetched_utc.cmp(&left.fetched_utc))
                .then_with(|| left.filename.cmp(&right.filename))
        });
        let eligible_unique_candidates = ranked.len();
        let filtered_low_priority_candidates =
            total_unique_candidates.saturating_sub(eligible_unique_candidates);
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
            capped,
            top_companies,
            top_themes,
            sample_titles,
            tag_counts,
        }
    }

    pub(super) fn collect_regex_matches(
        &self,
        candidates: &mut HashMap<String, CandidateArticle>,
        regexes: &[(String, Regex)],
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
                        snippet = build_snippet(&entry.content, regex);
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

            let candidate = candidates
                .entry(candidate_key(entry))
                .or_insert_with(|| self.make_candidate(entry, entity_entry, snippet.clone()));
            super::merge_strings(&mut candidate.matched_patterns, matched_patterns);
            candidate.title_pattern_hits += title_pattern_hits;
            candidate.url_pattern_hits += url_pattern_hits;
            if candidate.snippet.is_empty() {
                candidate.snippet = snippet;
            }
        }
        matched_articles
    }

    pub(super) fn collect_entity_matches(
        &self,
        candidates: &mut HashMap<String, CandidateArticle>,
        entity_names: &[String],
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
                    self.make_candidate(article, Some(entity_entry), String::new())
                });
                super::push_unique(&mut candidate.matched_entities, entity_name.clone());
            }
        }
        matched_articles.len()
    }

    pub(super) fn make_candidate(
        &self,
        entry: &ArticleEntry,
        entity_entry: Option<&EntityIndexEntry>,
        snippet: String,
    ) -> CandidateArticle {
        let summary_entry = entry
            .url
            .as_ref()
            .and_then(|url| self.summary_index.get(url));
        let summary = summary_entry.map(|item| item.result.summary.clone());
        let key_points = summary_entry
            .map(|item| item.result.key_points.clone())
            .unwrap_or_default();
        let default_snippet = if !snippet.is_empty() {
            snippet
        } else {
            fallback_excerpt(&entry.content)
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

fn candidate_is_eligible(candidate: &CandidateArticle, min_triage_priority: u8) -> bool {
    candidate
        .triage_priority
        .map(|priority| priority >= min_triage_priority)
        .unwrap_or(false)
}

fn match_score(candidate: &CandidateArticle) -> usize {
    candidate.matched_patterns.len() + candidate.matched_entities.len()
}

fn deterministic_match_score(candidate: &CandidateArticle) -> usize {
    let mut score = 0usize;
    score += candidate.title_pattern_hits * 1_000;
    score += candidate.triage_priority.unwrap_or(0) as usize * 200;
    score += candidate.matched_entities.len() * 250;
    score += candidate.matched_patterns.len() * 50;
    score += candidate.url_pattern_hits * 25;
    if candidate.summary.is_some() {
        score += 10;
    }
    if !candidate.key_points.is_empty() {
        score += 5;
    }
    score += match_score(candidate);
    score
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

fn build_snippet(content: &str, regex: &Regex) -> String {
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
    ordered
        .into_iter()
        .map(|index| lines[index])
        .collect::<Vec<_>>()
        .join("\n")
}

fn fallback_excerpt(content: &str) -> String {
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("\n")
}
