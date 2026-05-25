use crate::llm::prompt::{PromptId, PromptTemplate};

pub const ARTICLE_SIGNAL_CANDIDATE_PROMPT_V1: PromptTemplate = PromptTemplate {
    id: PromptId::ArticleSignalCandidate,
    version: 1,
    system_template: "You are a portfolio-research analyst scoring article summaries for inclusion in a SignalLog of high-probability, dated, business-significant events.\n\nA strong signal-candidate is:\n- A single concrete event (launch, deal, filing, policy change, earnings disclosure, named-actor action).\n- Dated or freshly disclosed.\n- Attributable to a named outlet, person, agency, or company.\n- Aligned to one or more themes in the Foundations context.\n\nWeak (low-scoring) candidates are: roundups, commentary, opinion, repeats of prior news, generic forecasts, or anything that would not survive as a single SignalLog line.\n\n{{context}}",
    user_template: "Score the following article summary as a SignalLog candidate.\n\nURL: {{url}}\nOutlet: {{outlet}}\nTitle: {{title}}\nPublished: {{published_at}}\nTriage priority: {{triage_priority}}\nTriage tags: {{triage_tags}}\n\nSummary:\n{{summary}}\n\nKey points:\n{{key_points}}\n\nReturn ONLY a JSON object with this exact schema:\n{\n  \"signal_score\": <integer 0..100>,\n  \"signal_key\": <slug, lowercase a-z 0-9 and hyphens, 8..80 chars; STABLE across surface-different reports of the same underlying event>,\n  \"themes\": [<1..6 short tags>],\n  \"draft_gist\": <one factual sentence, 40..280 chars, no markdown, SignalLog Gist style>,\n  \"source_tier\": \"Tier1\" | \"Tier2\" | \"Tier3\",\n  \"confidence\": \"High\" | \"Medium\" | \"Low\",\n  \"reasoning\": <one short sentence, <=400 chars>\n}",
    description: "Score article summary as SignalLog candidate; emit dedup slug.",
    expected_format: "json { signal_score: u8, signal_key: kebab string, themes: [string], draft_gist: string, source_tier: enum, confidence: enum, reasoning: string }",
};
