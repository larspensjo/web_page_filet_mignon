use std::sync::Arc;

use harvester_engine::llm::{ChatMessage, ChatRole, LlmError, LlmProvider, LlmRequest, ModelId};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::types::{
    CandidateArticle, QueryKnowledgeBaseInput, RelevanceScoreResponse, ScoredCandidate,
    SmartQueryEngine, MAX_KEY_FACTS, SCORING_CONCURRENCY,
};

impl SmartQueryEngine {
    pub(super) async fn score_candidates(
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

    let parsed: RelevanceScoreResponse = super::parse_json_response(response.content())?;
    Ok(ScoredCandidate {
        candidate,
        relevance_score: parsed.relevance_score.min(10),
        key_facts: parsed.key_facts.into_iter().take(MAX_KEY_FACTS).collect(),
    })
}
