# Implementation Plan: Reduce Pre-Triage Latency from Message-Loop Backlog and Synchronous Persistence

**Date:** 2026-02-27
**Status:** Draft
**Motivation:** `engine.log` shows pre-triage IO loads are relatively fast, but message application is delayed by main-thread backlog and synchronous persistence work.

---

## Draft Diary Entry

**Context:** Polling and pre-triage loading complete quickly, but user-visible readiness is delayed because reducer/UI message handling is blocked by synchronous disk persistence and extra per-message work. The delay appears as a large gap between `[pre-triage-refresh] load done` and `[pre-triage-refresh-coord] apply`.

**Change:** Introduce asynchronous/coalesced persistence and reduce reducer-loop overhead so pre-triage results apply promptly after load completion. Scope includes `harvester_app`, `harvester_io`, and small scheduling adjustments in `harvester_core`.

---


## Problem Statement (Observed)

From `engine.log` (2026-02-27 run):

- `[poll-all-timing] all-sources completed ... elapsed_ms=588` (polling itself is fast).
- `[pre-triage-refresh] load done request_id=2 ... elapsed_ms=431` at `06:03:00.361`.
- `[pre-triage-refresh-coord] apply request_id=2` at `06:03:19.915`.
- Gap is ~19.6s, much larger than loader time.
- During the gap, many `[pre-triage-refresh-coord] request scheduled reason=JobDone` entries continue.
- `Loaded persisted completed jobs ... .harvester_state.ron` appears repeatedly in the hot path.

Interpretation:

- We have reduced full refresh count, but end-to-end latency is now dominated by dispatch/apply backlog and synchronous persistence overhead on the app thread.

---

## Goals

Primary:

1. Reduce `load done -> apply` latency to near message-loop cadence (target: <500ms in typical runs).
2. Keep pre-triage refresh batching behavior deterministic and unidirectional.
3. Remove avoidable hot-path disk reads/writes from the app message loop.

Secondary:

1. Avoid scheduling refresh demand for non-impacting `JobDone` cases.
2. Improve observability so queue lag is measurable in logs.

Non-goals:

- Rewriting `load_and_prepare_articles_filtered` internals.
- Large architecture rewrites beyond persistence and scheduler-trigger refinements.

---

## Scope

### In Scope

- Asynchronous/coalesced persistence for completed jobs and pre-triage overrides.
- Eliminate read-before-write path for override persistence in hot loop.
- Reduce per-message work in app dispatch loop (`state.view()` only when required).
- Optional scheduler trigger refinement for `JobDone` failure handling.
- Metrics/logging for message queue lag and apply latency.
- Unit/integration tests covering burst behavior and delayed apply regression.

### Out of Scope

- New storage engine.
- Full persistence subsystem redesign.

---

## Architecture Decisions

1. **Persistence leaves the UI/reducer hot path**
   - Reducer remains pure and unchanged in principle.
   - App dispatch loop emits persistence intents; IO worker performs actual disk writes.

2. **Coalescing policy is explicit**
   - Persist state snapshots with trailing-edge debounce (e.g., 250-500ms).
   - Last-write-wins semantics for snapshots and overrides.

3. **No back-channels**
   - Persistence worker does not mutate state directly.
   - Optional failure reporting returns via `Msg` channel.

4. **Traceability first**
   - Add timing logs around enqueue/dequeue/apply to prove bottlenecks and gains.

---

## Async/Burst Feature Planning Checklist

- **Burst behavior / backpressure:** Coalesce repeated persistence requests during bursts into one disk write per debounce window; bounded channel to avoid unbounded memory growth.
- **Async result safety:** Persistence acknowledgments include sequence/version IDs; stale acks ignored.
- **Performance envelope:** Per message remains O(1) for persistence intent enqueue; no repeated parse/read of `.harvester_state.ron` in hot loop.
- **Observability:** Add logs for `persist_enqueued`, `persist_flushed`, `queue_lag_ms`, and `triage_apply_lag_ms`.
- **Failure semantics:** Persistence errors are logged and surfaced as non-fatal UI status; workflow continues.
- **Starvation/livelock guard:** Force flush after max interval (e.g., 2s) even under constant churn.
- **Burst test case:** Simulate many `JobDone` in burst and assert exact persistence flush count (coalesced) and bounded apply lag.

---

## Detailed Implementation Slices

## Slice 1 - Move Persistence Off App Dispatch Thread

### Changes

- Introduce persistence effect(s) + worker path in `harvester_io` (or dedicated persistence queue in app runtime), driven asynchronously.
- Replace direct calls in app dispatch loop:
  - `persist_completed_jobs(...)`
  - `persist_pre_triage_overrides(...)`

### Target Files

- `crates/harvester_app/src/platform/app.rs`
- `crates/harvester_core/src/effect.rs`
- `crates/harvester_io/src/effect_runner.rs`
- `crates/harvester_io/src/persistence.rs`

### Acceptance

- No direct state-file write calls in `dispatch_msg` hot path.
- App still persists completed jobs and overrides correctly.

---

## Slice 2 - Coalesced Persistence and Remove Read-Before-Write

### Changes

- Add coalesced snapshot persistence API that writes completed jobs and overrides together from in-memory snapshots.
- Remove `persist_pre_triage_overrides` dependency on `load_completed_jobs` in hot flow.
- Add debounce window + max flush interval.

### Target Files

- `crates/harvester_io/src/persistence.rs`
- `crates/harvester_io/src/effect_runner.rs` (or new worker module)
- `crates/harvester_core/src/effect.rs`

### Acceptance

- Repeated `Loaded persisted completed jobs ...` no longer appears during poll burst processing.
- Flush count is significantly lower than `JobDone` count in burst test.

---

## Slice 3 - Reduce Per-Message CPU in App Dispatch

### Changes

- Avoid unconditional `state.view()` construction in `dispatch_msg`; build only when `was_dirty`.
- Keep behavior identical for rendering/commands.

### Target Files

- `crates/harvester_app/src/platform/app.rs`

### Acceptance

- Functional behavior unchanged.
- CPU overhead reduced under high message throughput.

---

## Slice 4 - Refine Pre-Triage Refresh Triggering for `JobDone`

### Changes

- Re-evaluate whether failed `JobDone` should always schedule refresh.
- Prefer scheduling only when ordered completed URL set changes (or preserve current behavior with explicit rationale).
- Maintain deterministic reducer logic and tests.

### Target Files

- `crates/harvester_core/src/update.rs`
- `crates/harvester_core/src/state.rs` (if helper needed)

### Acceptance

- No unnecessary refresh demand resets from non-impacting completions.
- Existing success-path behavior preserved.

---

## Slice 5 - Observability and Regression Tests

### Changes

- Add log points/metrics:
  - message dequeue/apply lag,
  - pre-triage `load done -> apply` lag,
  - persistence enqueue/flush counts and flush latency.
- Add tests:
  - burst coalescing flush count,
  - no main-thread blocking from persistence,
  - stale ack handling (if ack IDs used),
  - bounded apply latency scenario.

### Target Files

- `crates/harvester_app/src/platform/app.rs`
- `crates/harvester_io/src/effect_runner.rs`
- `crates/harvester_core/src/update.rs` tests
- `crates/harvester_io` tests

### Acceptance

- New tests lock behavior.
- Logs clearly separate IO time from queue delay.

---

## Validation Plan

1. `cargo build`
2. Targeted tests for persistence worker/coalescing and pre-triage scheduling.
3. Manual run with `Poll Sources`, then inspect `engine.log`:
   - Compare `load done -> apply` lag before/after.
   - Confirm reduced persistence chatter during bursts.
4. Final lint gate:
   - `cargo clippy --all-targets -- -D warnings`

---

## Risks and Mitigations

1. **Risk:** Coalescing could delay crash-durability.
   - **Mitigation:** Max flush interval + flush on shutdown.

2. **Risk:** Async persistence introduces stale write ordering.
   - **Mitigation:** Sequence IDs and last-write-wins semantics.

3. **Risk:** Behavior drift in pre-triage scheduling.
   - **Mitigation:** Existing coordinator tests + added regression sequences.

4. **Risk:** Logging overhead from new telemetry.
   - **Mitigation:** Keep INFO concise; use DEBUG/TRACE for high-volume details.

---

## Proposed File Touches

- `crates/harvester_app/src/platform/app.rs`
- `crates/harvester_core/src/effect.rs`
- `crates/harvester_core/src/update.rs`
- `crates/harvester_io/src/effect_runner.rs`
- `crates/harvester_io/src/persistence.rs`
- `crates/harvester_core` test modules
- `crates/harvester_io` test modules
- `docs/EngineeringDiary.md` (finalized after implementation)

---

## Completion Criteria

- `load done -> apply` lag is no longer multi-second under normal bursts.
- Persistence is asynchronous/coalesced and no longer blocks app dispatch loop.
- Pre-triage refresh demand is not over-triggered by non-impacting events.
- Tests added for burst/coalescing behavior and regression lock-in.
- Final validation passes build + clippy.
