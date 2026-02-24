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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
pub struct BriefingTheme {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateBriefing {
    pub executive_summary: String,
    pub themes: Vec<BriefingTheme>,
    pub article_count: u32,
}
