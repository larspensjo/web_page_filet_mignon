# Plan: Concurrent LLM Processing for Triage and Briefing

## Problem statement
Triage and briefing currently process article-level LLM requests largely in serial order, which makes end-to-end latency grow roughly linearly with article count. The dominant cost is remote API wait time, not local CPU.

Observed characteristics:
- Per-request durations are multi-second and variable.
- Request throughput is constrained by single-flight orchestration.
- Existing architecture already has the right UDF loop and effect boundaries; the missing capability is bounded concurrency.

## Goals
- Reduce total triage and briefing time with bounded parallel LLM processing.
- Preserve UDF architecture:
  - Reducers stay pure.
  - IO remains in effect handlers/workers.
  - State changes only via `Msg` events.
- Keep behavior robust under out-of-order completion, partial failure, and quota/rate-limit conditions.
- Add tests that lock in concurrency invariants and prevent regressions.
- Add clear observability for in-flight requests, queue depth, and latency/failure causes.

## Non-goals
- Replacing the current UDF architecture with a separate scheduler framework.
- Introducing unbounded fan-out.
- Switching providers or changing prompt semantics.
- Building full streaming UI in this plan.

## Related FutureIdeas items (included by design)
- `FI-Performance-LlmProcessing-0001` (primary): bounded concurrent LLM processing.
- `FI-LLM-RetryPolicy-0001` (partially included): explicit rate-limit/timeout retry policy for robustness.
- `FI-Observability-ReplayDiagnostics-0001` (partially included): latency/failure diagnostics for tuning concurrency.
- `FI-LLM-Budgeting-0001` (guardrails): respect quota accounting while increasing concurrency.

## Current-state summary
- `dispatch_next_triage_step` (`update.rs:546–573`): emits exactly one `Effect::RequestLlmCompletion` per call — purely sequential.
- `dispatch_next_briefing_step` (`update.rs:575–705`): has a cache-bypass inner loop, but still emits at most one live LLM request per invocation.
- `TriagePhase::Triaging { current_index, total }` / `BriefingPhase::Summarizing { current_index, total }`: `current_index` is set to `article_id + 1` in `start_article()` — it is a highest-dispatched pointer, NOT a completed count. Progress text in `view_model.rs` reads this value directly.
- `find_article_by_request_id`: linear scan — fine for 10–50 articles.
- Quota is checked and recorded **after** the API call completes inside `handle.rs:handle_completion`. There is no pre-call reservation.
- The effect runner event loop posts one `LlmEvent` → one `Msg::LlmCompleted` at a time — safe under concurrent workers with no changes needed.
- `AppState.llm_requests: BTreeMap<u64, LlmRequestState>` — secondary observability index, not used for session routing.

## Architecture constraints and invariants
- UDF traceability must remain:
  - `Msg` -> `update()` -> `Effect` -> effect runner -> follow-up `Msg`.
- Reducer purity:
  - No waiting, sleeping, IO, random, or global mutation in `update()`.
- Single source of truth:
  - Triage/briefing article states remain authoritative in session structs.
- No back-channels:
  - Worker/services never mutate core state directly.
- External immutability:
  - Keep private fields; expose capability methods to preserve invariants.

## Proposed design

### 1. Add bounded in-flight orchestration at reducer level
- Introduce configurable per-flow limits:
  - `triage_max_in_flight`
  - `summary_max_in_flight`
- Replace single-dispatch progression with "fill pipeline":
  - While `in_flight < limit` and pending exists, emit additional `Effect::RequestLlmCompletion`.
- Continue aggregate briefing as single request after summaries settle.

### 2. Add bounded concurrent execution in LLM worker
- Keep current command/event API shape (`LlmCommand`, `LlmEvent`).
- Implement bounded execution pool (threads or async semaphore) behind `LlmHandle`.
- Ensure request handling remains independent and completion events are emitted per request.

### 3. Make quota/rate-limit accounting concurrency-safe
- Move quota check to **pre-call reservation** inside `handle.rs`:
  - Before calling `provider.complete()`: atomically `check + reserve` budget under `Mutex`.
  - On call failure: release the reservation.
  - On call success: commit (finalize reservation as spent).
- This replaces the current post-call `check_call() + record_call()` pattern.
- Define behavior when quota is exhausted with requests still in-flight:
  - New requests rejected early with `LlmCompletionError::QuotaExhausted`.
  - In-flight requests still complete and report their own outcome.

### 4. Improve progress model for multi-flight sessions
- Remove `current_index` from `TriagePhase::Triaging` and `BriefingPhase::Summarizing` enum variants — the field is meaningless under multi-dispatch and must be removed, not supplemented.
- Add methods to `TriageSession` and `BriefingSession`:
  - `in_progress_count() -> usize`
  - `pending_count() -> usize`
  - `completed_count() -> usize`
  - `failed_count() -> usize`
  - `total() -> usize`
  - `can_dispatch_more(limit: usize) -> bool` — `in_progress_count() < limit && pending_count() > 0`
- Update `view_model.rs` to derive progress text from these methods instead of phase-embedded indices.
- Render progress text from counters, not request order.

### 5. Add observability for tuning and operations
- Emit structured logs with category tags:
  - dispatch accepted/rejected
  - in-flight counts
  - request queue depth
  - latency per request
  - retry/rate-limit events
- Add run-level summary at end of triage/briefing:
  - total requests, success/failure, p50/p95 duration, max in-flight reached.

## Data model and API changes

### `harvester_core`
- Remove `current_index` from `TriagePhase::Triaging` and `BriefingPhase::Summarizing`.
- Extend session types to expose:
  - `in_progress_count()`
  - `pending_count()`
  - `completed_count()`
  - `failed_count()`
  - `total()`
  - `can_dispatch_more(limit)`
- Add helper methods to dispatch up to available slots in one reducer pass.
- Keep request-to-article mapping by `request_id` as source of truth for completion routing.
- Note: `find_article_by_request_id` (linear scan) is sufficient for 10–50 articles; no change needed.

### `harvester_engine`
- Extend `LlmConfig` with concurrency settings (safe defaults, e.g. 3 or 4).
- Keep provider trait unchanged.
- Update LLM handle internals to execute multiple commands concurrently within cap.
- Ensure graceful shutdown semantics for pooled workers.

### `harvester_app`
- Ensure effect runner can enqueue bursts of `Effect::RequestLlmCompletion` without blocking UI loop.
- Keep all state changes via `Msg::LlmCompleted`.

## Phased implementation plan

### Phase 1: Reducer-level bounded dispatch (core)
1. Add concurrency limits to app/runtime config with defaults. Add `max_in_flight` upper-bound validation at config load time (e.g., ceiling of 10).
2. Refactor triage progression:
   - Dispatch up to `triage_max_in_flight` initial requests.
   - Backfill one slot on each completion until no pending.
3. Refactor briefing summary progression:
   - The existing cache-bypass inner loop in `dispatch_next_briefing_step` is extended to a fill loop: loop until `in_progress_count() >= limit` or all articles processed, completing cache hits inline.
   - On each live cache-miss: allocate `request_id`, call `start_article()`, emit `RequestLlmCompletion`, continue loop.
4. Keep briefing aggregate call gated on `pending_count() == 0 && in_progress_count() == 0`.

Deliverables:
- Multi-request emission from `update()` for triage/summary phases.
- Updated progress text based on counts.

### Phase 2: LLM worker bounded concurrency (engine)
1. Introduce concurrent request execution behind `LlmHandle`.
2. Preserve command/event interface and validation/persistence behavior.
3. Protect shared mutable state (quota tracker, replay cache updates) with clear locking strategy.
4. Shutdown/drain protocol for `LlmHandle`:
   - On drop or explicit stop: signal workers to accept no new commands.
   - Drain: wait for all in-flight requests to emit their `LlmEvent::Completed` before dropping the event receiver.
   - Implement `drain_and_stop()` method for the app to call on session end.

Deliverables:
- Up to N LLM requests can execute concurrently.
- Completion events may arrive out of order and are handled correctly.

### Phase 3: Robustness policies
1. Add bounded retry policy for transient failures (timeout/429/network).
2. Respect provider `Retry-After` when available.
3. Add jittered backoff and retry budget per request.
4. Ensure retries are visible in logs and metrics.

Deliverables:
- Reduced failure rate during brief provider instability.
- No unbounded retry loops.

### Phase 4: Observability and diagnostics
1. Add session-level latency/throughput counters.
2. Add in-flight and queue-depth logs with `[llm-concurrency]` category.
3. Emit end-of-run summary logs for triage and briefing.

Deliverables:
- Actionable logs for tuning concurrency cap and retry budget.

### Phase 5: Integration hardening
1. Validate behavior with cache hits/misses mixed with live requests.
2. Validate quota-exhaustion behavior under concurrency.
3. Validate repeated runs for deterministic state transitions.

Deliverables:
- Stable behavior under mixed real-world conditions.

## Test strategy

### Unit tests (`harvester_core`)
- Dispatch fill behavior:
  - On article load, emits min(limit, pending) LLM effects.
  - On completion, emits at most needed backfill effects.
- Out-of-order completions:
  - Correct article mapping by `request_id`.
  - Correct final state and completion criteria.
- Progress counters:
  - Correct `pending/in_progress/completed/failed` transitions.
  - `progress_counts_correct_under_multi_dispatch`
- Quota exhaustion message handling:
  - Remaining pending articles fail with explicit reason where expected.
- Triage reducer integration tests (currently zero coverage — must be added):
  - `triage_clicked_emits_load_effect`
  - `triage_articles_loaded_dispatches_up_to_limit_requests`
  - `triage_completion_backfills_one_slot`
  - `triage_out_of_order_completion_routes_correctly`
  - `triage_quota_exhausted_fails_all_pending`
- Aggregate briefing gate:
  - `briefing_aggregate_not_dispatched_until_all_articles_settled` — verify aggregate is not dispatched while any article is still `InProgress`.

### Unit tests (`harvester_engine`)
- Worker concurrency cap:
  - Never exceeds configured max in-flight.
  - `concurrent_requests_never_exceed_cap` — extend `mock_provider.rs` with a `BlockingMockProvider` (barrier-controlled), verify N+1 concurrent attempts respect the semaphore.
- Quota synchronization:
  - `quota_reservation_prevents_concurrent_overbilling` — two threads attempt to exceed quota simultaneously; only one succeeds.
  - Quota check is pre-call reservation, not post-call record.
- Retry logic:
  - Retries only retryable errors.
  - Honors retry budget and retry-after.
  - Retry holds semaphore slot across attempts (not released between retries).

### Integration tests
- End-to-end triage with N articles:
  - Runtime shorter than serial baseline (with deterministic mock delays).
- End-to-end briefing:
  - Summary stage runs bounded-parallel, aggregate stage remains single call.
- Mixed cache + live:
  - Cached articles complete immediately; remaining slots fill with live calls.

### Regression tests
- Existing serial assumptions in tests must be rewritten to bounded-concurrent expectations.
- Preserve previous correctness for single-article flows.

## Blockers and risks
- BLOCKER: Quota check is post-call — unsafe under concurrency.
  - Current: quota is checked and recorded after `provider.complete()` returns in `handle.rs`. Under N concurrent workers, all calls can complete simultaneously and all pass their pre-record quota check, billing all of them even if quota is exceeded.
  - Mitigation: Move to pre-call quota reservation. Atomically `check + reserve` budget before calling the provider (under `Mutex`); release on failure; commit on success.
- BLOCKER: `current_index` in phase enums must be removed, not supplemented.
  - `TriagePhase::Triaging { current_index }` and `BriefingPhase::Summarizing { current_index }` store the highest-dispatched article index. Under multi-dispatch this is meaningless. Adding helper methods alongside the field is insufficient — the field must be removed from both variants.
  - Mitigation: Remove `current_index` from both enum variants. Derive all progress from article state counts via session methods. Update `view_model.rs` accordingly.
- BLOCKER: Aggregate briefing dispatch gate needs two conditions.
  - `dispatch_next_briefing_step` currently fires aggregate briefing when no articles are `Pending`. Under concurrent dispatch, some articles may still be `InProgress` at that point.
  - Mitigation: Gate on `pending_count() == 0 && in_progress_count() == 0`, checked after every `LlmCompleted` for an article summary.
- Risk: quota tracker race conditions in concurrent worker.
  - Mitigation: pre-call reservation pattern (see BLOCKER above) + tests with high contention.
- Risk: provider rate limiting under higher fan-out.
  - Mitigation: conservative default cap + adaptive backoff + retry budget.
- Risk: test flakiness due to timing.
  - Mitigation: deterministic mock provider with barrier/latch-style synchronization (`BlockingMockProvider` in `mock_provider.rs`).
- Risk: memory growth from too many queued commands.
  - Mitigation: keep bounded dispatch and avoid unbounded buffering.

## Correctness-by-construction notes
- Illegal states should be unrepresentable:
  - article request state transitions only through session methods.
  - no direct external mutation of session internals.
- Reducer helpers should encode invariants:
  - "dispatch only when slot available"
  - "complete only matching in-progress request id"

## Logging and telemetry requirements
- Use `engine_logging` macros.
- Required new categories:
  - `[llm-concurrency]`
  - `[llm-retry]`
  - `[llm-quota]` (extended fields)
- Include identifiers in failure logs:
  - `request_id`, `prompt_id`, article index/url hash where applicable.

## Rollout strategy
1. Land Phase 1 behind config default `1` (behavior-preserving).
2. Land Phase 2 worker changes, still default `1`.
3. Enable default `3` after tests and local benchmarking.
4. Tune cap based on logs and provider behavior.

## Future ideas enabled by this plan

The following `FutureIdeas.md` items become directly viable once this plan is implemented:

- `FI-UX-SessionControls-0001` (pause/resume/cancel): Pause = stop dispatching new slots; in-flight calls complete naturally. Cancel = drain and fail pending with explicit reason.
- `FI-LLM-Budgeting-0001` (priority-weighted budgets): With concurrent dispatch, per-article budget allocation based on triage priority can vary per slot.
- `FI-LLM-Briefing-0002` (triage-informed briefing): Filter by triage priority before filling slots.
- `FI-Observability-ReplayDiagnostics-0001`: The `LlmResultIndex` + per-request latency tracking from Phase 4 directly feeds this.

New ideas surfaced by this plan:
- Adaptive concurrency cap: start at default (3), auto-reduce on repeated 429 responses, restore on success. Builds on the retry/rate-limit infrastructure from Phase 3.
- Real-time in-flight indicator in status bar (e.g. "2/5 done, 3 in flight") — directly enabled by the count-based progress model from this plan.
- Priority-cut on quota exhaustion: complete in-flight high-priority articles, cancel pending low-priority ones. Requires priority metadata on the request.

## Acceptance criteria
- Triage and summary stages support bounded concurrency with configurable limits.
- Session state remains correct under out-of-order completions.
- Reducer remains pure; effect boundaries unchanged.
- Quota/rate-limit handling is robust under parallel execution.
- New unit/integration tests lock in behavior.
- Logs provide sufficient detail to explain latency and failures.

## Implementation checklist
- [ ] Add config fields for triage/summary max in-flight with upper-bound validation (ceiling of 10).
- [ ] Remove `current_index` from `TriagePhase::Triaging` and `BriefingPhase::Summarizing` enum variants.
- [ ] Add `in_progress_count()`, `pending_count()`, `completed_count()`, `failed_count()`, `total()`, `can_dispatch_more(limit)` to session types.
- [ ] Refactor reducer dispatch to fill available slots (triage + briefing).
- [ ] Gate aggregate briefing dispatch on `pending_count() == 0 && in_progress_count() == 0`.
- [ ] Refactor progress text in `view_model.rs` to count-based model.
- [ ] Move quota check to pre-call reservation in `handle.rs` (atomic check+reserve before `provider.complete()`).
- [ ] Add concurrent worker pool/semaphore in LLM handle.
- [ ] Define and implement shutdown/drain protocol for `LlmHandle` (`drain_and_stop()`).
- [ ] Add retry policy with retry-after support (retry holds semaphore slot across attempts).
- [ ] Add observability logs and run summaries.
- [ ] Add triage reducer integration tests to `update.rs` (currently zero coverage).
- [ ] Extend `mock_provider.rs` with `BlockingMockProvider` (barrier-controlled) for concurrency tests.
- [ ] Update and add tests (core + engine + integration).
- [ ] Remove or document `Msg::RequestLlmCompletion` (unused dead variant).
- [ ] Validate with `cargo build`.
- [ ] Validate final with `cargo clippy --all-targets -- -D warnings`.

