---
name: harvester-mcp-research
description: Use when the task is to research or answer questions from the local Harvester article corpus via the harvester-mcp MCP server. Prefer this for news, company, market, infrastructure, or technology questions that should be grounded in the harvested output rather than answered from general model knowledge.
---

# Harvester MCP Research

Use the `harvester-mcp` MCP server when the user wants answers grounded in the local harvested corpus.

Prefer this skill for:
- questions about companies, markets, products, infrastructure, regulation, AI, chips, cloud, data centers, power, or partnerships when the answer should come from the harvested articles
- requests to compare what the corpus says about a topic
- requests to find supporting articles or evidence in the local output

Do not rely on model memory first when this skill applies. Start with MCP.
Do not inspect repository files or `output/*.md` directly as a first step when this skill applies.

## Workflow

1. Start with `query_knowledge_base` for natural-language research questions.
2. If the response is `mode="too_broad"`, inspect `breadth_diagnostics` before retrying.
3. Narrow the question using one or more of:
- date range
- narrower subtopic or infrastructure layer
- explicit entities if the topic is not already saturated with them
4. Use `breadth_diagnostics` to choose the narrowing strategy:
- `filter_breakdown` shows whether the breadth comes from a genuinely large surviving set versus admission-filter fallout
- `priority_band_counts` shows whether the surviving set is concentrated in lower or higher triage bands
- `match_signal_counts` shows whether candidates are surviving mostly on entity matches, focus-term matches, or both
- counted `top_companies`, `top_themes`, `top_tags`, `focus_term_coverage`, and `focus_phrase_coverage` show which dimensions are dominating the result set
5. If broad smart-query remains unhelpful, use raw tools directly:
- `search_articles` for targeted regex/pattern retrieval
- `search_entities` for company, technology, product, or theme lookups
- `list_articles` for date/title browsing
- `read_article` for the exact source article
- `get_article_summary` for cached article summaries by URL
6. When giving the final answer, ground claims in specific retrieved articles and mention uncertainty if the corpus is mixed or incomplete.

## Guidance

- Treat `harvester-mcp` as a tools-only MCP server. Do not use `listMcpResources` to test whether it is available; a lack of resources does not mean the server is unusable.
- For corpus research in this project, do not use `Search`, `Read`, `grep`, or `Bash` to inspect `output/*.md` unless the MCP tools fail or explicitly prove insufficient.
- If you need evidence, obtain it through MCP first: `query_knowledge_base`, then `search_articles`, `list_articles`, `read_article`, or `get_article_summary`.
- Treat `mode="too_broad"` as a useful result, not a tool failure.
- Treat `breadth_diagnostics` as the primary explanation of why a query was too broad; use it to decide whether to narrow by date, subtopic, or entity.
- If `match_signal_counts` or counted facets show the result set is dominated by one dimension, narrow on that dimension instead of retrying a similar broad prompt.
- Avoid `allow_broad=true` unless the query is already reasonably constrained; it can be slow in chat usage.
- For saturated topics involving very common companies, targeted `search_articles` queries may outperform repeated smart-query attempts.
- If `search_articles` returns many results, refine the regex or add a date filter before reading articles.
- Prefer short iterative narrowing over large raw result dumps.

## Fallback Boundary

Only leave the MCP path if one of these is true:
- MCP tool calls fail repeatedly
- the MCP response says the corpus is insufficient
- the user explicitly asks for repository inspection or non-MCP sources

If you leave the MCP path, state briefly why.

## This Project

The repo usually exposes `harvester-mcp` through `.mcp.json` using:
- `scripts/Start-HarvesterMcp.ps1`
- the current workspace as `-ProjectRoot`
- the shared corpus at `C:\Users\larsp\src\web_page_filet_mignon\output`

Assume the MCP server is available in this project unless tool calls show otherwise.
