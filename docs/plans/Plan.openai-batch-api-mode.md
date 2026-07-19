# OpenAI Batch API Mode for harvester_batch — Implementation Plan

> **For agentic workers:** implement this plan phase-by-phase. Each phase is an
> independently buildable/testable slice. Steps use checkbox (`- [ ]`) syntax.
> **Do NOT commit any changes** — repo rule: implemented plans are reviewed
> before committing. Replace every "commit" step with a verification step.

> **Revised after Codex review (Issues Found).** All review issues and advisory
> recommendations applied; two reviewer questions resolved by user decision (see
> **Settled decisions** below). Not sent back for review — single-shot workflow.

**Goal:** Cut LLM spend (~50% on batched traffic) by routing harvester_batch's
non-interactive, cache-keyed LLM stages — **triage** (`gpt-5.4-nano`),
**summaries** (`gpt-5.4-mini`), and **signal-candidate scoring** — through
OpenAI's asynchronous Batch API (`/v1/files` upload + `/v1/batches`
create/poll/download; JSONL request lines; ≤24 h turnaround; 50% discount).
With the flag on, a running harvester_batch submits new triage/summary/
signal-candidate work as batch jobs, collects finished jobs on later cycles (or
the next process run), and articles progress through the pipeline with
hours-scale latency and no lost work across restarts. Briefing prompts stay
synchronous.

**Non-goals:** harvester_app (interactive streaming briefing UI), MCP
`smart_query` (interactive), `--refresh-stale-summaries` mode (stays
synchronous; possible follow-up), and **import mode** (`--import-saved-web-dir`
stays synchronous — see Settled decisions). The public output corpus is
untouched.

---

## Settled decisions (from review + user answers — do not re-litigate)

1. **Import mode stays synchronous.** `--batch-api` and `--import-saved-web-dir`
   are **mutually exclusive** (clap `conflicts_with`, CLI error). *Reason:*
   import is a distinct one-shot workflow; batching it is a separable follow-up,
   and coupling them now widens the blast radius. Lifting the restriction is a
   possible follow-up.
2. **Manifest is a dot-prefixed RON file** (`.batch_manifest.ron`) in the output
   dir. *Reason:* `docs/CorpusFormat.md` classifies `.*.ron` as internal state
   that external readers ignore, so `harvester-corpus.json` generation,
   `CorpusFormat.md`, and their tests stay untouched and no
   `CORPUS_SCHEMA_VERSION` bump is needed. (A `.json` manifest would **not** be
   covered — note `.summary_refresh_last.json` is explicitly listed as a
   *generated artifact*, not internal state.) This overrides the review's
   corpus-layout issue by choosing the internal-state-compatible name. All batch
   durable state (manifest transitions, collected-line snapshots) lives in this
   RON file; batch audit **replay records** are written to the existing
   internal-state `llm_results/` directory — both already covered by the rule.
3. **Collection writes only caches; the next-cycle cache-hit replay does the
   rest.** Verified in code: a triage cache hit (`update/triage.rs:285-318`) and
   a summary cache hit (`update/briefing.rs:380-408`) already complete the
   article, emit `UpsertEntityIndexEntry`, and call `signal_candidate::try_enqueue`;
   a signal cache hit (`update/signal_candidate.rs:172`) completes the URL. So
   collection reproduces post-processing by **only** writing the validated
   result into the content-addressed cache under the frozen key (plus recording
   batch-priced usage and an audit replay record). Article completion,
   entity-index upserts, and signal enqueueing happen exactly once, on the
   subsequent cache-hit replay — never duplicated, never omitted.
4. **Routing lives in the batch runner, not in `harvester_io`.** `harvester_batch`
   depends on `harvester_io`, and `EffectRunner` has neither `AppState` access
   nor cache-key inputs, so interception cannot live in
   `harvester_io/effect_runner/dispatch.rs`. Instead the runner's dispatch loop
   diverts batch-eligible `Effect::RequestLlmCompletion`s (which it holds in
   `queued_effects`, with full `AppState` access) *before* `effect_runner.enqueue`.
   `dispatch.rs` is unchanged.

---

## Architecture (Approach B: explicit submit/collect in the runner)

Layers and their owning crates:

1. **`openai_provider_kit`** — a *generic, fakeable* JSONL Batch transport
   (`BatchTransport` trait) + request/response codecs. No Harvester domain
   terms; speaks `custom_id` + arbitrary JSON bodies only.
2. **`harvester_engine`** — extract a public request-preparation API so batch
   bodies are byte-identical to synchronous ones; add batch (50%) pricing.
3. **`harvester_core`** — pure reducer additions: a `DeferredToBatch` outcome
   that settles the cycle without failing the article, deferred-aware session
   phases, a runner-driven re-arm epoch message, `BatchResultsCollected`
   handling, and read-only frozen-cache-key accessors.
4. **`harvester_batch`** — the `--batch-api` flag, the `.batch_manifest.ron`
   store, the `BatchCoordinator` that renders/uploads/creates/collects, effect
   diversion, submission-budget (denial-of-wallet) enforcement, and
   startup/collection reconciliation.

### Per-cycle flow with batch mode on

```
cycle start
  ├─ RECONCILE (first cycle only): resolve manifest entries with batch_id=None
  │                                via remote list-batches by input_file_id.
  ├─ COLLECT: for each manifest batch with a batch_id: retrieve; if Completed,
  │           download output+error JSONL, snapshot per-line results durably
  │           into .batch_manifest.ron (status=Collected), then emit
  │           Msg::BatchResultsCollected. Remove a batch only after the frozen
  │           keys are confirmed present in the on-disk cache store (idempotent).
  ├─ Msg::RearmDeferredBatchStages  (epoch boundary: Deferred -> re-dispatchable)
  ├─ POLL sources -> download/extract (unchanged)
  └─ DISPATCH triage/summaries/signal (reducer emits RequestLlmCompletion per miss)
        runner diverts batch-eligible effects -> BatchCoordinator buffer
        at quiescence: render lines, upload file, RESERVE manifest(file_id)+save,
          create batch, ATTACH batch_id+save, then send DeferredToBatch replies
        -> cycle SETTLES (deferred requests are neither pending nor in-flight)
```

This stays inside UDF: the new outcome/message are explicit reducer values,
validation and cache writes happen in the pure reducer (timestamps supplied by
the caller), and all I/O (render, upload, poll, download, persistence) stays in
the runner layer.

---

## Key design decision — cycle settlement + re-dispatch epoch

`AppState::batch_status()` (`state/batch.rs:121`) returns `Settled` only when all
in-flight/pending counters are zero, and the dispatch loop (`runner.rs:1569`)
spins until `Settled` (`MAX_ITERATIONS = 10_000`, then `Err`). Today
`Effect::RequestLlmCompletion` is answered by `Msg::LlmCompleted { request_id }`,
which retires the in-flight request. Batch interception must supply an equivalent
non-terminal retirement, and a distinct trigger to re-dispatch later.

**`DeferredToBatch` outcome (settles, not terminal):**
- New `LlmResultKind::DeferredToBatch` (produced *only* by the runner's
  `BatchCoordinator`, never by the worker), delivered as the normal reply:
  `Msg::LlmCompleted { request_id, result: DeferredToBatch, metadata: None }`.
- `record_llm_result` maps it to a new `LlmRequestState::Deferred { prompt_id }`.
- Triage and briefing sessions gain `defer_article(idx)` → a new per-article
  `Deferred` outcome (parallel to complete/fail), counted as **neither pending,
  in-flight, completed, nor failed**. `SignalCandidateSession` gains a
  `Deferred` URL state (see Phase 3 — signal deferral).
- `dispatch_next_triage_step` and `dispatch_next_briefing_step` become
  **deferred-aware**: when pending + in-flight reach zero **and** deferred > 0,
  the session transitions to a new **`AwaitingBatch`** phase instead of
  `Complete`/`Failed`. `batch_status()` treats `AwaitingBatch` + all-deferred as
  no active work, so the cycle settles.

**Re-dispatch epoch (fixes "no trigger / would loop within a cycle"):**
- State persists across cycles, so re-dispatch must be gated on an explicit
  cycle boundary, not on `AwaitingBatch` alone (which would re-loop inside the
  same dispatch loop). The runner sends **`Msg::RearmDeferredBatchStages`** once
  per cycle, *after COLLECT and before POLL*. Its reducer moves `Deferred`
  articles/URLs back to a re-dispatchable state and resets `AwaitingBatch` phases
  so the existing orchestration (`batch_next_action`) re-runs the stage.
- `batch_next_action` does **not** re-arm `AwaitingBatch` on its own — only the
  runner's per-cycle `RearmDeferredBatchStages` does — so there is no within-cycle
  loop.
- On re-dispatch the reducer re-checks the cache: **hit** (collected) → article
  completes via the normal cache-hit replay (post-processing runs once);
  **still pending** → the coordinator sees the `.batch_manifest.ron` custom_id
  and replies `DeferredToBatch` again (no resubmit); **collected-but-failed /
  expired** → released for re-dispatch (Phase 6 policy).

**Across restarts:** in-memory `Deferred` state is rebuilt naturally — stages
re-run, a miss re-emits the effect, the coordinator sees the persisted manifest
entry and replies `DeferredToBatch` without resubmitting. The
**`.batch_manifest.ron`**, not session state, is the durable record of paid work.

---

## Global constraints (from `Agents.md`)

- Reducers stay **pure and unit-testable**; all I/O in the runner layer. State
  mutates only in the update step. **No `Utc::now()` in the reducer** — the
  runner supplies `created_at_utc` on every collected entry.
- `openai_provider_kit` stays **generic OpenAI infrastructure** (transport +
  codecs + a fakeable trait). Stages, cache keys, retries, budgets, and the
  manifest are **Harvester-owned**. Bump the kit's version + CHANGELOG when it
  changes.
- New CLI flag ⇒ **update `scripts/Start-HarvesterBatch.ps1` in the same change**.
- Keep shared constants DRY (stage models in `harvester_engine/src/llm/mod.rs:17-26`;
  batch pricing derives from the existing `PricingRegistry`).
- `engine_logging`; every batch error log carries **batch id / custom_id /
  cache-key** context.
- Corpus untouched (`.batch_manifest.ron` + `llm_results/` are internal state;
  `CORPUS_SCHEMA_VERSION` unaffected).
- Regression tests favor **reducer behavior, emitted effects, public contracts**;
  manifest + collect logic get round-trip, crash-injection, and partial-failure
  tests.
- After the final phase: `cargo clippy --all-targets -- -D warnings` then
  `cargo fmt`. Kill `harvester_mcp` processes if they block the build.
- Add an `docs/EngineeringDiary.md` entry when implemented (final phase).
- **Do not commit** — leave changes for review.

### Note on a CI batch-on test

A test-only in-process fake `BatchTransport` drives the full submit → manifest →
collect → cache loop with `--batch-api` on, so CI always exercises the new path
even though the *production* default is off (design decision #2). This is kept
because it is valuable, **not** because of any "defaults must exercise the new
path" rule — that rule is not in this repo's `Agents.md` and is not cited.

---

## Phase 1 — Generic JSONL Batch transport + codecs in `openai_provider_kit`

**Smallest independent slice:** tested transport with no Harvester wiring.

**Files:** create `crates/openai_provider_kit/src/batch.rs`; export from
`lib.rs`; bump `Cargo.toml` (minor) + `CHANGELOG.md`.

**Interfaces (generic; no domain terms):**
- `trait BatchTransport` (fakeable) with `async` methods:
  `upload_input(&self, jsonl: &[u8]) -> Result<FileId, LlmError>`
  (`POST /v1/files`, multipart, `purpose=batch`);
  `create_batch(&self, input_file_id, endpoint, completion_window) -> Result<BatchHandle, LlmError>`;
  `retrieve_batch(&self, &BatchId) -> Result<BatchHandle, LlmError>`;
  `list_batches(&self, after: Option<&str>) -> Result<Vec<BatchHandle>, LlmError>`
  (for reconciliation by `input_file_id`);
  `download_file(&self, &FileId) -> Result<Vec<u8>, LlmError>`;
  `cancel_batch(&self, &BatchId) -> Result<BatchHandle, LlmError>` (defensive).
- `OpenAiProvider` implements `BatchTransport` (reusing its client + `map_status_code`).
- `struct BatchInputLine { custom_id, method, url, body: serde_json::Value }`
  (one JSONL line). **Public codec** `pub fn openai_chat_completion_body(&LlmRequest) -> serde_json::Value`
  (extracted from the currently-private body builder in `openai.rs`) so callers
  build request bodies byte-identical to the synchronous path.
- `struct BatchHandle { id, status: BatchLifecycle, input_file_id, output_file_id, error_file_id, request_counts:{total,completed,failed} }`.
- `enum BatchLifecycle { Validating, InProgress, Finalizing, Completed, Failed, Expired, Cancelling, Cancelled }` — unknown status string → `Err` (never silently ignored).
- `struct BatchOutputLine { custom_id, response: Option<{status_code, body}>, error: Option<Value> }`.

**Tests (`cargo test -p openai_provider_kit`):** JSONL round-trip; parse a
completed-batch body → `BatchHandle`; parse mixed output JSONL (successes + one
`error` line); unknown lifecycle → `Err`; `openai_chat_completion_body` matches
the body the sync `complete()` path sends for the same `LlmRequest`. HTTP tests
reuse the existing `with_base_url` + `reqwest-passthrough` seam against a mock;
if absent, unit-test codecs and keep transport behind the trait (note which).

**Verify:** `cargo build`, `cargo test -p openai_provider_kit`.

- [ ] `BatchTransport` trait + `OpenAiProvider` impl
- [ ] Public `openai_chat_completion_body` codec + line/handle types
- [ ] Tests pass; version bump + CHANGELOG

---

## Phase 2 — Reusable request preparation + batch pricing in `harvester_engine`

Extract the private synchronous preparation so batch bodies are provably
identical, and add discounted pricing. (Kept in the engine, not the kit, because
it involves model resolution + prompt templates — Harvester-owned.)

**Files:** `crates/harvester_engine/src/llm/handle.rs` (extract),
`crates/harvester_engine/src/llm/mod.rs` (re-export),
`crates/harvester_engine/src/llm/pricing.rs`.

**Interfaces:**
- `pub struct PreparedCompletion { model: ModelId, system_message: String, user_message: String, request: LlmRequest }`.
- `pub fn prepare_completion(fields, registry, config) -> Result<PreparedCompletion, LlmCompletionError>`
  — factor `resolve_model` (`handle.rs:826`) + template selection + rendering
  (`handle.rs:440-530`) out of the worker so both the worker and the
  `BatchCoordinator` call it. The worker is refactored to use it (no behavior
  change); a test asserts the worker still produces identical requests.
- Pricing: `pub fn batch_cost_microdollars(&self, usage: &TokenUsage) -> u64` on
  `ModelPricing` (= standard rates halved; batch has no cached-input tier), and a
  `PricingRegistry::batch_cost_microdollars(model_name, usage)`. Unit tests
  assert exactly 50% of the standard cost.

**Tests (`cargo test -p harvester_engine`):** worker-vs-`prepare_completion`
parity (same rendered messages + `LlmRequest`); batch price = 50% of standard for
several models incl. dated-variant prefix match.

**Verify:** `cargo build`, `cargo test -p harvester_engine`.

- [ ] Extract `prepare_completion`; worker uses it
- [ ] Batch pricing helpers + tests
- [ ] Tests pass

---

## Phase 3 — Reducer: deferral, re-arm epoch, collection, frozen-key accessors (pure, tested)

No batch I/O; exercised entirely by feeding messages. Lands and tests
independently.

**Files:**
- `crates/harvester_core/src/msg.rs` — `LlmResultKind::DeferredToBatch`;
  `Msg::RearmDeferredBatchStages`; `Msg::BatchResultsCollected { entries: Vec<CollectedEntry> }`.
- `crates/harvester_core/src/state/*` — `LlmRequestState::Deferred { prompt_id }`;
  `Deferred` outcome + `AwaitingBatch` phase for triage (`triage.rs`), briefing
  (`briefing.rs`), and a `Deferred` URL state for `signal_candidate.rs`; counter/
  phase updates in `observation_counts()`, `pending_count()`,
  `in_progress_count()`, `is_active()`, `state/batch.rs`.
- `crates/harvester_core/src/update/llm_completed.rs` — `DeferredToBatch` in
  `record_llm_result` (→ `Deferred`) and in each stage handler (→ `defer_article`,
  no fail/entity/usage).
- `crates/harvester_core/src/update/signal_candidate.rs` — add `DeferredToBatch`
  arm to the exhaustive match (defer, don't fail, clear snapshot); make
  `try_enqueue` re-enqueue a previously-deferred URL after re-arm.
- `crates/harvester_core/src/update/triage.rs`, `briefing.rs` — deferred-aware
  `dispatch_next_*_step` (→ `AwaitingBatch`, not `Complete`); handle
  `RearmDeferredBatchStages`.
- New frozen-key accessors (pure, read-only) used by the runner:
  `frozen_batch_key_for_request(request_id) -> Option<FrozenBatchKey>` returning
  the exact per-stage key components (triage/summary: `content_hash`; signal:
  `signal_input_hash` from the input snapshot) + `prompt_id`, `prompt_version`,
  `model_id`, `context_hash`, `stage`, `url`, and the rendered system/user
  messages captured for audit. Key logic stays in core (DRY, correct).

**`CollectedEntry` contract (fixes the too-thin schema):**
```
CollectedEntry {
    batch_id: String,
    custom_id: String,
    stage: StageKind,
    key: FrozenBatchKey,          // exact submit-time key, incl. signal_input_hash
    created_at_utc: String,       // supplied by runner (reducer stays pure)
    outcome: CollectedOutcome,
}
CollectedOutcome =
    Success { raw_output_json: String, usage: TokenUsage, resolved_model: String }
  | LineError { detail: String }   // per-line error or non-2xx status
```

**`BatchResultsCollected` reducer behavior (collection = cache write only):**
- `Success`: **validate** with the existing pure validators (`validate_triage` /
  `validate_summary` / `validate_signal_candidate` — untrusted until validated).
  On valid: reconstruct the core cache key from `key` and **insert into the
  in-memory cache** under the frozen key using the supplied `created_at_utc`;
  record **batch-priced** usage (Phase 2) into `llm_usage_by_model`; emit the
  existing cache-store persistence effect **and** the audit replay-record effect
  (Phase 5). Do **not** complete articles, upsert entity index, or enqueue
  signal — those run on the next-cycle cache-hit replay (decision #3).
  On invalid: log with batch/custom_id/cache-key context; no cache write
  (re-dispatch next cycle).
- `LineError`: log with context; no cache write; release for re-dispatch.
- **Cache-key drift:** always store under the frozen key. If the on-disk
  prompt/context key now differs, the value simply won't be hit under the new key;
  log the drift as paid-but-unused (never silent).

**Tests (`cargo test -p harvester_core`):**
- `DeferredToBatch` on an in-flight triage/summary/signal request → request state
  `Deferred`, stage in-flight = 0, article/URL not failed, `batch_status() == Settled`.
- All-deferred session → `AwaitingBatch`, `is_active() == false`.
- `RearmDeferredBatchStages` → deferred items re-dispatchable; a re-dispatch after
  a collected cache write completes the item via cache hit with **no**
  `RequestLlmCompletion` effect **and** the expected `UpsertEntityIndexEntry` +
  `try_enqueue` (post-processing runs exactly once on replay).
- `BatchResultsCollected` `Success` (valid) → in-memory cache hit under the frozen
  key + batch-priced usage recorded; `LineError` and invalid output → no cache
  entry, item stays eligible.
- Signal deferral round-trip: deferred URL → re-arm → `try_enqueue` re-runs and
  cache-hits.
- Exact output cardinality: N collected successes → N cache inserts, 0 article
  completions in the same message.

**Verify:** `cargo test -p harvester_core`, `cargo build`.

- [ ] Deferral outcome + deferred-aware phases (all three stages)
- [ ] Re-arm epoch message + re-dispatch
- [ ] `BatchResultsCollected` (cache-write-only) + frozen-key accessors
- [ ] Tests pass

---

## Phase 4 — Manifest store + `--batch-api` flag + submit path (end-to-end submit slice)

**Files:** create `crates/harvester_batch/src/batch_manifest.rs` and
`crates/harvester_batch/src/batch_coordinator.rs`;
`crates/harvester_batch/src/cli.rs`; `scripts/Start-HarvesterBatch.ps1`;
`crates/harvester_batch/src/runner.rs`.

**CLI:** `--batch-api` (`bool`, default `false`). `conflicts_with` **`--dry-run`**,
**`--refresh-stale-summaries-limit`**, and **`--import-saved-web-dir`** (Settled
decision #1). Compatible with `--single-shot` (submit then exit; collect next
run). Add parse/conflict tests. Update the PowerShell script in the same change.

**Manifest (`.batch_manifest.ron`, atomic write via `AtomicFileWriter`):**
```
BatchManifest { version: u32, batches: Vec<PendingBatch> }
PendingBatch {
    input_file_id: String,
    batch_id: Option<String>,          // None between reserve and attach
    stage: StageKind,                  // Triage | Summary | SignalCandidate
    completion_window: String,
    submitted_at_utc: String,
    status: BatchState,                // Created | Submitted | Collected | Failed
    entries: Vec<PendingEntry>,
}
PendingEntry {
    custom_id: String,                 // stable = hash of the frozen key
    key: FrozenKeyFields,              // triage/summary: content_hash;
                                       // signal: signal_input_hash; + prompt_id,
                                       // prompt_version, model_id, context_hash
    url: String,
    rendered_system: String,           // frozen at submit for exact audit replay
    rendered_user: String,
    attempts: u32,                     // batch retry counter (Phase 6)
    collected: Option<CollectedLine>,  // durable per-line snapshot after download
}
```
`StageKind` and all manifest types are **owned by `harvester_batch`** (resolves
"referenced before an owning type is specified"). The signal key stores
`signal_input_hash`, not `content_hash` (resolves the wrong-key issue).

**Manifest API:** `load` (corrupt/unreadable → **fail closed**: return an error
and refuse to submit, never treat as empty — prevents duplicate submissions);
`save` (atomic); `reserve(stage, input_file_id, entries)+save` (**before**
create); `attach_batch_id+save` (**immediately after** create); `mark_collected`,
`mark_failed`, `remove_batch`; `pending_custom_ids() -> HashSet<String>` for
dedupe.

**`BatchCoordinator`** (owns transport (Phase 1 trait), registry + model map +
`LlmConfig` for `prepare_completion`, pricing, manifest path, submission budget).
Constructed in `build_effect_runner`'s AI branch when `--batch-api` is on.

**Submit path (routing in the runner's dispatch loop — decision #4):**
1. In the dispatch loop, before `effect_runner.enqueue(queued_effects)`,
   **partition** `RequestLlmCompletion` effects: batch-eligible prompt_ids
   (`ArticleTriage | ArticleSummary | ArticleSignalCandidate`) go to the
   coordinator buffer; everything else (briefing prompts + all effects when the
   flag is off) is enqueued unchanged. `dispatch.rs` is not modified.
2. For each diverted request, the runner reads `frozen_batch_key_for_request`
   from `AppState` (the effect payload lacks the key). If its `custom_id` is
   already in `pending_custom_ids()` → reply `DeferredToBatch` immediately (no
   resubmit).
3. Otherwise render the line via `prepare_completion` (Phase 2) +
   `openai_chat_completion_body` (Phase 1), storing rendered messages in the
   pending entry.
4. **Flush protocol (fixes the circular-wait):** the coordinator holds the buffer
   across drains. At the dispatch loop's settlement-check point, when the buffer
   is non-empty **and** there is no other pending work (no queued effects, no
   non-deferred in-flight requests, `recv` idle), `flush_pending_batch_submissions`
   runs: per stage, enforce the **submission budget** (Phase 5), upload JSONL
   (chunked to configured caps), `reserve`+save, `create_batch`, `attach_batch_id`+save,
   then send `Msg::LlmCompleted { DeferredToBatch }` for each request in the
   flushed group. To minimize batch count, batch mode raises
   triage/summary `max_in_flight` to the session limit so one drain buffers the
   whole stage.
5. **Submit-failure handling (fixes the wedge):** if rendering a line fails →
   reply that `request_id` with `LlmResultKind::Failed`. If upload or
   `create_batch` fails → reply **every** buffered request in that group with
   `Failed` (they re-dispatch next cycle, possibly batch again), and `mark_failed`
   / drop any partial reservation so no `batch_id: None` entry dedupe-wedges the
   work. Every buffered request receives *some* reply, so `MAX_ITERATIONS` is
   never hit.

**JSONL chunking (open question — verify at implementation):** cap each file at a
conservative line/byte budget (target current OpenAI per-file limits, ~50,000
lines / ~200 MB as of 2026 — **re-verify**); split a stage into multiple
`PendingBatch`es when exceeded.

**Tests (`cargo test -p harvester_batch`, fake `BatchTransport`):**
- CLI: flag parses; conflicts with dry-run / refresh / import enforced;
  script includes the flag.
- Routing: triage/summary/signal buffered; briefing prompts enqueued synchronous.
- Submit: 3 triage misses → one batch, manifest with 3 entries + `batch_id`, 3
  `DeferredToBatch`, cycle **settles** (no `MAX_ITERATIONS`).
- Dedupe: an already-pending `custom_id` is not resubmitted; unknown/duplicate
  `custom_id` handled.
- Submit-failure: upload error → all buffered replied `Failed`, no dangling
  reservation; render error → single `Failed`.
- Manifest round-trip + crash-injection: reserve-without-attach loads as
  `batch_id: None`; corrupt file → **fail closed** (error, not empty).

**Verify:** `cargo build`, `cargo test -p harvester_batch`.

**Human testing recommended:** a real `--batch-api --single-shot` run against a
throwaway key + tiny sources set; confirm a batch is created and the manifest
records its id (a few cents).

- [ ] Manifest store (fail-closed load) + coordinator
- [ ] CLI flag + conflicts + PowerShell script
- [ ] Effect diversion + flush protocol + submit-failure handling
- [ ] Tests pass

---

## Phase 5 — Collect path, durable collection, batch cost + audit (completes the loop)

**Files:** `crates/harvester_batch/src/runner.rs` (collect step + re-arm),
`crates/harvester_batch/src/batch_coordinator.rs`,
`crates/harvester_core/src/state/llm.rs` (usage plumbing),
`crates/harvester_engine/src/llm/replay.rs` (batch replay records).

**Collect step (top of each cycle, before `RearmDeferredBatchStages` + POLL):**
- For each `PendingBatch` with a `batch_id`: `retrieve_batch`. When `Completed`,
  `download_file(output)` (+ `error_file`), parse `BatchOutputLine`s, correlate
  each to its `PendingEntry` by `custom_id`, build `CollectedEntry`s (runner
  supplies `created_at_utc`), **durably snapshot** them into the manifest
  (`status = Collected`, `entry.collected = Some(..)`) and `save` **before**
  emitting `Msg::BatchResultsCollected`.
- **Removal is deferred and idempotent (fixes fire-and-forget deletion):** a
  `Collected` batch is only removed once the frozen keys are confirmed present in
  the on-disk cache store (checked at the next collect). Re-emitting
  `BatchResultsCollected` from a durable `Collected` snapshot is safe (validate +
  cache insert are idempotent under the frozen key). A crash between emit and
  cache persistence therefore loses nothing — the snapshot survives and replays.
- `Failed`/`Expired`/`Cancelled` batches → Phase 6 policy.

**Denial-of-wallet (preserve the quota contract the handle enforces):** the
coordinator enforces an explicit **submission budget** before every
`create_batch` — max requests, estimated input tokens, and estimated cost per run
(seeded from the existing `LlmQuotas`/config). Exceeding it stops further
submission this run and surfaces an explicit logged outcome; already-submitted
batches still collect. This replaces, for the batch path, the per-call quota
enforcement in `LlmHandle` that interception bypasses.

**Audit (preserve the replay contract):** collection writes a full
`ReplayRecord` to `llm_results/` for each collected line — rendered system/user
messages (from the frozen manifest entry), raw response, validation outcome,
usage, and **batch-priced** cost — so the audit trail matches the synchronous
path. (Internal state; no corpus impact.)

**Observable batch cost (concrete artifact):** batch usage recorded via
`BatchResultsCollected` flows into `state.llm_usage_by_model` and thus the cycle
table (`format_llm_usage_lines(&state.llm_usage_rows())`, `runner.rs:1342`) —
visible without touching `LlmHandle` accounting. Add a **run-report line**
(realized batch tokens + 50%-discounted cost) to `print_cycle_summary` /
`print_final_summary`, plus a test asserting the discounted microdollar total for
a known collected line. This makes the savings claim verifiable.

**Polling cadence (open question — resolved):** collect runs **once per cycle**,
at cycle start (batch turnaround is hours-scale; `--poll-interval` governs
cadence). `--single-shot` collects at start, submits during dispatch, exits;
the next run collects.

**Tests:**
- `cargo test -p harvester_core`: collected usage → exact batch-priced microdollars.
- `cargo test -p harvester_batch` (fake transport): submit cycle 1 → defer →
  settle; cycle 2 collect writes caches → re-arm → re-dispatch cache-hits →
  articles complete; manifest empty after key-presence confirmation.
- Crash-injection: crash after `Collected` snapshot but before cache persistence →
  next cycle replays from snapshot and completes (no lost results).
- Restart: rebuild runner state between cycles with the manifest persisted →
  collection still succeeds (manifest authoritative).
- Submission-budget stop is exercised and logged.

**Verify:** `cargo build`, `cargo test -p harvester_batch`,
`cargo test -p harvester_core`, `cargo test -p harvester_engine`.

**Human testing recommended:** a real two-run sequence (submit; wait for batch;
collect) confirming articles reach summarized state and the cost line shows ~50%
of standard.

- [ ] Collect step + durable `Collected` snapshot + idempotent removal
- [ ] Submission budget + `llm_results/` audit records
- [ ] Visible batch-cost artifact + tests
- [ ] Tests pass

---

## Phase 6 — Failure/reconciliation, shutdown, docs

**Files:** `runner.rs`, `batch_coordinator.rs`, `batch_manifest.rs`,
`docs/Architecture.md`, `docs/EngineeringDiary.md`.

**Failure policy (open question — resolved + justified):**
- **Per-line error / non-2xx:** log with batch/custom_id/cache-key context; no
  cache write; release for re-dispatch.
- **Whole-batch `Failed`/`Expired`/`Cancelled`:** log; `mark_failed`; release
  entries.
- **Re-dispatch:** released entries retry **via batch** (default), bounded by the
  per-entry `attempts` counter; after N attempts (default 2) fall back to
  **synchronous** dispatch for that request. *Reason:* batch is the savings goal,
  but a persistently failing batch must not wedge an article forever — sync
  fallback guarantees progress. (Immediate first-failure sync fallback is a
  one-line policy change; see Open Questions.)

**Orphan reconciliation (fixes the batch_id=None false-negative):** create-time
ordering (file_id before create, batch_id after) narrows but does not eliminate
the window where OpenAI created a batch but the second save was lost. At startup,
for every `batch_id: None` reservation, call `list_batches` and match by
`input_file_id`: if a remote batch exists → **adopt** its `batch_id` (paid work
recovered); if none exists → safe to release entries and drop the input file
reference. This runs on the first cycle before COLLECT.

**Shutdown:** the immediate-exit signal handler (`runner.rs:1687`) is unchanged —
the manifest is already durable at create time and at collection. In-flight
batches are **not cancelled** on shutdown (paid/queued; collect next run).
Documented.

**`--single-shot`:** submit-then-exit; collect next run; the RAII run lock
(`lock.rs`) prevents concurrent double-submit/double-collect.

**Docs / coupled artifacts:**
- `docs/Architecture.md`: add the batch async submit/collect path as an
  automation-path evolution; note it preserves UDF (`DeferredToBatch` +
  `BatchResultsCollected` + runner-driven re-arm epoch) and the trust boundary
  (collected output validated before caching).
- `docs/EngineeringDiary.md`: entry — `DeferredToBatch` settlement + re-arm epoch;
  manifest durability ordering + `list_batches` reconciliation; cache-key + prompt
  freezing across submit→collect; collection-writes-cache-only vs replay
  post-processing; the 50% cost artifact. Reusable lessons: (1) an async external
  stage inside a settle-until-quiescent loop needs an explicit non-terminal
  "deferred" outcome **and** an explicit cross-cycle re-dispatch epoch, not a
  hidden interception; (2) freeze every key input *and* the rendered prompt at
  submit — disk-reloaded templates/contexts drift; (3) create-time ordering alone
  cannot prove non-creation — reconcile against the provider.
- **No** `docs/CorpusFormat.md` / `CORPUS_SCHEMA_VERSION` change (Settled
  decision #2). `openai_provider_kit` version/CHANGELOG handled in Phase 1.

**Decision-log note:** the repo keeps no formal decision log; `docs/EngineeringDiary.md`
is the durable record and the entry above serves that role. Do not reference this
plan's phase numbers from the diary or code — name behaviors (`DeferredToBatch`,
`.batch_manifest.ron`, batch-priced usage, re-arm epoch) instead.

**Verify (full workspace):** `cargo build`; `cargo test`;
`cargo clippy --all-targets -- -D warnings`; `cargo fmt`.

- [ ] Failure/reconciliation (incl. `list_batches`) + tests
- [ ] Shutdown + single-shot documented
- [ ] Architecture.md + EngineeringDiary.md
- [ ] Full workspace verification
- [ ] **STOP — do not commit; leave for review**

---

## Documents to update

- `crates/openai_provider_kit/CHANGELOG.md` + `Cargo.toml` (Phase 1).
- `scripts/Start-HarvesterBatch.ps1` (Phase 4, same change as the flag).
- `docs/Architecture.md`, `docs/EngineeringDiary.md` (Phase 6).
- Not updated (intentionally): `docs/CorpusFormat.md`, `CORPUS_SCHEMA_VERSION`
  (Settled decision #2).

---

## Open Questions

1. **Batch failure fallback aggressiveness.** Default retries via batch up to 2
   attempts, then synchronous fallback. Prefer immediate synchronous fallback on
   the first batch failure instead?
2. **JSONL / account limits.** Confirm current OpenAI per-file line/byte caps and
   max in-flight batches per account at implementation; set chunk size + the
   submission budget accordingly.
3. **Submission-budget defaults.** Confirm the per-run request/token/cost caps
   (denial-of-wallet) — seed from existing `LlmQuotas`, or introduce dedicated
   `--batch-*` limits?
4. **Cross-stage batching.** Plan submits one batch per stage per cycle (clean
   per-stage collection + pricing). Confirm a single mixed batch is not preferred.
5. **Rendered-prompt storage in the manifest.** To preserve exact audit replay
   under template drift, the plan stores rendered system/user messages in each
   `PendingEntry`. Confirm the resulting `.batch_manifest.ron` size is acceptable
   (bounded by in-flight request count), vs. writing a partial replay record at
   submit time instead.
