# Phase 7 — Headless Batch Runner (Reviewed + Rewritten)

Generated: 2026-02-17

## 1) Review Summary (Current Code vs Existing Plan)

This section records what was verified in source before rewriting this plan.

### 1.1 Verified current state
- Workspace currently has crates: `harvester_app`, `harvester_core`, `harvester_engine`, `engine_logging`.
- `harvester_app` owns effect execution and many IO helpers under `crates/harvester_app/src/platform/`.
- `harvester_core` already contains the reducer and state needed for polling/download/triage orchestration.
- `cargo build` passes.
- `cargo test -p harvester_core` passes.

### 1.2 Confirmed mismatches or risks in the old Phase 7 plan
- `AppState` APIs suggested by the old plan (`ordered_completed_job_urls`, `triage_metadata_ready`, `pre_triage`) are currently `pub(crate)`, not public.
- `EffectRunner` is coupled to app-local modules and includes Windows-only browser launch inline.
- Path defaults still depend on `current_dir()` in several modules (`sources`, caches, output), which is fragile for Task Scheduler.
- `Script` source polling is explicitly unimplemented and currently emits failure.
- State persistence for completed jobs/pre-triage overrides is currently executed in `harvester_app` driver code, not by reducer-emitted effect.

### 1.3 Architecture concern to address now
To keep strict unidirectional data flow, the batch runner should not mutate state directly or do hidden side effects outside reducer/effect flow. Any new runtime control logic must dispatch `Msg`, run `update`, then execute returned `Effect` only.

## 2) Phase 7 Scope (Final)

### 2.1 In scope
- New headless batch binary runnable from Task Scheduler.
- Runs in a repeating cycle: Poll sources -> Download pipeline settles -> Pre-triage + Triage -> wait for next poll interval. Continues until interrupted (Ctrl+C).
- No briefing generation in this phase.
- `--dry-run` mode: poll + ingestion accounting only, no downloads/triage.
- Deterministic exit codes and structured end-of-run summary.
- Per-run cost reporting (tokens + microdollar cost from existing `LlmQuotaTracker`).
- Single-run lock to prevent overlap.

### 2.2 Out of scope
- Built-in cron/daemon scheduler inside app.
- New GUI for scheduling.
- Full script-source runtime implementation (explicit blocker/decision below).

## 3) Blockers and Early Decisions

### Blocker A: CWD-based defaults are unsafe for scheduled runs
`current_dir()` defaults can resolve to unexpected paths under Task Scheduler.

Decision:
- Introduce explicit runtime paths object and pass paths down.
- Treat CLI `--output-dir` as authoritative base.
- Derive cache/state/seen-set paths from output-dir, not process CWD.

### Blocker B: Script sources are not implemented
Current source polling path warns and fails for `SourceType::Script`.

Decision:
- Phase 7 ships with explicit behavior:
  - default: fail run with clear message if enabled script source exists;
  - optional `--allow-unsupported-sources` downgrades to warning and skips.

### Blocker C: Persistence side-channel in app driver
Current GUI persists state in `AppEventHandler::dispatch_msg`, outside reducer-emitted effects.

Decision:
- For Phase 7, keep behavior parity but isolate persistence in `harvester_io` runtime helpers.
- Add explicit follow-up technical debt item: migrate runtime state persistence to core effects in a separate hardening step.

### Blocker D: Lock stale detection via PID can be brittle
PID-only stale detection can fail via PID reuse and permission limits.

Decision:
- Use atomic lock acquisition (`create_new`) as the hard guarantee.
- Store metadata (`pid`, `started_utc`, `owner`) for diagnostics only.
- Provide explicit `--force-unlock` for operator override.

## 4) Target Architecture

## 4.1 New crate layout
- `crates/harvester_io`: shared IO modules + `EffectRunner` + runtime path helpers.
- `crates/harvester_batch`: CLI, lock handling, batch orchestration loop.
- `crates/harvester_app`: keeps Win32 UI and uses `harvester_io`.

```text
harvester_batch
  -> harvester_io
  -> harvester_core
  -> harvester_engine

harvester_app
  -> harvester_io
  -> harvester_core
  -> harvester_engine
  -> commanductui
```

### 4.2 Path contract (new)
Introduce a single `RuntimePaths` value in `harvester_io`:
- `output_dir: PathBuf`
- `sources_path: PathBuf`
- `contexts_dir: PathBuf`
- `prompts_dir: PathBuf`
- `summary_cache_path: PathBuf`
- `triage_cache_path: PathBuf`
- `seen_set_path: PathBuf`
- `state_path: PathBuf`

All IO stores and `EffectRunner` receive this struct (or relevant subset) instead of using `current_dir()` internally.

## 5) Detailed Implementation Plan

### Part 1: Create `harvester_io` crate and move shared modules

Files:
- `crates/harvester_io/Cargo.toml`
- `crates/harvester_io/src/lib.rs`
- `crates/harvester_io/src/runtime_paths.rs`
- moved modules from `crates/harvester_app/src/platform/`

Move/refactor:
- `source_loader.rs`
- `seen_set_store.rs`
- `summary_cache_store.rs`
- `triage_cache_store.rs`
- `prompt_template_store.rs`
- `persistence.rs`
- `effects.rs` -> `effect_runner.rs`

Key refactors:
- Replace `pub(crate)` with public APIs suitable across crates.
- Remove implicit cwd path builders; use `RuntimePaths`.
- Keep `lib.rs` thin and re-export only stable entry points.

### Part 2: Decouple platform-only effect handling

In `harvester_io/src/effect_runner.rs`:
- Introduce trait:
  - `PlatformEffectHandler::open_url(&self, url: &str)`
- Provide:
  - `NoOpPlatformHandler` for batch
  - `Win32PlatformHandler` in `harvester_app`
- Route `Effect::OpenUrlInBrowser` through handler.

Result:
- `harvester_io` remains non-UI and batch-safe.
- `harvester_app` keeps Win32 shell behavior.

### Part 3: Introduce `harvester_batch` crate

Files:
- `crates/harvester_batch/Cargo.toml`
- `crates/harvester_batch/src/main.rs`
- `crates/harvester_batch/src/cli.rs`
- `crates/harvester_batch/src/lock.rs`
- `crates/harvester_batch/src/runner.rs`

Workspace updates:
- add `crates/harvester_io` and `crates/harvester_batch` to workspace members.
- add CLI dependency (`clap`) at workspace level if needed.

### Part 4: CLI contract

Required behavior:
- `--sources <path>` default `sources.ron`
- `--output-dir <path>` default `output`
- `--contexts-dir <path>` default `contexts`
- `--prompts-dir <path>` default `prompts`
- `--llm-concurrency <n>` default `3`, clamp `[1,10]`
- `--force-unlock` optional
- `--allow-unsupported-sources` optional
- `--dry-run` optional (poll + ingestion accounting only, no downloads/triage)
- `--poll-interval <minutes>` default `15`, clamp `[1, 1440]` — wait time between end of one cycle and start of next poll
- `OPENAI_API_KEY` required for triage stage (not required for `--dry-run`)

### Part 5: Lock implementation

Lock file:
- `{output_dir}/.harvester_batch.lock`

Acquire flow:
1. Attempt atomic create-new lock file.
2. If exists: read metadata and fail with `AlreadyRunning` unless `--force-unlock`.
3. On `--force-unlock`: replace lock and continue.

Metadata payload:
- `pid`
- `started_utc`
- `owner` (stable random/unique token)
- `command` (optional diagnostic)

Release:
- RAII guard removes lock in `Drop`.

### Part 6: Batch runner orchestration loop

Startup:
1. Init logging (file-only if `--dry-run`, see Part 7).
2. Parse CLI and build `RuntimePaths`.
3. Acquire lock.
4. Register signal handler (Ctrl+C / `SIGINT` / `SIGTERM`). Sets a shared shutdown flag.
5. Build reducer state and effect runner (same LLM bootstrap model as app).
6. Hydrate persisted artifacts (completed jobs, summary/triage cache, pre-triage overrides, prompt files, metadata).

Main loop (repeating cycles):
7. Dispatch `Msg::PollSourcesClicked`.
8. Process messages through `update` and execute emitted effects until cycle settles (poll done -> downloads done -> triage done for this batch).
   - In `--dry-run` mode: stop after first poll settles. Do not dispatch download or triage. Print progress per Part 7 and exit.
   - After each triage completion: print progress line to stdout (see Part 7).
9. Print cycle summary (items found, triaged, cumulative cost).
10. Check shutdown flag. If set, break to shutdown.
11. Sleep for `--poll-interval` minutes (interruptible — if shutdown signal arrives during sleep, wake immediately and break).
12. Go to step 7.

Shutdown:
13. Stop dispatching new work. Wait briefly (30s drain timeout) for in-flight LLM calls to complete.
14. Persist state (same artifacts as step 6 hydration).
15. Read `LlmQuotaTracker` snapshot, print final summary (Part 8), exit.

Loop mechanism:
- Event-driven: each inner iteration awaits the next `Msg` from effect completion via channel. No busy-polling.
- Between cycles, sleep is interruptible by the shutdown signal.

Graceful shutdown (Ctrl+C):
- Expected stop mechanism for long runs. Not treated as an error.
- In-flight LLM calls get a 30s drain window; after that, abandon and persist what we have.

Cycle settlement detection:
- After each `update`, check `batch_observation()` snapshot.
- A cycle is settled when: poll is complete, no downloads in flight, and triage queue is empty (all items triaged or failed).
- If a poll finds zero new items, the cycle settles immediately; the runner still waits for the next interval (new items may appear later).

Terminal conditions (per cycle):
- Cycle settled with zero failures -> cycle success.
- Cycle settled with partial failures -> cycle partial.
- Fatal runtime/setup errors -> exit immediately.

State observation API recommendation:
- Prefer a single public snapshot method for batch orchestration instead of exposing many internals.
- Add something like `AppState::batch_observation()` returning:
  - poll in progress
  - completed job counts
  - job settlement counts
  - pre-triage phase
  - triage phase and counts
- Expose `LlmQuotaTracker` snapshot (cumulative calls, input/output tokens, cost_microdollars) for end-of-run reporting.

This keeps encapsulation stronger than promoting multiple `pub(crate)` getters.

### Part 7: Console output modes

The batch runner has two output personalities depending on `--dry-run`:

**Normal mode (no `--dry-run`):**
- `engine_logging` writes to `engine.log` as usual.
- Console (stderr) receives engine log output at the configured level.
- Periodic progress lines printed to stdout (not from logging framework):
  - After each triage completion: `[batch] 5/18 triaged | llm_calls=5 cost=$0.035 elapsed=12m`
  - After each cycle settles: `[batch] cycle #2 done | new_items=8 triaged=8 | cumulative: llm_calls=24 cost=$0.142 elapsed=1h32m`
  - During inter-cycle sleep: no output (quiet).
- Final summary line printed to stdout on exit (see Part 8).

**Dry-run mode (`--dry-run`):**
- `engine_logging` still writes to `engine.log` (full detail preserved).
- Console log output is suppressed (no engine log lines on stderr).
- Instead, a compact progress stream is printed to stdout:
  - Phase transitions: `[dry-run] polling 3 sources...`, `[dry-run] poll complete`.
  - Per-source one-liner: `[dry-run] source "HN" found 12 new items (4 previously seen)`.
  - Final accounting summary (see Part 8).
- The goal is a quick-glance confirmation that something happened, not a debug trace.

Implementation:
- Configure `engine_logging` with a file-only appender when `--dry-run` is active (no stderr layer).
- Progress lines are direct `println!` from the orchestration loop, not from the logging framework.
- Progress is driven by observing state transitions via `AppState::batch_observation()` between loop iterations.

### Part 8: Exit codes, summaries, and cost reporting

Exit codes:
- `0`: all completed work succeeded (or nothing eligible to triage)
- `1`: partial completion (some failures among completed work, outputs persisted)
- `2`: fatal (startup/lock/config/API key)

On graceful shutdown via signal (Ctrl+C): use `0` if all completed work succeeded, `1` if there were partial failures. The signal itself is not an error — it is the expected way to stop a long run.

Always emit final summary to stdout, on both natural cycle exit and signal-interrupted shutdown. The summary includes cost information sourced from the existing `LlmQuotaTracker` (which already accumulates per-request `input_tokens`, `output_tokens`, and `cost_microdollars` via `ModelPricing`).

Normal mode example:
- `[batch] finished cycles=3 elapsed=3h12m jobs_total=54 jobs_success=48 triage_completed=45 triage_failed=3 llm_calls=45 input_tokens=148200 output_tokens=10800 cost=$0.2531 exit_code=1`

Dry-run mode example:
```
[dry-run] summary
  sources polled:    3
  new items found:  12
  previously seen:   4
  eligible:          8  (would proceed to download + triage)
  duration:        1.2s
  llm_calls:         0
  cost:          $0.00
```

Cost formatting:
- Use `LlmQuotaTracker::snapshot()` (or equivalent read method) at end of run to get accumulated totals.
- Display cost as `$X.XXXX` (4 decimal places, converted from microdollars: `cost_microdollars / 1_000_000`).
- Token counts displayed as plain integers.
- This requires no OpenAI admin API key — all data comes from per-request `usage` fields already captured in the response handling path (`handle.rs`).

## 6) Testing Strategy

### 6.1 Unit tests (required)
- `lock.rs`: acquire/reject/force-unlock/release.
- CLI parsing and clamp rules (including `--dry-run`, `--poll-interval`).
- batch run-state classifier (full/partial/fatal) as pure function.
- `RuntimePaths` derivation tests (no cwd leakage).
- cost formatting: microdollars to `$X.XXXX` string conversion.

### 6.2 Core reducer tests (required additions)
Add or extend reducer tests that lock in batch-critical behavior:
- polling completion to idle state transitions.
- pre-triage auto-ready path when no manual review required.
- triage complete vs failed handling for mixed outcomes.

### 6.3 Integration tests
- headless run against local mock server and temporary output dir.
- lock contention scenario with second process.
- run with unsupported script source and verify configured behavior.
- cache/state persistence is readable by `harvester_app` startup path.
- `--dry-run` produces no LLM calls, exits 0 after first poll, and prints progress to stdout.
- `--dry-run` still creates `engine.log` with expected content.
- graceful shutdown: send SIGINT during run, verify drain + summary + clean exit.
- repeating cycle: verify second poll dispatches after interval elapses.

## 7) Verification Commands

Run in order:
1. `cargo build`
2. `cargo test -p harvester_core`
3. `cargo test -p harvester_io` (new)
4. `cargo test -p harvester_batch` (new)
5. `cargo build -p harvester_app`
6. `cargo build -p harvester_batch`
7. `cargo clippy --all-targets -- -D warnings`

Manual verification:
- `cargo run -p harvester_batch -- --output-dir output --sources sources.ron`
- `cargo run -p harvester_batch -- --output-dir output --sources sources.ron --dry-run` (confirm clean progress output, no log spam, engine.log still written).
- verify lock behavior via two concurrent invocations.
- verify non-zero exit codes for partial/fatal scenarios.
- verify cost line in final summary after a real triage run.
- open GUI and confirm triage/cache/state hydration parity.

## 8) Future Ideas Mapping

### Close when Phase 7 completes
- `FI-Ingestion-SourceDryRun-0006` (`--dry-run` is now in scope).

### Not closed by this phase
- `FI-Ingestion-Scheduling-0004`: this phase is scheduler-ready, not per-source interval scheduling.
- `FI-Ingestion-SourceCursoring-0005`: not implemented here.
- `FI-LLM-RetryPolicy-0001`: not implemented here.
- `FI-Networking-RequestScheduling-0001`: not implemented here.
- `FI-Observability-SourceHealth-0006` and `FI-Observability-SourceHealth-0007`: partial logging only, no full telemetry/backoff policy yet.
- `FI-Storage-ExportArtifacts-0001`: briefing export out of scope for this phase.

## 9) Implementation Notes for the Agent

- Keep `main.rs` and `lib.rs` thin wrappers.
- Use `engine_logging` macros for all logs (`[batch]` category prefix).
- Avoid hard-coded path/string limits.
- Keep reducer pure; orchestration loop dispatches messages only.
- Do not introduce mutable shared shadow state in batch runner.
- Add tests for every bug fix discovered during implementation.
