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
- One run performs: Poll sources -> Download pipeline settles -> Pre-triage + Triage.
- No briefing generation in this phase.
- Deterministic exit codes and structured end-of-run summary.
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
- `OPENAI_API_KEY` required for triage stage

Nice-to-have in same phase:
- `--dry-run` (poll + ingestion accounting only, no downloads/triage)

If `--dry-run` lands, mark `FI-Ingestion-SourceDryRun-0006` as completed.

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

Main loop responsibilities:
1. Init logging.
2. Parse CLI and build `RuntimePaths`.
3. Acquire lock.
4. Build reducer state and effect runner (same LLM bootstrap model as app).
5. Hydrate persisted artifacts (completed jobs, summary/triage cache, pre-triage overrides, prompt files, metadata).
6. Dispatch `Msg::PollSourcesClicked`.
7. Loop until terminal condition, processing messages through `update` and executing emitted effects.
8. Determine final run status and exit code.

Terminal conditions:
- Poll done and no successful completed jobs -> success (nothing to triage).
- Triage phase complete with zero failures -> success.
- Triage phase complete with partial failures -> partial.
- Fatal runtime/setup errors -> fatal.

State observation API recommendation:
- Prefer a single public snapshot method for batch orchestration instead of exposing many internals.
- Add something like `AppState::batch_observation()` returning:
  - poll in progress
  - completed job counts
  - job settlement counts
  - pre-triage phase
  - triage phase and counts

This keeps encapsulation stronger than promoting multiple `pub(crate)` getters.

### Part 7: Exit codes and summaries

Exit codes:
- `0`: full success (or nothing eligible to triage)
- `1`: partial completion (some failures, outputs persisted)
- `2`: fatal (startup/lock/config/API key)

Always emit final one-line summary, for example:
- `[batch] completed duration=42.1s jobs_total=18 jobs_success=15 triage_completed=12 triage_failed=3 exit_code=1`

## 6) Testing Strategy

### 6.1 Unit tests (required)
- `lock.rs`: acquire/reject/force-unlock/release.
- CLI parsing and clamp rules.
- batch run-state classifier (full/partial/fatal) as pure function.
- `RuntimePaths` derivation tests (no cwd leakage).

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
- verify lock behavior via two concurrent invocations.
- verify non-zero exit codes for partial/fatal scenarios.
- open GUI and confirm triage/cache/state hydration parity.

## 8) Future Ideas Mapping

### Close when Phase 7 completes
- `FI-Ingestion-SourceDryRun-0006` (only if `--dry-run` is implemented in this phase).

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
