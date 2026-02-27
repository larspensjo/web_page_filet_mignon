# Implementation Plan: Pre-Triage Refresh Coordinator (Reducer-Owned Quiet-Period Batching)

**Date:** 2026-02-26
**Status:** Implemented (2026-02-27)
**Motivation:** `Poll Sources` is fast, but repeated post-poll pre-triage refreshes dominate total time.

---

## Draft Diary Entry

**Context:** `Poll Sources` completes RSS polling and URL fetching quickly, but each completed job
triggers a full pre-triage article reload and content-prep pass. This repeats corpus-wide work and
makes the feature feel slow even when source polling itself is fast.

**Change:** Add a reducer-owned pre-triage refresh coordinator that batches refresh demand using a
quiet-period policy with poll-burst awareness, request IDs, and stale-result rejection. Dispatch
pre-triage loads from `Msg::Tick` using explicit in-flight job gating
(`batch_observation().jobs_in_flight`) and log scheduling/dispatch/apply decisions. Affected
subsystems: `harvester_core`, `harvester_io`.

---

## Goal

Replace per-`JobDone` pre-triage refresh dispatches with a robust coordinator that performs one (or
very few) pre-triage rebuild(s) after a poll/import burst settles.

Primary outcome:
- During a typical `Poll Sources` burst, the app does not run one full pre-triage rebuild per job.

Secondary outcomes:
- Stale async loader results cannot overwrite newer pre-triage state.
- Scheduling remains traceable as `Msg -> Reducer -> Effect -> Msg`.
- Behavior is deterministic and unit-testable via `Msg::Tick`.

---

## Design Summary

Implement a **PreTriageRefreshCoordinator** in `harvester_core` as a reducer-owned state machine:

1. Records refresh demand (`dirty`) and the latest desired URL snapshot.
2. Tracks a single in-flight pre-triage load request ID.
3. Uses `Msg::Tick` (75 ms cadence) to dispatch `Effect::LoadArticlesForTriage` after a quiet
   period.
4. Uses `request_id` on triage load effects/results so stale responses are ignored.
5. Applies stronger poll-burst gating immediately using `state.batch_observation().jobs_in_flight`.
6. Includes a max-wait guard to prevent livelock/starvation when jobs arrive at the quiet interval.

Key architectural decision:
- **Scheduling policy lives in reducer/state.**
- `harvester_io` executes loader IO and returns results; it does not decide cross-message batching.

---

## Current Problem (Observed)

From recent `engine.log` runs:
- `[poll-all-timing] all-sources completed ...` shows RSS polling completes in well under a second.
- Last polled job completion occurs only a few seconds later.
- Most remaining time is repeated post-job pre-triage refresh passes.
- Each pass reloads persisted completed jobs and re-runs `content_prep` across the corpus.
- The interim effect-runner debounce worker improved observability but not coalescing in spaced job
  completions.

Implication:
- Debounce at the IO worker layer is insufficient. Batching must be burst-aware in the reducer.

---

## Scope

### In scope
- Reducer-owned pre-triage refresh coordinator and tick-driven dispatch.
- `request_id` propagation for `LoadArticlesForTriage` and corresponding result messages.
- Poll-burst-aware gating using `batch_observation().jobs_in_flight`.
- Livelock prevention via max-wait cap.
- Explicit failure behavior for background pre-triage refresh failures.
- Logging and tests for scheduling, dispatch, stale results, and poll bursts.

### Out of scope
- Incremental/delta pre-triage loading (future optimization).
- Refactoring `load_and_prepare_articles_filtered` internals.
- UI features beyond optional refresh status telemetry later.

---

## Proposed Coordinator Module and Contracts

## A. New module

Create a dedicated module for the state machine:

- `crates/harvester_core/src/pre_triage_coordinator.rs`

Rationale:
- non-trivial state machine logic should not be embedded in `state.rs`;
- easier isolated unit testing and review.

`AppState` keeps a private field and exposes behavior methods only.

## B. Request ID propagation (external contract)

Update contracts:

- `Effect::LoadArticlesForTriage { request_id: u64, ordered_urls: Vec<String> }`
- `Msg::TriageArticlesLoaded { request_id: u64, articles: Vec<LoadedArticle> }`
- `Msg::TriageArticlesLoadFailed { request_id: u64, reason: String }`

Internal coordinator representation can use `Option<NonZeroU64>` for stronger invariants; convert to
`u64` at the effect/message boundary.

## C. Tick-driven scheduling

Use `Msg::Tick` (already emitted every 75 ms) to drive dispatch:
- `state.advance_tick()`
- compute dispatch eligibility with current tick + `jobs_in_flight`
- emit one triage load effect when due

No additional timers or cross-thread reducer callbacks.

---

## Proposed Refined Coordinator Shape (Target)

```rust
// crates/harvester_core/src/pre_triage_coordinator.rs

use std::num::NonZeroU64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreTriageRefreshReason {
    JobDone,
    RestoreCompletedJobs,
    PollEnded,
    ManualRetry,
}

pub(crate) struct PreTriageRefreshDispatch {
    pub request_id: u64,
    pub ordered_urls: Vec<String>,
}

pub(crate) enum PreTriageRefreshScheduleResult {
    Scheduled,
    ImmediateReset,
}

pub(crate) struct PreTriageRefreshCoordinator {
    dirty: bool,
    pending_ordered_urls: Vec<String>,
    in_flight_request_id: Option<NonZeroU64>,
    next_request_id: u64,          // starts at 0, alloc returns 1+
    earliest_dispatch_tick: u64,   // dispatch gate
    demand_started_tick: Option<u64>, // livelock/max-wait guard

    // Poll burst tracking
    poll_burst_active: bool,
    poll_sources_ended: bool,
    last_job_done_tick: Option<u64>,
}

const QUIET_TICKS_NORMAL: u64 = 4;      // ~300 ms at 75 ms/tick
const QUIET_TICKS_AFTER_POLL: u64 = 16; // ~1200 ms at 75 ms/tick
const MAX_WAIT_TICKS: u64 = 80;         // ~6 s hard ceiling
```

Important design constraints:
- No redundant `queued_while_in_flight` field (derivable from `dirty && in_flight.is_some()`).
- No separate `latest_change_tick` field unless strictly needed; `earliest_dispatch_tick` and
  `demand_started_tick` are sufficient.
- Empty URL set returns `ImmediateReset` and does not schedule deferred loading.

---

## Behavioral Decisions (Explicit)

## 1. Empty corpus refresh request

When `ordered_urls.is_empty()`:
- do **not** schedule deferred pre-triage load;
- immediately reset pre-triage session and clear manual overrides (same behavior as current
  `refresh_pre_triage_if_needed`).

## 2. Background pre-triage refresh failure

For a **matching** `TriageArticlesLoadFailed { request_id, .. }`:
- clear manual overrides (preserve current safety behavior),
- mark pre-triage as failed / unavailable,
- **do not** automatically fail the active `TriageSession` unless triage was explicitly being
  started and the failure is in that path.

This avoids a background refresh error poisoning the user's triage session state.

## 3. Stale result handling

For non-matching `request_id`:
- log and ignore;
- do not mutate pre-triage session;
- do not clear in-flight request for the active request.

---

## Detailed Implementation Plan (Slices)

## Slice 1 - Request IDs and Safe Result Application

**Goal:** Add request IDs and stale-result rejection before changing scheduling policy.

### 1.1 Update contracts

**Files:**
- `crates/harvester_core/src/effect.rs`
- `crates/harvester_core/src/msg.rs`
- `crates/harvester_io/src/effect_runner.rs`

Changes:
- Add `request_id: u64` to `Effect::LoadArticlesForTriage`.
- Add `request_id: u64` to `Msg::TriageArticlesLoaded`.
- Add `request_id: u64` to `Msg::TriageArticlesLoadFailed`.
- Pass through `request_id` in `EffectRunner`.

### 1.2 Add minimal in-flight request tracking in `AppState`

**Files:**
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/update.rs`

Changes:
- Track current in-flight pre-triage load request ID (temporary location in `AppState`; moved into
  coordinator in Slice 2).
- Existing `refresh_pre_triage_if_needed()` still dispatches immediately, but now allocates a
  request ID and marks it in flight.

### 1.3 Apply matching results only

**File:** `crates/harvester_core/src/update.rs`

Changes:
- `Msg::TriageArticlesLoaded` applies only if `request_id` matches.
- `Msg::TriageArticlesLoadFailed` applies only if `request_id` matches.
- Stale results logged via `[pre-triage-refresh-coord] stale result ignored ...`.

### 1.4 Failure-path behavior refinement

**File:** `crates/harvester_core/src/update.rs`

Changes:
- For matching `TriageArticlesLoadFailed`, keep clearing manual overrides.
- Avoid failing the `TriageSession` for passive background refresh failures; instead mark pre-triage
  state failed/unavailable and log.
- If there is an explicit triage-start path that depends on the load, keep its failure semantics
  distinct and explicit.

### 1.5 Test updates (compile-breaking checklist)

This slice is intentionally compile-breaking and requires updating all pattern-match sites for:
- `Effect::LoadArticlesForTriage`
- `Msg::TriageArticlesLoaded`
- `Msg::TriageArticlesLoadFailed`

Add/adjust tests:
- Reducer accepts matching `TriageArticlesLoaded`.
- Reducer ignores stale `TriageArticlesLoaded`.
- Reducer ignores stale `TriageArticlesLoadFailed`.
- Matching `TriageArticlesLoadFailed` clears manual overrides.
- Stale result does not mutate pre-triage state fingerprint.

---

## Slice 2 - Tick Counter and Coordinator Module (Generic Batching)

**Goal:** Move scheduling policy into a dedicated reducer-owned coordinator and dispatch from `Msg::Tick`.

### 2.1 Add tick counter to `AppState`

**File:** `crates/harvester_core/src/state.rs`

Add:
- private monotonic tick counter (`u64`)
- methods:
  - `advance_tick(&mut self)` (cheap, wrapping add)
  - `current_tick(&self) -> u64`

Notes:
- Tick is logical/monotonic, not wall-clock time.
- Tests drive it by replaying `Msg::Tick`.

### 2.2 Add coordinator module and wire into `AppState`

**Files:**
- `crates/harvester_core/src/pre_triage_coordinator.rs` (new)
- `crates/harvester_core/src/lib.rs` / module wiring
- `crates/harvester_core/src/state.rs`

Add:
- `PreTriageRefreshCoordinator`
- `PreTriageRefreshReason`
- `PreTriageRefreshDispatch`
- `PreTriageRefreshScheduleResult`

`AppState` methods (examples):
- `request_pre_triage_refresh(...)`
- `maybe_dispatch_pre_triage_refresh(...)`
- `complete_pre_triage_refresh(...)`

### 2.3 Replace immediate dispatch with scheduler request

**File:** `crates/harvester_core/src/update.rs`

Current call sites:
- `Msg::JobDone`
- `Msg::RestoreCompletedJobs`

New behavior:
- Compute current `ordered_urls`.
- Call coordinator `request(...)`.
- If `ImmediateReset`, preserve current behavior:
  - reset pre-triage session
  - clear manual overrides
  - no effect dispatch
- If `Scheduled`, set pre-triage loading state and wait for `Msg::Tick` dispatch.

### 2.4 Dispatch from `Msg::Tick`

**File:** `crates/harvester_core/src/update.rs`

New `Msg::Tick` behavior:
- `state.advance_tick()`
- read `tick = state.current_tick()`
- read `jobs_in_flight = state.batch_observation().jobs_in_flight`
- ask coordinator `maybe_dispatch(tick, jobs_in_flight > 0)`
- if due:
  - mark request in-flight/loading in state
  - emit `Effect::LoadArticlesForTriage { request_id, ordered_urls }`

### 2.5 Generic batching tests

Add test helper(s):
- `advance_ticks(state, n) -> (state, Vec<Vec<Effect>>)`
- `count_triage_loads(...)`

Tests:
- Multiple `JobDone`s within quiet window emit exactly one triage load.
- `RestoreCompletedJobs` schedules refresh and dispatches after eligible ticks.
- New demand while in-flight sets `dirty` and does not double-dispatch until response.
- Matching response clears in-flight and allows queued refresh to dispatch later.
- Empty `ordered_urls` path returns immediate reset and no loader effect.

---

## Slice 3 - Poll-Burst-Aware Policy (Using Existing `jobs_in_flight`)

**Goal:** Batch poll-related completions into one post-burst pre-triage refresh (or minimal refresh count).

### 3.1 Hook poll lifecycle into coordinator

**Files:**
- `crates/harvester_core/src/update.rs`
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/pre_triage_coordinator.rs`

Reducer hooks:
- `Msg::PollSourcesClicked` -> `note_poll_started()`
- `Msg::AllSourcesPollEnded` -> `note_poll_sources_ended()` and optionally schedule/poke refresh
  policy with current URL snapshot only if there is demand
- `Msg::JobDone` during poll -> `request(..., reason=JobDone, ...)` and record `last_job_done_tick`

### 3.2 Poll-burst dispatch gating

Coordinator `maybe_dispatch(current_tick, has_in_flight_engine_jobs)` rules:
- if `!dirty` or `in_flight_request_id.is_some()` -> no dispatch
- if `poll_burst_active && !poll_sources_ended` -> no dispatch (unless forced by max-wait)
- if `poll_burst_active && has_in_flight_engine_jobs` -> no dispatch (unless forced by max-wait)
- require `current_tick >= earliest_dispatch_tick` or `MAX_WAIT_TICKS` force

This uses `batch_observation().jobs_in_flight` from day one, not as a follow-up.

### 3.3 Livelock / starvation guard

Add `demand_started_tick` and `MAX_WAIT_TICKS`:
- if demand persists too long due to repeated arrivals near the quiet interval, force one dispatch.

This prevents indefinite postponement in degenerate steady streams.

### 3.4 Poll-burst reset semantics

Coordinator clears poll-burst state when appropriate (e.g., dispatch occurs and no in-flight engine
jobs remain, or after matching load completion with no new demand).

Keep this explicit and logged to avoid hidden transitions.

### 3.5 Poll-specific tests

- Poll burst with multiple `JobDone`s yields exactly one post-burst triage load.
- Poll ends before engine jobs finish; dispatch waits until jobs are no longer in flight.
- Poll returns zero URLs; no unnecessary triage load dispatched.
- Continuous arrivals near quiet interval still dispatch eventually due to `MAX_WAIT_TICKS`.
- Non-poll ingestion behavior remains correct (regression test).

---

## Slice 4 - Simplify `harvester_io` and Preserve Timing Logs

**Goal:** Remove interim effect-runner debounce worker and keep effect runner as pure IO execution.

### 4.1 Remove interim triage debounce worker

**File:** `crates/harvester_io/src/effect_runner.rs`

Changes:
- Remove `TriageRefreshWorkerMsg` and debounced worker path.
- `Effect::LoadArticlesForTriage { request_id, ordered_urls }` runs a direct loader task.
- Preserve loader timing logs and include `request_id`.

### 4.2 Keep and extend telemetry

Add/keep:
- `[pre-triage-refresh-coord] request ...`
- `[pre-triage-refresh-coord] dispatch request_id=...`
- `[pre-triage-refresh] load start request_id=...`
- `[pre-triage-refresh] load done request_id=... elapsed_ms=...`
- `[pre-triage-refresh-coord] apply request_id=...`
- stale result ignored logs

### 4.3 `harvester_io` tests

- Success path preserves `request_id`.
- Failure path preserves `request_id`.

---

## Slice 5 - Tuning, Validation, and Documentation

**Goal:** Validate real-world polling behavior and finalize policy constants/documentation.

### 5.1 Manual validation runbook

1. Start app.
2. Run `Poll Sources`.
3. Inspect `engine.log` for:
   - `[poll-all-timing]`
   - `[pre-triage-refresh-coord]`
   - `[pre-triage-refresh]`
4. Confirm:
   - RSS polling remains fast,
   - post-poll pre-triage load count is greatly reduced (ideally 1),
   - repeated `content_prep::derive` volume drops accordingly.

### 5.2 Tune constants

Tune:
- `QUIET_TICKS_NORMAL`
- `QUIET_TICKS_AFTER_POLL`
- `MAX_WAIT_TICKS`

Criteria:
- low churn after polls,
- responsive updates for non-poll single-job usage,
- no starvation in continuous streams.

### 5.3 Documentation and comments

Document in code:
- why scheduling is reducer-owned,
- why `Msg::Tick` is used,
- why `request_id` is required,
- quiet-period assumptions (75 ms tick),
- failure semantics for background pre-triage refresh errors.

Optional future doc mention:
- brief note in changelog/release notes if user-visible behavior changes significantly.

---

## Testing Strategy (Additional Guidance)

### Helper utilities

Add reducer test helpers:
- `advance_ticks(...)`
- `count_triage_loads(...)`
- `extract_triage_load_request_id(...)`

### Assert exact dispatch counts

Burst tests should assert **exactly one** `Effect::LoadArticlesForTriage`, not merely presence.

### Core poll sequence test (must-have)

Add a sequence test covering:
- `PollSourcesClicked`
- multiple `JobDone` during poll
- `AllSourcesPollEnded`
- ticks through quiet window
- exactly one triage load dispatch
- matching `TriageArticlesLoaded`
- no immediate second dispatch after matching response

### Stale result integrity test

Assert stale response does not mutate pre-triage state (fingerprint/snapshot check), not just that
it is ignored logically.

---

## Proposed File Touches

Likely files:
- `crates/harvester_core/src/effect.rs`
- `crates/harvester_core/src/msg.rs`
- `crates/harvester_core/src/pre_triage_coordinator.rs` (new)
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/update.rs`
- `crates/harvester_io/src/effect_runner.rs`
- `crates/harvester_core/tests/...` (new/expanded reducer tests)
- `crates/harvester_io` tests (request ID forwarding)
- `docs/EngineeringDiary.md` (when implementation completes)

---

## Acceptance Criteria

Functional:
- `Poll Sources` still ingests URLs and updates pre-triage state correctly.
- Matching triage load results apply; stale results are ignored safely.
- Empty-URL refresh requests reset pre-triage immediately without dispatching a loader effect.
- Background pre-triage load failures do not incorrectly fail the entire `TriageSession`.

Performance/behavioral:
- In a typical poll burst, post-poll pre-triage refresh count drops from "one per completed job" to
  "one or very few."
- `engine.log` clearly shows scheduling decisions, dispatches, and loader timings.
- Continuous arrival patterns do not starve refresh forever (`MAX_WAIT_TICKS` guard).

Architecture:
- Scheduling policy lives in reducer/state.
- `harvester_io` performs IO and returns results, without owning batching policy.
- Coordinator logic is isolated in its own module and unit-testable.

---

## Risks and Mitigations

### Risk 1 - Over-delayed updates for small/non-poll changes
- **Mitigation:** separate normal vs poll quiet windows.
- **Mitigation:** tune `QUIET_TICKS_NORMAL` conservatively.

### Risk 2 - State machine complexity
- **Mitigation:** dedicated coordinator module.
- **Mitigation:** sequence tests and exact dispatch-count assertions.
- **Mitigation:** structured logs on each transition.

### Risk 3 - Contract migration churn in Slice 1
- **Mitigation:** explicitly budget a test/call-site update pass in Slice 1.5.

### Risk 4 - Ambiguous failure semantics
- **Mitigation:** make background refresh failure behavior explicit in Slice 1.4 and test it.

---

## Implementation Order Recommendation

1. Slice 1 - Request IDs and safe result application
2. Slice 2 - Tick counter + coordinator module + generic batching
3. Slice 3 - Poll-burst-aware gating + max-wait guard
4. Slice 4 - Remove interim effect-runner debounce worker
5. Slice 5 - Tuning and validation

---

## Validation Commands (when implementing)

- `cargo build`
- targeted `harvester_core` reducer/coordinator tests
- targeted `harvester_io` request-ID forwarding tests
- `cargo clippy --all-targets -- -D warnings`

