use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::article_index::{ArticleEntry, ArticleIndex};
use harvester_core::{ArticleTriageResult, EntityIndex, EntityIndexEntry, SummaryCacheEntry};
use harvester_engine::llm::{
    ChatMessage, ChatRole, LlmError, LlmProvider, LlmRequest, ModelId, ProviderKind,
    OPENAI_MODEL_GPT_5_4_MINI, OPENAI_MODEL_GPT_5_4_NANO,
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
pub const DEFAULT_MAX_SCORING_CANDIDATES: usize = 10;
pub const DEFAULT_TOO_BROAD_THRESHOLD: usize = 100;
pub const DEFAULT_MIN_TRIAGE_PRIORITY: u8 = 2;
const EXPANSION_INITIAL_MAX_OUTPUT_TOKENS: u32 = 400;
const EXPANSION_RETRY_MAX_OUTPUT_TOKENS: u32 = 700;
const MID_BAND_TAG_MIN_COUNT: usize = 5;
const MID_BAND_TAG_MAX_COUNT: usize = 200;

#[derive(Clone)]
pub struct SmartQueryEngine {
    article_index: Arc<ArticleIndex>,
    entity_index: Arc<EntityIndex>,
    summary_index: Arc<HashMap<String, SummaryCacheEntry>>,
    triage_index: Arc<HashMap<String, ArticleTriageResult>>,
    provider: Option<Arc<dyn LlmProvider>>,
    agent_model: ModelId,
    expansion_model: ModelId,
    context_budget: usize,
    scoring_candidate_cap: usize,
    too_broad_threshold: usize,
    min_triage_priority: u8,
}

#[derive(Debug, Clone)]
pub struct SmartQueryOptions {
    pub agent_model: String,
    pub context_budget: usize,
    pub scoring_candidate_cap: usize,
    pub too_broad_threshold: usize,
    pub min_triage_priority: u8,
}

impl SmartQueryOptions {
    pub fn new(agent_model: impl Into<String>, context_budget: usize) -> Self {
        Self {
            agent_model: agent_model.into(),
            context_budget,
            scoring_candidate_cap: DEFAULT_MAX_SCORING_CANDIDATES,
            too_broad_threshold: DEFAULT_TOO_BROAD_THRESHOLD,
            min_triage_priority: DEFAULT_MIN_TRIAGE_PRIORITY,
        }
    }

    pub fn with_scoring_candidate_cap(mut self, scoring_candidate_cap: usize) -> Self {
        self.scoring_candidate_cap = scoring_candidate_cap;
        self
    }

    pub fn with_too_broad_threshold(mut self, too_broad_threshold: usize) -> Self {
        self.too_broad_threshold = too_broad_threshold;
        self
    }

    pub fn with_min_triage_priority(mut self, min_triage_priority: u8) -> Self {
        self.min_triage_priority = min_triage_priority;
        self
    }
}

#[derive(Debug, Clone)]
pub struct QueryKnowledgeBaseInput {
    pub question: String,
    pub max_results: usize,
    pub scope_entities: Vec<String>,
    pub scope_date_from: Option<String>,
    pub scope_date_to: Option<String>,
    pub allow_broad: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryKnowledgeBaseResponse {
    pub mode: String,
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub synthesis: Option<String>,
    pub ranked_articles: Vec<RankedArticleDigest>,
    pub warnings: Vec<String>,
    pub total_token_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_match_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filtered_low_priority_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refinement_suggestions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_companies: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_themes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_titles: Vec<String>,
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
    companies: Vec<String>,
    themes: Vec<String>,
    triage_tags: Vec<String>,
    title_pattern_hits: usize,
    url_pattern_hits: usize,
    triage_priority: Option<u8>,
}

#[derive(Debug, Clone)]
struct ScoredCandidate {
    candidate: CandidateArticle,
    relevance_score: u8,
    key_facts: Vec<String>,
}

#[derive(Debug, Clone)]
struct CandidateSelection {
    candidates: Vec<CandidateArticle>,
    regex_match_count: usize,
    entity_match_count: usize,
    total_unique_candidates: usize,
    eligible_unique_candidates: usize,
    filtered_low_priority_candidates: usize,
    scoring_candidates: usize,
    capped: bool,
    top_companies: Vec<String>,
    top_themes: Vec<String>,
    sample_titles: Vec<String>,
    tag_counts: Vec<(String, usize)>,
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
        triage_index: Arc<HashMap<String, ArticleTriageResult>>,
        provider: Option<Arc<dyn LlmProvider>>,
        options: SmartQueryOptions,
    ) -> Self {
        let agent_model_name = options.agent_model;
        Self {
            article_index,
            entity_index,
            summary_index,
            triage_index,
            provider,
            agent_model: ModelId::new(ProviderKind::OpenAi, agent_model_name.clone()),
            expansion_model: ModelId::new(
                ProviderKind::OpenAi,
                preferred_expansion_model_name(&agent_model_name),
            ),
            context_budget: options.context_budget,
            scoring_candidate_cap: options.scoring_candidate_cap,
            too_broad_threshold: options.too_broad_threshold,
            min_triage_priority: options.min_triage_priority,
        }
    }

    pub async fn query(&self, input: QueryKnowledgeBaseInput) -> QueryKnowledgeBaseResponse {
        match self.query_with_agent(&input).await {
            Ok(response) => self.enforce_context_budget(response),
            Err(err) => {
                let fallback_reason = llm_error_code(&err);
                let detail = render_llm_error(&err);
                engine_logging::engine_warn!(
                    "[smart-query] agent path unavailable; fallback_reason={} detail={}",
                    fallback_reason,
                    detail
                );
                let response = self.build_raw_fallback(
                    &input,
                    vec![format!(
                        "Smart-query agent unavailable; returned raw results instead (reason={} detail={})",
                        fallback_reason,
                        detail
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

        let mut warnings = Vec::new();
        let expansion = match self.expand_query(provider.clone(), input).await {
            Ok(expansion) => expansion,
            Err(err) => {
                let fallback_reason = llm_error_code(&err);
                let detail = render_llm_error(&err);
                engine_logging::engine_warn!(
                    "[smart-query] expansion fallback activated; reason={} detail={}",
                    fallback_reason,
                    detail
                );
                warnings.push(format!(
                    "Expansion agent unavailable; used heuristic expansion instead (reason={} detail={})",
                    fallback_reason, detail
                ));
                heuristic_query_expansion(input)
            }
        };
        let selection = self.collect_candidates(input, &expansion);
        engine_logging::engine_info!(
            "[smart-query] candidate selection regex_matches={} entity_matches={} total_unique_candidates={} eligible_unique_candidates={} filtered_low_priority_candidates={} scoring_candidates={} candidates_with_triage={} capped={} threshold={} allow_broad={}",
            selection.regex_match_count,
            selection.entity_match_count,
            selection.total_unique_candidates,
            selection.eligible_unique_candidates,
            selection.filtered_low_priority_candidates,
            selection.scoring_candidates,
            selection
                .candidates
                .iter()
                .filter(|candidate| candidate.triage_priority.is_some())
                .count(),
            selection.capped,
            self.too_broad_threshold,
            input.allow_broad
        );
        if !selection.tag_counts.is_empty() {
            engine_logging::engine_info!(
                "[smart-query] triage tag stats eligible_unique_candidates={} unique_tags={} top_tags={}",
                selection.eligible_unique_candidates,
                selection.tag_counts.len(),
                format_ranked_counts(&selection.tag_counts, 25)
            );
            let mid_band_tag_counts = mid_band_tag_counts(
                &selection.tag_counts,
                MID_BAND_TAG_MIN_COUNT,
                MID_BAND_TAG_MAX_COUNT,
            );
            if !mid_band_tag_counts.is_empty() {
                engine_logging::engine_info!(
                    "[smart-query] triage tag mid-band stats min_count={} max_count={} tags={}",
                    MID_BAND_TAG_MIN_COUNT,
                    MID_BAND_TAG_MAX_COUNT,
                    format_ranked_counts(&mid_band_tag_counts, 25)
                );
            }
            let query_overlap_tag_counts =
                query_overlap_tag_counts(&selection.tag_counts, &input.question);
            if !query_overlap_tag_counts.is_empty() {
                engine_logging::engine_info!(
                    "[smart-query] triage tag query-overlap stats query_terms={} tags={}",
                    format_query_terms(&input.question),
                    format_ranked_counts(&query_overlap_tag_counts, 25)
                );
            }
        }

        if selection.candidates.is_empty() {
            return Ok(QueryKnowledgeBaseResponse {
                mode: "smart".to_string(),
                question: input.question.clone(),
                message: Some(if selection.total_unique_candidates > 0 {
                    format!(
                        "The query matched articles, but all {} candidates were filtered out because they were untriaged or had triage priority {} or lower.",
                        selection.total_unique_candidates,
                        self.min_triage_priority.saturating_sub(1)
                    )
                } else {
                    "No matching articles were found in the current corpus.".to_string()
                }),
                synthesis: Some(if selection.total_unique_candidates > 0 {
                    "No matching high-priority articles were found in the current corpus."
                        .to_string()
                } else {
                    "No matching articles were found in the current corpus.".to_string()
                }),
                ranked_articles: Vec::new(),
                warnings,
                total_token_count: 0,
                candidate_count: Some(selection.eligible_unique_candidates),
                total_match_count: Some(selection.total_unique_candidates),
                filtered_low_priority_count: Some(selection.filtered_low_priority_candidates),
                threshold: Some(self.too_broad_threshold),
                refinement_suggestions: vec![
                    "Widen the time range or remove scope filters if you want more results."
                        .to_string(),
                    format!(
                        "Only articles with triage priority {} or higher are considered eligible.",
                        self.min_triage_priority
                    ),
                ],
                top_companies: selection.top_companies,
                top_themes: selection.top_themes,
                sample_titles: selection.sample_titles,
            });
        }

        if !input.allow_broad
            && self.too_broad_threshold > 0
            && selection.eligible_unique_candidates > self.too_broad_threshold
        {
            engine_logging::engine_warn!(
                "[smart-query] too broad early exit eligible_unique_candidates={} threshold={} top_companies={:?} top_themes={:?} sample_titles={:?}",
                selection.eligible_unique_candidates,
                self.too_broad_threshold,
                selection.top_companies,
                selection.top_themes,
                selection.sample_titles
            );
            return Ok(self.build_too_broad_response(input, warnings, selection));
        }

        let scored = self
            .score_candidates(provider.clone(), input, selection.candidates)
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
            message: None,
            synthesis: Some(synthesis),
            ranked_articles,
            warnings,
            total_token_count: 0,
            candidate_count: Some(selection.eligible_unique_candidates),
            total_match_count: Some(selection.total_unique_candidates),
            filtered_low_priority_count: Some(selection.filtered_low_priority_candidates),
            threshold: Some(self.too_broad_threshold),
            refinement_suggestions: Vec::new(),
            top_companies: Vec::new(),
            top_themes: Vec::new(),
            sample_titles: Vec::new(),
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

        let response = match self
            .request_expansion(
                provider.clone(),
                &user_prompt,
                EXPANSION_INITIAL_MAX_OUTPUT_TOKENS,
            )
            .await
        {
            Ok(response) => response,
            Err(err) if should_retry_empty_length_response(&err) => {
                engine_logging::engine_warn!(
                    "[smart-query] expansion retry activated after empty length response; model={} max_output_tokens={}",
                    self.expansion_model.model_name(),
                    EXPANSION_RETRY_MAX_OUTPUT_TOKENS
                );
                self.request_expansion(provider, &user_prompt, EXPANSION_RETRY_MAX_OUTPUT_TOKENS)
                    .await?
            }
            Err(err) => return Err(err),
        };

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

    async fn request_expansion(
        &self,
        provider: Arc<dyn LlmProvider>,
        user_prompt: &str,
        max_output_tokens: u32,
    ) -> Result<harvester_engine::llm::LlmResponse, LlmError> {
        engine_logging::engine_info!(
            "[smart-query] expansion request model={} max_output_tokens={}",
            self.expansion_model.model_name(),
            max_output_tokens
        );
        provider
            .complete(
                &LlmRequest::new(
                    self.expansion_model.clone(),
                    vec![
                        ChatMessage::new(
                            ChatRole::System,
                            "You expand knowledge-base questions into retrieval hints. Return JSON only.",
                        ),
                        ChatMessage::new(ChatRole::User, user_prompt.to_string()),
                    ],
                )
                .with_temperature(0.0)
                .with_max_output_tokens(max_output_tokens)
                .with_json_response(),
            )
            .await
    }

    fn collect_candidates(
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
        let scope_entities = normalize_terms(input.scope_entities.clone());
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
        let top_companies = top_terms(
            ranked
                .iter()
                .flat_map(|candidate| candidate.companies.iter().cloned()),
            5,
        );
        let top_themes = top_terms(
            ranked
                .iter()
                .flat_map(|candidate| candidate.themes.iter().cloned()),
            5,
        );
        let tag_counts = ranked_term_counts(
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

    fn collect_regex_matches(
        &self,
        candidates: &mut HashMap<String, CandidateArticle>,
        regexes: &[(String, Regex)],
        scope_entities: &[String],
        date_from: Option<&str>,
        date_to: Option<&str>,
    ) -> usize {
        let mut matched_articles = 0;
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
                    push_unique(&mut matched_patterns, pattern.clone());
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
            merge_strings(&mut candidate.matched_patterns, matched_patterns);
            candidate.title_pattern_hits += title_pattern_hits;
            candidate.url_pattern_hits += url_pattern_hits;
            if candidate.snippet.is_empty() {
                candidate.snippet = snippet;
            }
        }
        matched_articles
    }

    fn collect_entity_matches(
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
                if !date_in_range(article.fetched_utc.as_deref(), date_from, date_to) {
                    continue;
                }
                if !matches_scope_entities(article, Some(entity_entry), scope_entities) {
                    continue;
                }

                matched_articles.insert(candidate_key(article));
                let candidate = candidates.entry(candidate_key(article)).or_insert_with(|| {
                    self.make_candidate(article, Some(entity_entry), String::new())
                });
                push_unique(&mut candidate.matched_entities, entity_name.clone());
            }
        }
        matched_articles.len()
    }

    fn make_candidate(
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
        let citation_rows: Vec<(String, String)> = scored
            .iter()
            .take(input.max_results)
            .enumerate()
            .map(|(index, item)| (format!("C{}", index + 1), item.candidate.filename.clone()))
            .collect();
        let mut article_block = String::new();
        for ((citation_id, _filename), item) in citation_rows.iter().zip(scored.iter()) {
            let facts = if item.key_facts.is_empty() {
                item.candidate.key_points.join(" | ")
            } else {
                item.key_facts.join(" | ")
            };
            article_block.push_str(&format!(
                "citation_id: [{}]\nfilename: {}\nurl: {}\nscore: {}\nfacts: {}\n\n",
                citation_id,
                item.candidate.filename,
                item.candidate.url.as_deref().unwrap_or("n/a"),
                item.relevance_score,
                facts
            ));
        }

        let user_prompt = format!(
            "Question: {}\nTop articles:\n{}\nReturn JSON with one field: synthesis. The synthesis should be a short paragraph that answers the question using only the provided bracketed citation_id values like [C1] or [C2]. Do not invent, shorten, or alter citations.",
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
        Ok(expand_digest_citations(&parsed.synthesis, &citation_rows))
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
            .candidates
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
            message: None,
            synthesis: None,
            ranked_articles,
            warnings,
            total_token_count: 0,
            candidate_count: None,
            total_match_count: None,
            filtered_low_priority_count: None,
            threshold: None,
            refinement_suggestions: Vec::new(),
            top_companies: Vec::new(),
            top_themes: Vec::new(),
            sample_titles: Vec::new(),
        }
    }

    fn build_too_broad_response(
        &self,
        input: &QueryKnowledgeBaseInput,
        warnings: Vec<String>,
        selection: CandidateSelection,
    ) -> QueryKnowledgeBaseResponse {
        QueryKnowledgeBaseResponse {
            mode: "too_broad".to_string(),
            question: input.question.clone(),
            message: Some(format!(
                "The query matched {} eligible high-priority articles, which exceeds the threshold of {}. Refine the question or rerun with allow_broad=true.",
                selection.eligible_unique_candidates,
                self.too_broad_threshold
            )),
            synthesis: None,
            ranked_articles: Vec::new(),
            warnings,
            total_token_count: 0,
            candidate_count: Some(selection.eligible_unique_candidates),
            total_match_count: Some(selection.total_unique_candidates),
            filtered_low_priority_count: Some(selection.filtered_low_priority_candidates),
            threshold: Some(self.too_broad_threshold),
            refinement_suggestions: build_refinement_suggestions(
                &selection.top_companies,
                &selection.top_themes,
            ),
            top_companies: selection.top_companies,
            top_themes: selection.top_themes,
            sample_titles: selection.sample_titles,
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

fn heuristic_query_expansion(input: &QueryKnowledgeBaseInput) -> QueryExpansion {
    let mut entity_names = normalize_terms(input.scope_entities.clone());
    entity_names.truncate(MAX_EXPANSION_ENTITIES);
    QueryExpansion {
        regex_patterns: heuristic_patterns(&input.question),
        entity_names,
        date_from: input.scope_date_from.clone(),
        date_to: input.scope_date_to.clone(),
    }
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

fn expand_digest_citations(synthesis: &str, citation_rows: &[(String, String)]) -> String {
    let mut expanded = synthesis.to_string();
    for (citation_id, filename) in citation_rows {
        expanded = expanded.replace(&format!("[{}]", citation_id), &format!("[{}]", filename));
    }
    expanded
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
    for pattern in demand_growth_patterns(question) {
        push_unique(&mut patterns, pattern);
        if patterns.len() >= MAX_EXPANSION_PATTERNS {
            return patterns;
        }
    }
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

fn candidate_is_eligible(candidate: &CandidateArticle, min_triage_priority: u8) -> bool {
    candidate
        .triage_priority
        .map(|priority| priority >= min_triage_priority)
        .unwrap_or(false)
}

fn top_terms(terms: impl IntoIterator<Item = String>, limit: usize) -> Vec<String> {
    ranked_term_counts(terms)
        .into_iter()
        .take(limit)
        .map(|(term, _)| term)
        .collect()
}

fn ranked_term_counts(terms: impl IntoIterator<Item = String>) -> Vec<(String, usize)> {
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

fn format_ranked_counts(counts: &[(String, usize)], limit: usize) -> String {
    counts
        .iter()
        .take(limit)
        .map(|(term, count)| format!("{term}:{count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn mid_band_tag_counts(
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

fn query_overlap_tag_counts(counts: &[(String, usize)], query: &str) -> Vec<(String, usize)> {
    let query_terms = query_hint_terms(query);
    counts
        .iter()
        .filter(|(tag, _)| tag_overlaps_query_terms(tag, &query_terms))
        .cloned()
        .collect()
}

fn format_query_terms(query: &str) -> String {
    query_hint_terms(query).join(", ")
}

fn query_hint_terms(query: &str) -> Vec<String> {
    significant_terms(query)
        .into_iter()
        .map(|term| term.to_lowercase())
        .collect()
}

fn tag_overlaps_query_terms(tag: &str, query_terms: &[String]) -> bool {
    let tag_terms: Vec<String> = tag
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|segment| segment.len() >= 3)
        .map(|segment| segment.to_lowercase())
        .collect();

    tag_terms.iter().any(|tag_term| {
        query_terms
            .iter()
            .any(|query_term| query_term.contains(tag_term) || tag_term.contains(query_term))
    })
}

fn build_refinement_suggestions(top_companies: &[String], top_themes: &[String]) -> Vec<String> {
    let mut suggestions = Vec::new();
    if !top_companies.is_empty() {
        suggestions.push(format!(
            "Narrow to one company or vendor set, for example {}.",
            top_companies
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !top_themes.is_empty() {
        suggestions.push(format!(
            "Focus on one infrastructure layer, for example {}.",
            top_themes
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    suggestions.push(
        "Add a date range or explicit scope_entities filter to reduce the candidate set."
            .to_string(),
    );
    suggestions.push(
        "Rerun with allow_broad=true only if you want a slower deep pass over a broad topic."
            .to_string(),
    );
    suggestions
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

fn llm_error_code(err: &LlmError) -> String {
    match err {
        LlmError::Configuration { detail } => {
            if detail.contains("OPENAI_API_KEY") {
                "configuration_missing_api_key".to_string()
            } else {
                "configuration".to_string()
            }
        }
        LlmError::InvalidResponse { detail } => {
            if detail.contains("assistant refusal") {
                "invalid_response_refusal".to_string()
            } else if detail.contains("choice missing content") {
                "invalid_response_empty_content".to_string()
            } else if detail.contains("json parse failure") {
                "invalid_response_json_parse".to_string()
            } else {
                "invalid_response".to_string()
            }
        }
        LlmError::Network { .. } => "network_request_failed".to_string(),
        LlmError::QuotaExhausted { .. } => "quota_exhausted".to_string(),
        LlmError::Http { status, body } => {
            if *status == 400 && body.contains("unsupported_parameter") {
                "http_400_unsupported_parameter".to_string()
            } else {
                format!("http_{status}")
            }
        }
        LlmError::RateLimited { .. } => "rate_limited".to_string(),
        LlmError::AuthenticationFailed => "authentication_failed".to_string(),
        LlmError::Timeout => "timeout".to_string(),
        LlmError::ContentFiltered => "content_filtered".to_string(),
    }
}

fn should_retry_empty_length_response(err: &LlmError) -> bool {
    matches!(
        err,
        LlmError::InvalidResponse { detail }
            if detail.contains("choice missing content") && detail.contains("finish_reason=length")
    )
}

fn preferred_expansion_model_name(agent_model_name: &str) -> String {
    if agent_model_name.eq_ignore_ascii_case(OPENAI_MODEL_GPT_5_4_NANO) {
        OPENAI_MODEL_GPT_5_4_MINI.to_string()
    } else {
        agent_model_name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

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
        test_engine_with_model(provider, OPENAI_MODEL_GPT_5_4_NANO, context_budget)
    }

    fn test_engine_with_model(
        provider: Option<Arc<dyn LlmProvider>>,
        agent_model: &str,
        context_budget: usize,
    ) -> SmartQueryEngine {
        test_engine_with_articles(
            vec![
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
            EntityIndex {
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
            },
            HashMap::from([(
                "https://example.com/alpha".to_string(),
                summary_entry(
                    "Anthropic says stronger model evaluations improve security testing.",
                ),
            )]),
            HashMap::from([
                (
                    "https://example.com/alpha".to_string(),
                    ArticleTriageResult {
                        category: "security".to_string(),
                        priority: 3,
                        tags: vec!["ai-security".to_string()],
                        rationale: "high-value".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
                (
                    "https://example.com/beta".to_string(),
                    ArticleTriageResult {
                        category: "roi".to_string(),
                        priority: 1,
                        tags: vec!["roi".to_string()],
                        rationale: "low-priority".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
            ]),
            provider,
            agent_model,
            context_budget,
        )
    }

    fn test_engine_with_articles(
        articles: Vec<ArticleEntry>,
        entity_index: EntityIndex,
        summary_index: HashMap<String, SummaryCacheEntry>,
        triage_index: HashMap<String, ArticleTriageResult>,
        provider: Option<Arc<dyn LlmProvider>>,
        agent_model: &str,
        context_budget: usize,
    ) -> SmartQueryEngine {
        let article_index = ArticleIndex { articles };

        SmartQueryEngine::new(
            Arc::new(article_index),
            Arc::new(entity_index),
            Arc::new(summary_index),
            Arc::new(triage_index),
            provider,
            SmartQueryOptions {
                agent_model: agent_model.to_string(),
                context_budget,
                scoring_candidate_cap: DEFAULT_MAX_SCORING_CANDIDATES,
                too_broad_threshold: DEFAULT_TOO_BROAD_THRESHOLD,
                min_triage_priority: DEFAULT_MIN_TRIAGE_PRIORITY,
            },
        )
    }

    #[tokio::test]
    async fn smart_query_returns_scored_digest() {
        let provider = Arc::new(MockLlmProvider::new());
        provider.queue_json_success(
            r#"{"regex_patterns":["(?i)anthropic","(?i)security"],"entity_names":["Anthropic"],"date_from":null,"date_to":null}"#,
        );
        provider.queue_json_success(r#"{"relevance_score":9,"key_facts":["Anthropic discussed stronger security testing."]}"#);
        provider.queue_json_success(
            r#"{"synthesis":"Anthropic emphasized stronger security testing for AI systems [C1]."}"#,
        );

        let engine = test_engine(Some(provider.clone()), 500);
        let response = engine
            .query(QueryKnowledgeBaseInput {
                question: "What did Anthropic say about AI security?".to_string(),
                max_results: 5,
                allow_broad: false,
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
    }

    #[tokio::test]
    async fn smart_query_uses_heuristic_expansion_when_agent_expansion_fails() {
        let provider = Arc::new(MockLlmProvider::new());
        provider.queue_response(Err(LlmError::InvalidResponse {
            detail: "choice missing content (finish_reason=length)".to_string(),
        }));
        provider.queue_response(Err(LlmError::InvalidResponse {
            detail: "choice missing content (finish_reason=length)".to_string(),
        }));
        provider.queue_json_success(
            r#"{"relevance_score":9,"key_facts":["Anthropic discussed stronger security testing."]}"#,
        );
        provider.queue_json_success(
            r#"{"synthesis":"Anthropic emphasized stronger security testing for AI systems [C1]."}"#,
        );

        let engine = test_engine(Some(provider.clone()), 500);
        let response = engine
            .query(QueryKnowledgeBaseInput {
                question: "What did Anthropic say about AI security?".to_string(),
                max_results: 5,
                allow_broad: false,
                scope_entities: vec!["Anthropic".to_string()],
                scope_date_from: None,
                scope_date_to: None,
            })
            .await;

        assert_eq!(response.mode, "smart");
        assert_eq!(response.ranked_articles[0].filename, "alpha.md");
        assert!(response
            .warnings
            .iter()
            .any(|warning| warning.contains("used heuristic expansion instead")));
        assert!(response
            .synthesis
            .unwrap_or_default()
            .contains("[alpha.md]"));
    }

    #[tokio::test]
    async fn smart_query_handles_broad_match_sets_without_falling_back() {
        let mut articles = Vec::new();
        let mut triage_index = HashMap::new();
        for idx in 0..30 {
            let filename = if idx == 0 {
                "priority.md".to_string()
            } else {
                format!("article-{idx:02}.md")
            };
            let title = if idx == 0 {
                "Anthropic security bulletin".to_string()
            } else {
                format!("Background article {idx:02}")
            };
            let url = format!("https://example.com/{idx:02}");
            let fetched = format!("2026-04-{:02}T10:00:00Z", (idx % 28) + 1);
            let body = format!("# Article {idx}\nThis article mentions security in the body.");
            articles.push(sample_article(&filename, &title, &url, &fetched, &body));
            triage_index.insert(
                url,
                ArticleTriageResult {
                    category: "security".to_string(),
                    priority: 3,
                    tags: vec!["security".to_string()],
                    rationale: "eligible".to_string(),
                    input_tokens: 0,
                    output_tokens: 0,
                },
            );
        }

        let provider = Arc::new(MockLlmProvider::new());
        provider.queue_json_success(
            r#"{"regex_patterns":["(?i)security"],"entity_names":[],"date_from":null,"date_to":null}"#,
        );
        for _ in 0..DEFAULT_MAX_SCORING_CANDIDATES {
            provider.queue_json_success(
                r#"{"relevance_score":7,"key_facts":["The article discusses security."]}"#,
            );
        }
        provider.queue_json_success(r#"{"synthesis":"Broad security coverage appears in [C1]."}"#);

        let engine = test_engine_with_articles(
            articles,
            EntityIndex {
                schema_version: 1,
                entries: Default::default(),
            },
            HashMap::new(),
            triage_index,
            Some(provider),
            "mock-model",
            10_000,
        );

        let response = engine
            .query(QueryKnowledgeBaseInput {
                question: "What do the loaded articles say about security?".to_string(),
                max_results: 5,
                allow_broad: true,
                scope_entities: Vec::new(),
                scope_date_from: None,
                scope_date_to: None,
            })
            .await;

        assert_eq!(response.mode, "smart");
        assert!(response.warnings.is_empty());
        assert_eq!(response.ranked_articles.len(), 5);
        assert!(response
            .ranked_articles
            .iter()
            .all(|article| article.relevance_score == Some(7)));
        assert!(response.synthesis.unwrap_or_default().contains(".md]"));
        assert_eq!(response.candidate_count, Some(30));
        assert_eq!(response.filtered_low_priority_count, Some(0));
    }

    #[tokio::test]
    async fn smart_query_returns_too_broad_without_scoring_by_default() {
        let mut articles = Vec::new();
        let mut entity_entries = BTreeMap::new();
        let mut triage_index = HashMap::new();
        for idx in 0..120 {
            let filename = format!("vendor-{idx:03}.md");
            let title = format!("Vendor {idx:03} expands AI data center footprint");
            let url = format!("https://example.com/vendor-{idx:03}");
            let fetched = format!("2026-04-{:02}T10:00:00Z", (idx % 28) + 1);
            let body = "# Vendor\nCloud, chips, and data center expansion for AI demand.";
            articles.push(sample_article(&filename, &title, &url, &fetched, body));
            entity_entries.insert(
                url.clone(),
                EntityIndexEntry {
                    fetched_utc: Some(fetched),
                    content_hash: Some(format!("hash-{idx:03}")),
                    companies: vec![format!("Vendor {idx:03}")],
                    technologies: vec!["ai infrastructure".to_string()],
                    products: vec![],
                    themes: vec!["data centers".to_string()],
                },
            );
            triage_index.insert(
                url,
                ArticleTriageResult {
                    category: "infrastructure".to_string(),
                    priority: 3,
                    tags: vec!["capacity".to_string()],
                    rationale: "eligible".to_string(),
                    input_tokens: 0,
                    output_tokens: 0,
                },
            );
        }

        let provider = Arc::new(MockLlmProvider::new());
        provider.queue_json_success(
            r#"{"regex_patterns":["(?i)(data center|chip|cloud)"],"entity_names":[],"date_from":null,"date_to":null}"#,
        );

        let engine = test_engine_with_articles(
            articles,
            EntityIndex {
                schema_version: 1,
                entries: entity_entries,
            },
            HashMap::new(),
            triage_index,
            Some(provider),
            "mock-model",
            10_000,
        );

        let response = engine
            .query(QueryKnowledgeBaseInput {
                question: "Which companies are best positioned for AI infrastructure demand?"
                    .to_string(),
                max_results: 5,
                allow_broad: false,
                scope_entities: Vec::new(),
                scope_date_from: None,
                scope_date_to: None,
            })
            .await;

        assert_eq!(response.mode, "too_broad");
        assert!(response.ranked_articles.is_empty());
        assert!(response.synthesis.is_none());
        assert_eq!(response.candidate_count, Some(120));
        assert_eq!(response.threshold, Some(DEFAULT_TOO_BROAD_THRESHOLD));
        assert!(!response.refinement_suggestions.is_empty());
        assert!(!response.sample_titles.is_empty());
    }

    #[tokio::test]
    async fn smart_query_filters_untriaged_and_priority_one_articles() {
        let provider = Arc::new(MockLlmProvider::new());
        provider.queue_json_success(
            r#"{"regex_patterns":["(?i)security"],"entity_names":[],"date_from":null,"date_to":null}"#,
        );
        provider.queue_json_success(
            r#"{"relevance_score":8,"key_facts":["Only the high-priority article remains eligible."]}"#,
        );
        provider.queue_json_success(
            r#"{"synthesis":"Only one eligible article remained after priority filtering [C1]."}"#,
        );

        let engine = test_engine_with_articles(
            vec![
                sample_article(
                    "high.md",
                    "High priority security article",
                    "https://example.com/high",
                    "2026-04-12T10:00:00Z",
                    "# High\nSecurity response and mitigations.",
                ),
                sample_article(
                    "low.md",
                    "Low priority security article",
                    "https://example.com/low",
                    "2026-04-11T10:00:00Z",
                    "# Low\nSecurity response and mitigations.",
                ),
                sample_article(
                    "none.md",
                    "Untriaged security article",
                    "https://example.com/none",
                    "2026-04-10T10:00:00Z",
                    "# None\nSecurity response and mitigations.",
                ),
            ],
            EntityIndex {
                schema_version: 1,
                entries: Default::default(),
            },
            HashMap::new(),
            HashMap::from([
                (
                    "https://example.com/high".to_string(),
                    ArticleTriageResult {
                        category: "security".to_string(),
                        priority: 3,
                        tags: vec!["security".to_string()],
                        rationale: "eligible".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
                (
                    "https://example.com/low".to_string(),
                    ArticleTriageResult {
                        category: "security".to_string(),
                        priority: 1,
                        tags: vec!["security".to_string()],
                        rationale: "filtered".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
            ]),
            Some(provider),
            "mock-model",
            10_000,
        );

        let response = engine
            .query(QueryKnowledgeBaseInput {
                question: "What do the loaded articles say about security?".to_string(),
                max_results: 5,
                allow_broad: false,
                scope_entities: Vec::new(),
                scope_date_from: None,
                scope_date_to: None,
            })
            .await;

        assert_eq!(response.mode, "smart");
        assert_eq!(response.ranked_articles.len(), 1);
        assert_eq!(response.ranked_articles[0].filename, "high.md");
        assert_eq!(response.candidate_count, Some(1));
        assert_eq!(response.total_match_count, Some(3));
        assert_eq!(response.filtered_low_priority_count, Some(2));
    }

    #[tokio::test]
    async fn smart_query_heuristic_expansion_surfaces_supply_side_articles() {
        let provider = Arc::new(MockLlmProvider::new());
        provider.queue_response(Err(LlmError::InvalidResponse {
            detail: "choice missing content (finish_reason=length)".to_string(),
        }));
        provider.queue_response(Err(LlmError::InvalidResponse {
            detail: "choice missing content (finish_reason=length)".to_string(),
        }));
        provider.queue_json_success(
            r#"{"relevance_score":9,"key_facts":["TSMC is expanding semiconductor capacity for AI demand."]}"#,
        );
        provider.queue_json_success(
            r#"{"relevance_score":8,"key_facts":["Amazon is scaling Trainium and cloud infrastructure."]}"#,
        );
        provider.queue_json_success(
            r#"{"synthesis":"Supply-side AI readiness shows up most clearly in [C1] and [C2]."}"#,
        );

        let engine = test_engine_with_articles(
            vec![
                sample_article(
                    "tsmc.md",
                    "TSMC expands AI chip capacity",
                    "https://example.com/tsmc",
                    "2026-04-12T10:00:00Z",
                    "# TSMC\nTSMC is adding semiconductor and foundry capacity for AI chip demand.",
                ),
                sample_article(
                    "amazon.md",
                    "Amazon scales Trainium infrastructure",
                    "https://example.com/amazon",
                    "2026-04-11T10:00:00Z",
                    "# Amazon\nAmazon is expanding Trainium, data center, and cloud infrastructure for AI workloads.",
                ),
                sample_article(
                    "scribes.md",
                    "AI scribes in hospitals",
                    "https://example.com/scribes",
                    "2026-04-10T10:00:00Z",
                    "# Scribes\nHospital documentation workflows improved modestly.",
                ),
            ],
            EntityIndex {
                schema_version: 1,
                entries: Default::default(),
            },
            HashMap::new(),
            HashMap::from([
                (
                    "https://example.com/tsmc".to_string(),
                    ArticleTriageResult {
                        category: "infrastructure".to_string(),
                        priority: 4,
                        tags: vec!["chips".to_string()],
                        rationale: "eligible".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
                (
                    "https://example.com/amazon".to_string(),
                    ArticleTriageResult {
                        category: "infrastructure".to_string(),
                        priority: 3,
                        tags: vec!["cloud".to_string()],
                        rationale: "eligible".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
                (
                    "https://example.com/scribes".to_string(),
                    ArticleTriageResult {
                        category: "workflow".to_string(),
                        priority: 1,
                        tags: vec!["clinical".to_string()],
                        rationale: "filtered".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
            ]),
            Some(provider),
            OPENAI_MODEL_GPT_5_4_NANO,
            10_000,
        );

        let response = engine
            .query(QueryKnowledgeBaseInput {
                question: "Which companies appear best positioned to meet rising AI demand through chips, cloud, data centers, and power infrastructure?".to_string(),
                max_results: 3,
                allow_broad: false,
                scope_entities: Vec::new(),
                scope_date_from: None,
                scope_date_to: None,
            })
            .await;

        assert_eq!(response.mode, "smart");
        assert!(response
            .warnings
            .iter()
            .any(|warning| warning.contains("used heuristic expansion instead")));
        assert_eq!(response.ranked_articles.len(), 2);
        assert_eq!(response.ranked_articles[0].filename, "tsmc.md");
        assert_eq!(response.ranked_articles[1].filename, "amazon.md");
        assert!(response.synthesis.unwrap_or_default().contains("[tsmc.md]"));
    }

    #[tokio::test]
    async fn smart_query_falls_back_when_provider_missing() {
        let engine = test_engine(None, 500);
        let response = engine
            .query(QueryKnowledgeBaseInput {
                question: "What did Anthropic say about AI security?".to_string(),
                max_results: 5,
                allow_broad: false,
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
                allow_broad: false,
                scope_entities: Vec::new(),
                scope_date_from: None,
                scope_date_to: None,
            })
            .await;

        assert!(response.total_token_count <= 20);
        assert!(response.ranked_articles.len() <= 1);
    }
}
