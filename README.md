# web_page_filet_mignon
<p align="left">
  <img src="resources/app-image.jpg" alt="web_page_filet_mignon application screenshot" width="400" />
</p>

Serve your LLM the premium cut. A native Rust Windows app for collecting web pages, extracting clean text, and preparing reliable context for analysis. It emphasizes deterministic processing, security boundaries for untrusted content, and a clear, message-driven workflow. The experience focuses on previewing extracted content, tracking progress and budgets, and generating briefings from completed pages.

You need an Open-AI API key, but the costs are very low.

## Documentation
- [docs/ApplicationDescription.md](docs/ApplicationDescription.md)
- [docs/Architecture.md](docs/Architecture.md)
- [docs/PromptContextFiles.md](docs/PromptContextFiles.md)
- [docs/ThreatModel.md](docs/ThreatModel.md)

## harvester_mcp

An MCP server that exposes the Harvester article corpus to Claude Code and Codex as a queryable knowledge base.

### Usage

```
harvester_mcp [--output-dir <path>] [--log-dir <path>] [--agent-model <model-id>] [--context-budget <tokens>]
```

### Tools

- `search_articles` — regex search across the article corpus
- `read_article` — read a single article by filename
- `list_articles` — list articles with optional date/title filters
- `search_entities` — search the entity index by company, technology, product, or theme
- `get_article_summary` — retrieve a cached article summary by URL
- `query_knowledge_base` — expand a free-text question, rank relevant articles, and assemble a bounded digest

Logs are written to `<output-dir>/logs/mcp.log`.
