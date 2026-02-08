use super::{PromptId, PromptTemplate};

pub const BRIEFING_PROMPT: PromptTemplate = PromptTemplate {
    id: PromptId::AggregateBriefing,
    version: 1,
    system_template: "You are an executive briefing assistant.",
    user_template: "Documents: {{collection}}\nProduce an executive summary and themes.",
    description: "Aggregate briefing",
    expected_format:
        "json { \"executive_summary\": string, \"themes\": [string], \"article_count\": number }",
};
