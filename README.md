# web_page_filet_mignon
Serve your LLM the premium cut. `web_page_filet_mignon` is a Rust workspace
for collecting web pages, extracting clean text, triaging and summarizing
articles, and exposing the resulting corpus through batch and desktop tools.

You need an OpenAI API key, but the costs are intentionally kept low by pushing as much work as possible into deterministic processing.

The supported desktop experience is a Tauri window backed by the Rust core and
the built frontend. Deterministic processing keeps API use and costs bounded.

## Repository tour

- `crates/harvester_core` — domain state, reducer/update logic, pipeline
  messages, and view snapshots
- `crates/harvester_engine` — fetching, extraction, persistence, and LLM
  workflows
- `crates/harvester_io` — runtime paths, host bootstrap, run locking, and
  effect execution
- `crates/harvester_ui_bridge` — the restricted desktop IPC contract and
  snapshot projection
- `crates/harvester_ui` — the non-default Tauri desktop host
- `crates/harvester_batch` — batch-oriented ingestion and export host
- `crates/engine_logging` and `crates/openai_provider_kit` — shared workspace
  support crates
- `frontend/` — the separately built desktop frontend
- `scripts/` — launchers and supporting PowerShell utilities
- `docs/` — architecture, security, corpus format, plans, and engineering
  history

## Prerequisites

- Rust toolchain with `cargo`
- Node.js/npm for the desktop frontend
- Microsoft Edge WebView2 Evergreen Runtime for the Tauri desktop window
- PowerShell 7
- A PowerShell profile that loads the external `SecretLaunch` module and
  provides `Invoke-WithSecretMap` and `Test-SecretStorePromptAvailable`, plus
  SecretStore vault entries named `BraveSearchApiKey` and
  `OpenAIProductionKey`. Run launchers from a session with that profile loaded;
  they do not load it themselves.
- The launchers retrieve `BRAVE_SEARCH_API_KEY` and `OPENAI_API_KEY` from
  encrypted SecretStore entries and inject them into the launched process. If
  either variable already has a non-empty parent value, the launcher warns
  rather than refuses; that value is inherited by Cargo and the child process.

Each Brave source names its own key environment variable with `api_key_env` in
`output/.sources.ron`; the name in use is `BRAVE_SEARCH_API_KEY`, so a Brave
source added by hand should use that same name rather than inventing a second
one.

## Common Commands

Build the workspace:

```powershell
cargo build
```

Run the Tauri desktop UI:

```powershell
.\scripts\Start-HarvesterUi.ps1
```

Launch the batch workflow:

```powershell
.\scripts\Start-HarvesterBatch.ps1
```

The launchers are interactive because they never add vault secrets to the
environment of a process an LLM coding agent controls or can spawn: harvested
article content flows into agent context, and environment variables are
inherited downward. Retrieving a secret therefore requires typing the
SecretStore password into a session the user started. This stops inheritance,
not an agent editing launcher, helper, or application source that the user
later runs with real keys.

## Harvester Output

The Harvester tools operate on an `output/` directory containing harvested article markdown files plus derived caches such as:

- `harvester-corpus.json`
- `.sources.ron`
- `.entity_index.ron`
- `.summary_cache.ron`
- `.triage_cache.ron`

`harvester-corpus.json` is the public corpus format marker for applications
that read the output folder directly. Its `schema_version` is the compatibility
signal for the article layout. External readers should treat root `*.md` files
and `linked/*.md` files as article records, and should treat hidden `.ron` files
as Harvester-internal state. See [docs/CorpusFormat.md](docs/CorpusFormat.md).
The editable source registry is stored as `output/.sources.ron` by default, so
backing up the output directory also preserves the configured web sources.

For day-to-day use across multiple worktrees, each worktree runs its own binaries against its own `output/` folder.

## Documentation

- [docs/ApplicationDescription.md](docs/ApplicationDescription.md)
- [docs/Architecture.md](docs/Architecture.md)
- [docs/CorpusFormat.md](docs/CorpusFormat.md)
- [docs/PromptContextFiles.md](docs/PromptContextFiles.md)
- [docs/ThreatModel.md](docs/ThreatModel.md)
