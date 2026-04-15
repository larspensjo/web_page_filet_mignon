use std::collections::HashMap;
use std::sync::Arc;

use crate::article_index::ArticleIndex;
use harvester_core::{ArticleTriageResult, EntityIndex, SummaryCacheEntry};
use harvester_engine::llm::{LlmProvider, ModelId};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_EXPANSION_PATTERNS: usize = 3;
pub(crate) const MAX_EXPANSION_ENTITIES: usize = 5;
pub(crate) const MAX_KEY_FACTS: usize = 2;
pub(crate) const SCORING_CONCURRENCY: usize = 4;
pub const DEFAULT_MAX_SCORING_CANDIDATES: usize = 10;
pub const DEFAULT_TOO_BROAD_THRESHOLD: usize = 100;
pub const DEFAULT_MIN_TRIAGE_PRIORITY: u8 = 2;
pub(crate) const EXPANSION_INITIAL_MAX_OUTPUT_TOKENS: u32 = 400;
pub(crate) const EXPANSION_RETRY_MAX_OUTPUT_TOKENS: u32 = 700;
pub(crate) const MID_BAND_TAG_MIN_COUNT: usize = 5;
pub(crate) const MID_BAND_TAG_MAX_COUNT: usize = 200;

#[derive(Clone)]
pub struct SmartQueryEngine {
    pub(crate) article_index: Arc<ArticleIndex>,
    pub(crate) entity_index: Arc<EntityIndex>,
    pub(crate) summary_index: Arc<HashMap<String, SummaryCacheEntry>>,
    pub(crate) triage_index: Arc<HashMap<String, ArticleTriageResult>>,
    pub(crate) provider: Option<Arc<dyn LlmProvider>>,
    pub(crate) agent_model: ModelId,
    pub(crate) expansion_model: ModelId,
    pub(crate) context_budget: usize,
    pub(crate) scoring_candidate_cap: usize,
    pub(crate) too_broad_threshold: usize,
    pub(crate) min_triage_priority: u8,
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
pub(crate) struct QueryExpansion {
    pub(crate) regex_patterns: Vec<String>,
    pub(crate) entity_names: Vec<String>,
    pub(crate) focus_terms: Vec<String>,
    pub(crate) focus_phrases: Vec<String>,
    pub(crate) date_from: Option<String>,
    pub(crate) date_to: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidateArticle {
    pub(crate) filename: String,
    pub(crate) title: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) fetched_utc: Option<String>,
    pub(crate) snippet: String,
    pub(crate) summary: Option<String>,
    pub(crate) key_points: Vec<String>,
    pub(crate) matched_patterns: Vec<String>,
    pub(crate) matched_entities: Vec<String>,
    pub(crate) companies: Vec<String>,
    pub(crate) themes: Vec<String>,
    pub(crate) triage_tags: Vec<String>,
    pub(crate) title_pattern_hits: usize,
    pub(crate) url_pattern_hits: usize,
    pub(crate) triage_priority: Option<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScoredCandidate {
    pub(crate) candidate: CandidateArticle,
    pub(crate) relevance_score: u8,
    pub(crate) key_facts: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidateSelection {
    pub(crate) candidates: Vec<CandidateArticle>,
    pub(crate) regex_match_count: usize,
    pub(crate) entity_match_count: usize,
    pub(crate) total_unique_candidates: usize,
    pub(crate) eligible_unique_candidates: usize,
    pub(crate) filtered_low_priority_candidates: usize,
    pub(crate) scoring_candidates: usize,
    pub(crate) capped: bool,
    pub(crate) top_companies: Vec<String>,
    pub(crate) top_themes: Vec<String>,
    pub(crate) sample_titles: Vec<String>,
    pub(crate) tag_counts: Vec<(String, usize)>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct QueryExpansionResponse {
    #[serde(default)]
    pub(crate) regex_patterns: Vec<String>,
    #[serde(default)]
    pub(crate) entity_names: Vec<String>,
    #[serde(default)]
    pub(crate) focus_terms: Vec<String>,
    #[serde(default)]
    pub(crate) focus_phrases: Vec<String>,
    pub(crate) date_from: Option<String>,
    pub(crate) date_to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RelevanceScoreResponse {
    pub(crate) relevance_score: u8,
    #[serde(default)]
    pub(crate) key_facts: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DigestAssemblyResponse {
    pub(crate) synthesis: String,
}
