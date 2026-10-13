# Plan: faster runs (model budget, parallel downloads, run timing)

Written 2026-10-12 from the design brief "Faster runs (download pipeline + run throughput)".
Revised 2026-10-12 after a single-shot Codex review. All three issues and both recommendations
were applied: download start notification, redirect-aware per-site slots, engine shutdown for
a blocking event receive, one deadline per fetch, and wave timing read from the wave ledger.
The three open questions were settled by the orchestrating assistant.
This plan is ephemeral: durable documents (Architecture, DecisionLog, code comments) must
name behaviours, never the phase numbers used here.

**Status (2026-10-13): paused after Phase 3, by the user's decision.**

- Phases 1-3 are committed and pushed.
- Live result: 137 s, against about 214 s before. Downloads are now the critical path.
- Phases 4-6 (parallel downloads) are deferred. They are estimated to bring a run to 50-65 s.
- Phase 7 is dropped.
- To resume, start at Phase 4 and read "Checkpoint after Phase 3" first. The Phase 8 close-out
  (DecisionLog entries, EngineeringDiary, FutureIdeas statuses, full workspace test) is still
  owed for the work already landed.

## Goal

A typical desktop Full run with about 90 new articles finishes in markedly less wall time
than today's roughly 3.5 minutes (estimated target 1 to 1.5 minutes), with:

- identical article files (same names, same bytes for the same input and fetch time), the
  same archive output and the same Stop semantics (generalised to several in-flight
  downloads);
- no increase in failed or halted model items compared with today;
- timing evidence in `engine.log` showing where each run's time went.

**The wall-time target is confirmed only by the user's next live desktop run**, read from the
per-run timing summary line this plan adds. No keyless path verifies it: `replay_bench` has no
network, polling or downloads, and the IPC probe has no throughput gate. Keyless checks prove
correctness (ordering, Stop with several downloads in flight, the per-site cap, adaptive-cap
transitions, retry behaviour) and give synthetic throughput evidence only.

## Measured baseline (desktop run 2026-10-12 04:51:24 to 04:54:58, run_id=1)

| Step | Today | Notes |
|------|-------|-------|
| Source polling | 31 s, sequential | 22 Brave searches at about 1.2 s dominate; overlaps downloads |
| Downloads | 89 fetches in about 72 s, one at a time | about 0.8 s each; several 402/403/404 with retries |
| Model stages | 224 requests, about 607 s of request time at `llm_max_in_flight=3` | about 3.4 min; ran 2.5 min past the last download. **Critical path.** |
| Corpus index refresh | about 0.9 s per refresh, 11.4 s cold | |
| Runtime-state flush | about 420 ms, debounced | |

Expected shape after this plan: model request time 607 s / 10 slots is about 61 s; downloads
become bounded by poll arrival (about 31 s) plus a short tail; the post-change floor is the
sequential Brave polling (about 26 s) and the per-article triage, summary and scoring chain
(about 8 s). That gives an estimated 70 to 90 s. This is an estimate, not a verified result.

## Decisions and who made them

### User decisions

1. Scope is the whole run: parallel downloads **and** raising the desktop's default concurrent
   model-request budget from 3 to 10, matching the command-line host. Total model cost is
   unchanged (same requests, sooner). The user said "you decide" on the recommendation; it is
   recorded as the user's decision.
2. Checkpoint after Phase 3: implementation stops after Phase 3; the user does a live desktop
   run, and Phases 4-7 are decided from its timing line (see "Checkpoint after Phase 3").

### Settled by the orchestrating assistant (cheap to change; open to challenge)

- Measure first: the first phase adds per-step download timing, stage hand-off timing, a
  per-run timing summary line and provider rate-limit headroom, and changes no behaviour.
- Download parallelism: about 6 concurrent downloads globally, at most 1 in flight per site,
  over one shared, connection-reusing HTTP client.
- The per-site cap is keyed on the **registrable domain** (as the domain blacklist is), so
  `www.x.com` and `news.x.com` share one slot and blacklist strikes do not interleave.
- Parallelism is in-process (async tasks and threads), not separate OS processes.
- Source polling stays sequential. RSS parallel polling is dropped from scope (saves at most
  about 3 s, overlaps downloads, and races the per-feed seen-set file); Brave stays sequential
  because its plan rate limit is unknown.
- A rate-limited model request waits and retries the same request a bounded number of times,
  honouring the provider's Retry-After hint with a cap. The "three consecutive rate-limit
  failures halt the run" counter then counts only exhausted retries.
- The automatic model slowdown (FI-Performance-LlmProcessing-0002) is **reducer-owned** in the
  shared scheduler (`update/model_dispatch.rs`). It must not be a worker-side gate: the LLM
  worker's FIFO semaphore would break the scoring > summary > triage priority and the stage
  in-flight counts.
- "Each identity is admitted at most once per stage per run" is unchanged; failed items are not
  re-queued within a run.
- The corpus-manifest write moves out of the per-job path, as a required step before parallel
  downloads.
- Session quota stays as it is: only the URL count (500) is enforced, at job start, which is
  already concurrency-safe. No byte or token enforcement is added.
- Exact numbers (6 global, 1 per site, 10 model requests) may be tuned after measurement.
- **Measurement-gated host costs:** the gated items are decided at the checkpoint after
  Phase 3 from the user's live run (user decision 2 supersedes the earlier no-run fallback).
- **Stop does not cut short a rate-limit wait.** In-flight model requests finish, bounded by
  the retry caps (about 60 s at most). This keeps the 2026-09-27 Stop decision.
- **Wave-size tuning is out of scope.** Judge it from the first live run's wave line and handle
  it in a separate plan if needed. It touches the 2026-09-27 wave commitment.

### Settled by this plan (implementation-level; open to challenge)

- **Run timing is observed at the host boundary**, not emitted by the reducer. A shared
  `RunTimeline` in `harvester_io` is fed by the host loops (after each reducer step, reading
  state) and by `EffectRunner` I/O hooks. The reducer gains no clock and no new effect; this
  follows the existing driver rule "timings live at the host boundary".
- **Shared client with explicit redirect following.** reqwest's custom redirect policy cannot
  carry a per-request redirect counter on a shared client, so the fetcher follows redirects
  itself (GET only, same limit, URL policy checked on every hop, redirect count reported).
- **One deadline per fetch attempt.** Explicit redirects share one `request_timeout` deadline
  across every hop and the final body, matching reqwest's whole-request timeout today.
- **The per-site cap covers every request, including redirect destinations.** A job holds at
  most one site slot at a time: the slot of the host it is currently requesting. Before a hop to
  a different site it releases its current slot and then acquires the destination's slot.
  Because no job holds one slot while waiting for another, redirects that cross between
  occupied sites cannot deadlock. The slot is released once the response body is read, so
  extraction and writing do not hold a site.
- **Downloads report their start at dispatch.** The engine sends a start notification through
  the existing progress action when it dispatches a job, before any network I/O. The reducer
  therefore counts requests that are still waiting for response headers as in-flight
  downloads.
- **Runner drop shuts the engine down explicitly.** A dedicated shutdown command, separate from
  the resumable Stop, ends the engine worker so that event forwarding can use a blocking
  receive.
- **URL-policy DNS resolution and CPU-bound job steps leave the async workers** (each step in
  `spawn_blocking` under its existing timeout, preserving per-stage failure kinds).
- **A corpus-marker write failure no longer fails the article.** It is logged with the output
  folder and retried on the next run. This follows the owner's stated preference for tolerance
  over refusal.
- **Rate-limit retry budget:** at most 3 rate-limit retries per request; each wait is the
  provider's Retry-After capped at 20 s, or 2 s, 4 s, 8 s with jitter when no hint is given.
- **Adaptive cap rule:** halve the effective cap (floor 1) on a rate-limit signal from a request
  dispatched under the current cap; add 1 after 10 consecutive clean completions, up to the
  configured budget; reset to the configured budget at each run start.
- Download concurrency is an `EngineConfig` setting with defaults 6 and 1. There is no new CLI
  flag or environment variable, so launch scripts are unchanged.

## Verified facts (code survey, 2026-10-12)

- `crates/harvester_engine/src/engine.rs` `worker_loop`: one std thread with its own tokio
  runtime runs one job at a time via `runtime.block_on(run_job(..))`. Stop drains the queue
  with `Cancelled`; Resume creates a fresh `CancellationToken`. `quota_tracker.check_url()` runs
  before each job. The quota tracker lives for the worker's lifetime.
- `run_job` writes the article and then `write_corpus_manifest` inside one `spawn_blocking`;
  any error in either returns `ProcessingError` for the job. `AtomicFileWriter::write`
  (`persist.rs`) removes an existing target before persisting the temporary file, so two
  concurrent writers of one file can fail on Windows.
- The marker is also written by `export.rs` and `import.rs`; those writers stay unchanged.
- Article filenames come from `deterministic_filename(title, url)` (sanitized title plus a short
  URL hash), so they do not depend on completion order.
- Tokens are counted twice per job: once for the Tokenizing progress event and again inside
  `build_markdown_document` (`frontmatter.rs`), on the same Markdown.
- `fetch.rs` `fetch_once` builds a new `reqwest::Client` per attempt. Its custom redirect policy
  stores the hop count in a per-client `Arc<AtomicUsize>` and checks the URL policy on each hop.
  `FetchMetadata::redirect_count` is asserted only by `tests/fetch.rs`. The client's
  `timeout(request_timeout)` today covers the whole request, including redirects and the body.
- `fetch_once` emits its first progress event (`Stage::Downloading`, `bytes: Some(0)`) only
  after the response headers arrived and the status, size and content-type checks passed.
  Until then the reducer still sees the job as `Stage::Queued`:
  - `preserve_unstarted_downloads_for_retry` (`state/ingest.rs`) preserves it into pending
    intake on Stop;
  - `stopping_in_flight_count` (`state/run_progress.rs`) excludes it, because it counts only
    `download_started_job_ids`, which `Msg::JobProgress` fills.
  This gap exists today for one download and grows with several in flight.
- `EffectRunner::spawn_event_loop` keeps a full `EngineHandle` clone, including the command
  sender, in a forwarding thread that never exits. Runner drop sends `stop(true)`. The engine
  worker exits only when every command sender is dropped, so after drop it stays alive waiting
  for `Resume`, and its event sender stays connected. A blocking `recv` on that thread would
  therefore never return.
- `UrlPolicy::check` resolves DNS with blocking `to_socket_addrs`; it is called inside the
  async `fetch_once` and synchronously in `EffectRunner::validate_effect` for every `EnqueueUrl`.
- `registrable_domain(url)` (`harvester_engine/src/domain.rs`) returns `None` for IP literals and
  hosts without a public suffix (for example `127.0.0.1`, `localhost`).
- `EffectRunner::spawn_event_loop` (`harvester_io/src/effect_runner/mod.rs`) forwards engine
  events with `try_recv` plus a 20 ms sleep, sending `FetchOutcomeClassified` then `JobDone`.
- The reducer requests `PersistRuntimeState` (a full `PersistenceSnapshot::capture`) on both
  `FetchOutcomeClassified{Success|PermanentBlock}` and `JobDone{Success}`
  (`harvester_core/src/update/mod.rs`), so twice per successful download.
- The reducer already models several downloads in flight: `RunProgress` keeps
  `download_started_job_ids` and `download_finished_job_ids`, `RunState::Stopping{in_flight}`
  counts them, and wave tests already exercise `Stopping { in_flight: 2 }`. Stop preserves
  only `Stage::Queued` jobs into pending intake.
- Processing loads URLs from `ordered_completed_job_urls_snapshot()`, which iterates a
  `BTreeMap<JobId, JobState>`, so load order is job-id (enqueue) order regardless of completion
  order. Wave **membership** already depends on timing (which jobs completed before a refresh);
  only membership timing, not ordering, changes with parallel downloads.
- The wave ledger `PipelineWaves.waves` (`pipeline_waves.rs`) records each released wave with
  its number, run id, stage and members. Per-stage admitted totals in `RunProgress` can grow by
  several waves in one reducer step, so individual wave sizes must come from the ledger.
- Model dispatch: `has_slot` compares `article_model_requests_in_flight()` with
  `llm_max_in_flight()`. Request ids come from one monotonic
  `allocate_next_llm_request_id()`. The LLM worker (`harvester_engine/src/llm/handle.rs`)
  bounds concurrency with a `Semaphore` sized to the same budget and retries only
  `is_retryable()` errors once after 2 s (`MAX_ATTEMPTS = 2`).
- `openai_provider_kit` maps HTTP 429 to `LlmError::RateLimited { retry_after_secs }` (integer
  Retry-After) unless the body says `insufficient_quota`; `RateLimited` is non-retryable.
  `LlmResponse` exposes no response headers. The kit is versioned (0.4.0) with a CHANGELOG.
- `harvester_core/src/state/provider_alert.rs`: three consecutive `RateLimited` results halt
  model dispatch for the run; any owned success resets the counter.
- Desktop default `DEFAULT_LLM_MAX_CONCURRENT_REQUESTS = 3` in
  `harvester_io/src/host_bootstrap.rs`, overridable by `LLM_MAX_CONCURRENT_REQUESTS`, clamped
  by `MAX_LLM_CONCURRENT_REQUESTS = 10`. No launch script sets this variable.
- Both hosts use `EffectRunner`, so parallel downloads apply to the command-line host too.
  The desktop driver (`harvester_ui_bridge/src/driver.rs`) and the batch dispatch loop
  (`harvester_batch/src/runner/dispatch_loop.rs`) already time each reducer step and call a
  `reducer_observer(kind, elapsed)`.
- `harvester_batch/examples/replay_bench.rs` supports `--host desktop` and
  `--llm-latency-ms`. It takes its model concurrency from the batch argument default (10) and
  has no flag to change it.
- `harvester_engine` has `wiremock` and a tokio `TcpListener` stub pattern in its tests
  (`tests/fetch.rs`, `tests/engine_stop.rs`).

## Phases

Run every command from the repository root `C:\Users\larsp\src\web_page_filet_mignon`.

Standard Rust gate, used by every code phase:

```
cargo build
cargo test -p <each touched crate>
cargo clippy --all-targets -- -D warnings
cargo fmt
```

- If a `harvester_batch` run is active (it locks `target/debug/harvester_batch.exe`), build with
  `cargo build --workspace --exclude harvester_batch` and defer the batch crate's tests.
- If `crates/harvester_ui` changes, also run `cargo clippy -p harvester_ui --all-targets -- -D warnings`.
- The full workspace `cargo test` (which includes timing tests) needs a quiet window from the
  "Project orchestration" session. Per-crate tests do not need one. The desktop driver
  snapshot test has a 20 ms timeout and can be flaky under load; rerun it before blaming a change.
- No phase changes the frontend or launch scripts, so `npm` checks and Pester are not needed.
- Tests follow Agents.md: they cover reducer behaviour, emitted effects and public contracts
  rather than implementation details. Pin the baseline test count of each touched crate before
  a phase and confirm that no existing test was removed, except where a phase says a test is
  replaced.

### Phase 1 - Measurement baseline (no behaviour change)

Goal: every run logs where its time went, and the user's next live run gives a baseline.

Changes:

1. **Per-job download timing** (`harvester_engine`).
   - Add `JobTiming` (queue wait, fetch including retries, fetch attempts, decode, extract,
     tokenize, document build, article write, marker write, total, bytes) and attach it to
     `EngineEvent::JobCompleted` for both success and failure, plus the `Instant` the event was
     sent.
   - `run_job` measures each step with `Instant` only. Outcomes, ordering and bytes do not
     change.
   - Log one `[job-timing] job_id=.. url=.. outcome=.. queue_wait_ms=.. fetch_ms=.. attempts=..
     decode_ms=.. extract_ms=.. tokenize_ms=.. build_ms=.. write_ms=.. manifest_ms=..
     total_ms=.. bytes=..` line through `engine_logging`.
2. **Provider rate-limit headroom** (`openai_provider_kit`, `harvester_engine`).
   - The kit parses `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`,
     `x-ratelimit-limit-tokens`, `x-ratelimit-remaining-tokens`, `x-ratelimit-reset-requests`
     and `x-ratelimit-reset-tokens` when present. It exposes them through an additive accessor
     on `LlmResponse` (for example `rate_limit_headroom() -> Option<RateLimitHeadroom>`),
     set by a builder method so `LlmResponse::new` stays source-compatible.
   - The LLM worker adds the headroom fields to its existing per-request completion log line and
     copies them into `LlmRunMetadata` as an optional `#[serde(default)]` field. Replay records
     stay write-only forensics; the field is additive.
   - Bump `openai_provider_kit` to 0.5.0 and add an "Added" CHANGELOG entry.
3. **Run timeline and summary** (`harvester_io`, both host loops).
   - Add `RunTimeline` to `harvester_io`. It is a plain aggregator with injected `Instant`s so it
     can be unit-tested.
   - `EffectRunner` owns one `Arc<Mutex<RunTimeline>>` and feeds it from its I/O boundaries:
     - poll start and end (poll thread and `PollGuard`);
     - `EnqueueUrl` hand-off, including the duration of `validate_effect`;
     - each engine `JobCompleted` (its `JobTiming` plus the forwarding delay from send to
       `Msg`);
     - each `RequestLlmCompletion` dispatch and each LLM completion per prompt id (wall time,
       outcome, headroom).
   - Both host loops call one `RunTimeline` hook after every reducer step, through the existing
     `reducer_observer` seam, widened to also receive `&AppState`. The hook reads only
     read-only accessors: run id, `pipeline_run_phase`, model requests in flight, reducer
     elapsed time by message kind, and the released waves.
   - Released waves are read from the wave ledger through a new read-only `AppState` accessor
     that returns `(number, stage, member count)` for this run's waves. The hook records each
     newly seen wave number, so several waves released in one reducer step are still logged
     individually. The accessor adds no state and changes no behaviour.
   - When the hook sees the run reach Terminal, it logs one summary line and resets. The line
     has wall time and outcome; poll end; first download start and last download completion;
     download count and outcomes; sums and maxima of fetch, extract, write and marker time; max
     downloads in flight; and for each model stage, request count, first and last dispatch,
     last completion, average wall time and failures. It also has max model requests in
     flight; rate-limited results; minimum observed remaining requests and tokens; reducer time
     on `JobDone`, `FetchOutcomeClassified` and `LlmCompleted`; total effect hand-off time for
     `EnqueueUrl`; and maximum engine-event forwarding delay. Example: `[run-timing]
     run_id=.. host=desktop outcome=completed wall_ms=.. ...`.
   - A second line, `[run-timing-waves] run_id=.. stage=.. releases=<wave:offset_ms:members,...>`,
     lists each wave's release offset and size per stage. Waves released in the same reducer
     step share an offset. Offsets are relative to run start. If a run is
     stopped, the line also gives the Stop offset and the drain duration.
   - The command-line host logs the same lines with `host=batch`.
4. **Keyless benchmarks.**
   - `harvester_batch/examples/replay_bench.rs` gains `--llm-concurrency N` (default unchanged)
     so the desktop-host replay can compare model budgets with synthetic latency.
   - New `harvester_engine/examples/download_bench.rs` starts K local stub sites (distinct
     ports on `127.0.0.1`), each with configurable latency and body size. It enqueues N jobs
     spread over the sites into a temporary output folder and reports wall time, max
     concurrency and per-step sums. In this phase it measures the sequential baseline.

Regression tests:

- `RunTimeline` unit tests with synthetic instants:
  - summary fields and offsets;
  - wave detection from the ledger, including two waves released in one step being logged
    separately with their own sizes;
  - Stop offset and drain;
  - reset between runs;
  - a run with no downloads or no model work.
- Core: the wave-ledger accessor returns this run's waves in release order with correct
  stages and member counts. A reducer test confirms that calling it does not change state.
- Kit: header parsing with all, some and none of the headers present; malformed values
  ignored.
- Engine: `JobCompleted` carries timing on success and on each failure path; the existing
  `engine_stop.rs` and `fetch.rs` tests pass unchanged apart from pattern updates for the new
  event field.
- Existing reducer tests pass unchanged. The reducer gains only read-only accessors
  (ledger, run id, terminal status, model requests in flight, stop status and provider halt).

Verification:

- Standard Rust gate for `openai_provider_kit`, `harvester_engine`, `harvester_core`,
  `harvester_io`, `harvester_ui_bridge`, `harvester_batch`.
- `cargo run -p harvester_engine --example download_bench` (record the sequential baseline in
  the phase hand-off).
- IPC probe, because the desktop driver changed: `cargo run -p harvester_ui -- --probe-ipc`
  (needs a display and the GUI lock, about 150 s, writes `.local/probe/ipc-report.json`, exits
  non-zero on a failed gate).
- **External human testing optional:** a normal live desktop Full run by the user confirms
  the `[run-timing]` baseline in `engine.log`. The assistant does not run launchers. The
  deciding run for Phases 4-7 is the one at the checkpoint after Phase 3.

Docs: Architecture.md gains a short "Run timing" note (host-boundary observation; the reducer
stays clock-free). `openai_provider_kit/CHANGELOG.md` gets an entry.

**Phase 1 hand-off (done 2026-10-12).**

- Implemented by Codex and reviewed once by Opus. All 5 findings were applied: the benchmark
  crash on Windows, poison-tolerant timeline locks, flat key=value log fields, Windows-safe
  timing tests, and the outcome read from state as completed/stopped/halted.
- The core gained only read-only accessors: the wave ledger, run id, terminal status, and
  model requests in flight.
- Verified keylessly:
  - `cargo build` passes.
  - Per-crate tests for the six touched crates: 1384 passed, 0 failed, 2 ignored. The baseline
    was 1352 tests and none were removed.
  - Both clippy gates pass.
  - `cargo fmt` is clean.
  - The IPC probe passed every gate.
- Sequential `download_bench` baseline (4 sites, 12 jobs, 50 ms latency, 8 KiB bodies):
  `wall_ms=682 max_concurrency=1 failures=0`. The step sums are `queue_wait_ms=3766 fetch_ms=624
  extract_ms=9 write_ms=17 manifest_ms=19 total_ms=4446`.
- Follow-ups for Phase 2:
  - Rate-limit headroom is captured only from successful responses. 429 responses also carry
    the headers, but recording them needs an additive extension to the kit's error contract.
    Consider this when adding rate-limit retry.
  - A batch run interrupted with Ctrl-C logs no `[run-timing]` line.
- Not yet observed: the `[run-timing]` lines from a live desktop run (optional; the user's run).
- Next: Phase 2.

### Phase 2 - Rate-limit retry and reducer-owned adaptive cap (desktop default still 3)

Goal: a brief "too fast" from the provider no longer fails an article stage, and the scheduler
slows down by itself under rate-limit pressure. This is the safeguard required before the
budget is raised.

Changes:

1. **Worker-side bounded retry** (`harvester_engine/src/llm/handle.rs`).
   - `LlmError::RateLimited` gets its own retry loop, separate from the transient-error retry.
     It allows up to 3 rate-limit retries per request. Each wait is `retry_after_secs` capped at
     20 s, or 2 s, 4 s, 8 s with ±25 % jitter when no hint is given.
   - The request keeps its worker permit and its reducer slot while it waits. No request is
     re-queued, and the identity is still admitted once per stage per run.
   - Stop does not cut the wait short. A waiting request is in flight and finishes under the
     2026-09-27 Stop rule, bounded by the caps (about 60 s at most).
   - The retry count is recorded in `LlmRunMetadata` as `rate_limit_retries` (`#[serde(default)]`).
   - When retries are exhausted, the request returns `RateLimited` exactly as today.
   - Delays come from `LlmConfig` so tests can use milliseconds.
   - `openai_provider_kit` keeps `RateLimited` non-retryable in `is_retryable()`. The policy
     lives in Harvester, not in the kit.
   - Log each wait as `[llm-retry] request_id=.. reason=rate_limited attempt=.. wait_ms=..`.
2. **Reducer-owned adaptive cap** (`harvester_core`).
   - `AppState` keeps an `effective_llm_cap`, initialised from `llm_max_in_flight` at run
     start.
   - `has_slot` compares in-flight requests with `effective_llm_cap`. The configured budget
     remains the upper bound, and the worker semaphore stays at the configured budget, so the
     worker never queues.
   - A **rate-limit signal** is an article-model `LlmCompleted` with a `RateLimited` result or
     with `metadata.rate_limit_retries > 0`.
   - On a signal from a request id at or above `cap_epoch_request_id`, the cap is halved
     (floor 1) and `cap_epoch_request_id` is set to the next request id. That allows one
     decrease per "window" of requests issued under the previous cap.
   - After 10 consecutive clean article-model completions, the cap rises by 1, up to the
     configured budget.
   - Requests already in flight above a lowered cap finish normally; only new dispatch waits.
   - Stage priority (scoring, then summary, then triage) and in-stage admission order are
     unchanged, because selection logic is untouched and only the slot count shrinks.
   - Log each change as `[llm-adaptive-cap] run_id=.. from=.. to=.. reason=rate_limited|recovered
     request_id=..`. The run-timing summary adds the minimum effective cap and the number of
     changes.
3. **Halt semantics.** `note_provider_rate_limited` is unchanged in code. Because `RateLimited`
   now arrives only after exhausted retries, the halt counts exhausted retries. A success that
   needed retries still resets the counter.

Regression tests:

- Worker (`harvester_engine/tests/llm_handle.rs` style, mock provider):
  - 429, 429, then success returns success with `rate_limit_retries = 2`;
  - four 429s return `RateLimited`;
  - a Retry-After above the cap is clamped;
  - transient-error retry behaviour is unchanged;
  - `QuotaExhausted` (insufficient_quota) is never retried.
- Reducer (`update/model_dispatch_tests.rs`, `provider_alert_tests.rs`):
  - the cap halves on a `RateLimited` completion and on a recovered success with retries;
  - a second signal from a request dispatched before the decrease does not halve again;
  - the floor is 1;
  - 10 clean completions add 1, and the cap never exceeds the configured budget;
  - the cap resets at run start;
  - with a lowered cap and pending scoring, summary and triage, freed slots still go scoring
    first;
  - in-flight requests above a lowered cap are not failed or withdrawn;
  - three consecutive exhausted `RateLimited` results still halt with the existing reason;
  - recovered successes do not increment the halt counter;
  - Stop while the cap is lowered still withdraws pending work and drains in-flight work.
- `effect_helpers.rs`: `map_llm_event` carries `rate_limit_retries` through metadata.

Verification: standard Rust gate for `harvester_engine`, `harvester_core`, `harvester_io`.
Live behaviour under real rate limits cannot be verified keylessly; the summary line from the
user's next live run reports rate-limited results, retries and cap changes.

Docs:

- Architecture.md "Shared article-model request budget" describes:
  - the effective cap and its rules;
  - the bounded rate-limit retry;
  - the halt counting exhausted retries.

**Phase 2 hand-off (done 2026-10-12).**

- Implemented by Codex and reviewed once by Opus. The verdict was Approved, with 2 minor
  findings and 3 advisory cleanups, all applied:
  - a request that hit a 429 and then failed for another reason now carries
    `rate_limit_retries` on its failure, so it counts as a rate-limit signal and is not counted
    as clean;
  - the `[llm-run]` and provider-error log lines gain `transient_attempts=` and
    `rate_limit_retries=`;
  - the cap change is a named `LlmCapChange` struct;
  - the owning stage is resolved once per completion;
  - the `[llm-concurrency]` logs gain `effective_cap=`.
- Cap-change logging (`[llm-adaptive-cap]`) and the summary fields (minimum effective cap,
  change count) are produced at the host boundary by `RunTimeline`, which reads read-only
  `AppState` accessors. The reducer gained no clock and no effect.
- Not applied, because it conflicts with the plan's spec: jitter on waits that follow a
  Retry-After hint (only the no-hint path is jittered). This is a possible tuning point if the
  live run shows synchronized retries.
- Verified keylessly:
  - `cargo build` passes.
  - `cargo test -p harvester_engine -p harvester_core -p harvester_io -p harvester_batch
    -p harvester_ui_bridge`: 1375 passed, 0 failed, 1 ignored. Before the phase, the engine
    had 438 tests, core 609, io 169 and batch 98. No test was removed.
  - `cargo clippy --all-targets -- -D warnings` passes.
  - `cargo fmt --check` is clean.
  - Not run: the full workspace `cargo test` (deferred to the close-out quiet window) and the
    IPC probe (no desktop host change; the plan does not require it for this phase).
- Reviewer notes for the close-out:
  - DecisionLog entry 2: say that a rate-limit retry during Stop belongs to the in-flight
    request, so it is not a "new model request" under 2026-09-27. Also say that the
    effective cap is runtime adaptation, not frozen configuration.
  - Recovery counts clean completions from requests sent before a decrease, so after a drop
    from 10 to 5 the cap can rise to 6 quickly. That matches the spec (slow up, fast down).
    Watch it in the live run's `[llm-adaptive-cap]` lines.
  - The process-shutdown drain gives each request 30 s, which a request waiting on a rate
    limit can exceed. This affects app close only, not run Stop.
- Still open from Phase 1: rate-limit headroom is still captured only from successful
  responses (the kit's error contract is unchanged), and Ctrl-C batch runs log no
  `[run-timing]` line.
- Next: Phase 3 (desktop default 3 -> 10).

### Phase 3 - Desktop model budget 3 -> 10 (user decision)

Goal: the critical path shrinks, which gives the biggest single wall-time gain.

Changes:

- `DEFAULT_LLM_MAX_CONCURRENT_REQUESTS = 10` in `harvester_io/src/host_bootstrap.rs`.
- `LLM_MAX_CONCURRENT_REQUESTS` still overrides it (clamped 1 to 10).
- Update the host-bootstrap tests that pin the default.
- No launch-script change, because the launch policy does not set this variable.

Regression tests:

- Host bootstrap:
  - an absent variable yields 10;
  - explicit `3` yields 3;
  - `999` clamps to 10;
  - `0` clamps to 1.
- The desktop startup state receives 10 as `llm_max_in_flight` and as the worker budget.

Verification:

- Standard Rust gate for `harvester_io` (and `harvester_ui_bridge` if its fixtures pin the
  value).
- Keyless synthetic evidence: run `cargo run -p harvester_batch --example replay_bench -- --host
  desktop --llm-latency-ms 2500 --llm-concurrency 3`, then repeat with `--llm-concurrency 10`.
  It reads the local `output` folder read-only into `.local/bench/`. Record both wall times;
  they show only model-stage scaling.
- IPC probe: `cargo run -p harvester_ui -- --probe-ipc`.
- **External human testing recommended:** a live desktop run. Expect markedly shorter
  model-stage time and no new failed or halted items. Downloads (about 72 s) now become the
  visible floor.

Docs: Architecture.md budget paragraph changes "Desktop uses `LLM_MAX_CONCURRENT_REQUESTS`
(default 3)" to default 10.

**Phase 3 hand-off (done 2026-10-12, commit d4f14de).**

- Implemented by Codex and reviewed once by Opus. The verdict was Approved, with 1 minor
  finding, which was applied: the startup test now feeds one parsed budget to both the desktop
  startup state and the real effect-runner builder, as `harvester_ui` does, and asserts the
  literal 10 for each.
- Verified keylessly:
  - `cargo build` passes.
  - `cargo test -p harvester_io`: 177 passed, 0 failed. One test was added and none was
    removed. Two assertions were tightened from constants to the literal 10, and `"0"`
    clamping to 1 was added.
  - `cargo clippy --all-targets -- -D warnings` passes, and `cargo fmt --check` is clean.
  - `harvester_ui_bridge` fixtures set the budget explicitly, so they were unaffected.
  - Synthetic `replay_bench` (desktop host, 2500 ms model latency): driver wall 83.2 s at
    concurrency 3 and 33.1 s at concurrency 10. This shows model-stage scaling only.
  - IPC probe: passed, all four cases.
  - Not run: the full workspace `cargo test` (deferred to the close-out quiet window).
- Reviewer notes for the checkpoint run:
  - Desktop waves grow from 12 to 40 members, because wave size is four times the budget.
    `[run-timing-waves]` shows fewer, larger waves. This is expected; wave tuning is out of
    scope.
  - No `[llm-concurrency]` line is logged when the variable is absent. Core's
    `[model-budget] llm_max_in_flight=10` line confirms the new default.
- Next: the checkpoint. The user runs a live desktop Full run and returns its `[run-timing]`,
  `[run-timing-waves]` and any `[llm-adaptive-cap]` lines. Do not start Phase 4 before the
  checkpoint decision is recorded here.

### Checkpoint after Phase 3 - live run decides Phases 4-7 (user decision)

Implementation stops after Phase 3. The user performs a live desktop run with Phases 1-3 in
place. The user and the implementer then read that run's per-run timing summary and wave
lines and decide which of Phases 4-7 proceed, in what form, or whether they are dropped.
Record the decision and the numbers it rests on in this plan before starting Phase 4. Phase 4
remains a prerequisite of Phase 6 if parallel downloads proceed.

**Checkpoint run result (desktop, 2026-10-13 04:08:49 to 04:11:06, run_id=1, budget 10).**

| Measure | Baseline (budget 3) | This run (budget 10) |
|---------|---------------------|----------------------|
| Wall time | about 214 s | 137 s |
| Downloads | 89 fetches in about 72 s | 122 fetches (108 ok, 14 failed), last completion at 119 s |
| Per-article wall time | about 2.4 s | about 1.3 s |
| Model requests | 224 | 299 (triage 101, summary 99, scoring 99), 0 failures |
| Model tail after the last download | about 150 s | 18 s |

- **Downloads are now the critical path.** Max downloads in flight was 1, the summed fetch
  time was 112 s, and the longest queue wait was 85 s. Polling ended at 33 s, so downloads ran
  about 86 s past the end of polling. Model stages kept pace and finished 18 s after the last
  download.
- **Model budget was healthy:** max in flight 10, no rate-limited results, no rate-limit
  retries, no adaptive cap change (minimum effective cap 10), and minimum remaining provider
  headroom 9984 requests and about 9.96M tokens.
- **Problems:** none attributable to this plan. The 14 download failures were 12 HTTP 403s, one
  site 429 (basenor.com, after 3 attempts) and one network error, which is the usual mix for
  these sources. No ERROR lines appeared, and no model item failed or halted.
- **Host costs are small:** reducer time on `JobDone` was 1.8 s, on `FetchOutcomeClassified`
  1.9 s and on `LlmCompleted` 1.0 s, and the `EnqueueUrl` hand-off was 4.6 s in total.
- **Recommendation (assistant; awaiting the user's decision):**
  - Proceed with Phases 4, 5 and 6 (parallel downloads). The estimated run is about 50 to
    65 s: polling (33 s) plus a short download tail plus the about 9 s triage, summary and
    scoring chain. That saves another 70 to 85 s, roughly halving today's 137 s. This is an
    estimate, not a measurement.
  - Drop or defer Phase 7. Its measured share is under about 10 s in total, and part of it
    overlaps other work.

**Checkpoint decision (user, 2026-10-13, relayed by the orchestrating assistant).**

- The current speed is acceptable for now.
- Phases 4-6 are deferred, not cancelled. They can resume later from this plan as written.
- Phase 7 is dropped.
- The plan is paused here.

### Phase 4 - Corpus marker written once per run (required before parallel downloads)

Goal: no job writes `harvester-corpus.json`, so concurrent jobs never race on one file, and a
marker problem never marks a written article as failed.

Changes (`harvester_engine/src/engine.rs`):

- Remove `write_corpus_manifest` from `run_job`'s write closure.
- The engine writes the marker once per **run epoch**, before dispatching the first job after
  engine start or after `Resume`, on the scheduler side. No job can be in flight then, so there
  is never a concurrent writer.
- The marker therefore still appears in a folder only once Harvester downloads into it.
- On failure, the engine logs `engine_warn!("[corpus-marker] output_dir=.. error=..")`, lets
  the jobs proceed, and retries in the next epoch.
- Export and import keep their own marker writes.
- The marker layout, `CORPUS_SCHEMA_VERSION` and `harvester-corpus.json` content generation are
  unchanged. Only the refresh timing of `written_at_utc` changes. There is no schema bump.

Regression tests (`harvester_engine/tests/`):

- After several successful jobs in one run, the marker exists. If it is deleted mid-run, later
  jobs in the same run do not recreate it. After `stop` and `resume`, the next run recreates it.
- When the marker path is unwritable (a directory named `harvester-corpus.json`), jobs still
  complete `Ok` and their article files exist.
- The existing `write_corpus_manifest_creates_output_marker` and `engine_stop.rs` tests pass.

Verification: standard Rust gate for `harvester_engine` and `harvester_io`.

Docs: CorpusFormat.md adds one sentence saying `written_at_utc` records when Harvester last
refreshed the marker (once per download run, and on archive export and import), not a
per-article time. There is no layout or schema change.

### Phase 5 - Download jobs ready for concurrency (still one at a time)

Goal: make each job safe and cheap to run beside others while the engine is still sequential,
so byte-for-byte parity can be checked before parallelism.

Changes (`harvester_engine`):

1. **One shared HTTP client per fetcher.**
   - `ReqwestFetcher` builds one `reqwest::Client` (keep-alive pool) with redirects disabled and
     follows redirects itself.
   - The redirect loop:
     - handles 301, 302, 303, 307 and 308 with GET;
     - resolves `Location` relative to the current URL;
     - enforces `redirect_limit` (`RedirectLimitExceeded`);
     - checks the URL policy on every hop (`UrlPolicyViolation`);
     - sets `Referer` as reqwest does today (omitted on an https-to-http downgrade);
     - reports the real hop count in `FetchMetadata::redirect_count`.
   - **One deadline per attempt.** An attempt sets `deadline = start + request_timeout` once.
     Every hop's request (connect and headers) and the final body stream run under
     `tokio::time::timeout_at(deadline, ..)`, so the allowance does not restart at each hop and
     an attempt still ends within `request_timeout`, as today. Expiry maps to
     `FailureKind::Timeout`, which is retryable as today. `connect_timeout` still applies per
     connection. Each retry attempt gets a fresh deadline, as each attempt does today. Time
     spent waiting for a site slot (added in the parallel-downloads phase) is not charged to
     the deadline; it is bounded separately (see that phase).
   - **Per-hop gate hook.** Before each request the loop calls a `HopGate` with the hop's URL.
     The gate is a no-op here; the parallel-downloads phase supplies the site-slot
     implementation. The `Fetcher` trait gains this as a fetcher-held dependency, not a
     per-call argument, so standalone fetcher users keep the no-op.
   - The byte cap, content-type checks, progress events, retries and Retry-After handling are
     unchanged.
2. **No blocking DNS on async workers.** The fetcher runs `UrlPolicy::check` (blocking
   `to_socket_addrs`) via `spawn_blocking`, for the initial URL and for each redirect hop.
3. **CPU steps off async workers.**
   - Decode, extract (with blocked-page detection), tokenize and document build each run in
     `spawn_blocking` under their existing timeouts.
   - The failure kinds (`ProcessingTimeout{Sanitizing|Converting|Tokenizing}`,
     `ProcessingError`), the progress events and the cancellation checks between steps stay the
     same.
   - A timed-out blocking step's result is discarded, as today.
4. **Count tokens once.** Use one count for both the Tokenizing progress event and the
   frontmatter `token_count` (for example a `build_markdown_document_with_count` variant). The
   article bytes stay identical.

Regression tests:

- `tests/fetch.rs` (wiremock):
  - a redirect chain reports the right `redirect_count` and `final_url`;
  - exceeding the limit gives `RedirectLimitExceeded`;
  - a redirect to a blocked host or private IP (policy with private IPs blocked) gives
    `UrlPolicyViolation`;
  - a relative `Location` resolves;
  - 303 is followed as GET;
  - two sequential fetches to one stub reuse a connection (the stub counts accepted
    connections);
  - **delayed redirect chain:** three hops that each delay about 40 % of a short
    `request_timeout` fail with `Timeout` within about one `request_timeout` per attempt,
    instead of succeeding hop by hop. This protects the Stop drain bound;
  - a delayed body after fast redirects is cut off by the same deadline;
  - `HopGate` is called once per hop with each hop's URL, using a recording test gate;
  - existing retry, timeout and too-large tests pass.
- **Byte-parity test:** a fixed set of stub articles with a fixed `fetched_utc` clock produces
  the same filenames and the same bytes as a checked-in expected listing (file name plus
  SHA-256), including `token_count`.
- Existing extraction, frontmatter-security and `engine_stop.rs` tests pass.

Verification:

- Standard Rust gate for `harvester_engine` and `harvester_io`.
- `cargo run -p harvester_engine --example download_bench` (sequential; connection reuse may
  already help same-site runs).
- Review `docs/ThreatModel.md` (SSRF mitigation). The policy is still checked on every hop, so
  no change is expected; record the review in the phase hand-off.

Docs:

- Architecture.md "Content pipeline" states that:
  - one pooled client follows redirects explicitly under the URL policy;
  - one deadline covers each attempt's redirect chain and body;
  - CPU steps run off the async workers.

### Phase 6 - Parallel downloads with a per-site cap

Goal: downloads stop being the floor. Up to 6 run at once, at most 1 per registrable domain,
with Stop semantics generalised to several in-flight downloads.

Changes (`harvester_engine/src/engine.rs` and `fetch.rs`, `harvester_io` event forwarding,
`harvester_core` progress handling):

1. **Configuration.** `EngineConfig.download_concurrency: DownloadConcurrency { global: 6,
   per_site: 1 }` (clamped: global 1 to 16, per-site 1 to global). Defaults exercise the new
   path. Both hosts get it through `EffectRunner`, with no CLI flag and no environment variable.
2. **Site key.** `registrable_domain(url)`, falling back to the lowercase `host[:port]` when
   there is none (IP literals, `localhost`), then to the raw URL for unparseable input. This
   keeps blacklist accounting aligned and lets local stub servers on distinct ports act as
   distinct sites in tests.
3. **Site slots shared by dispatch and redirects.**
   - One `SiteSlots` registry (per-site permits up to `per_site`, plus a notifier) is owned by
     the scheduler and shared with the fetcher as its `HopGate` implementation from phase 5.
   - Dispatch takes the origin site's slot.
   - **Redirect hops:** when a hop's site differs from the site whose slot the job holds, the
     job releases its current slot, then waits for the destination's slot, then sends the hop.
     A job never holds two site slots and never waits while holding one. Two jobs that redirect
     into each other's sites therefore cannot deadlock, and two origins that redirect to one
     destination never request it concurrently.
   - A slot wait is cancellable by the run's token and does not count against the attempt
     deadline. It is bounded in practice because every slot holder is itself under a deadline.
     The wait is recorded in `site_wait_ms`.
   - Same-site hops keep the slot.
   - Retry attempts start again from the original URL and so reacquire the origin site's slot,
     in the same release-then-acquire way.
   - The slot is released as soon as the body is read (or the attempt fails). Decode,
     extraction and writing hold only the global slot. A job keeps the site slot of the URL it
     will retry during fetch-retry backoff (politeness).
   - Releasing a slot notifies the scheduler, so queued jobs for that site become eligible.
4. **Scheduler.**
   - The worker thread runs an async scheduler on its runtime. Commands arrive on the existing
     channel, bridged so the scheduler can await them, and jobs run as tasks in a `JoinSet`
     sharing one `Arc<ReqwestFetcher>`.
   - Dispatch picks the **earliest-enqueued** queued job whose origin site has a free slot
     while global slots remain. A busy site never blocks later jobs from other sites, and
     same-site jobs keep FIFO order. A job waiting for a redirect destination's slot keeps its
     global slot; the global cap (6) bounds how many can do so.
   - **Start notification at dispatch.** Before spawning a job's task, the scheduler sends
     `EngineEvent::Progress { stage: Downloading, bytes: None, tokens: None }`. The existing
     forwarding thread maps it to `Msg::JobProgress`, which marks the job started (it leaves
     `Stage::Queued` and enters `download_started_job_ids`). The notice is sent on the same
     event channel before any completion for that job, so it always arrives first. This also
     fixes today's single-download gap, where a request still waiting for headers was treated
     as unstarted.
   - `quota_tracker.check_url()` still runs at dispatch (job start). `record_job` runs when a
     task completes.
   - The marker write from the run-epoch rule in phase 4 happens before the epoch's first
     dispatch.
   - `JobTiming` gains `site_wait_ms`: time queued while only the site cap blocked the job.
     This satisfies FI-Networking-RequestScheduling-0001's "queueing caused by per-host caps is
     observable" criterion. The run summary adds max downloads in flight and the total site
     wait.
5. **Stop and Resume.**
   - `Stop{immediate:false}` cancels every queued job that was never dispatched, including
     those waiting for an origin slot, with `Cancelled`. **Every dispatched download, including
     one waiting for a redirect destination's slot, completes and is kept.** Enqueue is
     rejected until `Resume`.
   - `Stop{immediate:true}` (runner drop) cancels the token for all in-flight jobs.
   - `Resume` creates a fresh token for the next epoch, as today.
6. **Reducer reconciliation for late start notices** (`harvester_core`).
   - A start notice can be reduced after Stop when the engine dispatched the job before it
     processed the Stop command. At Stop, the reducer saw the job as `Queued` and preserved its
     URL into pending intake.
   - On the late notice, the reducer marks the job started, counts it in `Stopping{in_flight}`
     and keeps the run Stopping until its `JobDone`.
   - The preserved pending-intake URL is resolved by the existing rule at the next Full run:
     discarded after a success or a non-cancelled failure, kept after a cancellation. It is not
     removed eagerly, because the job may still end cancelled.
   - `apply_progress` must accept a `Downloading` notice for a job in `Queued` during Stopping
     (verify it does not ignore progress while Stopping, and adjust if it does).
7. **Retry backoff.** A job in fetch-retry backoff keeps its global slot and the site slot of the
   URL it will retry. Note this in the summary as a tuning point; no change.

Regression tests:

- Engine integration tests (`tests/engine_concurrency.rs`, using a counting `TcpListener` stub
  per site):
  - with 8 sites and 12 jobs, the max concurrency observed is at most 6 and at least 2;
  - **per-site cap:** 3 jobs on one site (two hostnames of one registrable domain are covered
    by a site-key unit test) never overlap, and later jobs on other sites start while the
    first site is busy;
  - FIFO among eligible jobs;
  - **redirects into one site:** two jobs on different origin sites both redirect to one
    destination stub. The destination never sees overlapping requests, and both jobs succeed;
  - **crossing redirects:** job A on site a redirects to site b while job B on site b
    redirects to site a. Both complete within a bounded timeout, with no deadlock;
  - a redirect hop waiting for a busy destination does not time out because of the wait, and
    its `site_wait_ms` is non-zero;
  - **start notice at dispatch:** with stubs that accept connections but withhold response
    headers, every dispatched job emits a `Downloading` progress event before any request
    bytes are answered;
  - **Stop with several downloads in flight:** after at least 3 are dispatched, `stop(false)`
    completes all dispatched jobs (`Ok`, or their own failure), cancels all queued ones, makes
    no new requests, rejects enqueue until `resume`, and lets the next run succeed with a fresh
    token;
  - `stop(true)` cancels in-flight jobs;
  - quota: with `max_urls_per_session = 2`, the third and fourth jobs fail `QuotaExceeded` at
    dispatch without a network request.
- Site-key unit tests: `www.x.com` and `news.x.com` share a key; `127.0.0.1:1` and
  `127.0.0.1:2` differ; unparseable URLs get their own key.
- **Determinism:** the byte-parity test from phase 5 runs with `global: 6` and with `global: 1`
  and yields identical filenames and bytes.
- **Boundary integration test** (`harvester_io` effect-runner tests, real `EffectRunner` built
  with `with_engine_config`, real reducer loop pumping `msg_rx` into `harvester_core::update`).
  Injected reducer messages cannot verify this boundary, so this test drives the real path:
  - enqueue 5 jobs on 5 stub sites that accept connections and withhold headers, and 2 more
    jobs that stay queued behind the global cap (lowered to 5 for the test);
  - once the stubs report 5 open requests, reduce `StopFinishClicked`;
  - after the start notices are reduced, `run_state()` is `Stopping { in_flight: n }` with
    `n >= 5`, and pending intake contains only the 2 never-dispatched URLs;
  - then release the stubs: 3 respond 200, and 2 respond 500 after their delay (requests that
    fail after starting);
  - the run stays Stopping until all 5 `JobDone` messages are reduced, then becomes Terminal
    and Idle;
  - the 2 failed jobs are failed, not preserved as unstarted;
  - a following Full run's pending-intake ingestion re-enqueues only the 2 cancelled URLs.
- Reducer (`harvester_core`):
  - a `Downloading` progress notice reduced **after** Stop, for a job preserved as queued,
    marks it started, raises `Stopping{in_flight}` by one and delays Terminal until its
    `JobDone`. At the next Full run the existing rule discards the preserved URL after
    success and keeps it after cancellation;
  - `JobDone` messages arriving out of job-id order leave `ordered_completed_job_urls_snapshot`
    and the `LoadArticlesForTriage` request in job-id order;
  - Stop with 3 downloads started and 2 queued gives `Stopping{in_flight: 3}` (plus model work),
    preserves only the 2 queued URLs into pending intake, and settles Terminal only after all 3
    `JobDone` messages;
  - blacklist strikes from two same-domain completions are recorded in arrival order without
    re-arming cooldown (existing rule).
- Existing `engine_stop.rs` passes: its single-site scenario still drains one in-flight job.
- Site-slot unit tests (registry alone): release-then-acquire never holds two permits; a
  release wakes the scheduler notifier; a cancelled waiter does not leak a permit.

Verification:

- Standard Rust gate for `harvester_engine`, `harvester_io`, `harvester_core`.
- `cargo run -p harvester_engine --example download_bench -- --global 6 --per-site 1`, compared
  with `--global 1`. Record both. The bench gains these flags in this phase.
- IPC probe: `cargo run -p harvester_ui -- --probe-ipc`.
- **External human testing recommended:**
  - a live desktop Full run, judged against the done criteria from the `[run-timing]` line;
  - a Stop pressed during downloads, to see `Stopping…` settle after the in-flight downloads.

Docs (Architecture.md):

- The Stop paragraph says "the engine cancels queued downloads while every in-flight download
  completes and is kept" and "keeps its in-flight fetches alive for Finish". It also says that
  a download counts as started when the engine dispatches it, before any response.
- "Content pipeline" describes the scheduler: global cap; per-registrable-domain cap applied
  to every request, including redirect hops, with at most one site slot held at a time; FIFO
  among eligible jobs; quota check at job start.

### Phase 7 - Per-download host costs (gated by measurement)

Goal: remove per-download overhead on the serialized reducer and host loop **only where the
`[run-timing]` evidence shows it matters**. Whether and which items proceed is decided at
the checkpoint after Phase 3 from the user's live run (a later run's summary may be used if
one exists). Each item is independent.

| Item | Change | Apply when the summary shows |
|------|--------|------------------------------|
| A. Double runtime snapshot | Request `PersistRuntimeState` on `FetchOutcomeClassified` only for `PermanentBlock`. A success is always followed by `JobDone{Success}`, which persists the same blacklist state. | Reducer time on `FetchOutcomeClassified` plus `JobDone` above 1 s per run, or above 5 % of download-phase wall time |
| B. 20 ms engine-event poll | Replace `try_recv` plus sleep with a blocking receive on the forwarding thread, together with the lifecycle change below | Maximum forwarding delay above 10 ms affecting more than a handful of events, or measurable idle CPU |
| C. `EnqueueUrl` DNS on the host loop | Move `validate_effect`'s DNS-resolving policy check off the effect hand-off path. The engine already enforces the policy at fetch time; keep the same failure message and job outcome. | `EnqueueUrl` hand-off above 2 s per run |

Item B lifecycle change. A blocking receive alone would leave the forwarding thread blocked
forever (see Verified facts), so B also does the following:

- **Separate receiver ownership.** `EngineHandle` splits off an `EngineEvents` receiver handle
  (for example `EngineHandle::take_events()`). The forwarding thread owns only that receiver
  and holds no command sender, so it never keeps the engine alive.
- **Explicit shutdown.** Add `EngineCommand::Shutdown` and `EngineHandle::shutdown()`. The
  engine worker cancels the current token, cancels queued jobs, waits (bounded, for example
  5 s) for in-flight tasks to observe cancellation, then exits its loop and drops its event
  sender. The forwarding thread's blocking `recv` then returns `Disconnected` and the thread
  exits.
- `EffectRunner`'s `Drop` calls `shutdown()` where it calls `stop(true)` today. It keeps the
  forwarding thread's `JoinHandle` and joins it with a bounded wait, logging through
  `engine_logging` if the wait expires.
- Ordinary `Stop` (finish or immediate) stays resumable and never ends the worker. Only
  `Shutdown` does.

Regression tests (for the items applied):

- A: one `PersistRuntimeState` per successful download; a `PermanentBlock` still persists the
  blacklist; restart restores blacklist strikes.
- B:
  - engine events reach the `Msg` channel in order with no polling delay;
  - dropping the `EffectRunner` terminates both the engine worker and the forwarding thread
    within the bounded wait, observed by joining their handles in a test hook;
  - an in-flight job at drop ends `Cancelled`;
  - `stop(false)` and `stop(true)` followed by `resume()` still accept and complete a new job
    (the existing `engine_stop.rs` scenario passes unchanged);
  - `Shutdown` followed by `enqueue` neither panics nor hangs.
- C: a policy-violating URL still yields the same failed job outcome and log context.

Verification: standard Rust gate for the touched crates. Run the IPC probe if A, B or C was
applied, since B changes desktop shutdown and A and C change desktop-visible timing.

### Phase 8 - Close-out documentation and final verification

- **Architecture.md:** confirm it describes, as current behaviour:
  - run timing observed at the host boundary;
  - the download scheduler, including the per-site cap on redirect hops;
  - downloads counted as started at dispatch;
  - Stop with several in-flight downloads;
  - the shared pooled client with one deadline per fetch attempt;
  - engine shutdown on runner drop, if item B was applied;
  - the marker refreshed once per run;
  - the desktop default of 10;
  - the effective cap;
  - the bounded rate-limit retry and halt semantics.
- **DecisionLog.md**: append two new entries. Do not edit earlier entries.
  1. "Downloads run in parallel under a per-site cap; Stop keeps every in-flight download". It
     refines 2026-09-27 "Stop halts new work and drains; export waits only for the run", whose
     singular "in-flight download" becomes plural. It also records:
     - the registrable-domain key, applied to every request including redirect destinations;
     - a download counting as started from dispatch;
     - in-process parallelism;
     - the marker refreshed once per run;
     - a marker failure no longer failing an article.
  2. "The desktop shares the command-line model budget and slows itself under rate limits"
     (user decision for 10; owner via orchestrator for the rest). It records the reducer-owned
     adaptive cap, the bounded Retry-After retry (Stop does not cut a wait short), the halt
     counting exhausted retries, and that the 2026-09-27 priority rule and once-per-stage
     admission are unchanged.
- **EngineeringDiary.md:** one Implementation entry per the "How to use" section. It records
  what landed and the lessons:
  - shared-client redirect counting and the per-attempt deadline;
  - blocking DNS in async code;
  - the marker race on Windows replace;
  - downloads that looked unstarted until their headers arrived;
  - redirects escaping a per-site cap keyed on the original URL.

  Include before and after numbers only if the user's live run supplied them. Add separate
  Bug Fix entries for any bug found along the way. The started-at-dispatch fix closes a gap
  that exists today with one download, so it gets its own Bug Fix entry.
- **FutureIdeas.md** status updates:
  - FI-Networking-RequestScheduling-0001 becomes Implemented;
  - FI-Performance-IoThroughput-0003 becomes Partially Implemented (downloads only; feed
    fetches and file writes are not pooled);
  - FI-Performance-LlmProcessing-0002 becomes Implemented;
  - FI-LLM-RetryPolicy-0001 becomes Partially Implemented (rate-limit retries honour
    Retry-After; there is no per-run retry budget);
  - FI-Performance-Polling-0008 stays Candidate, with a note that RSS parallel polling was
    evaluated and deferred (gain of 3 s or less, seen-set race) and that Brave stays sequential
    pending its rate limit.
- **CorpusFormat.md:** `written_at_utc` wording (landed in phase 4); no schema change.
- **openai_provider_kit/CHANGELOG.md:** 0.5.0 entry (landed in phase 1).
- **ThreatModel.md:** no change expected (phase 5 review).
- **VisualDesignSpec.md, IPC schema, launch scripts:** unchanged.

Verification:

- Full workspace `cargo test` in a quiet window granted by "Project orchestration";
  `cargo clippy --all-targets -- -D warnings`; `cargo fmt`.
- If any code changed after the last per-phase IPC probe, run `cargo run -p harvester_ui --
  --probe-ipc`.
- **The wall-time target is confirmed only by the user's next live desktop run.** Report it as
  "implemented and keylessly verified; wall time pending live run" until the user's
  `[run-timing]` line shows the result.

## Documentation obligations (summary)

| Document | Change | Phase |
|----------|--------|-------|
| docs/Architecture.md | Run timing; budget default 10, effective cap, rate-limit retry and halt; pooled client, explicit redirects and one deadline per attempt; download scheduler with redirect-aware site slots; downloads started at dispatch; Stop with several in-flight downloads; engine shutdown on runner drop (if 7B applied) | 1, 2, 3, 5, 6, 7, 8 |
| docs/DecisionLog.md | Two new entries (parallel downloads and Stop; desktop budget and adaptive cap) | 8 |
| docs/EngineeringDiary.md | Implementation entry; Bug Fix entries if any | 8 |
| docs/FutureIdeas.md | Status updates listed in phase 8 | 8 |
| docs/CorpusFormat.md | `written_at_utc` refresh wording; no schema bump | 4 |
| crates/openai_provider_kit/CHANGELOG.md + Cargo.toml | 0.5.0, headroom accessor | 1 |
| docs/ThreatModel.md | Review only | 5 |

There is no change to `CORPUS_SCHEMA_VERSION`, `harvester-corpus.json` layout, the archive
export format, desktop IPC, the frontend or launch scripts.

## Risks

- **Provider rate limits at 10 concurrent requests.** Mitigated by the bounded retry and the
  adaptive cap. The residual risk is a lower-tier account that is slower than expected; the
  summary line shows cap changes. `LLM_MAX_CONCURRENT_REQUESTS` remains the manual override.
- **Stop drain gets longer.** A request waiting for a rate-limit retry can hold Stop for up to
  about 60 s in the worst case; that is accepted to keep the 2026-09-27 Stop rule. Up to 6
  downloads (each attempt bounded by one 15 s deadline across redirects and body, plus
  backoff) must finish. A download waiting for a redirect destination's slot adds at most the
  current holder's remaining deadline. Drain time is roughly the maximum across in-flight work,
  not the sum.
- **Redirect parity.** Explicit redirect following could differ from reqwest in edge cases
  (Referer, relative Location, 303, deadline). Covered by fetch tests; watch the live run's
  failure mix.
- **Sites throttling parallel clients.** The per-site cap of 1 applies to every request,
  including redirect hops, so no site sees more than one concurrent request from Harvester,
  as today. Only cross-site concurrency is new. The cost is that a redirect into a busy site
  waits (visible as `site_wait_ms`).
- **Start-notice ordering.** A job dispatched just before the engine processes Stop is reduced
  as started only after Stop. The reconciliation rule and the boundary integration test cover
  this; until the notice arrives, the desktop's `Stopping` count can briefly be lower.
- **Burstier `JobDone` arrival** can postpone pre-triage refresh up to its 80-tick maximum
  (about 6 s) because quiet ticks become rarer. The summary's first-triage offset shows it.
- **Reducer per-download costs** (snapshot capture twice per job, cloning about 12,300
  records) are paid on the serialized reducer. Gated by the phase 7 measurement.
- **Same-file writes.** Article filenames include the original URL's hash; the same URL is
  never in flight twice (reducer URL dedup plus the per-site cap). A short-hash collision
  between two different URLs with the same title is negligible.
- **Engine shutdown (if item 7B is applied).** A bounded shutdown wait could cut short an
  in-flight job at desktop close. That job was already cancelled by `stop(true)` today, so
  the outcome does not change.
- **Timing-sensitive tests.** Concurrency tests assert bounds (at most 6, at least 2, no
  same-site overlap), not durations. Benchmarks report, they do not gate.
- **Unverifiable headline.** No keyless path proves the wall-time target (see Goal).

## Open Questions

None. The three earlier questions (the phase 7 gating, Stop versus rate-limit waits, and
wave-size scope) were settled by the orchestrating assistant and are recorded under
"Settled by the orchestrating assistant", open to challenge.
