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

## Workflow

1. Start with `query_knowledge_base` for natural-language research questions.
2. If the response is `mode="too_broad"`, do not keep retrying broad smart queries blindly.
3. Narrow the question using one or more of:
- date range
- narrower subtopic or infrastructure layer
- explicit entities if the topic is not already saturated with them
4. If broad smart-query remains unhelpful, use raw tools directly:
- `search_articles` for targeted regex/pattern retrieval
- `search_entities` for company, technology, product, or theme lookups
- `list_articles` for date/title browsing
- `read_article` for the exact source article
- `get_article_summary` for cached article summaries by URL
5. When giving the final answer, ground claims in specific retrieved articles and mention uncertainty if the corpus is mixed or incomplete.

## Guidance

- Treat `mode="too_broad"` as a useful result, not a tool failure.
- Avoid `allow_broad=true` unless the query is already reasonably constrained; it can be slow in chat usage.
- For saturated topics involving very common companies, targeted `search_articles` queries may outperform repeated smart-query attempts.
- If `search_articles` returns many results, refine the regex or add a date filter before reading articles.
- Prefer short iterative narrowing over large raw result dumps.

## This Project

The repo usually exposes `harvester-mcp` through `.mcp.json` using:
- `scripts/Start-HarvesterMcp.ps1`
- the current workspace as `-ProjectRoot`
- the shared corpus at `C:\Users\larsp\src\web_page_filet_mignon\output`

Assume the MCP server is available in this project unless tool calls show otherwise.
