# Prompt Context Files

This document describes where to place prompt context files and how to format them.

## Location

Create a `contexts/` directory at the workspace root (next to `Cargo.toml`). One TOML file per prompt id:

```
contexts/
  article_triage.toml
  article_summary.toml
  aggregate_briefing.toml
  archive/
    article_triage.v6.toml
```

If the directory or a file is missing, the application continues with empty context (degraded but functional).

## File format

Each context file uses the following structure:

```toml
[meta]
prompt_id = "ArticleTriage" # One of: ArticleTriage | ArticleSummary | AggregateBriefing
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
- `prompt_id` must match a known prompt id exactly (case-sensitive).
- The `context` value is injected into prompt templates via `{{context}}`.
- Keep values concise to avoid token budget overruns.

## Git ignore

The `contexts/` directory should be ignored in `.gitignore` because it is user-specific and frequently changing. Keep any examples in `docs/` if needed.
