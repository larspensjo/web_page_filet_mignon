# Prompt Context Files

This document describes where to place prompt context files and how to format them.

## Location

Create a `contexts/` directory at the workspace root (next to `Cargo.toml`). One TOML file per prompt id:

```
contexts/
  article_triage.toml
  article_summary.toml
  article_signal_candidate.toml
  aggregate_briefing.toml
  archive/
    article_triage.v6.toml
```

If the directory or a file is missing, the application continues with empty context (degraded but functional).

## File format

Each context file uses the following structure:

```toml
[meta]
prompt_id = "ArticleTriage" # One of: ArticleTriage | ArticleSummary | ArticleSignalCandidate | AggregateBriefing
schema_version = 1
version = 1
updated = "2026-02-09"
description = "Optional human-readable summary"
changelog = "Optional change log"

[variables]
context = """
[CORE HOLDINGS]
NVIDIA, Microsoft, Alphabet, Amazon, TSMC, Broadcom.

[THEMES]
1. AI Infrastructure
2. Space Industrialization

[EXCLUDE]
Consumer gadget reviews.
"""
```

### Notes

- `schema_version` is currently **1**. Other values are rejected.
- `prompt_id` must match a known prompt id exactly (case-sensitive): `ArticleTriage`, `ArticleSummary`, `ArticleSignalCandidate`, `AggregateBriefing`, `BriefingExecutiveSummary`, or `BriefingNextItem`.
- The `context` value is injected into prompt templates via `{{context}}`.
- Context variables are sorted by key on load before rendering, so equivalent context files produce deterministic prompt bytes.
- Keep values concise to avoid token budget overruns.

Known files:

- `contexts/article_triage.toml` - `ArticleTriage`
- `contexts/article_summary.toml` - `ArticleSummary`
- `contexts/article_signal_candidate.toml` - `ArticleSignalCandidate`, scores article summaries for SignalLog admission
- `contexts/aggregate_briefing.toml` - `AggregateBriefing`, `BriefingExecutiveSummary`, and `BriefingNextItem`

`BriefingExecutiveSummary` and `BriefingNextItem` intentionally reuse `aggregate_briefing.toml`; they do not have separate context files. Their static templates share one byte-identical system prefix, and only their user-template suffix differs. That shared prefix plus deterministic context ordering preserves OpenAI prefix-cache stability for the multi-step briefing stream.

## Git ignore

The `contexts/` directory should be ignored in `.gitignore` because it is user-specific and frequently changing. Keep any examples in `docs/` if needed.
