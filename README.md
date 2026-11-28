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

## API keys

From PowerShell under the Windows account that will run Harvester, store each key
in Windows Credential Manager.

```powershell
cmdkey /generic:Harvester/OpenAI /user:harvester /pass
cmdkey /generic:Harvester/BraveSearch /user:harvester /pass
```

Each command prompts for the key value. Paste or type it at the prompt. Use
Harvester-specific keys with provider spending limits.

Both launchers ask Harvester to read the stored keys at startup. Starting either
binary directly without the launcher flag leaves it key-free. At startup the
batch prints the status of each credential, including the character count and
encoding for a found key. When a launcher asked for stored keys, the desktop
header shows a single line such as `Keys: OpenAI found, Brave missing`; each
key is `found`, `missing`, or `unreadable`. Started without the launcher flag,
the desktop shows no key line.

Every Brave source uses `Harvester/BraveSearch`. New entries in
`output/.sources.ron` omit `api_key_env`. Older entries still load, and the poll
worker logs the source IDs that carry an ignored key name once per run. If the
Brave key is missing, only Brave sources due for a search report a failure.
Recently searched Brave sources still skip, and other sources continue polling.

## Common Commands

Build the workspace:

```powershell
cargo build
```

Run the Tauri desktop UI:

```powershell
.\scripts\Start-HarvesterUi.ps1
```

Launch one command-line cycle (poll, download, synchronous processing, then exit):

```powershell
.\scripts\Start-HarvesterBatch.ps1
```

With no arguments, the batch launcher builds the working tree and supplies the
fixed stored-key flag. External scheduling starts a new process for each cycle. Run
`harvester_batch --help` for browser-page import and checkpoint commands.

The unattended morning run uses the local release branch `morning`.
Before the first run, create it at the commit on `main` whose full checks passed
and which the Orchestrator named in `advance morning to <hash>`. Use the same
command for each approved fast-forward. It prints the old and new commit hashes.

```powershell
.\scripts\Advance-MorningBranch.ps1 -Commit <hash>
```

Run the morning batch and write the portfolio archive:

```powershell
.\scripts\Start-HarvesterBatch.ps1 --archive-out C:\Users\larsp\src\AI_portfolio\archive.md
```

The launcher accepts only `--archive-out <absolute path>` or `--archive-out=<absolute path>`.
The target's parent directory must exist, and the target must be outside the batch
output directory. A morning run requires the stored OpenAI key. A missing Brave
key counts as a failed source and other sources still poll. The morning run
prints and logs the ID and reason for each failed source.

The launcher manages `C:\Users\larsp\src\harvester-morning` as a separate clone
of the local `morning` branch. Commits on `main` take effect in the morning run
when the release branch advances. The launcher creates the clone on the first
run and accepts only fast-forward updates. It uses `cargo build --locked` to
build the batch executable there, then runs it with the main repository's
output, sources, contexts, and prompts.

The launcher or batch executable ends each reported morning outcome with one
ASCII line in the form `HARVESTER_RESULT {json}` with `"v":2`. Exit codes are 0 for `exported`, 10 for
`nothing_new`, 20 for `cycle_failed`, 21 for `export_failed`, 22 for a held output
lock, 23 for `update_failed`, 24 for `build_failed`, and 2 for `usage_error`.
Exit 24 also covers an executable that the launcher cannot start.
The JSON includes the commit hash, archive path, window bounds, source and
article counts, and estimated OpenAI cost when available. The `left_out` array
names articles missing required results by URL, title and reason, even on a
`nothing_new` morning. A quota, provider-credit, or repeated rate-limit halt
gives `cycle_failed` with reason `quota_halted`. The binary updates the checkpoint
after publication. Plain runs keep their existing exit codes.
See [Archive Export Format](docs/ArchiveExportFormat.md#batch-result-line).

Each cycle shows a compact progress block with elapsed time, cost so far, and
remaining work for source scanning, downloading, triage, summaries and scoring.
Counts match the desktop and exclude reused work; stopping shows work done.
Terminal output redraws in place; redirected output prints stage-status changes,
the start of stopping, changed forced updates and a 60-second heartbeat. Count-only
changes wait for the next update; identical lines are suppressed. Ctrl+C stops
safely; pressing it again exits immediately. The former `--verbose-progress` and
`--ascii-progress` options are removed.

Checkpoint commands are `--set-checkpoint <RFC3339>`, `--set-checkpoint-now`,
`--clear-checkpoint` and `--show-checkpoint`. The old `--set-briefing-since`,
`--set-briefing-since-now`, `--clear-briefing-since` and `--show-briefing-since`
spellings remain accepted as hidden aliases. The file remains
`.briefing_checkpoint.ron`.

The launchers can start unattended under the Windows account that stored the
keys. Any process running as Lars, including an LLM coding agent, can now read
the entries directly without a human gate. The only remaining guard against
agents reading the entries is the `Agents.md` rule that agents never obtain
API keys. Agents use the key-free build and test paths. If either legacy key
environment variable is set at any scope, the launchers warn that Harvester
ignores it.

## Harvester Output

The Harvester tools operate on an `output/` directory containing harvested article markdown files plus derived caches such as:

- `harvester-corpus.json`
- `.sources.ron`
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
