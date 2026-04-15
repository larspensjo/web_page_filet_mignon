use std::sync::Arc;

use harvester_engine::llm::{ChatMessage, ChatRole, LlmError, LlmProvider, LlmRequest};
use harvester_engine::{TokenCounter, WhitespaceTokenCounter};

use super::types::{
    CandidateSelection, DigestAssemblyResponse, QueryExpansion, QueryKnowledgeBaseInput,
    QueryKnowledgeBaseResponse, RankedArticleDigest, ScoredCandidate, SmartQueryEngine,
    MAX_KEY_FACTS,
};
use super::{expansion, heuristics, refinement};

impl SmartQueryEngine {
    pub(super) async fn assemble_digest(
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

        let parsed: DigestAssemblyResponse = super::parse_json_response(response.content())?;
        Ok(expand_digest_citations(&parsed.synthesis, &citation_rows))
    }

    pub(super) fn build_raw_fallback(
        &self,
        input: &QueryKnowledgeBaseInput,
        warnings: Vec<String>,
    ) -> QueryKnowledgeBaseResponse {
        let expansion = QueryExpansion {
            regex_patterns: heuristics::heuristic_patterns(&input.question),
            entity_names: expansion::normalize_terms(input.scope_entities.clone()),
            focus_terms: heuristics::heuristic_focus_terms(&input.question),
            focus_phrases: heuristics::heuristic_focus_phrases(&input.question),
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

    pub(super) fn build_too_broad_response(
        &self,
        input: &QueryKnowledgeBaseInput,
        expansion: &QueryExpansion,
        warnings: Vec<String>,
        selection: CandidateSelection,
    ) -> QueryKnowledgeBaseResponse {
        let overlap_tags = refinement::query_overlap_tag_counts(
            &selection.tag_counts,
            &expansion.focus_terms,
            &expansion.focus_phrases,
        );

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
            refinement_suggestions: refinement::build_refinement_suggestions(
                input,
                &selection.top_companies,
                &selection.top_themes,
                &overlap_tags,
            ),
            top_companies: selection.top_companies,
            top_themes: selection.top_themes,
            sample_titles: selection.sample_titles,
        }
    }

    pub(super) fn enforce_context_budget(
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
                super::push_unique(
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

fn expand_digest_citations(synthesis: &str, citation_rows: &[(String, String)]) -> String {
    let mut expanded = synthesis.to_string();
    for (citation_id, filename) in citation_rows {
        expanded = expanded.replace(&format!("[{}]", citation_id), &format!("[{}]", filename));
    }
    expanded
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
