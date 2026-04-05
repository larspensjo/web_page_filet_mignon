# MCP Knowledge Base Server — Design & Implementation Plan

## Overview

A standalone MCP server (`harvester_mcp`) that exposes the Harvester article corpus to Claude Code and Codex as a queryable knowledge base. The server provides both raw data access tools and a smart agent layer that uses a cheap LLM to pre-filter, score, and summarize results before they reach the frontier model.

### Core Idea

Instead of building a compiled wiki and index infrastructure inside Harvester, this approach:

- Keeps the raw article corpus and sidecars as-is
- Uses in-process regex search over the article corpus (no external dependencies, no pre-built indices)
- Exposes data via MCP tools so Claude/Codex becomes the reasoning layer
- Uses a cheap model inside the MCP server as a mini-agent to reduce context sent to the frontier model

### Relationship to Existing Architecture Doc

This MCP server is an alternative path that prioritizes:

- Simplicity over completeness
- External reasoning (Claude/Codex) over internal orchestration
- Regex search over purpose-built indices
- Immediate usability over incremental compilation

The two approaches are not mutually exclusive — the compiled wiki could be added later as a data source the MCP server also exposes.

## Architecture

### Tiered Intelligence

```
User
  |
  v
Claude / Codex  (frontier model — reasoning, synthesis, answers)
  |
  v
harvester_mcp   (MCP server, rmcp + Tokio + stdio)
  |-- Raw tools:   regex search, file reads, sidecar queries  (deterministic)
  |-- Smart tools:  cheap-model agent layer                   (LLM-powered)
  |
  v
output/          (article corpus + sidecars)
```

The frontier model never sees the full corpus. The MCP server's job is to return bounded, curated context.

### Crate Structure

New crate: `crates/harvester_mcp/`

```
crates/
  harvester_core/       # domain types (EntityIndex, SummaryCache, etc.)
  harvester_engine/     # LLM client, OpenAI provider, prompts
  harvester_io/         # sidecar loading functions
  harvester_batch/      # CLI batch runner
  harvester_app/        # GUI app
  engine_logging/       # logging infrastructure
  harvester_mcp/        # NEW — MCP server binary
```

Workspace dependencies:
- `harvester_core` — `EntityIndex`, `SummaryCache`, `TriageCache` types, article metadata types
- `harvester_engine` — OpenAI API client (`LlmRequest`, `OpenAiChatCompletionRequest`, provider logic)
- `harvester_io` — sidecar loading functions (`load_entity_index`, `load_summary_cache`, `load_triage_cache`)
- `engine_logging` — logging infrastructure

### MCP Transport

Stdio (stdin/stdout JSON-RPC) — the standard transport for Claude Code and Codex MCP integrations.

### Logging

File-based logging to `output/logs/mcp.log`. Logs go to file only (stdout is the MCP transport, stderr is reserved for MCP protocol errors).

`engine_logging` currently only supports writing to `./engine.log` with no configurable path. Phase 1 must extend `engine_logging` with a path-based file initializer.

Every tool invocation is logged with: tool name, parameters, result size (tokens/bytes), timing. Smart tool invocations additionally log: cheap-model prompts, responses, and relevance scores.

## MCP Tools

### Raw Data Tools (deterministic, no LLM)

#### `search_articles`

Searches article markdown files in `output/` using in-process regex matching. The searchable corpus is restricted to files with valid Harvester frontmatter (matching the criteria used by `harvester_engine::briefing`'s archive scanner). Non-article files (archive exports, knowledge-base material, sidecars) are excluded.

Implementation: At startup, enumerate eligible article files using frontmatter validation and load their content into an in-memory article index. Search uses the `regex` crate directly — no external dependencies.

Parameters:
- `pattern` (string, required) — regex pattern
- `date_from` (string, optional) — ISO date, filter by `fetched_utc`
- `date_to` (string, optional) — ISO date, filter by `fetched_utc`
- `max_results` (integer, optional, default 20) — cap on returned matches

Returns: matching snippets with filenames, line context, and article frontmatter (title, url, fetched_utc).

#### `read_article`

Reads a single article by filename.

Parameters:
- `filename` (string, required) — article filename in `output/`

Returns: full article Markdown content.

#### `list_articles`

Lists articles in the corpus.

Parameters:
- `date_from` (string, optional) — ISO date filter
- `date_to` (string, optional) — ISO date filter
- `title_pattern` (string, optional) — regex filter on title

Returns: list of (filename, title, url, fetched_utc, token_count).

#### `search_entities`

Queries the loaded `EntityIndex`.

Parameters:
- `company` (string, optional) — company name to search for
- `technology` (string, optional) — technology to search for
- `product` (string, optional) — product to search for
- `theme` (string, optional) — theme to search for

At least one parameter is required. All provided parameters are ANDed.

Returns: list of matching article URLs with their full entity metadata (companies, technologies, products, themes).

#### `get_article_summary`

Looks up a pre-computed summary from `SummaryCache`.

Note: `SummaryCache` is keyed by `(content_hash, prompt_id, prompt_version, model_id, context_hash)`, not by URL. The tool resolves URLs to content hashes via `EntityIndex`, then returns the newest matching `ArticleSummary` entry for that content hash.

Parameters:
- `url` (string, required) — article URL

Returns: cached summary text, or indication that no summary is available.

Implementation: At startup, build a secondary index from URL → content_hash (via `EntityIndex`), then from content_hash → newest summary entry. This avoids exposing the internal cache key structure to MCP clients.

### Smart Tools (cheap-model powered)

#### `query_knowledge_base`

The main entry point for complex questions. Orchestrates a multi-step agent workflow internally.

Parameters:
- `question` (string, required) — free-text question
- `max_results` (integer, optional, default 10) — max articles in the digest
- `scope_entities` (list of strings, optional) — limit search to articles mentioning these entities
- `scope_date_from` (string, optional) — ISO date
- `scope_date_to` (string, optional) — ISO date

Returns: a structured digest containing:
- Ranked list of relevant articles (title, url, filename, relevance score)
- Per-article extract: 1-2 key facts relevant to the question
- Brief synthesis paragraph with citations
- Total token count of the response

### Smart Tool Agent Workflow

When `query_knowledge_base` is called, the MCP server executes:

**Step 1 — Query expansion** (cheap model)
Given the question, produce:
- 2-3 regex search patterns
- Likely entity names to check against `EntityIndex`
- Date range hints if the question implies temporality

**Step 2 — Candidate retrieval** (deterministic)
- Run regex search with expanded patterns
- Query `EntityIndex` with expanded entity names
- Merge and deduplicate results
- Collect summaries from `SummaryCache` for matched articles

**Step 3 — Relevance scoring** (cheap model, parallelized)
For each candidate, send summary/snippet + original question to cheap model:
- Score relevance 0-10
- Extract 1-2 key facts related to the question
- Run in parallel, batched to respect rate limits

**Step 4 — Digest assembly** (cheap model)
Given top-ranked results with extracted facts:
- Produce a synthesis paragraph answering the question
- Include citations (filenames, URLs)
- Enforce context budget — trim if over limit

### Context Budget

Configurable limit on total tokens returned by smart tools. Default: 4,000 tokens. This ensures the frontier model receives bounded, curated context regardless of corpus size.

### Graceful Degradation

If the cheap model is unavailable (API error, rate limit, no API key), smart tools degrade to returning raw search results — more context, but the query doesn't fail. This is logged as a warning.

## Configuration

### CLI

```
harvester_mcp [--output-dir <path>] [--agent-model <model-id>] [--context-budget <tokens>] [--log-dir <path>]
```

- `--output-dir` — path to the article corpus (default: `./output`)
- `--agent-model` — model ID for the cheap agent tier (default: derived from `harvester_engine::DEFAULT_TRIAGE_MODEL`)
- `--context-budget` — max tokens returned from smart tools (default: 4000)
- `--log-dir` — log directory (default: `<output-dir>/logs`)

API key from environment: `OPENAI_API_KEY` (same as the rest of Harvester).

### Startup Sequence

1. Parse CLI args
2. Initialize logging to `<log-dir>/mcp.log`
3. Load sidecars from output dir: entity index, summary cache, triage cache
4. Start MCP server on stdio
5. Register all tools
6. Serve requests

Sidecars are loaded once at startup. At ~1,461 articles, they fit comfortably in memory.

## Implementation Phases

### Phase 1: Skeleton + Raw Data Tools

- [ ] Create `crates/harvester_mcp/` with `Cargo.toml`, `main.rs`, workspace membership
- [ ] Add `rmcp` dependency with Tokio runtime and stdio transport
- [ ] Implement stdio JSON-RPC transport and tool registration
- [ ] Enumerate eligible article files at startup (frontmatter validation), build in-memory article index
- [ ] Log startup timing: article enumeration, sidecar loading, total startup duration
- [ ] Implement `search_articles` — in-process regex search over article index using `regex` crate
- [ ] Implement `read_article` — read file, return content
- [ ] Implement `list_articles` — scan `output/`, parse frontmatter, support date/title filters
- [ ] Implement `search_entities` — load `EntityIndex` via `harvester_io`, query in memory
- [ ] Implement `get_article_summary` — build URL→content_hash→summary secondary index at startup, look up by URL
- [ ] Extend `engine_logging` with configurable log file path
- [ ] Set up file logging to `output/logs/mcp.log`
- [ ] Log every tool call: name, params, result size, timing
- [ ] Update `README.md` with `harvester_mcp` description
- [ ] `cargo clippy --all-targets -- -D warnings` passes

**Evaluation checkpoint:** Connect to Claude Code, run queries, inspect `mcp.log`.

### Phase 2: Smart Agent Layer

- [ ] Implement cheap-model client using `harvester_engine` OpenAI provider
- [ ] Implement query expansion step (cheap model prompt)
- [ ] Implement candidate retrieval (merge regex search + entity search results)
- [ ] Implement relevance scoring step (cheap model, parallel batched)
- [ ] Implement digest assembly step (cheap model prompt)
- [ ] Implement context budget enforcement (token counting, trim to limit)
- [ ] Implement graceful degradation (fall back to raw results on LLM error)
- [ ] Implement `query_knowledge_base` tool wiring the steps together
- [ ] Enhanced logging: cheap-model prompts, responses, scores
- [ ] `cargo clippy --all-targets -- -D warnings` passes

**Evaluation checkpoint:** Run same queries as Phase 1, compare context usage and answer quality.

### Phase 3: Evaluation Skill

- [ ] Create superpowers skill `evaluate-mcp`
- [ ] Define structured evaluation workflow: tool call verification, log inspection, answer grounding
- [ ] Define benchmark query categories: factual lookup, temporal analysis, cross-entity synthesis, gap analysis
- [ ] Create per-query evaluation checklist: right tools called? logs consistent? context budget respected? answer grounded?
- [ ] Add guidance on reading `mcp.log` to diagnose issues
- [ ] Create `docs/mcp_evaluation/benchmark_queries.md` with predefined test queries and expected characteristics
- [ ] Create evaluation result template

**Evaluation checkpoint:** Use the skill to systematically evaluate Phases 1 and 2.

### Phase 4: Refinement

- [ ] Tune cheap-model prompts based on evaluation findings
- [ ] Fine-tune summary cache lookup policy (handle re-fetched articles with new content hashes, multiple summary versions)
- [ ] Consider digest caching for repeated queries
- [ ] Update `docs/EngineeringDiary.md` per Agents.md
- [ ] `cargo clippy --all-targets -- -D warnings` passes

## Open Questions

- **Cheap model selection:** Starting default is `harvester_engine::DEFAULT_TRIAGE_MODEL`. May need to test alternatives based on quality/speed/cost at evaluation time.
- **Sidecar refresh:** Phase 1 loads sidecars once at startup. If the corpus changes while the MCP server is running, a restart is needed. A file-watcher or reload command could be added later.
