use std::sync::Arc;

use harvester_engine::llm::{
    ChatMessage, ChatRole, LlmError, LlmProvider, LlmRequest, LlmResponse,
};

use super::heuristics;
use super::types::{
    QueryExpansion, QueryExpansionResponse, QueryKnowledgeBaseInput, SmartQueryEngine,
    EXPANSION_INITIAL_MAX_OUTPUT_TOKENS, EXPANSION_RETRY_MAX_OUTPUT_TOKENS, MAX_EXPANSION_ENTITIES,
    MAX_EXPANSION_PATTERNS,
};

impl SmartQueryEngine {
    pub(super) async fn expand_query(
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
            "Question: {}\nScope entities: {}\nScope date_from: {}\nScope date_to: {}\nReturn JSON with regex_patterns, entity_names, focus_terms, focus_phrases, date_from, date_to. regex_patterns must be safe Rust regex strings and prefer (?i) case-insensitive patterns. focus_terms should contain the most important content words for refinement. focus_phrases should contain short noun phrases like 'data centers' or 'inference capacity' when relevant. Use null for absent dates.",
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
            Err(err) if super::should_retry_empty_length_response(&err) => {
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

        let parsed: QueryExpansionResponse = super::parse_json_response(response.content())?;
        let mut regex_patterns = normalize_patterns(parsed.regex_patterns);
        if regex_patterns.is_empty() {
            regex_patterns = heuristics::heuristic_patterns(&input.question);
        }

        let mut entity_names = normalize_terms(parsed.entity_names);
        for scope in normalize_terms(input.scope_entities.clone()) {
            super::push_unique(&mut entity_names, scope);
        }
        entity_names.truncate(MAX_EXPANSION_ENTITIES);
        let focus_terms = normalize_focus_terms(parsed.focus_terms, &input.question);
        let focus_phrases = normalize_focus_phrases(parsed.focus_phrases, &input.question);

        Ok(QueryExpansion {
            regex_patterns,
            entity_names,
            focus_terms,
            focus_phrases,
            date_from: parsed.date_from.or_else(|| input.scope_date_from.clone()),
            date_to: parsed.date_to.or_else(|| input.scope_date_to.clone()),
        })
    }

    pub(super) async fn request_expansion(
        &self,
        provider: Arc<dyn LlmProvider>,
        user_prompt: &str,
        max_output_tokens: u32,
    ) -> Result<LlmResponse, LlmError> {
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
}

pub(super) fn normalize_patterns(patterns: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for pattern in patterns {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            continue;
        }
        super::push_unique(&mut normalized, trimmed.to_string());
        if normalized.len() >= MAX_EXPANSION_PATTERNS {
            break;
        }
    }
    normalized
}

pub(crate) fn normalize_terms(terms: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for term in terms {
        let trimmed = term.trim();
        if trimmed.is_empty() {
            continue;
        }
        super::push_unique(&mut normalized, trimmed.to_string());
    }
    normalized
}

pub(super) fn normalize_focus_terms(terms: Vec<String>, question: &str) -> Vec<String> {
    let mut normalized = Vec::new();
    for term in terms {
        let trimmed = term.trim();
        if trimmed.is_empty() {
            continue;
        }
        super::push_unique(&mut normalized, trimmed.to_lowercase());
    }
    if normalized.is_empty() {
        normalized = heuristics::heuristic_focus_terms(question);
    }
    normalized.truncate(6);
    normalized
}

pub(super) fn normalize_focus_phrases(phrases: Vec<String>, question: &str) -> Vec<String> {
    let mut normalized = Vec::new();
    for phrase in phrases {
        let trimmed = phrase.trim();
        if trimmed.is_empty() {
            continue;
        }
        super::push_unique(&mut normalized, trimmed.to_lowercase());
    }
    if normalized.is_empty() {
        normalized = heuristics::heuristic_focus_phrases(question);
    }
    normalized.truncate(4);
    normalized
}
