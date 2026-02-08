use super::{PromptId, PromptTemplate};

pub const SUMMARY_PROMPT: PromptTemplate = PromptTemplate {
    id: PromptId::ArticleSummary,
    version: 1,
    system_template: "You are a helpful summarizer.",
    user_template: "Document: {{content}}\nCreate a concise article summary.",
    description: "Per-article summary",
    expected_format: "json { \"title\": string, \"summary\": string, \"key_points\": [string] }",
};
