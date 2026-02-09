use crate::llm::{PromptId, PromptTemplate};

pub const TRIAGE_PROMPT: PromptTemplate = PromptTemplate {
    id: PromptId::ArticleTriage,
    version: 1,
    system_template: "You are a triage assistant that tags and scores articles.",
    user_template: "Document: {{content}}\nSummarize priority and tags.",
    description: "Initial triage for filtering articles",
    expected_format: "json { \"priority\": number, \"tags\": [string] }",
};
