mod candidates;
mod digest;
mod expansion;
mod heuristics;
mod refinement;
mod scoring;
mod types;

pub use types::{
    QueryKnowledgeBaseInput, QueryKnowledgeBaseResponse, RankedArticleDigest, SmartQueryEngine,
    SmartQueryOptions, DEFAULT_MAX_SCORING_CANDIDATES, DEFAULT_MIN_TRIAGE_PRIORITY,
    DEFAULT_TOO_BROAD_THRESHOLD,
};

use std::collections::HashMap;
use std::sync::Arc;

use harvester_core::{ArticleTriageResult, EntityIndex, SummaryCacheEntry};
use harvester_engine::llm::{
    LlmError, LlmProvider, ModelId, ProviderKind, OPENAI_MODEL_GPT_5_4_MINI,
    OPENAI_MODEL_GPT_5_4_NANO,
};
use serde::Deserialize;

use crate::article_index::ArticleIndex;
use refinement::{format_ranked_counts, mid_band_tag_counts, query_overlap_tag_counts};
use types::{MID_BAND_TAG_MAX_COUNT, MID_BAND_TAG_MIN_COUNT};

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
                heuristics::heuristic_query_expansion(input)
            }
        };
        engine_logging::engine_info!(
            "[smart-query] expansion focus_terms={:?} focus_phrases={:?}",
            expansion.focus_terms,
            expansion.focus_phrases
        );
        let selection = self.collect_candidates(input, &expansion);
        engine_logging::engine_info!(
            "[smart-query] candidate selection regex_matches={} entity_matches={} total_unique_candidates={} eligible_unique_candidates={} filtered_low_priority_candidates={} filtered_admission_candidates={} scoring_candidates={} candidates_with_triage={} capped={} threshold={} allow_broad={}",
            selection.regex_match_count,
            selection.entity_match_count,
            selection.total_unique_candidates,
            selection.eligible_unique_candidates,
            selection.filtered_low_priority_candidates,
            selection.filtered_admission_candidates,
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
            let query_overlap_tag_counts = query_overlap_tag_counts(
                &selection.tag_counts,
                &expansion.focus_terms,
                &expansion.focus_phrases,
            );
            if !query_overlap_tag_counts.is_empty() {
                engine_logging::engine_info!(
                    "[smart-query] triage tag query-overlap stats focus_terms={} focus_phrases={} tags={}",
                    expansion.focus_terms.join(", "),
                    expansion.focus_phrases.join(", "),
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
                        "The query matched articles, but all {} candidates were filtered out before scoring because they were low-priority or lacked enough query-specific evidence.",
                        selection.total_unique_candidates
                    )
                } else {
                    "No matching articles were found in the current corpus.".to_string()
                }),
                synthesis: Some(if selection.total_unique_candidates > 0 {
                    "No matching high-confidence articles were found in the current corpus."
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
                    "Add a more specific subtopic phrase so weak broad mentions are less likely to survive admission filtering."
                        .to_string(),
                ],
                top_companies: selection.top_companies,
                top_themes: selection.top_themes,
                sample_titles: selection.sample_titles,
                breadth_diagnostics: None,
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
            return Ok(self.build_too_broad_response(input, &expansion, warnings, selection));
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
            breadth_diagnostics: None,
        })
    }
}

fn push_unique(target: &mut Vec<String>, value: String) {
    if !target.iter().any(|existing| existing == &value) {
        target.push(value);
    }
}

fn merge_strings(target: &mut Vec<String>, source: Vec<String>) {
    for value in source {
        push_unique(target, value);
    }
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
    use crate::smart_query::types::QueryExpansion;
    use harvester_core::{ArticleSummaryResult, EntityIndex, EntityIndexEntry, SummaryCacheEntry};
    use harvester_engine::llm::MockLlmProvider;

    fn sample_article(
        filename: &str,
        title: &str,
        url: &str,
        fetched_utc: &str,
        body: &str,
    ) -> crate::article_index::ArticleEntry {
        crate::article_index::ArticleEntry {
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
        articles: Vec<crate::article_index::ArticleEntry>,
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
            r#"{"regex_patterns":["(?i)(data center|chip|cloud)"],"entity_names":[],"focus_terms":["cloud","chip"],"focus_phrases":["data center"],"date_from":null,"date_to":null}"#,
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
        let diagnostics = response
            .breadth_diagnostics
            .expect("too_broad response should include breadth diagnostics");
        assert_eq!(diagnostics.filter_breakdown.total_unique_candidates, 120);
        assert_eq!(
            diagnostics
                .filter_breakdown
                .filtered_low_priority_candidates,
            0
        );
        assert_eq!(
            diagnostics.filter_breakdown.filtered_admission_candidates,
            0
        );
        assert_eq!(diagnostics.filter_breakdown.eligible_unique_candidates, 120);
        assert_eq!(diagnostics.priority_band_counts.len(), 1);
        assert_eq!(diagnostics.priority_band_counts[0].priority, 3);
        assert_eq!(diagnostics.priority_band_counts[0].count, 120);
        assert_eq!(diagnostics.match_signal_counts.entity_only_candidates, 0);
        assert_eq!(diagnostics.match_signal_counts.focus_only_candidates, 120);
        assert_eq!(
            diagnostics.match_signal_counts.entity_and_focus_candidates,
            0
        );
        assert_eq!(
            diagnostics.match_signal_counts.title_supported_candidates,
            120
        );
        assert_eq!(diagnostics.match_signal_counts.body_only_candidates, 0);
        assert_eq!(diagnostics.focus_term_coverage.len(), 2);
        assert!(diagnostics
            .focus_term_coverage
            .iter()
            .all(|coverage| coverage.count == 120));
        assert_eq!(diagnostics.focus_phrase_coverage.len(), 1);
        assert_eq!(diagnostics.focus_phrase_coverage[0].value, "data center");
        assert_eq!(diagnostics.focus_phrase_coverage[0].count, 120);
        assert_eq!(diagnostics.top_themes[0].value, "data centers");
        assert_eq!(diagnostics.top_themes[0].count, 120);
        assert_eq!(diagnostics.top_tags[0].value, "capacity");
        assert_eq!(diagnostics.top_tags[0].count, 120);
        assert!(response
            .refinement_suggestions
            .iter()
            .any(|suggestion| suggestion.contains("infrastructure layer or subtopic")));
        assert!(!response
            .refinement_suggestions
            .iter()
            .any(|suggestion| suggestion.contains("one company")));
    }

    #[tokio::test]
    async fn smart_query_relationship_too_broad_suggestions_focus_on_dimensions() {
        let mut articles = Vec::new();
        let mut entity_entries = BTreeMap::new();
        let mut triage_index = HashMap::new();
        for idx in 0..120 {
            let filename = format!("relationship-{idx:03}.md");
            let title = format!("Microsoft and OpenAI shift data center strategy {idx:03}");
            let url = format!("https://example.com/relationship-{idx:03}");
            let fetched = format!("2026-04-{:02}T10:00:00Z", (idx % 28) + 1);
            let body = "# Relationship\nMicrosoft and OpenAI are renegotiating data center and compute terms.";
            articles.push(sample_article(&filename, &title, &url, &fetched, body));
            entity_entries.insert(
                url.clone(),
                EntityIndexEntry {
                    fetched_utc: Some(fetched),
                    content_hash: Some(format!("hash-rel-{idx:03}")),
                    companies: vec!["Microsoft".to_string(), "OpenAI".to_string()],
                    technologies: vec!["compute".to_string()],
                    products: vec![],
                    themes: vec!["partnership".to_string()],
                },
            );
            triage_index.insert(
                url,
                ArticleTriageResult {
                    category: "partnership".to_string(),
                    priority: 3,
                    tags: vec!["data-center".to_string(), "contract-terms".to_string()],
                    rationale: "eligible".to_string(),
                    input_tokens: 0,
                    output_tokens: 0,
                },
            );
        }

        let provider = Arc::new(MockLlmProvider::new());
        provider.queue_json_success(
            r#"{"regex_patterns":["(?i)(microsoft|openai|data center|compute)"],"entity_names":["Microsoft","OpenAI"],"focus_terms":["microsoft","openai","data","center","compute"],"focus_phrases":["data center","compute terms"],"date_from":null,"date_to":null}"#,
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
                question: "How is the Microsoft and OpenAI partnership changing around data centers and compute?".to_string(),
                max_results: 5,
                allow_broad: false,
                scope_entities: vec!["Microsoft".to_string(), "OpenAI".to_string()],
                scope_date_from: None,
                scope_date_to: None,
            })
            .await;

        assert_eq!(response.mode, "too_broad");
        assert!(response
            .refinement_suggestions
            .iter()
            .any(|suggestion| suggestion.contains("relationship dimension")));
        assert!(response
            .refinement_suggestions
            .iter()
            .any(|suggestion| suggestion.contains("data center")));
        assert!(!response
            .refinement_suggestions
            .iter()
            .any(|suggestion| suggestion.contains("one company")));
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
    async fn smart_query_requires_entity_and_focus_cooccurrence_for_entity_scoped_queries() {
        let provider = Arc::new(MockLlmProvider::new());
        provider.queue_json_success(
            r#"{"regex_patterns":["(?i)(anthropic|security)"],"entity_names":["Anthropic"],"focus_terms":["anthropic","security"],"focus_phrases":["ai security"],"date_from":null,"date_to":null}"#,
        );
        provider.queue_json_success(
            r#"{"relevance_score":9,"key_facts":["Anthropic discussed AI security controls."]}"#,
        );
        provider.queue_json_success(
            r#"{"synthesis":"Only the article that matched Anthropic and AI security survived filtering [C1]."}"#,
        );

        let engine = test_engine_with_articles(
            vec![
                sample_article(
                    "relevant.md",
                    "Anthropic expands AI security testing",
                    "https://example.com/relevant",
                    "2026-04-12T10:00:00Z",
                    "# Relevant\nAnthropic detailed new AI security evaluations.",
                ),
                sample_article(
                    "entity-only.md",
                    "Anthropic launches enterprise program",
                    "https://example.com/entity-only",
                    "2026-04-11T10:00:00Z",
                    "# Entity\nAnthropic described enterprise rollout milestones.",
                ),
                sample_article(
                    "focus-only.md",
                    "Security teams harden AI deployments",
                    "https://example.com/focus-only",
                    "2026-04-10T10:00:00Z",
                    "# Focus\nSecurity teams added new AI safeguards.",
                ),
            ],
            EntityIndex {
                schema_version: 1,
                entries: BTreeMap::from([
                    (
                        "https://example.com/relevant".to_string(),
                        EntityIndexEntry {
                            fetched_utc: Some("2026-04-12T10:00:00Z".to_string()),
                            content_hash: Some("hash-relevant".to_string()),
                            companies: vec!["Anthropic".to_string()],
                            technologies: vec!["AI security".to_string()],
                            products: vec![],
                            themes: vec!["security".to_string()],
                        },
                    ),
                    (
                        "https://example.com/entity-only".to_string(),
                        EntityIndexEntry {
                            fetched_utc: Some("2026-04-11T10:00:00Z".to_string()),
                            content_hash: Some("hash-entity-only".to_string()),
                            companies: vec!["Anthropic".to_string()],
                            technologies: vec!["enterprise".to_string()],
                            products: vec![],
                            themes: vec!["rollout".to_string()],
                        },
                    ),
                    (
                        "https://example.com/focus-only".to_string(),
                        EntityIndexEntry {
                            fetched_utc: Some("2026-04-10T10:00:00Z".to_string()),
                            content_hash: Some("hash-focus-only".to_string()),
                            companies: vec!["Example Security".to_string()],
                            technologies: vec!["AI security".to_string()],
                            products: vec![],
                            themes: vec!["security".to_string()],
                        },
                    ),
                ]),
            },
            HashMap::new(),
            HashMap::from([
                (
                    "https://example.com/relevant".to_string(),
                    ArticleTriageResult {
                        category: "security".to_string(),
                        priority: 4,
                        tags: vec!["ai-security".to_string()],
                        rationale: "eligible".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
                (
                    "https://example.com/entity-only".to_string(),
                    ArticleTriageResult {
                        category: "company".to_string(),
                        priority: 3,
                        tags: vec!["enterprise".to_string()],
                        rationale: "eligible".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
                (
                    "https://example.com/focus-only".to_string(),
                    ArticleTriageResult {
                        category: "security".to_string(),
                        priority: 3,
                        tags: vec!["ai-security".to_string()],
                        rationale: "eligible".to_string(),
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
        assert_eq!(response.ranked_articles[0].filename, "relevant.md");
        assert_eq!(response.candidate_count, Some(1));
        assert_eq!(response.total_match_count, Some(2));
        assert_eq!(response.filtered_low_priority_count, Some(0));
    }

    #[tokio::test]
    async fn smart_query_requires_both_entities_and_a_dimension_for_relationship_queries() {
        let provider = Arc::new(MockLlmProvider::new());
        provider.queue_json_success(
            r#"{"regex_patterns":["(?i)(microsoft|openai|partnership|data center|contract|azure)"],"entity_names":["Microsoft","OpenAI"],"focus_terms":["microsoft","openai","partnership","contract","azure"],"focus_phrases":["data center"],"date_from":null,"date_to":null}"#,
        );
        provider.queue_json_success(
            r#"{"relevance_score":9,"key_facts":["Microsoft and OpenAI renegotiated data-center contract terms."]}"#,
        );
        provider.queue_json_success(
            r#"{"synthesis":"Only the article that matched both entities plus a specific relationship dimension survived filtering [C1]."}"#,
        );

        let engine = test_engine_with_articles(
            vec![
                sample_article(
                    "relevant-relationship.md",
                    "Microsoft and OpenAI revisit data center contract terms",
                    "https://example.com/relevant-relationship",
                    "2026-04-12T10:00:00Z",
                    "# Relevant\nMicrosoft and OpenAI are renegotiating data center contract details.",
                ),
                sample_article(
                    "entity-pair-only.md",
                    "Microsoft and OpenAI partnership remains important",
                    "https://example.com/entity-pair-only",
                    "2026-04-11T10:00:00Z",
                    "# Pair\nMicrosoft and OpenAI said the partnership is still important.",
                ),
                sample_article(
                    "single-entity-dimension.md",
                    "Microsoft expands Azure data center footprint",
                    "https://example.com/single-entity-dimension",
                    "2026-04-10T10:00:00Z",
                    "# Single\nMicrosoft is expanding Azure data center capacity.",
                ),
            ],
            EntityIndex {
                schema_version: 1,
                entries: BTreeMap::from([
                    (
                        "https://example.com/relevant-relationship".to_string(),
                        EntityIndexEntry {
                            fetched_utc: Some("2026-04-12T10:00:00Z".to_string()),
                            content_hash: Some("hash-relevant-relationship".to_string()),
                            companies: vec!["Microsoft".to_string(), "OpenAI".to_string()],
                            technologies: vec!["Azure".to_string()],
                            products: vec![],
                            themes: vec!["data centers".to_string()],
                        },
                    ),
                    (
                        "https://example.com/entity-pair-only".to_string(),
                        EntityIndexEntry {
                            fetched_utc: Some("2026-04-11T10:00:00Z".to_string()),
                            content_hash: Some("hash-entity-pair-only".to_string()),
                            companies: vec!["Microsoft".to_string(), "OpenAI".to_string()],
                            technologies: vec![],
                            products: vec![],
                            themes: vec!["partnership".to_string()],
                        },
                    ),
                    (
                        "https://example.com/single-entity-dimension".to_string(),
                        EntityIndexEntry {
                            fetched_utc: Some("2026-04-10T10:00:00Z".to_string()),
                            content_hash: Some("hash-single-entity-dimension".to_string()),
                            companies: vec!["Microsoft".to_string()],
                            technologies: vec!["Azure".to_string()],
                            products: vec![],
                            themes: vec!["data centers".to_string()],
                        },
                    ),
                ]),
            },
            HashMap::new(),
            HashMap::from([
                (
                    "https://example.com/relevant-relationship".to_string(),
                    ArticleTriageResult {
                        category: "partnership".to_string(),
                        priority: 4,
                        tags: vec!["contract-terms".to_string()],
                        rationale: "eligible".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
                (
                    "https://example.com/entity-pair-only".to_string(),
                    ArticleTriageResult {
                        category: "partnership".to_string(),
                        priority: 3,
                        tags: vec!["partnership".to_string()],
                        rationale: "eligible".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
                (
                    "https://example.com/single-entity-dimension".to_string(),
                    ArticleTriageResult {
                        category: "cloud".to_string(),
                        priority: 3,
                        tags: vec!["azure".to_string()],
                        rationale: "eligible".to_string(),
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
                question:
                    "How is the Microsoft and OpenAI relationship changing around data centers and contract terms?"
                        .to_string(),
                max_results: 5,
                allow_broad: false,
                scope_entities: vec!["Microsoft".to_string(), "OpenAI".to_string()],
                scope_date_from: None,
                scope_date_to: None,
            })
            .await;

        assert_eq!(response.mode, "smart");
        assert_eq!(response.ranked_articles.len(), 1);
        assert_eq!(
            response.ranked_articles[0].filename,
            "relevant-relationship.md"
        );
        assert_eq!(response.candidate_count, Some(1));
        assert_eq!(response.total_match_count, Some(3));
        assert_eq!(response.filtered_low_priority_count, Some(0));
    }

    #[test]
    fn collect_candidates_drops_low_scoring_weak_tail_candidates() {
        let engine = test_engine_with_articles(
            vec![
                sample_article(
                    "strong.md",
                    "Security bulletin for enterprise AI",
                    "https://example.com/strong",
                    "2026-04-12T10:00:00Z",
                    "# Strong\nSecurity controls were tightened after model red-team findings.",
                ),
                sample_article(
                    "weak.md",
                    "Enterprise update",
                    "https://example.com/weak",
                    "2026-04-11T10:00:00Z",
                    "# Weak\nA short note mentioned security in passing near the footer.",
                ),
            ],
            EntityIndex {
                schema_version: 1,
                entries: Default::default(),
            },
            HashMap::new(),
            HashMap::from([
                (
                    "https://example.com/strong".to_string(),
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
                    "https://example.com/weak".to_string(),
                    ArticleTriageResult {
                        category: "security".to_string(),
                        priority: 2,
                        tags: vec!["security".to_string()],
                        rationale: "eligible".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
            ]),
            None,
            "mock-model",
            10_000,
        );

        let selection = engine.collect_candidates(
            &QueryKnowledgeBaseInput {
                question: "What do the loaded articles say about security?".to_string(),
                max_results: 5,
                allow_broad: false,
                scope_entities: Vec::new(),
                scope_date_from: None,
                scope_date_to: None,
            },
            &QueryExpansion {
                regex_patterns: vec!["(?i)security".to_string()],
                entity_names: Vec::new(),
                focus_terms: vec!["security".to_string()],
                focus_phrases: Vec::new(),
                date_from: None,
                date_to: None,
            },
        );

        assert_eq!(selection.eligible_unique_candidates, 1);
        assert_eq!(selection.filtered_admission_candidates, 1);
        assert_eq!(selection.candidates[0].filename, "strong.md");
    }

    #[test]
    fn collect_candidates_strongly_boosts_exact_focus_phrase_matches() {
        let engine = test_engine_with_articles(
            vec![
                sample_article(
                    "phrase.md",
                    "Microsoft faces competitive rivalry with OpenAI",
                    "https://example.com/phrase",
                    "2026-04-12T10:00:00Z",
                    "# Phrase\nMicrosoft described the competitive rivalry with OpenAI in detail.",
                ),
                sample_article(
                    "terms.md",
                    "Microsoft expands model lineup",
                    "https://example.com/terms",
                    "2026-04-11T10:00:00Z",
                    "# Terms\nMicrosoft said it sees OpenAI as a competitor and that rivalry is increasing.",
                ),
            ],
            EntityIndex {
                schema_version: 1,
                entries: BTreeMap::from([
                    (
                        "https://example.com/phrase".to_string(),
                        EntityIndexEntry {
                            fetched_utc: Some("2026-04-12T10:00:00Z".to_string()),
                            content_hash: Some("hash-phrase".to_string()),
                            companies: vec!["Microsoft".to_string(), "OpenAI".to_string()],
                            technologies: vec![],
                            products: vec![],
                            themes: vec!["competition".to_string()],
                        },
                    ),
                    (
                        "https://example.com/terms".to_string(),
                        EntityIndexEntry {
                            fetched_utc: Some("2026-04-11T10:00:00Z".to_string()),
                            content_hash: Some("hash-terms".to_string()),
                            companies: vec!["Microsoft".to_string(), "OpenAI".to_string()],
                            technologies: vec![],
                            products: vec![],
                            themes: vec!["competition".to_string()],
                        },
                    ),
                ]),
            },
            HashMap::new(),
            HashMap::from([
                (
                    "https://example.com/phrase".to_string(),
                    ArticleTriageResult {
                        category: "competition".to_string(),
                        priority: 3,
                        tags: vec!["competition".to_string()],
                        rationale: "eligible".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
                (
                    "https://example.com/terms".to_string(),
                    ArticleTriageResult {
                        category: "competition".to_string(),
                        priority: 3,
                        tags: vec!["competition".to_string()],
                        rationale: "eligible".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
            ]),
            None,
            "mock-model",
            10_000,
        );

        let selection = engine.collect_candidates(
            &QueryKnowledgeBaseInput {
                question: "How is the Microsoft and OpenAI relationship changing around competitive rivalry?"
                    .to_string(),
                max_results: 5,
                allow_broad: false,
                scope_entities: vec!["Microsoft".to_string(), "OpenAI".to_string()],
                scope_date_from: None,
                scope_date_to: None,
            },
            &QueryExpansion {
                regex_patterns: vec!["(?i)(microsoft|openai|competition|rivalry)".to_string()],
                entity_names: vec!["Microsoft".to_string(), "OpenAI".to_string()],
                focus_terms: vec!["competition".to_string(), "rivalry".to_string()],
                focus_phrases: vec!["competitive rivalry".to_string()],
                date_from: None,
                date_to: None,
            },
        );

        assert_eq!(selection.eligible_unique_candidates, 2);
        assert_eq!(selection.candidates[0].filename, "phrase.md");
    }

    #[test]
    fn collect_candidates_penalizes_boilerplate_snippet_evidence() {
        let boilerplate = "window.__s_data={\"routing\":{\"locationBeforeTransitions\":null},\"navStatus\":{\"pageType\":\"page\"},\"article\":\"security security security\"}";
        let engine = test_engine_with_articles(
            vec![
                sample_article(
                    "clean.md",
                    "Security review of Microsoft systems",
                    "https://example.com/clean",
                    "2026-04-12T10:00:00Z",
                    "# Clean\nSecurity review found concrete Microsoft identity hardening work.",
                ),
                sample_article(
                    "boilerplate.md",
                    "Imported page shell",
                    "https://example.com/boilerplate",
                    "2026-04-11T10:00:00Z",
                    boilerplate,
                ),
            ],
            EntityIndex {
                schema_version: 1,
                entries: Default::default(),
            },
            HashMap::new(),
            HashMap::from([
                (
                    "https://example.com/clean".to_string(),
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
                    "https://example.com/boilerplate".to_string(),
                    ArticleTriageResult {
                        category: "security".to_string(),
                        priority: 3,
                        tags: vec!["security".to_string()],
                        rationale: "eligible".to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                ),
            ]),
            None,
            "mock-model",
            10_000,
        );

        let selection = engine.collect_candidates(
            &QueryKnowledgeBaseInput {
                question: "What do the loaded articles say about security?".to_string(),
                max_results: 5,
                allow_broad: false,
                scope_entities: Vec::new(),
                scope_date_from: None,
                scope_date_to: None,
            },
            &QueryExpansion {
                regex_patterns: vec!["(?i)security".to_string()],
                entity_names: Vec::new(),
                focus_terms: vec!["security".to_string()],
                focus_phrases: Vec::new(),
                date_from: None,
                date_to: None,
            },
        );

        assert_eq!(selection.eligible_unique_candidates, 1);
        assert_eq!(selection.filtered_admission_candidates, 1);
        assert_eq!(selection.candidates[0].filename, "clean.md");
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
