use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::article_index::{ArticleEntry, ArticleIndex};
use harvester_core::{EntityIndex, EntityIndexEntry, SummaryCacheEntry};
use harvester_engine::llm::{
    ChatMessage, ChatRole, LlmError, LlmProvider, LlmRequest, ModelId, ProviderKind,
};
use harvester_engine::{TokenCounter, WhitespaceTokenCounter};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const MAX_EXPANSION_PATTERNS: usize = 3;
const MAX_EXPANSION_ENTITIES: usize = 5;
const MAX_KEY_FACTS: usize = 2;
const SCORING_CONCURRENCY: usize = 4;

#[derive(Clone)]
pub struct SmartQueryEngine {
    article_index: Arc<ArticleIndex>,
    entity_index: Arc<EntityIndex>,
    summary_index: Arc<HashMap<String, SummaryCacheEntry>>,
    provider: Option<Arc<dyn LlmProvider>>,
    agent_model: ModelId,
    context_budget: usize,
}

#[derive(Debug, Clone)]
pub struct QueryKnowledgeBaseInput {
    pub question: String,
    pub max_results: usize,
    pub scope_entities: Vec<String>,
    pub scope_date_from: Option<String>,
    pub scope_date_to: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryKnowledgeBaseResponse {
    pub mode: String,
    pub question: String,
    pub synthesis: Option<String>,
    pub ranked_articles: Vec<RankedArticleDigest>,
    pub warnings: Vec<String>,
    pub total_token_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankedArticleDigest {
    pub filename: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub fetched_utc: Option<String>,
    pub relevance_score: Option<u8>,
    pub key_facts: Vec<String>,
    pub excerpt: String,
    pub matched_patterns: Vec<String>,
    pub matched_entities: Vec<String>,
}

#[derive(Debug, Clone)]
struct QueryExpansion {
    regex_patterns: Vec<String>,
    entity_names: Vec<String>,
    date_from: Option<String>,
    date_to: Option<String>,
}

#[derive(Debug, Clone)]
struct CandidateArticle {
    filename: String,
    title: Option<String>,
    url: Option<String>,
    fetched_utc: Option<String>,
    snippet: String,
    summary: Option<String>,
    key_points: Vec<String>,
    matched_patterns: Vec<String>,
    matched_entities: Vec<String>,
}

#[derive(Debug, Clone)]
struct ScoredCandidate {
    candidate: CandidateArticle,
    relevance_score: u8,
    key_facts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct QueryExpansionResponse {
    #[serde(default)]
    regex_patterns: Vec<String>,
    #[serde(default)]
    entity_names: Vec<String>,
    date_from: Option<String>,
    date_to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RelevanceScoreResponse {
    relevance_score: u8,
    #[serde(default)]
    key_facts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DigestAssemblyResponse {
    synthesis: String,
}

impl SmartQueryEngine {
    pub fn new(
        article_index: Arc<ArticleIndex>,
        entity_index: Arc<EntityIndex>,
        summary_index: Arc<HashMap<String, SummaryCacheEntry>>,
        provider: Option<Arc<dyn LlmProvider>>,
        agent_model: impl Into<String>,
        context_budget: usize,
    ) -> Self {
        Self {
            article_index,
            entity_index,
            summary_index,
            provider,
            agent_model: ModelId::new(ProviderKind::OpenAi, agent_model.into()),
            context_budget,
        }
    }

    pub async fn query(&self, input: QueryKnowledgeBaseInput) -> QueryKnowledgeBaseResponse {
        match self.query_with_agent(&input).await {
            Ok(response) => self.enforce_context_budget(response),
            Err(err) => {
                engine_logging::engine_warn!(
                    "[smart-query] agent path unavailable, falling back to raw results: {}",
                    render_llm_error(&err)
                );
                let response = self.build_raw_fallback(
                    &input,
                    vec![format!(
                        "Smart-query agent unavailable; returned raw results instead ({})",
                        render_llm_error(&err)
                    )],
                );
                self.enforce_context_budget(response)
            }
        }
    }

    async fn query_with_agent(
        &self,
        input: &QueryKnowledgeBaseInput,
    ) -> Result<QueryKnowledgeBaseResponse, LlmError> {
        let provider = self
            .provider
            .clone()
            .ok_or_else(|| LlmError::Configuration {
                detail: "OPENAI_API_KEY missing; agent provider unavailable".to_string(),
            })?;

        let expansion = self.expand_query(provider.clone(), input).await?;
        let candidates = self.collect_candidates(input, &expansion);
        if candidates.is_empty() {
            return Ok(QueryKnowledgeBaseResponse {
                mode: "smart".to_string(),
                question: input.question.clone(),
                synthesis: Some(
                    "No matching articles were found in the current corpus.".to_string(),
                ),
                ranked_articles: Vec::new(),
                warnings: Vec::new(),
                total_token_count: 0,
            });
        }

        let scored = self
            .score_candidates(provider.clone(), input, candidates)
            .await?;
        let synthesis = self.assemble_digest(provider, input, &scored).await?;
        let ranked_articles = scored
            .into_iter()
            .take(input.max_results)
            .map(|item| RankedArticleDigest {
                filename: item.candidate.filename,
                title: item.candidate.title,
                url: item.candidate.url,
                fetched_utc: item.candidate.fetched_utc,
                relevance_score: Some(item.relevance_score),
                key_facts: item.key_facts,
                excerpt: item.candidate.summary.unwrap_or(item.candidate.snippet),
                matched_patterns: item.candidate.matched_patterns,
                matched_entities: item.candidate.matched_entities,
            })
            .collect();

        Ok(QueryKnowledgeBaseResponse {
            mode: "smart".to_string(),
            question: input.question.clone(),
            synthesis: Some(synthesis),
            ranked_articles,
            warnings: Vec::new(),
            total_token_count: 0,
        })
    }

    async fn expand_query(
        &self,
        provider: Arc<dyn LlmProvider>,
        input: &QueryKnowledgeBaseInput,
    ) -> Result<QueryExpansion, LlmError> {
        let scope_text = if input.scope_entities.is_empty() {
            "none".to_string()
        } else {
            input.scope_entities.join(", ")
        };
        let user_prompt = format!(
            "Question: {}\nScope entities: {}\nScope date_from: {}\nScope date_to: {}\nReturn JSON with regex_patterns, entity_names, date_from, date_to. regex_patterns must be safe Rust regex strings and prefer (?i) case-insensitive patterns. Use null for absent dates.",
            input.question,
            scope_text,
            input.scope_date_from.as_deref().unwrap_or("null"),
            input.scope_date_to.as_deref().unwrap_or("null"),
        );
        engine_logging::engine_info!("[smart-query] expansion prompt: {}", user_prompt);

        let response = provider
            .complete(
                &LlmRequest::new(
                    self.agent_model.clone(),
                    vec![
                        ChatMessage::new(
                            ChatRole::System,
                            "You expand knowledge-base questions into retrieval hints. Return JSON only.",
                        ),
                        ChatMessage::new(ChatRole::User, user_prompt.clone()),
                    ],
                )
                .with_temperature(0.0)
                .with_max_output_tokens(250)
                .with_json_response(),
            )
            .await?;

        engine_logging::engine_info!(
            "[smart-query] expansion response tokens={} body={}",
            response.usage().total(),
            response.content()
        );

        let parsed: QueryExpansionResponse = parse_json_response(response.content())?;
        let mut regex_patterns = normalize_patterns(parsed.regex_patterns);
        if regex_patterns.is_empty() {
            regex_patterns = heuristic_patterns(&input.question);
        }

        let mut entity_names = normalize_terms(parsed.entity_names);
        for scope in normalize_terms(input.scope_entities.clone()) {
            push_unique(&mut entity_names, scope);
        }
        entity_names.truncate(MAX_EXPANSION_ENTITIES);

        Ok(QueryExpansion {
            regex_patterns,
            entity_names,
            date_from: parsed.date_from.or_else(|| input.scope_date_from.clone()),
            date_to: parsed.date_to.or_else(|| input.scope_date_to.clone()),
        })
    }

    fn collect_candidates(
        &self,
        input: &QueryKnowledgeBaseInput,
        expansion: &QueryExpansion,
    ) -> Vec<CandidateArticle> {
        let date_from = expansion
            .date_from
            .as_deref()
            .or(input.scope_date_from.as_deref());
        let date_to = expansion
            .date_to
            .as_deref()
            .or(input.scope_date_to.as_deref());
        let scope_entities = normalize_terms(input.scope_entities.clone());
        let regexes = compile_patterns(&expansion.regex_patterns);
        let mut candidates = HashMap::new();

        self.collect_regex_matches(
            &mut candidates,
            &regexes,
            &scope_entities,
            date_from,
            date_to,
        );
        self.collect_entity_matches(
            &mut candidates,
            &expansion.entity_names,
            &scope_entities,
            date_from,
            date_to,
        );

        let mut ranked: Vec<_> = candidates.into_values().collect();
        ranked.sort_by(|left, right| {
            match_score(right)
                .cmp(&match_score(left))
                .then_with(|| right.fetched_utc.cmp(&left.fetched_utc))
                .then_with(|| left.filename.cmp(&right.filename))
        });
        ranked
    }

    fn collect_regex_matches(
        &self,
        candidates: &mut HashMap<String, CandidateArticle>,
        regexes: &[(String, Regex)],
        scope_entities: &[String],
        date_from: Option<&str>,
        date_to: Option<&str>,
    ) {
        for entry in &self.article_index.articles {
            if !date_in_range(entry.fetched_utc.as_deref(), date_from, date_to) {
                continue;
            }
            let entity_entry = self.url_entity_entry(entry.url.as_deref());
            if !matches_scope_entities(entry, entity_entry, scope_entities) {
                continue;
            }

            let mut matched_patterns = Vec::new();
            let mut snippet = String::new();
            for (pattern, regex) in regexes {
                if regex.is_match(&entry.content) {
                    push_unique(&mut matched_patterns, pattern.clone());
                    if snippet.is_empty() {
                        snippet = build_snippet(&entry.content, regex);
                    }
                }
            }

            if matched_patterns.is_empty() {
                continue;
            }

            let candidate = candidates
                .entry(candidate_key(entry))
                .or_insert_with(|| self.make_candidate(entry, entity_entry, snippet.clone()));
            merge_strings(&mut candidate.matched_patterns, matched_patterns);
            if candidate.snippet.is_empty() {
                candidate.snippet = snippet;
            }
        }
    }

    fn collect_entity_matches(
        &self,
        candidates: &mut HashMap<String, CandidateArticle>,
        entity_names: &[String],
        scope_entities: &[String],
        date_from: Option<&str>,
        date_to: Option<&str>,
    ) {
        for entity_name in entity_names {
            for (url, entity_entry) in &self.entity_index.entries {
                if !entity_entry_matches(entity_entry, entity_name) {
                    continue;
                }
                let Some(article) = self.article_by_url(url) else {
                    continue;
                };
                if !date_in_range(article.fetched_utc.as_deref(), date_from, date_to) {
                    continue;
                }
                if !matches_scope_entities(article, Some(entity_entry), scope_entities) {
                    continue;
                }

                let candidate = candidates.entry(candidate_key(article)).or_insert_with(|| {
                    self.make_candidate(article, Some(entity_entry), String::new())
                });
                push_unique(&mut candidate.matched_entities, entity_name.clone());
            }
        }
    }

    fn make_candidate(
        &self,
        entry: &ArticleEntry,
        _entity_entry: Option<&EntityIndexEntry>,
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
        }
    }

    async fn score_candidates(
        &self,
        provider: Arc<dyn LlmProvider>,
        input: &QueryKnowledgeBaseInput,
        candidates: Vec<CandidateArticle>,
    ) -> Result<Vec<ScoredCandidate>, LlmError> {
        let semaphore = Arc::new(Semaphore::new(SCORING_CONCURRENCY));
        let mut set = JoinSet::new();

        for candidate in candidates {
            let permit =
                semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| LlmError::InvalidResponse {
                        detail: "scoring semaphore closed".to_string(),
                    })?;
            let provider = provider.clone();
            let model = self.agent_model.clone();
            let question = input.question.clone();
            set.spawn(async move {
                let _permit = permit;
                score_candidate(provider, model, &question, candidate).await
            });
        }

        let mut scored = Vec::new();
        while let Some(joined) = set.join_next().await {
            let item = joined.map_err(|err| LlmError::InvalidResponse {
                detail: format!("scoring task join failure: {err}"),
            })??;
            scored.push(item);
        }

        scored.sort_by(|left, right| {
            right
                .relevance_score
                .cmp(&left.relevance_score)
                .then_with(|| right.candidate.fetched_utc.cmp(&left.candidate.fetched_utc))
                .then_with(|| left.candidate.filename.cmp(&right.candidate.filename))
        });
        Ok(scored)
    }

    async fn assemble_digest(
        &self,
        provider: Arc<dyn LlmProvider>,
        input: &QueryKnowledgeBaseInput,
        scored: &[ScoredCandidate],
    ) -> Result<String, LlmError> {
        let mut article_block = String::new();
        for item in scored.iter().take(input.max_results) {
            let facts = if item.key_facts.is_empty() {
                item.candidate.key_points.join(" | ")
            } else {
                item.key_facts.join(" | ")
            };
            article_block.push_str(&format!(
                "filename: {}\nurl: {}\nscore: {}\nfacts: {}\n\n",
                item.candidate.filename,
                item.candidate.url.as_deref().unwrap_or("n/a"),
                item.relevance_score,
                facts
            ));
        }

        let user_prompt = format!(
            "Question: {}\nTop articles:\n{}\nReturn JSON with one field: synthesis. The synthesis should be a short paragraph that answers the question using bracketed filename citations like [article.md].",
            input.question, article_block
        );
        engine_logging::engine_info!("[smart-query] digest prompt: {}", user_prompt);

        let response = provider
            .complete(
                &LlmRequest::new(
                    self.agent_model.clone(),
                    vec![
                        ChatMessage::new(
                            ChatRole::System,
                            "You assemble concise grounded syntheses from ranked article evidence. Return JSON only.",
                        ),
                        ChatMessage::new(ChatRole::User, user_prompt),
                    ],
                )
                .with_temperature(0.0)
                .with_max_output_tokens(400)
                .with_json_response(),
            )
            .await?;

        engine_logging::engine_info!(
            "[smart-query] digest response tokens={} body={}",
            response.usage().total(),
            response.content()
        );

        let parsed: DigestAssemblyResponse = parse_json_response(response.content())?;
        Ok(parsed.synthesis)
    }

    fn build_raw_fallback(
        &self,
        input: &QueryKnowledgeBaseInput,
        warnings: Vec<String>,
    ) -> QueryKnowledgeBaseResponse {
        let expansion = QueryExpansion {
            regex_patterns: heuristic_patterns(&input.question),
            entity_names: normalize_terms(input.scope_entities.clone()),
            date_from: input.scope_date_from.clone(),
            date_to: input.scope_date_to.clone(),
        };

        let ranked_articles = self
            .collect_candidates(input, &expansion)
            .into_iter()
            .take(input.max_results.max(1))
            .map(|candidate| RankedArticleDigest {
                filename: candidate.filename,
                title: candidate.title,
                url: candidate.url,
                fetched_utc: candidate.fetched_utc,
                relevance_score: None,
                key_facts: candidate
                    .key_points
                    .into_iter()
                    .take(MAX_KEY_FACTS)
                    .collect(),
                excerpt: candidate.summary.unwrap_or(candidate.snippet),
                matched_patterns: candidate.matched_patterns,
                matched_entities: candidate.matched_entities,
            })
            .collect();

        QueryKnowledgeBaseResponse {
            mode: "raw_fallback".to_string(),
            question: input.question.clone(),
            synthesis: None,
            ranked_articles,
            warnings,
            total_token_count: 0,
        }
    }

    fn article_by_url(&self, url: &str) -> Option<&ArticleEntry> {
        self.article_index
            .articles
            .iter()
            .find(|entry| entry.url.as_deref() == Some(url))
    }

    fn url_entity_entry(&self, url: Option<&str>) -> Option<&EntityIndexEntry> {
        url.and_then(|item| self.entity_index.entries.get(item))
    }

    fn enforce_context_budget(
        &self,
        mut response: QueryKnowledgeBaseResponse,
    ) -> QueryKnowledgeBaseResponse {
        let mut trimmed = false;

        loop {
            let tokens = count_response_tokens(&response);
            if tokens as usize <= self.context_budget {
                response.total_token_count = tokens;
                return response;
            }

            if !response.ranked_articles.is_empty() {
                response.ranked_articles.pop();
                trimmed = true;
                continue;
            }

            if let Some(synthesis) = response.synthesis.clone() {
                let shortened = truncate_words(&synthesis);
                if shortened != synthesis {
                    response.synthesis = Some(shortened);
                    trimmed = true;
                    continue;
                }
            }

            response.total_token_count = tokens;
            if trimmed {
                push_unique(
                    &mut response.warnings,
                    format!(
                        "Response still exceeds context budget of {} tokens after trimming",
                        self.context_budget
                    ),
                );
            }
            return response;
        }
    }
}

async fn score_candidate(
    provider: Arc<dyn LlmProvider>,
    model: ModelId,
    question: &str,
    candidate: CandidateArticle,
) -> Result<ScoredCandidate, LlmError> {
    let evidence = candidate
        .summary
        .clone()
        .unwrap_or_else(|| candidate.snippet.clone());
    let user_prompt = format!(
        "Question: {}\nFilename: {}\nURL: {}\nEvidence: {}\nReturn JSON with relevance_score (0-10) and key_facts (0-2 concise strings).",
        question,
        candidate.filename,
        candidate.url.as_deref().unwrap_or("n/a"),
        evidence
    );
    engine_logging::engine_info!(
        "[smart-query] scoring prompt filename={} prompt={}",
        candidate.filename,
        user_prompt
    );

    let response = provider
        .complete(
            &LlmRequest::new(
                model,
                vec![
                    ChatMessage::new(
                        ChatRole::System,
                        "You score article relevance for a knowledge-base question. Return JSON only.",
                    ),
                    ChatMessage::new(ChatRole::User, user_prompt),
                ],
            )
            .with_temperature(0.0)
            .with_max_output_tokens(180)
            .with_json_response(),
        )
        .await?;

    engine_logging::engine_info!(
        "[smart-query] scoring response filename={} tokens={} body={}",
        candidate.filename,
        response.usage().total(),
        response.content()
    );

    let parsed: RelevanceScoreResponse = parse_json_response(response.content())?;
    Ok(ScoredCandidate {
        candidate,
        relevance_score: parsed.relevance_score.min(10),
        key_facts: parsed.key_facts.into_iter().take(MAX_KEY_FACTS).collect(),
    })
}

fn parse_json_response<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, LlmError> {
    let payload = extract_json_payload(text).ok_or_else(|| LlmError::InvalidResponse {
        detail: "response did not contain a JSON object".to_string(),
    })?;
    serde_json::from_str(payload).map_err(|err| LlmError::InvalidResponse {
        detail: format!("json parse failure: {err}"),
    })
}

fn extract_json_payload(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed);
    }

    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    (start < end).then_some(&trimmed[start..=end])
}

fn normalize_patterns(patterns: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for pattern in patterns {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            continue;
        }
        push_unique(&mut normalized, trimmed.to_string());
        if normalized.len() >= MAX_EXPANSION_PATTERNS {
            break;
        }
    }
    normalized
}

fn normalize_terms(terms: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for term in terms {
        let trimmed = term.trim();
        if trimmed.is_empty() {
            continue;
        }
        push_unique(&mut normalized, trimmed.to_string());
    }
    normalized
}

fn heuristic_patterns(question: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let terms = significant_terms(question);
    if !terms.is_empty() {
        push_unique(&mut patterns, format!("(?i){}", terms.join("|")));
    }
    if question.trim().len() >= 4 {
        push_unique(
            &mut patterns,
            format!("(?i){}", regex::escape(question.trim())),
        );
    }
    for term in terms {
        push_unique(&mut patterns, format!("(?i){}", regex::escape(&term)));
        if patterns.len() >= MAX_EXPANSION_PATTERNS {
            break;
        }
    }
    patterns
}

fn significant_terms(text: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "about", "after", "against", "among", "and", "are", "corpus", "does", "for", "from",
        "have", "into", "said", "says", "that", "the", "their", "there", "these", "this", "those",
        "what", "when", "where", "which", "who", "with", "will", "would", "your",
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
        push_unique(&mut terms, term.to_string());
    }
    terms
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

fn candidate_key(entry: &ArticleEntry) -> String {
    entry.url.clone().unwrap_or_else(|| entry.filename.clone())
}

fn date_in_range(
    fetched_utc: Option<&str>,
    date_from: Option<&str>,
    date_to: Option<&str>,
) -> bool {
    if date_from.is_none() && date_to.is_none() {
        return true;
    }
    match fetched_utc {
        None => false,
        Some(ts) => {
            if let Some(from) = date_from {
                if ts < from {
                    return false;
                }
            }
            if let Some(to) = date_to {
                if ts > to {
                    return false;
                }
            }
            true
        }
    }
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

fn count_response_tokens(response: &QueryKnowledgeBaseResponse) -> u32 {
    let mut copy = response.clone();
    copy.total_token_count = 0;
    let json = serde_json::to_string(&copy).unwrap_or_default();
    WhitespaceTokenCounter.count(&json)
}

fn truncate_words(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= 24 {
        return text.to_string();
    }
    format!("{}...", words[..words.len() / 2].join(" "))
}

fn match_score(candidate: &CandidateArticle) -> usize {
    candidate.matched_patterns.len() + candidate.matched_entities.len()
}

fn merge_strings(target: &mut Vec<String>, source: Vec<String>) {
    for value in source {
        push_unique(target, value);
    }
}

fn push_unique(target: &mut Vec<String>, value: String) {
    if !target.iter().any(|existing| existing == &value) {
        target.push(value);
    }
}

fn render_llm_error(err: &LlmError) -> String {
    match err {
        LlmError::Configuration { detail }
        | LlmError::InvalidResponse { detail }
        | LlmError::Network { detail } => detail.clone(),
        LlmError::QuotaExhausted { description } => description.clone(),
        LlmError::Http { status, body } => format!("http {}: {}", status, body),
        LlmError::RateLimited { retry_after_secs } => {
            format!("rate limited (retry_after_secs={retry_after_secs:?})")
        }
        LlmError::AuthenticationFailed => "authentication failed".to_string(),
        LlmError::Timeout => "request timed out".to_string(),
        LlmError::ContentFiltered => "provider content filter".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::{ArticleSummaryResult, EntityIndex, EntityIndexEntry, SummaryCacheEntry};
    use harvester_engine::llm::MockLlmProvider;

    fn sample_article(
        filename: &str,
        title: &str,
        url: &str,
        fetched_utc: &str,
        body: &str,
    ) -> ArticleEntry {
        ArticleEntry {
            filename: filename.to_string(),
            path: filename.into(),
            title: Some(title.to_string()),
            url: Some(url.to_string()),
            fetched_utc: Some(fetched_utc.to_string()),
            token_count: Some(100),
            content: body.to_string(),
        }
    }

    fn summary_entry(summary: &str) -> SummaryCacheEntry {
        SummaryCacheEntry {
            result: ArticleSummaryResult {
                title: "Summary title".to_string(),
                summary: summary.to_string(),
                key_points: vec!["Key point".to_string()],
                input_tokens: 10,
                output_tokens: 5,
                entities: Default::default(),
            },
            created_at_utc: "2026-04-11T16:45:00Z".to_string(),
        }
    }

    fn test_engine(
        provider: Option<Arc<dyn LlmProvider>>,
        context_budget: usize,
    ) -> SmartQueryEngine {
        let article_index = ArticleIndex {
            articles: vec![
                sample_article(
                    "alpha.md",
                    "Anthropic expands security testing",
                    "https://example.com/alpha",
                    "2026-04-11T16:45:55Z",
                    "# Alpha\nAnthropic discusses AI security and testing.",
                ),
                sample_article(
                    "beta.md",
                    "Market reaction to AI budgets",
                    "https://example.com/beta",
                    "2026-04-10T10:00:00Z",
                    "# Beta\nBudget pressure and enterprise AI ROI.",
                ),
            ],
        };
        let entity_index = EntityIndex {
            schema_version: 1,
            entries: vec![
                (
                    "https://example.com/alpha".to_string(),
                    EntityIndexEntry {
                        fetched_utc: Some("2026-04-11T16:45:55Z".to_string()),
                        content_hash: Some("hash-alpha".to_string()),
                        companies: vec!["Anthropic".to_string()],
                        technologies: vec!["AI security".to_string()],
                        products: vec![],
                        themes: vec!["cybersecurity".to_string()],
                    },
                ),
                (
                    "https://example.com/beta".to_string(),
                    EntityIndexEntry {
                        fetched_utc: Some("2026-04-10T10:00:00Z".to_string()),
                        content_hash: Some("hash-beta".to_string()),
                        companies: vec!["KPMG".to_string()],
                        technologies: vec![],
                        products: vec![],
                        themes: vec!["roi-metrics".to_string()],
                    },
                ),
            ]
            .into_iter()
            .collect(),
        };
        let summary_index = HashMap::from([(
            "https://example.com/alpha".to_string(),
            summary_entry("Anthropic says stronger model evaluations improve security testing."),
        )]);

        SmartQueryEngine::new(
            Arc::new(article_index),
            Arc::new(entity_index),
            Arc::new(summary_index),
            provider,
            "mock-model",
            context_budget,
        )
    }

    #[tokio::test]
    async fn smart_query_returns_scored_digest() {
        let provider = Arc::new(MockLlmProvider::new());
        provider.queue_json_success(
            r#"{"regex_patterns":["(?i)anthropic","(?i)security"],"entity_names":["Anthropic"],"date_from":null,"date_to":null}"#,
        );
        provider.queue_json_success(r#"{"relevance_score":9,"key_facts":["Anthropic discussed stronger security testing."]}"#);
        provider.queue_json_success(r#"{"synthesis":"Anthropic emphasized stronger security testing for AI systems [alpha.md]."}"#);

        let engine = test_engine(Some(provider.clone()), 500);
        let response = engine
            .query(QueryKnowledgeBaseInput {
                question: "What did Anthropic say about AI security?".to_string(),
                max_results: 5,
                scope_entities: vec!["Anthropic".to_string()],
                scope_date_from: None,
                scope_date_to: None,
            })
            .await;

        assert_eq!(response.mode, "smart");
        assert_eq!(response.ranked_articles.len(), 1);
        assert_eq!(response.ranked_articles[0].filename, "alpha.md");
        assert_eq!(response.ranked_articles[0].relevance_score, Some(9));
        assert!(response
            .synthesis
            .unwrap_or_default()
            .contains("[alpha.md]"));
        assert_eq!(provider.recorded_requests().len(), 3);
    }

    #[tokio::test]
    async fn smart_query_falls_back_when_provider_missing() {
        let engine = test_engine(None, 500);
        let response = engine
            .query(QueryKnowledgeBaseInput {
                question: "What did Anthropic say about AI security?".to_string(),
                max_results: 5,
                scope_entities: vec!["Anthropic".to_string()],
                scope_date_from: None,
                scope_date_to: None,
            })
            .await;

        assert_eq!(response.mode, "raw_fallback");
        assert!(!response.ranked_articles.is_empty());
        assert!(!response.warnings.is_empty());
    }

    #[tokio::test]
    async fn context_budget_trims_ranked_articles() {
        let engine = test_engine(None, 20);
        let response = engine
            .query(QueryKnowledgeBaseInput {
                question: "Tell me about AI security and ROI".to_string(),
                max_results: 5,
                scope_entities: Vec::new(),
                scope_date_from: None,
                scope_date_to: None,
            })
            .await;

        assert!(response.total_token_count <= 20);
        assert!(response.ranked_articles.len() <= 1);
    }
}
