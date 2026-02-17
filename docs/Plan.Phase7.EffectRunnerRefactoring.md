# Phase 7 — EffectRunner Refactoring & Batch Orchestration (Reviewed + Rewritten)

Generated: 2026-02-17

## 1) Current state check (verified in source)

This rewrite reflects current implementation status:

- Shared IO crate exists with runtime paths and stores: [crates/harvester_io/src/lib.rs](crates/harvester_io/src/lib.rs), [crates/harvester_io/src/runtime_paths.rs](crates/harvester_io/src/runtime_paths.rs), [crates/harvester_io/src/persistence.rs](crates/harvester_io/src/persistence.rs).
- Shared `EffectRunner` is still a stub: [crates/harvester_io/src/effect_runner.rs](crates/harvester_io/src/effect_runner.rs#L1-L28).
- Full effect execution logic still lives in app-local module: [crates/harvester_app/src/platform/effects.rs](crates/harvester_app/src/platform/effects.rs).
- App still uses app-local `EffectRunner`: [crates/harvester_app/src/platform/app.rs](crates/harvester_app/src/platform/app.rs#L26), [crates/harvester_app/src/platform/app.rs](crates/harvester_app/src/platform/app.rs#L96-L105).
- Batch crate exists but orchestration is placeholder: [crates/harvester_batch/src/runner.rs](crates/harvester_batch/src/runner.rs#L1-L31).
- `AppState::batch_observation()` does not exist yet; batch-critical getters remain `pub(crate)`: [crates/harvester_core/src/state.rs](crates/harvester_core/src/state.rs#L658-L664), [crates/harvester_core/src/state.rs](crates/harvester_core/src/state.rs#L1300-L1302).

This means the plan must prioritize extraction completion before batch loop work.

---

## 2) Critical blockers and design corrections

### Blocker 1 — Shared runner not implemented
Until [crates/harvester_io/src/effect_runner.rs](crates/harvester_io/src/effect_runner.rs) is real, both app migration and batch orchestration are blocked.

### Blocker 2 — Lock acquisition is race-prone
Current lock flow uses `exists()` + `write()`, which is not atomic: [crates/harvester_batch/src/lock.rs](crates/harvester_batch/src/lock.rs#L35-L86).

Fix requirement:
- use atomic create-new (`OpenOptions::new().create_new(true)`) as hard guarantee;
- write owner metadata after successful create;
- in `Drop`, remove lock only if owner token still matches file content.

### Blocker 3 — Runtime paths can still leak relative defaults
`RuntimePaths::with_defaults()` currently introduces relative values for sources/contexts/prompts: [crates/harvester_io/src/runtime_paths.rs](crates/harvester_io/src/runtime_paths.rs#L41-L48).

Decision:
- keep CLI explicit paths authoritative in batch;
- in app migration, avoid `with_defaults()` unless intentionally preserving old behavior.

### Blocker 4 — Batch observation API missing
Without a public snapshot API, runner code will be forced to depend on internals or duplicate logic.

---

## 3) Architecture guardrails (must hold)

1. Preserve unidirectional flow: runner only dispatches `Msg` -> calls `update()` -> executes emitted `Effect`.
2. No direct state mutation in batch driver.
3. Reducer remains pure; IO only in `EffectRunner` / IO modules.
4. Keep app runnable after each commit.
5. Thin entry files (`main.rs`, `lib.rs`, `mod.rs`) and behavior-focused APIs.

---

## 4) Rewritten implementation plan

## Phase A — Complete extraction in `harvester_io`

### A1. Extract helper module from app effects

Create [crates/harvester_io/src/effect_helpers.rs](crates/harvester_io/src/effect_helpers.rs) and move portable functions/types from [crates/harvester_app/src/platform/effects.rs](crates/harvester_app/src/platform/effects.rs):

- `build_local_model_catalog()`
- `prompt_context_filename()`
- `RssPollContext` + `handle_rss_source_poll()`
- `PollGuard`
- `fetch_feed()`
- `download_link_page()`
- `map_stage()`
- `map_llm_event()`
- `MAX_FEED_RESPONSE_BYTES`, `FEED_ACCEPT_HEADER`

Changes:
- remove all implicit default path helpers from helper code;
- pass explicit `&RuntimePaths` or explicit `&Path` inputs;
- keep Windows-specific code out of helper module.

Commit gate: `cargo build --workspace` + `cargo nextest run`.

### A2. Implement full shared `EffectRunner`

Implement `EffectRunner` in [crates/harvester_io/src/effect_runner.rs](crates/harvester_io/src/effect_runner.rs) with constructor:

- `RuntimePaths`
- `mpsc::Sender<Msg>`
- optional `LlmHandle` + model metadata + prompt registry
- `Box<dyn PlatformEffectHandler>`

Requirements:
- effect validation parity with existing app runner (`EnqueueUrl`, `DownloadLinkedPage`, `DeleteLinkedPage`, LLM input length);
- `PollAllSources` uses `paths.sources_path`, `paths.seen_set_path`;
- cache/template/context effects use `paths.*` (never cwd);
- GUI-only effects in batch mode are safe no-op or warning (never panic).

Commit gate: `cargo nextest run`.

### A3. Port tests from app effects to IO runner

Port and adapt tests currently in [crates/harvester_app/src/platform/effects.rs](crates/harvester_app/src/platform/effects.rs#L1579-L2032) into `harvester_io`:

- URL policy rejection tests;
- context/template save/load tests;
- article load + resolver tests;
- LLM event mapping metadata propagation tests.

Add new tests for path-driven behavior:
- context/template paths honor `RuntimePaths`;
- `PollAllSources` reads configured `sources_path` and persists seen-set to configured path.

Commit gate: `cargo test -p harvester_io` + `cargo nextest run`.

---

## Phase B — Migrate `harvester_app` to shared runner

### B1. Add dependency + Win32 platform handler

- Add `harvester_io` to [crates/harvester_app/Cargo.toml](crates/harvester_app/Cargo.toml).
- Add [crates/harvester_app/src/platform/win32_platform_handler.rs](crates/harvester_app/src/platform/win32_platform_handler.rs) implementing `PlatformEffectHandler::open_url()` with current `ShellExecuteW` logic.

Commit gate: `cargo nextest run`.

### B2. Switch app wiring to shared runner and IO loaders

In [crates/harvester_app/src/platform/app.rs](crates/harvester_app/src/platform/app.rs):

- replace app-local `EffectRunner` usage with `harvester_io::EffectRunner`;
- construct explicit `RuntimePaths` from app config;
- use `harvester_io` loaders for completed jobs, caches, overrides, templates.

Note: preserve current app behavior first; no product behavior change in this step.

Commit gate: `cargo nextest run` + manual app smoke run.

### B3. Delete dead app-local effect runtime

- remove [crates/harvester_app/src/platform/effects.rs](crates/harvester_app/src/platform/effects.rs);
- remove redundant app-local store modules now superseded by `harvester_io` where feasible;
- keep wrappers only if needed temporarily for compatibility.

Commit gate: `cargo nextest run` and final `cargo clippy --workspace --all-targets -- -D warnings`.

---

## Phase C — Add batch observation API in core

### C1. Introduce `BatchObservation`

Add public snapshot API in [crates/harvester_core/src/state.rs](crates/harvester_core/src/state.rs):

- `pub struct BatchObservation`
- `pub fn batch_observation(&self) -> BatchObservation`

Suggested fields:

- `poll_in_progress: bool`
- `session_state: SessionState`
- `jobs_total`, `jobs_done`, `jobs_failed`, `jobs_in_flight`
- `pre_triage_phase: PreTriagePhase`
- `triage_phase: TriagePhase`
- `triage_total`, `triage_pending`, `triage_in_flight`, `triage_completed`, `triage_failed`

Testing:
- table-driven reducer-level tests for representative states;
- settlement predicate tests (poll done + no in-flight + triage drained).

Commit gate: `cargo test -p harvester_core` + `cargo nextest run`.

---

## Phase D — Implement batch runner orchestration

### D1. Startup, hydration, and validation

Implement in [crates/harvester_batch/src/runner.rs](crates/harvester_batch/src/runner.rs):

1. Build `RuntimePaths` from CLI.
2. Acquire lock.
3. Validate unsupported sources policy (`Script` + `--allow-unsupported-sources`).
4. Create `mpsc` channel.
5. Hydrate state via reducer messages (`RestoreCompletedJobs`, cache hydration, overrides).
6. Configure LLM unless `--dry-run`.
7. Build shared `EffectRunner` with `NoOpPlatformHandler`.

Commit gate: `cargo test -p harvester_batch` + `cargo nextest run`.

### D2. Inner dispatch loop

Implement blocking event loop function that:

- receives `Msg`;
- runs `update(state, msg)`;
- executes effects;
- performs persistence side-channel parity (for now);
- checks `batch_observation()` each iteration;
- exits on settlement or shutdown flag.

Add pure helper functions:
- `classify_cycle_outcome(observation: &BatchObservation) -> CycleOutcome`
- `should_settle_cycle(observation: &BatchObservation) -> bool`

Commit gate: targeted unit tests + `cargo nextest run`.

### D3. Outer repeating cycle

In `run()`:

- install signal handler with shared `AtomicBool`;
- dispatch `Msg::PollSourcesClicked` to start each cycle;
- run dispatch loop until settled;
- print cycle summary;
- sleep interruptibly by `--poll-interval`;
- graceful shutdown with LLM drain timeout and final persistence.

Commit gate: `cargo test -p harvester_batch` + `cargo nextest run`.

---

## Phase E — Dry-run behavior

### E1. Implement dry-run execution contract

`run_dry_run()` requirements:

- poll once;
- stop after poll settles;
- no download/triage dispatch;
- no state writes;
- stdout compact summary;
- file logging still enabled.

Add tests:
- exits 0 without `OPENAI_API_KEY`;
- no `RequestLlmCompletion` effects executed;
- no state artifacts modified.

Commit gate: `cargo test -p harvester_batch` + `cargo nextest run`.

---

## Phase F — Observability, cost, exit behavior

### F1. Add usage snapshot plumbing from LLM worker

Current `LlmHandle` does not expose quota totals directly: [crates/harvester_engine/src/llm/handle.rs](crates/harvester_engine/src/llm/handle.rs).

Preferred approach:
- add a read-only usage snapshot API on `LlmHandle` (`usage_totals()` returning immutable copy);
- keep lock ownership internal to worker/handle.

Alternative fallback:
- accumulate totals in batch runner from `Msg::LlmCompleted.metadata` if exposing handle state is too invasive.

### F2. Progress and final summary

Implement:

- per-triage progress lines;
- per-cycle summary;
- final summary with deterministic exit code:
  - `0` success,
  - `1` partial (work completed with failures),
  - `2` fatal startup/runtime.

Add helper + tests:
- `microdollars_to_display(u64) -> String` with exact rounding/format expectations.

Commit gate: `cargo nextest run`.

---

## Phase G — Lock robustness hardening (mandatory before release)

### G1. Make lock atomic and ownership-safe

Refactor [crates/harvester_batch/src/lock.rs](crates/harvester_batch/src/lock.rs):

- atomic acquire using `create_new`;
- include `owner` token in metadata;
- guard `Drop` verifies owner before delete;
- force-unlock path logs previous metadata for diagnostics.

Add unit tests for:

- concurrent acquire (one success, one failure);
- force unlock replacement;
- stale owner cannot delete newly acquired lock.

Commit gate: `cargo test -p harvester_batch` + `cargo nextest run`.

---

## Phase H — Final hygiene

### H1. Cleanup and policy pass

- remove temporary dead code and `#[allow(dead_code)]` introduced during migration;
- doc comments for all new public APIs;
- run full lint/test gates:
  - `cargo build`
  - `cargo nextest run`
  - `cargo clippy --workspace --all-targets -- -D warnings`

---

## 5) Testing matrix (minimum)

Unit tests:
- `harvester_io` effect validation/mapping/path tests.
- `harvester_core` `batch_observation()` and settlement classifier tests.
- `harvester_batch` CLI clamps, lock atomic semantics, exit classification, cost formatting.

Integration tests:
- batch run with temp output dir + local source fixture;
- unsupported script source behavior (with and without allow flag);
- dry-run no-write guarantee;
- signal-driven shutdown with persisted outputs;
- app hydration parity after batch-written artifacts.

---

## 6) Future extensions (post-Phase 7)

1. Replace persistence side-channel with reducer-emitted persistence effects (strict UDF compliance).
2. Add source health telemetry (latency/error rate/backoff metadata).
3. Add per-cycle artifact export (machine-readable JSON summaries).
4. Add optional idempotent “single-cycle mode” for external schedulers.
5. Add bounded worker pools for non-LLM IO effects to reduce thread spawning churn.

---

## 7) Dependency flow

```
A1 -> A2 -> A3
          |
          v
B1 -> B2 -> B3
          |
          v
         C1
          |
          v
D1 -> D2 -> D3 -> E1 -> F1 -> F2 -> G1 -> H1
```

Notes:
- `D*` requires both shared runner migration (`A/B`) and batch observation API (`C1`).
- `G1` is treated as release-critical, not optional hardening.
