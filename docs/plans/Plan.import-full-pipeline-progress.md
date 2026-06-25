# Import Mode Full Pipeline + Progress Implementation Plan

> **For agentic workers:** If available, use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Otherwise, follow tasks sequentially using the Agents.md workflow. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `--import-saved-web-dir` run the complete archive pipeline (import → triage → summaries) and emit live progress output during each phase.

**Architecture:** Currently `run_import_dispatch_loop` holds `msg_tx` but never uses it, so triage and summary orchestration messages are never dispatched. Fix by calling `maybe_dispatch_batch_ai_orchestration` in the import loop (the same way the regular batch loop does), then update `should_settle_import_cycle` to also wait for triage to drain. Add an `ImportProgressReporter` to `progress.rs` that receives periodic observation snapshots and overwrites a single status line on stdout.

**Tech Stack:** Rust, `mpsc`, `harvester_core::BatchObservation`, `std::io::Write`, carriage-return terminal trick (same as `ProgressReporter`).

## Global Constraints

- No new CLI flags; this changes runtime behavior of the existing `--import-saved-web-dir` mode.
- `ImportProgressReporter` must follow the existing `ProgressReporter` pattern: terminal-gated (`is_terminal()`), carriage-return status line. It shows aggregate failure counts in the status line but does not emit per-item sticky failure lines (those require an explicit event source not available in import mode).
- Reducers stay pure; no side effects inside `update()`.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt` must pass before done.
- Do not commit; changes go to review first (per Agents.md).

---

## File Map

| File | Change |
|---|---|
| `crates/harvester_batch/src/runner.rs` | Remove `_` from `_msg_tx`, add orchestration call, update settlement fn, thread progress reporter |
| `crates/harvester_batch/src/progress.rs` | Add `ImportProgressReporter` struct and impl |
| `crates/harvester_batch/src/runner/tests.rs` | Update settlement tests; add orchestration-in-import tests |

---

## Task 1: Wire AI orchestration into the import dispatch loop

**Files:**
- Modify: `crates/harvester_batch/src/runner.rs` (functions `run_import_dispatch_loop`, `should_settle_import_cycle`)
- Test: `crates/harvester_batch/src/runner/tests.rs`

**Interfaces:**
- Consumes: `maybe_dispatch_batch_ai_orchestration(state)` → `Option<Msg>` (already exists in same file)
- Produces: `should_settle_import_cycle` now waits for `triage_in_flight == 0 && triage_pending == 0` in addition to existing conditions

- [ ] **Step 1: Write a failing test for the updated settlement condition**

In `crates/harvester_batch/src/runner/tests.rs`, add after the existing settlement tests:

```rust
#[test]
fn import_cycle_does_not_settle_while_triage_in_flight() {
    let mut obs = observation_with_import(0, 0, 0, 0, 0, 0, 0, 1, 0);
    obs.import_phase = harvester_core::ImportPhase::Idle;
    obs.import_in_flight = false;
    obs.triage_in_flight = 2;
    obs.triage_pending = 0;
    obs.summary_in_flight = 0;
    obs.summary_pending = 0;
    assert!(!should_settle_import_cycle(&obs));
}

#[test]
fn import_cycle_does_not_settle_while_triage_pending() {
    let mut obs = observation_with_import(0, 0, 0, 0, 0, 0, 0, 1, 0);
    obs.import_phase = harvester_core::ImportPhase::Idle;
    obs.import_in_flight = false;
    obs.triage_in_flight = 0;
    obs.triage_pending = 3;
    obs.summary_in_flight = 0;
    obs.summary_pending = 0;
    assert!(!should_settle_import_cycle(&obs));
}

#[test]
fn import_cycle_settles_when_all_phases_drained() {
    let mut obs = observation_with_import(0, 0, 0, 0, 0, 0, 0, 1, 0);
    obs.import_phase = harvester_core::ImportPhase::Idle;
    obs.import_in_flight = false;
    obs.triage_in_flight = 0;
    obs.triage_pending = 0;
    obs.summary_in_flight = 0;
    obs.summary_pending = 0;
    assert!(should_settle_import_cycle(&obs));
}
```

- [ ] **Step 2: Run to verify tests fail**

```
cargo test -p harvester_batch import_cycle_does_not_settle_while_triage -- --nocapture
```

Expected: FAIL (current code doesn't check triage).

- [ ] **Step 3: Update `should_settle_import_cycle` in `runner.rs`**

Find the function at approximately line 175 and change it to:

```rust
fn should_settle_import_cycle(obs: &BatchObservation) -> bool {
    !obs.import_in_flight
        && !matches!(obs.import_phase, ImportPhase::Importing)
        && obs.triage_in_flight == 0
        && obs.triage_pending == 0
        && obs.summary_in_flight == 0
        && obs.summary_pending == 0
}
```

- [ ] **Step 4: Run tests to verify settlement tests pass**

```
cargo test -p harvester_batch import_cycle -- --nocapture
```

Expected: all three new tests PASS.

- [ ] **Step 5a: Write failing tests for orchestration dispatch behavior**

In `crates/harvester_batch/src/runner/tests.rs`, add tests that verify the import loop dispatches the right messages and skips settlement when it does. These rely on the `msg_tx`/`msg_rx` pair already threaded into the loop:

```rust
#[test]
fn import_loop_does_not_settle_when_orchestration_queued_dispatch_triage() {
    // Build a state where batch_next_action() returns DispatchTriage
    // and verify the loop sends Msg::TriageClicked and does NOT settle in that iteration.
    // (Concretely: enough articles present for triage readiness but triage_pending == 0.)
    let (tx, rx) = mpsc::channel::<Msg>();
    let mut state = make_state_ready_for_triage();   // helper sets up pre_triage=done, triage_pending=0
    let options = DispatchLoopOptions {
        enable_ai_orchestration: true,
        require_new_jobs_since: None,
        tick_interval: Duration::from_millis(0),
    };
    // Drive one iteration via a channel peek: send a no-op ping and receive once.
    let result = run_one_import_iteration(&mut state, &tx, &rx, &options);
    // TriageClicked should have been queued
    assert!(matches!(rx.try_recv(), Ok(Msg::TriageClicked)));
    // Loop must NOT have settled in this iteration
    assert!(!result.settled);
}

#[test]
fn import_loop_does_not_settle_when_orchestration_queued_dispatch_summaries() {
    // Build a state where batch_next_action() returns DispatchSummaries.
    let (tx, rx) = mpsc::channel::<Msg>();
    let mut state = make_state_ready_for_summaries(); // triage complete, summaries not started
    let options = DispatchLoopOptions {
        enable_ai_orchestration: true,
        require_new_jobs_since: None,
        tick_interval: Duration::from_millis(0),
    };
    let result = run_one_import_iteration(&mut state, &tx, &rx, &options);
    assert!(matches!(rx.try_recv(), Ok(Msg::PrepareSummariesClicked)));
    assert!(!result.settled);
}

#[test]
fn import_loop_skips_orchestration_when_disabled() {
    let (tx, rx) = mpsc::channel::<Msg>();
    let mut state = make_state_ready_for_triage();
    let options = DispatchLoopOptions {
        enable_ai_orchestration: false,
        require_new_jobs_since: None,
        tick_interval: Duration::from_millis(0),
    };
    let _result = run_one_import_iteration(&mut state, &tx, &rx, &options);
    // No orchestration message should have been queued
    assert!(rx.try_recv().is_err());
}
```

Note: `run_one_import_iteration`, `make_state_ready_for_triage`, and `make_state_ready_for_summaries` are test helpers to add in the test module. Adjust to match actual state-construction patterns in the existing test file.

- [ ] **Step 5b: Run to confirm tests fail**

```
cargo test -p harvester_batch import_loop_does_not_settle -- --nocapture
cargo test -p harvester_batch import_loop_skips_orchestration -- --nocapture
```

Expected: FAIL (wiring not yet added, helpers not yet defined).

- [ ] **Step 5: Wire `msg_tx` into the import dispatch loop**

In `run_import_dispatch_loop`, the parameter `_msg_tx` is currently unused. Change the signature and add orchestration dispatch — here is the full updated function body (the relevant added block goes right before the settlement check):

In the function signature, change `_msg_tx` → `msg_tx`:
```rust
fn run_import_dispatch_loop(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,   // was _msg_tx
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
) -> Result<CycleOutcome, String> {
```

Then inside the loop, just before the `should_settle_import_cycle` check (after the tick block), add:

```rust
        // Dispatch triage or summaries when the engine signals readiness.
        let mut orchestrated = false;
        if options.enable_ai_orchestration {
            if let Some(next_msg) = maybe_dispatch_batch_ai_orchestration(state) {
                msg_tx.send(next_msg).map_err(|e| {
                    format!("Failed to dispatch import orchestration message: {}", e)
                })?;
                orchestrated = true;
            }
        }
```

Also update the settlement guard to skip when an orchestration message was just queued (prevents settling before the queued message is processed):

```rust
        // Skip settlement this iteration when an orchestration message was just queued.
        if !orchestrated && should_settle_import_cycle(&obs) {
            engine_info!("[import] Cycle settled after {} iterations", iterations);
            return Ok(classify_import_cycle_outcome(&obs));
        }
```

- [ ] **Step 6: Verify the call site still compiles (caller already passes `msg_tx`)**

The call in `run_import_mode` at ~line 1759 already passes `&msg_tx`:
```rust
    let outcome = run_import_dispatch_loop(
        &mut state,
        &msg_tx,          // was already &msg_tx — no change needed here
        &msg_rx,
        ...
    )?;
```

- [ ] **Step 7: Build to verify no compile errors**

```
cargo build -p harvester_batch
```

Expected: compiles clean.

- [ ] **Step 8: Run all batch tests**

```
cargo test -p harvester_batch -- --nocapture
```

Expected: all pass.

- [ ] **Step 9: Stage for review**

```
git add crates/harvester_batch/src/runner.rs crates/harvester_batch/src/runner/tests.rs
git status --short
```

---

## Task 2: Add `ImportProgressReporter` and live progress output

**Files:**
- Modify: `crates/harvester_batch/src/progress.rs`
- Modify: `crates/harvester_batch/src/runner.rs` — `run_import_dispatch_loop` (add parameter), `run_import_mode` (create reporter, pass in)
- Test: add inline `#[cfg(test)]` block in `progress.rs`

**Interfaces:**
- Consumes: `&BatchObservation` from `harvester_core`
- Produces: `ImportProgressReporter::new(enabled: bool)`, `.startup_line(W)`, `.update_from_obs(&obs, O, E)`, `.finish(W)`

- [ ] **Step 1: Write failing tests for `ImportProgressReporter`**

At the bottom of `crates/harvester_batch/src/progress.rs`, inside the existing `#[cfg(test)]` block, add:

```rust
    // ── ImportProgressReporter tests ─────────────────────────────────────────

    use harvester_core::{BatchObservation, ImportPhase, PreTriagePhase, TriagePhase, SessionState};

    fn import_obs_idle() -> BatchObservation {
        BatchObservation {
            poll_in_progress: false,
            session_state: SessionState::Idle,
            jobs_total: 0, jobs_done: 0, jobs_failed: 0, jobs_in_flight: 0,
            pre_triage_phase: PreTriagePhase::Idle,
            pre_triage_total: 0, pre_triage_included: 0, pre_triage_review: 0, pre_triage_filtered: 0,
            triage_phase: TriagePhase::Idle,
            triage_total: 0, triage_pending: 0, triage_in_flight: 0,
            triage_completed: 0, triage_failed: 0,
            summary_total: 0, summary_pending: 0, summary_in_flight: 0,
            summary_completed: 0, summary_failed: 0,
            triage_cache_hits: 0, triage_cache_misses: 0, triage_cache_key_unavailable: 0,
            summary_cache_hits: 0, summary_cache_misses: 0, summary_cache_key_unavailable: 0,
            import_phase: ImportPhase::Idle,
            imports_completed: 0, imports_failed: 0, import_in_flight: false,
            source_poll_stats: vec![],
        }
    }

    #[test]
    fn import_progress_startup_line_contains_key_fields() {
        let reporter = ImportProgressReporter::new(true);
        let mut out = Vec::<u8>::new();
        reporter.startup_line(&mut out);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("[import]"), "startup line missing prefix: {s:?}");
        assert!(s.ends_with('\n'), "startup line must end with newline: {s:?}");
    }

    #[test]
    fn import_progress_disabled_startup_writes_nothing() {
        let reporter = ImportProgressReporter::new(false);
        let mut out = Vec::<u8>::new();
        reporter.startup_line(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn import_progress_update_writes_status_line() {
        let mut reporter = ImportProgressReporter::new(true);
        let mut obs = import_obs_idle();
        obs.imports_completed = 3;
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.update_from_obs(&obs, &mut out, &mut err);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.starts_with('\r'), "status line must start with CR: {s:?}");
        assert!(s.contains("3"), "should show import count: {s:?}");
    }

    #[test]
    fn import_progress_disabled_update_writes_nothing() {
        let mut reporter = ImportProgressReporter::new(false);
        let obs = import_obs_idle();
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.update_from_obs(&obs, &mut out, &mut err);
        assert!(out.is_empty() && err.is_empty());
    }

    #[test]
    fn import_progress_finish_prints_summary_line() {
        let mut reporter = ImportProgressReporter::new(true);
        let obs = import_obs_idle();
        let mut out = Vec::<u8>::new();
        let mut err = Vec::<u8>::new();
        reporter.update_from_obs(&obs, &mut out, &mut err);
        out.clear();
        reporter.finish("$0.01", &mut out);
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("$0.01"), "finish must show cost: {s:?}");
        assert!(s.contains("[import]"), "finish must show prefix: {s:?}");
        assert!(s.ends_with('\n'), "finish must end with newline: {s:?}");
        assert!(!s.contains('\r'), "finish line must not contain CR: {s:?}");
    }
```

- [ ] **Step 2: Run tests to confirm they fail**

```
cargo test -p harvester_batch import_progress -- --nocapture
```

Expected: FAIL (`ImportProgressReporter` not yet defined).

- [ ] **Step 3: Implement `ImportProgressReporter` in `progress.rs`**

Add the following after the existing `ProgressReporter` impl and before the `#[cfg(test)]` block:

```rust
/// Live progress reporter for `--import-saved-web-dir` mode.
///
/// Renders a single overwritten status line covering all three pipeline phases
/// (import → triage → summary). Disabled when stdout/stderr are not terminals.
pub struct ImportProgressReporter {
    enabled: bool,
    last_line_width: usize,
    painted_status: bool,
    start: Instant,
}

impl ImportProgressReporter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_line_width: 0,
            painted_status: false,
            start: Instant::now(),
        }
    }

    pub fn startup_line<W: Write>(&self, stdout: &mut W) {
        if !self.enabled {
            return;
        }
        let _ = writeln!(stdout, "[import] starting: import -> triage -> summary");
    }

    /// Call after each observation snapshot inside the dispatch loop.
    pub fn update_from_obs<O: Write, E: Write>(
        &mut self,
        obs: &harvester_core::BatchObservation,
        stdout: &mut O,
        _stderr: &mut E,
    ) {
        if !self.enabled {
            return;
        }
        let elapsed = format_elapsed(self.start.elapsed());
        let body = format!(
            "[import] {}  import={}/{}  triage={}/{}  summary={}/{}  t={}",
            phase_label(obs),
            obs.imports_completed,
            obs.imports_completed + obs.imports_failed,
            obs.triage_completed,
            obs.triage_total,
            obs.summary_completed,
            obs.summary_total,
            elapsed,
        );
        let pad = self.last_line_width.saturating_sub(body.len());
        let _ = write!(stdout, "\r{}{:pad$}", body, "", pad = pad);
        let _ = stdout.flush();
        self.last_line_width = body.len();
        self.painted_status = true;
    }

    pub fn finish<W: Write>(&mut self, cost_display: &str, stdout: &mut W) {
        if !self.enabled {
            return;
        }
        let elapsed = format_elapsed(self.start.elapsed());
        if self.painted_status {
            let _ = writeln!(stdout);
        }
        let _ = writeln!(
            stdout,
            "[import] done  elapsed={}  cost={}",
            elapsed, cost_display
        );
        let _ = stdout.flush();
        self.painted_status = false;
    }
}

impl Drop for ImportProgressReporter {
    fn drop(&mut self) {
        if self.enabled && self.painted_status {
            let mut out = std::io::stdout();
            let _ = writeln!(out);
        }
    }
}

fn phase_label(obs: &harvester_core::BatchObservation) -> &'static str {
    use harvester_core::ImportPhase;
    if obs.import_in_flight || matches!(obs.import_phase, ImportPhase::Importing) {
        return "IMPORTING";
    }
    if obs.triage_in_flight > 0 || obs.triage_pending > 0 {
        return "TRIAGING ";
    }
    if obs.summary_in_flight > 0 || obs.summary_pending > 0 {
        return "SUMMARIZE";
    }
    "SETTLING "
}
```

No additional `use` import is needed — the snippet already uses `harvester_core::BatchObservation` as a fully-qualified path throughout. Adding a bare `use harvester_core::BatchObservation;` would be unused and fail clippy.

- [ ] **Step 4: Run tests to verify they pass**

```
cargo test -p harvester_batch import_progress -- --nocapture
```

Expected: all pass.

- [ ] **Step 5: Thread reporter through `run_import_dispatch_loop`**

Add an optional progress parameter to the dispatch loop signature. Find the function signature in `runner.rs` and add the parameter:

```rust
fn run_import_dispatch_loop(
    state: &mut AppState,
    msg_tx: &mpsc::Sender<Msg>,
    msg_rx: &mpsc::Receiver<Msg>,
    effect_runner: &EffectRunner,
    shutdown_flag: &Arc<AtomicBool>,
    options: DispatchLoopOptions,
    progress: Option<&mut crate::progress::ImportProgressReporter>,
) -> Result<CycleOutcome, String> {
```

Inside the loop, after the message-batch block (after `effect_runner.enqueue(...)`), add:

```rust
        // Update live progress from current observation.
        let obs = state.batch_observation();
        if let Some(p) = progress.as_deref_mut() {
            p.update_from_obs(&obs, &mut std::io::stdout(), &mut std::io::stderr());
        }
```

Remove the existing `let obs = state.batch_observation();` line that is only used for the settlement check (now `obs` is computed once above and reused):

```rust
        // Settlement check uses the same obs snapshot computed above.
        // `orchestrated` comes from Task 1 Step 5 — skip settlement this iteration
        // when an orchestration message was just queued.
        if !orchestrated && should_settle_import_cycle(&obs) {
            engine_info!("[import] Cycle settled after {} iterations", iterations);
            return Ok(classify_import_cycle_outcome(&obs));
        }
```

- [ ] **Step 6: Create reporter in `run_import_mode` and pass it in**

In `run_import_mode` in `runner.rs`, find the existing progress setup area (currently just the final `println!`). Replace the two-line final println with:

```rust
    // Progress reporter — active only when both stdout and stderr are terminals.
    let progress_enabled = std::io::stdout().is_terminal() && std::io::stderr().is_terminal();
    let mut progress = crate::progress::ImportProgressReporter::new(progress_enabled);
    progress.startup_line(&mut std::io::stdout());
```

Then update the call to `run_import_dispatch_loop` to pass the reporter:

```rust
    let outcome = run_import_dispatch_loop(
        &mut state,
        &msg_tx,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            enable_ai_orchestration,
            require_new_jobs_since: None,
            tick_interval: Duration::from_millis(75),
        },
        Some(&mut progress),
    )?;
```

Replace the existing final `println!` (the "-- Import complete --" line) with the reporter's finish call. First compute cost from LLM usage if available (re-use the pattern from `run_refresh_stale_summaries_mode`):

```rust
    let obs = state.batch_observation();
    engine_info!(
        "[import] Settled: phase={:?} imported={} failed={}",
        obs.import_phase,
        obs.imports_completed,
        obs.imports_failed,
    );

    // LlmHandle is owned by effect_runner; usage totals are not accessible here yet.
    // Print "unavailable" rather than a misleading $0.00 — accurate cost is in engine logs.
    let cost_display = "unavailable".to_string();
    progress.finish(&cost_display, &mut std::io::stdout());
```

Note: cost display is `"unavailable"` because the `LlmHandle` is owned by `effect_runner` and usage totals are not accessible at this point in `run_import_mode`. Printing `"$0.00"` would be incorrect when triage or summaries actually ran. A follow-up can thread a cloned `LlmHandle` out of `build_effect_runner()` to get accurate totals.

- [ ] **Step 7: Build and verify**

```
cargo build -p harvester_batch
```

Expected: compiles clean.

- [ ] **Step 8: Run all batch tests**

```
cargo test -p harvester_batch -- --nocapture
```

Expected: all pass.

- [ ] **Step 9: Run clippy and fmt**

```
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Fix any warnings before proceeding.

- [ ] **Step 10: Stage for review**

```
git add crates/harvester_batch/src/progress.rs crates/harvester_batch/src/runner.rs crates/harvester_batch/src/runner/tests.rs
git status --short
```

---

## Self-Review

**Spec coverage check:**

| Requirement | Task |
|---|---|
| Import mode runs summaries | Task 1 (orchestration wiring) |
| Import mode runs triage | Task 1 (same orchestration wiring; `DispatchTriage` was already the first branch) |
| Settlement waits for triage | Task 1 (`should_settle_import_cycle` update) |
| Live progress output | Task 2 (`ImportProgressReporter`) |
| Progress shows import/triage/summary counts | Task 2 (`update_from_obs` format string) |
| Terminal-gated (no garbage in non-TTY) | Task 2 (`is_terminal()` gate) |
| No new CLI flags | confirmed — none added |

**Placeholder scan:** None found.

**Type consistency:**
- `ImportProgressReporter::new(enabled: bool)` — used consistently in Task 2 Steps 3, 6.
- `update_from_obs(&obs, O, E)` — parameter types match `ProgressReporter` style.
- `finish(cost_display: &str, W)` — consistent across Steps 3 and 6.
- `should_settle_import_cycle(&obs)` — `&BatchObservation` — consistent with the test helpers in Step 1.
