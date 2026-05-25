use serde::{Deserialize, Serialize};

/// Typed outputs for LLM prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriageResult {
    pub category: String,
    pub priority: TriagePriority,
    pub tags: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriagePriority(u8);

impl TriagePriority {
    pub fn new(value: u8) -> Option<Self> {
        if (1..=5).contains(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    pub fn value(self) -> u8 {
        self.0
    }
}

/// Structured entity lists extracted from an article summary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryEntities {
    pub companies: Vec<String>,
    pub technologies: Vec<String>,
    pub products: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleSummary {
    pub title: String,
    pub summary: String,
    pub key_points: Vec<String>,
    /// Structured entity lists (from V4+ prompt). Empty for V3 responses.
    pub entities: SummaryEntities,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefingStory {
    pub headline: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateBriefing {
    pub executive_summary: String,
    pub top_stories: Vec<BriefingStory>,
    pub article_count: u32,
}

/// Outlet authority tier. Lower variant = higher authority. `Tier1` is best.
/// Ord/PartialOrd derive ordering by variant position, so `Tier1 < Tier2 < Tier3`,
/// which matches the selection tie-breaker rule ("best `source_tier` wins").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceTier {
    Tier1,
    Tier2,
    Tier3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCandidateResult {
    pub signal_score: u8,
    pub signal_key: String,
    pub themes: Vec<String>,
    pub draft_gist: String,
    pub source_tier: SourceTier,
    pub confidence: Confidence,
    pub reasoning: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[cfg(test)]
mod signal_candidate_dto_tests {
    use super::*;

    #[test]
    fn source_tier_orders_tier1_best() {
        assert!(SourceTier::Tier1 < SourceTier::Tier2);
        assert!(SourceTier::Tier2 < SourceTier::Tier3);
    }

    #[test]
    fn signal_candidate_result_constructable() {
        let r = SignalCandidateResult {
            signal_score: 75,
            signal_key: "nvda-q4-earnings".to_string(),
            themes: vec!["inference-scarcity".to_string()],
            draft_gist: "Nvidia reports record data-center revenue in Q4 2026.".to_string(),
            source_tier: SourceTier::Tier1,
            confidence: Confidence::High,
            reasoning: "Direct earnings release.".to_string(),
            input_tokens: 1200,
            output_tokens: 80,
        };
        assert_eq!(r.signal_score, 75);
    }
}
