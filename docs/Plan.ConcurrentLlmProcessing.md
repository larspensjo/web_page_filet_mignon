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
- `harvester_core` dispatches one next article at a time for triage/summary progression.
- `harvester_engine::llm::handle` uses a single worker loop that processes commands sequentially.
- This creates a serial bottleneck in both orchestration and execution layers.

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
- Quota check + record must be atomic under concurrency.
- Define behavior when quota is exhausted with requests still queued/in-flight:
  - New requests rejected early with explicit reason.
  - In-flight requests still report their own completion/failure.

### 4. Improve progress model for multi-flight sessions
- Current progress based on `current_index` assumes serial order.
- Move to derived counters:
  - `pending`, `in_progress`, `completed`, `failed`, `total`.
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
- Extend session types to expose:
  - `in_progress_count()`
  - `pending_count()`
  - `can_dispatch_more(limit)`
- Add helper methods to dispatch up to available slots in one reducer pass.
- Keep request-to-article mapping by `request_id` as source of truth for completion routing.

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
1. Add concurrency limits to app/runtime config with defaults.
2. Refactor triage progression:
   - Dispatch up to `triage_max_in_flight` initial requests.
   - Backfill one slot on each completion until no pending.
3. Refactor briefing summary progression similarly.
4. Keep briefing aggregate call gated on summary completion criteria.

Deliverables:
- Multi-request emission from `update()` for triage/summary phases.
- Updated progress text based on counts.

### Phase 2: LLM worker bounded concurrency (engine)
1. Introduce concurrent request execution behind `LlmHandle`.
2. Preserve command/event interface and validation/persistence behavior.
3. Protect shared mutable state (quota tracker, replay cache updates) with clear locking strategy.
4. Ensure stop/shutdown drains safely.

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
- Quota exhaustion message handling:
  - Remaining pending articles fail with explicit reason where expected.

### Unit tests (`harvester_engine`)
- Worker concurrency cap:
  - Never exceeds configured max in-flight.
- Quota synchronization:
  - No race in check+record under parallel execution.
- Retry logic:
  - Retries only retryable errors.
  - Honors retry budget and retry-after.

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
- Blocker: serial progress/index assumptions in session phase models.
  - Mitigation: switch to count-based progress derivation.
- Risk: quota tracker race conditions in concurrent worker.
  - Mitigation: explicit synchronization and tests with high contention.
- Risk: provider rate limiting under higher fan-out.
  - Mitigation: conservative default cap + adaptive backoff + retry budget.
- Risk: test flakiness due to timing.
  - Mitigation: deterministic mock provider with barrier/latch-style synchronization.
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

## Acceptance criteria
- Triage and summary stages support bounded concurrency with configurable limits.
- Session state remains correct under out-of-order completions.
- Reducer remains pure; effect boundaries unchanged.
- Quota/rate-limit handling is robust under parallel execution.
- New unit/integration tests lock in behavior.
- Logs provide sufficient detail to explain latency and failures.

## Implementation checklist
- [ ] Add config fields for triage/summary max in-flight.
- [ ] Refactor reducer dispatch to fill available slots.
- [ ] Refactor progress text to count-based model.
- [ ] Add concurrent worker pool/semaphore in LLM handle.
- [ ] Make quota accounting concurrency-safe.
- [ ] Add retry policy with retry-after support.
- [ ] Add observability logs and run summaries.
- [ ] Update and add tests (core + engine + integration).
- [ ] Validate with `cargo build`.
- [ ] Validate final with `cargo clippy --all-targets -- -D warnings`.

