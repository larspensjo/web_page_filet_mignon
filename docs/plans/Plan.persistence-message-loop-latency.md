# Implementation Plan: Reduce Pre-Triage Latency from Message-Loop Backlog and Synchronous Persistence

**Date:** 2026-02-27  
**Status:** Draft (Updated after review)  
**Motivation:** `engine.log` shows pre-triage loading is fast, but `load done -> apply` is delayed by app-loop burst amplification and synchronous persistence work on the app thread.

## Draft Diary Entry

**Context:** Polling/pre-triage IO completes quickly, but user-visible readiness is delayed by burst-amplified app-thread work (per-message view/render/snapshot work and repeated completed-URL cloning) plus synchronous persistence overhead.  
**Change:** Implement batched app-loop post-processing and deferred refresh evaluation (single completed-URL snapshot per batch), plus a dedicated bounded latest-wins persistence worker with atomic writes and shutdown flush. Scope: `harvester_app`, `harvester_core`, `harvester_io` (and `harvester_batch` only if it has an independent dispatch loop).

## Problem Statement (Observed)

From `engine.log` (2026-02-27):

- Polling and load are fast (`all-sources` ~588ms, pre-triage load ~431ms).
- `load done` to coordinator `apply` gap is ~19.6s.
- Burst includes many `JobDone` schedules.
- Current app path does burst-multiplied work:
  - per-message `state.view()` call,
  - per-message render/snapshot handling,
  - per-`JobDone` `ordered_completed_job_urls()` cloning,
  - synchronous persistence on app thread.

## Goals

### Primary

1. Reduce `load done -> apply` to near loop cadence (target p95 `< 500ms` in typical local runs).
2. Keep unidirectional flow intact: `Action -> Reducer -> State -> Render`, with IO isolated in effects/workers.
3. Remove blocking persistence and burst-multiplied O(N) allocations from the app hot path.

### Secondary

1. Avoid non-impacting refresh scheduling on `JobDone`.
2. Add telemetry that separates queue delay, reducer/apply delay, and persistence delay.
3. Preserve durability with bounded memory under slow disk.

### Non-Goals

- Rewriting `load_and_prepare_articles_filtered` internals.
- Full storage subsystem replacement.
- Cross-artifact transactional format migration (single-file transaction for completed jobs + overrides).

## Key Design Decisions (Locked)

1. **M1+M2 are atomic in one PR/commit**
   - Batch processing alone does not remove per-`JobDone` O(N) cloning.
   - The batch-loop refactor and refresh-clone removal ship together.

2. **Refresh evaluation mechanism: Option A (push)**
   - Reducer emits `refresh_evaluation_needed` intent only.
   - At batch boundary, app computes ordered completed URLs once (if needed) and dispatches a dedicated evaluation action containing that snapshot.
   - Rationale: simpler than coordinator lazy API change and keeps refresh decision deterministic.

3. **Batch post-processing**
   - Reducer still runs per message.
   - Expensive work runs once per drained batch only:
     - `state.view()` (only if `any_dirty == true`),
     - enqueue render,
     - generate persistence snapshot requests,
     - dispatch refresh evaluation action.
   - `SetInputText("")` emitted at most once per batch when any enqueue-url effect appears.

4. **Persistence worker model**
   - Follow entity-index worker lifecycle pattern (dedicated thread, explicit control messages), adapted for latest-wins semantics.
   - Channel semantics: `Mutex<Option<PersistenceSnapshot>> + Condvar` (true latest-wins, bounded memory).
   - Worker performs coalesced trailing-edge flush with forced max interval.

5. **Durability from first worker release**
   - Atomic write (`temp + rename`) is required in the same milestone as worker introduction.
   - Production shutdown flush is required via `Shutdown { done: SyncSender<()> }`.

## Async/Burst Feature Planning Checklist

- **Burst behavior/backpressure:** drain inbox per loop turn; batch post-processing once; persistence latest-wins + coalescing.
- **Async result safety:** persistence snapshots carry monotonic sequence/version; stale completion/ack ignored.
- **Performance envelope:** reducer O(1) per message; expensive view/snapshot/URL materialization O(1) per batch.
- **Observability:** `[msg-loop]` batch size and queue lag added in M1; refresh/persist timings added with worker.
- **Failure semantics:** persistence errors are non-fatal, logged with context, surfaced via status action.
- **Starvation/livelock guard:** forced flush max interval guarantees progress under constant churn.
- **Burst test case:** high-volume `JobDone` burst asserts exact render/snapshot/evaluation counts per batch and bounded flush count.

## Scope Check (Pre-Implementation Gate)

### Step

- Verify whether `harvester_batch` has an independent message dispatch path equivalent to `process_pending_messages`.

### Outcome Rules

- If shared path: no `harvester_batch` code changes.
- If independent loop exists: apply equivalent batching + telemetry changes there in same implementation.

### Acceptance Criteria

- Scope decision documented in plan execution notes and reflected in touched files.

## Milestones

## Milestone 1: Batch App Loop + Refresh Clone Removal (Atomic)

### Changes

- Refactor app pending-message processing to:
  - drain current inbox into a batch,
  - run reducer per message,
  - aggregate batch flags:
    - `any_dirty`
    - `persist_completed_needed`
    - `persist_overrides_needed`
    - `clear_input_needed`
    - `refresh_evaluation_needed`
- Execute post-processing once per batch:
  - call `state.view()` only when `any_dirty`,
  - enqueue render once per dirty batch,
  - enqueue `SetInputText("")` at most once if `clear_input_needed`,
  - enqueue persistence snapshot intents once per batch.
- Implement refresh Option A:
  - remove per-`JobDone` full completed-URL cloning from reducer hot path,
  - compute ordered completed URLs once at batch boundary when `refresh_evaluation_needed`,
  - dispatch evaluation action carrying that snapshot.
- Add early telemetry:
  - `[msg-loop] batch_size=<n> queue_lag_ms=<x>`.

### Target Files

- `crates/harvester_app/src/platform/app.rs`
- `crates/harvester_core/src/update.rs`
- `crates/harvester_core/src/state.rs` (if helper/state access adjustments are needed)
- `crates/harvester_core/src/effect.rs` (if intent/action variants are added)

### Acceptance Criteria

- No per-message `state.view()` calls; no `state.view()` call on no-op batch (`any_dirty == false`).
- No per-`JobDone` `ordered_completed_job_urls()` cloning in reducer path.
- One render enqueue max per dirty batch.
- One refresh evaluation dispatch max per batch requiring evaluation.
- Behavior equivalent to prior single-message flow semantics.

## Milestone 2: Dedicated Latest-Wins Persistence Worker (with Durability and Shutdown)

### Changes

- Introduce dedicated persistence worker thread/lane (separate from network-heavy effect paths).
- Define worker messages including:
  - `UpdateSnapshot(PersistenceSnapshot, seq)`
  - `Shutdown { done: SyncSender<()> }`
- Implement latest-wins handoff using `Mutex<Option<PersistenceSnapshot>> + Condvar`.
- Implement coalescing policy:
  - trailing debounce: 250-500ms,
  - forced flush max interval: 2s.
- Ensure persistence writes are atomic (`temp file + rename`) for each artifact.
- Remove synchronous disk persistence from app dispatch path entirely.
- On app exit, send `Shutdown` and wait for completion ack.

### Target Files

- `crates/harvester_app/src/platform/app.rs`
- `crates/harvester_io/src/persistence_worker.rs` (new)
- `crates/harvester_io/src/persistence.rs`
- `crates/harvester_io/src/effect_runner.rs` (if lifecycle wiring requires integration)
- `crates/harvester_core/src/effect.rs` (intent definitions)

### Acceptance Criteria

- App thread performs enqueue/handoff only; no blocking disk IO in dispatch hot path.
- Memory remains bounded under synthetic slow-disk conditions.
- Worker always converges to latest snapshot.
- Shutdown path flushes latest pending state before exit completion.
- Atomic-write path is active for all worker persistence outputs.

## Milestone 3: Observability, Regression Tests, and Diary Finalization

### Changes

- Add/complete structured logs:
  - `[msg-loop] batch_size`, `queue_lag_ms`
  - `[pre-triage-refresh] load_to_apply_lag_ms`
  - `[persist] enqueued`, `coalesced`, `flushed`, `flush_latency_ms`, `overwritten_count`
- Add reducer/app-loop/worker tests for burst and stale-result safety.
- Finalize and append engineering diary entry with evidence and references.
- If Milestone 0 found `harvester_batch` independent loop, include parity tests there.

### Target Files

- `crates/harvester_app/src/platform/app.rs`
- `crates/harvester_core/src/update.rs` tests
- `crates/harvester_io/src/persistence_worker.rs` tests
- `crates/harvester_io/src/persistence.rs` tests
- `docs/EngineeringDiary.md`
- Optional: `harvester_batch` crate files/tests (conditional on Milestone 0)

### Acceptance Criteria

- Telemetry clearly distinguishes queue/apply/persist delays.
- Burst tests assert counts are batch-proportional, not message-proportional.
- Diary updated in required format with Evidence and Refs.

## Test Plan

1. `cargo build`
2. Unit tests (core):
   - refresh intent behavior on `JobDone` success/failure,
   - no per-message completed-URL clone behavior (proxy assertions around evaluation dispatch count).
3. App-loop tests:
   - burst drain produces single post-processing cycle per batch,
   - `state.view()` not called on no-dirty batch,
   - one `SetInputText("")` at most per batch,
   - batch-size telemetry emitted.
4. Persistence worker tests:
   - latest-wins overwrite under rapid updates,
   - debounce coalescing reduces flushes,
   - max-interval forced flush guarantees progress,
   - stale sequence completion ignored,
   - shutdown flush ack ensures no pending state loss.
5. Durability tests:
   - atomic write leaves readable files after interruption simulation.
6. Manual validation:
   - run poll flow; inspect `engine.log` for reduced `load done -> apply` lag and bounded persistence chatter.
7. Final lint gate:
   - `cargo clippy --all-targets -- -D warnings`

## Metrics and Acceptance Thresholds

- `load done -> apply` p95 under typical local run: `< 500ms`.
- Render enqueue count scales with dirty batches, not raw message count.
- Refresh evaluation dispatch count scales with batches requiring refresh, not `JobDone` count.
- Persistence flush count materially below `JobDone` count during bursts.
- No unbounded queue/memory growth under slow disk.
- No regression in pre-triage refresh correctness.

## Risks and Mitigations

1. **Risk:** Batched processing delays user-visible updates.
   **Mitigation:** Drain current queue only; preserve loop cadence; track batch latency telemetry.

2. **Risk:** Latest-wins drops intermediate snapshots.
   **Mitigation:** Snapshot persistence semantics accept intermediate loss; forced max-interval and shutdown flush preserve latest durable state.

3. **Risk:** Async ordering errors in persistence completion.
   **Mitigation:** Sequence IDs + stale completion ignore rules with tests.

4. **Risk:** Cross-file consistency between completed jobs and overrides is eventual, not transactional.
   **Mitigation:** Atomic per-file writes now; defer cross-artifact transaction format as separate decision.

## Notes / Assumptions

- Review suggestion applied: M1 and M2 merged as atomic milestone.
- Review suggestion applied: refresh mechanism explicitly chosen (Option A push model).
- Review suggestion applied: atomic writes and shutdown flush moved into worker introduction milestone.
- Review suggestion applied: persistence channel type fixed to `Mutex<Option<_>> + Condvar` for true latest-wins.
- `persist_pre_triage_overrides` ΓÇ£read-before-write removalΓÇ¥ will be implemented only if such disk read/merge exists; otherwise this item is treated as verified not applicable and removed during execution notes.
- `harvester_batch` changes are conditional on Milestone 0 scope check.

## Proposed File Touches

- `crates/harvester_app/src/platform/app.rs`
- `crates/harvester_core/src/effect.rs`
- `crates/harvester_core/src/update.rs`
- `crates/harvester_core/src/state.rs`
- `crates/harvester_io/src/persistence.rs`
- `crates/harvester_io/src/persistence_worker.rs` (new)
- `crates/harvester_io/src/effect_runner.rs` (if lifecycle integration needed)
- Related test modules in `harvester_app`, `harvester_core`, `harvester_io`
- `docs/EngineeringDiary.md`
- Conditional: `harvester_batch` crate files/tests (if independent loop confirmed)

## Completion Criteria

- Burst amplification removed: batch post-processing executes once per drained batch.
- Per-`JobDone` O(N) completed-URL clone removed from reducer hot path.
- Persistence is async, bounded, latest-wins, coalesced, atomic-write safe, and off app thread.
- Production shutdown flush guarantees latest pending snapshot durability.
- `load done -> apply` no longer multi-second under normal bursts.
- Build and lint gates pass: `cargo build`, `cargo clippy --all-targets -- -D warnings`.
- Engineering diary entry finalized with evidence and references.