# web_page_filet_mignon
<p align="left">
  <img src="resources/app-image.jpg" alt="web_page_filet_mignon application screenshot" width="400" />
</p>

Serve your LLM the premium cut. `web_page_filet_mignon` is a Rust workspace for collecting web pages, extracting clean text, triaging and summarizing articles, and exposing the resulting corpus to MCP clients such as Codex and Claude.

You need an OpenAI API key, but the costs are intentionally kept low by pushing as much work as possible into deterministic processing.

## What Is In This Repo

- `harvester_app` — the native desktop application
- `harvester_batch` — batch-oriented ingestion/export pipeline
- `harvester_mcp` — MCP server that exposes the harvested corpus as a queryable knowledge base
- `scripts/` — launchers, smoke tests, and supporting PowerShell utilities
- `docs/` — architecture notes, plans, prompt context, and engineering diary

## Prerequisites

- Rust toolchain with `cargo`
- PowerShell 7 for the modern launcher and smoke-test scripts
- `OPENAI_API_KEY` set in the environment if you want smart-query features

## Common Commands

Build the workspace:

```powershell
cargo build
```

Run the desktop app:

```powershell
cargo run -p harvester_app
```

Launch the batch workflow UI:

```powershell
pwsh -NoLogo -NoProfile -File .\scripts\Start-HarvesterBatch.ps1
```

Run the MCP smoke test against a query:

```powershell
pwsh -NoLogo -NoProfile -File .\scripts\Test-HarvesterMcpSmoke.ps1 `
  -Query "Which companies appear best positioned to meet rising AI demand through data centers?"
```

## Harvester Output

The Harvester tools operate on an `output/` directory containing harvested article markdown files plus derived caches such as:

- `.entity_index.ron`
- `.summary_cache.ron`
- `.triage_cache.ron`

For day-to-day use across multiple worktrees, the recommended setup is:

- keep one shared canonical output folder outside the worktrees
- point each launcher or MCP registration at that shared output folder
- keep the MCP server binary or source workspace local to the branch you are actively using

That gives you one source of truth for data while still letting each worktree run its own `harvester_mcp` code.

## MCP Server

`harvester_mcp` is a stdio MCP server that exposes the Harvester article corpus to Codex, Claude, and other compatible clients.

### Direct Usage

```powershell
cargo run -q -p harvester_mcp -- --output-dir .\output
```

Or with an already-built binary:

```powershell
.\target\debug\harvester_mcp.exe --output-dir .\output
```

Current CLI options:

```text
harvester_mcp
  [--output-dir <path>]
  [--log-dir <path>]
  [--agent-model <model-id>]
  [--context-budget <tokens>]
  [--scoring-candidate-cap <n>]
  [--too-broad-threshold <n>]
  [--min-triage-priority <n>]
  [--retain-log-runs <n>]
```

Logs are written to `<output-dir>/logs/mcp.log` by default, with older runs retained as `mcp.log.1` through `mcp.log.9` unless you override `--retain-log-runs`.

### Tools

- `search_articles` — regex search across the article corpus
- `read_article` — read a single article by filename
- `list_articles` — list articles with optional date/title filters
- `search_entities` — search the entity index by company, technology, product, or theme
- `get_article_summary` — retrieve a cached article summary by URL
- `query_knowledge_base` — expand a free-text question, rank relevant articles, and assemble a bounded digest

### Recommended Launcher For MCP Clients

Use the PowerShell launcher in this repo instead of registering a specific binary path globally:

```powershell
pwsh -NoLogo -NoProfile -File .\scripts\Start-HarvesterMcp.ps1 `
  -ProjectRoot C:\path\to\your\workspace `
  -OutputDir C:\path\to\shared-output
```

Why this is recommended:

- the launcher can target the current workspace or worktree explicitly
- the article corpus can live in one shared output directory
- you avoid hard-coding a single global `harvester_mcp.exe` while the server is still evolving
- the launcher prefers `target\debug\harvester_mcp.exe` and falls back to `cargo run` if needed

### Generic MCP Registration Example

Most MCP clients accept a stdio command plus arguments. A typical registration looks like:

```json
{
  "command": "pwsh",
  "args": [
    "-NoLogo",
    "-NoProfile",
    "-File",
    "C:\\path\\to\\web_page_filet_mignon\\scripts\\Start-HarvesterMcp.ps1",
    "-ProjectRoot",
    "C:\\path\\to\\web_page_filet_mignon",
    "-OutputDir",
    "C:\\path\\to\\shared-output"
  ]
}
```

If your MCP client supports workspace-local configuration, prefer that over a global registration. If it only supports global registration, keep the registered command stable and point it at the workspace and shared output folder you want to use.

## Documentation

- [docs/ApplicationDescription.md](docs/ApplicationDescription.md)
- [docs/Architecture.md](docs/Architecture.md)
- [docs/PromptContextFiles.md](docs/PromptContextFiles.md)
- [docs/ThreatModel.md](docs/ThreatModel.md)
- [docs/plans/Plan.McpKnowledgeBaseServer.md](docs/plans/Plan.McpKnowledgeBaseServer.md)
