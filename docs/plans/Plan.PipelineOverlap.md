# Plan: one Run, overlapping pipeline stages

Written 2026-09-23 from the design brief for a one-press desktop Run with overlapping stages.
Revised the same day after the Codex plan review (eight issues, all applied) and the user's
answers to the first draft's twelve open questions, which are now stated as decisions below.
Status: not started. Nothing below is built.

Terms:

- A **run** is one accepted pipeline execution, from the press (or the batch cycle start) until it
  has settled or finished stopping.
- The **window** is the set of harvested articles whose `fetched_utc` is at or after the archive
  checkpoint (all articles when no checkpoint is set) and whose URL belongs to a completed job.
- An article's **identity** is its URL plus its `content_hash`.
- A stage's **current key** is the cache identity that stage would use now: content hash, prompt
  version, model and context hash, plus the upstream result digests for scoring.
- The three **model stages** are triage, summary and signal scoring. **Intake** is poll, download
  and article preparation.
- A **wave** is a group of article identities released to one stage together.
- Work is **admitted** when a run places it in a stage queue. **Unadmitted** unfinished work
  exists only in the unfinished-work summary, never in a queue.

This plan is ephemeral. Durable documents (Architecture, DecisionLog, EngineeringDiary, code
comments) must describe behaviors by name and never cite this plan's phase numbers.

## Goal

One press on the desktop app runs poll, download, triage, summaries and signal scoring
unattended. Stages overlap: downloaded articles move on to triage in waves while other downloads
are still running, and triaged articles move on to summaries while triage continues. The batch
host gets the same overlap through the same reducer logic.

Done means:

- a desktop Run completes unattended and settles exactly once;
- stages overlap in waves on both hosts, and batch cycles, including `--batch-api` deferred mode
  and `--drain`, still settle and terminate correctly;
- hand-offs between stages never re-read or re-prepare articles whose preparation is still valid;
- all model work shares one request budget in a deterministic, tested priority order;
- a secondary action processes unfinished work without fetching and is enabled only when such
  work exists;
- Stop halts all new work, lets in-flight work finish and keeps its results;
- export is unavailable only while a run is working or draining, never because of unfinished
  articles;
- all Rust and frontend checks pass, and reducer tests cover the new behavior and emitted effects.

## Settled decisions

### From the design brief

1. **Wave-based release**, not per-article streaming and not a mere button merge. The model
   concurrency cap is the real ceiling once stages overlap.
2. **No spending gate.** A Run commits to paid model work, as batch does. Quota warnings are the
   safety net.
3. **The UI has one primary action.** Run is the primary button and does everything. The
   triage-and-summaries action remains as a secondary action, enabled only when a previous run
   left unfinished work. Follow `docs/visual_design/VisualDesignSpec.md`.
4. **No measurement or benchmark phase.** Timings depend on external websites.
5. **Corpus re-reading is fixed first**, then waves are added.
6. **Unfinished work is judged by identity.** It is every window article that lacks a complete
   result under the current key: new arrivals, prior failures, and work invalidated by a prompt
   or model change. The secondary action applies the same rule without fetching, and batch
   repeated cycles apply it too. Completeness is identity-based, never a count comparison.
7. **Furthest-along work goes first** in the shared request budget and the call quota, so a
   quota-cut run leaves complete articles rather than fragments. The order is deterministic and
   testable.
8. **Stop halts everything new.** No new downloads start and no new model requests are issued.
   In-flight work finishes and its results are kept, and the remainder is unfinished work.
9. **Exports are disabled while a run is busy** and re-enabled when it finishes or has stopped.
   Completion-time checkpoints were rejected because they would change the public corpus format.

### From the user's answers (2026-09-23)

10. **Stage priority is scoring, then summaries, then triage.** Scoring is the last step of an
    article, so "furthest along first" puts it first. The brief's wording "triage before new
    scoring" was an error; the principle is what the user chose.
11. **A missing signal score is unfinished work** for a scoring-eligible article (triage priority
    at or above the scoring cutoff, with a current-key summary). It is retried automatically and
    counted in the leftover count.
12. **Wave timing uses the defaults in Design section 4**, as implementation defaults.
13. **The large-reprocess warning is a notice only**, never a confirmation step. Its thresholds
    are implementation defaults.
14. **The stage list renders overlapping rows** as described in Design section 9.
15. **Batch API mode keeps one hand-off to triage per intake cycle.** Collect-only cycles only
    continue work that was already submitted, and do not re-admit the window.
16. **The Poll Sources button is removed.** Run replaces it. The user's original request was for
    Poll Sources itself to run triage and summaries.
17. **Run is available again after Stop** without restarting. The download engine accepts new
    work on the next run.
18. **Batch single-shot does not require new articles.** Unfinished work is retried whether or not
    anything new was downloaded.
19. **Model calls happen only inside a run.** Automatic startup scoring stops; unscored eligible
    work appears as unfinished.
20. **Batch synchronous concurrency matches what the worker enforces.** `--llm-concurrency`
    defaults to 10 with a maximum of 10. The Batch API buffering allowance is a separate setting
    and is not a concurrency limit.
21. **The corpus scan index is in memory only.** No new state file.
22. **Export never waits for unfinished articles.** It is blocked only while a run is actively
    working, including while a Stop drains in-flight requests. Once the run has settled or
    finished stopping, export is available even if some triage, summaries or scores failed.
    Failed articles stay on the leftover list and are retried on the next run. There is no retry
    cap for now; a cap is a possible later addition.

## Verified facts (checked against the source on 2026-09-23)

Intake and hand-offs:

- `PreTriageRefreshCoordinator::maybe_dispatch` (`crates/harvester_core/src/pre_triage_coordinator.rs`)
  blocks a refresh during a poll burst until `poll_sources_ended` and no engine job is in flight,
  unless `MAX_WAIT_TICKS` (80 ticks, about 6 s) has passed since the first unserved demand. Batch
  therefore already overlaps a little.
- Every refresh emits `Effect::LoadArticlesForTriage`, serviced by `run_triage_refresh_load`
  (`crates/harvester_io/src/effect_runner/worker.rs`) through
  `load_and_prepare_articles_filtered_with_progress` (`crates/harvester_engine/src/briefing.rs`).
  That call:
  - lists every `.md` file in `output/`;
  - reads each file in full;
  - derives clean text for every file inside the `since` window;
  - truncates it to a summary budget of `max_input_bytes` minus the active summary template's
    overhead (`prepare_loaded_articles_and_collection`);
  - builds an aggregate-briefing collection text that the triage path discards.

  The live corpus is 10,450 files, 699 MB. `content_hash` comes from the untruncated clean text,
  so a template change alters the valid preparation without altering identity.
- `schedule_pre_triage_refresh` sets pre-triage to `new_loading()` when demand is recorded, and
  `handle_articles_loaded` replaces it wholesale with `PreTriageSession::load_articles`.
- `start_triage_from_pretriage` (`crates/harvester_core/src/update/triage.rs`) consumes pre-triage
  and replaces the triage session (`TriageSession::new_loading(None)`, then `set_articles`).
- `AppState::batch_next_action` (`crates/harvester_core/src/state/batch.rs`) dispatches:
  - triage, only when `triage.can_start()` and `triage.total() < pre_triage_included`;
  - summaries, only when triage is Complete and `briefing.articles()` is empty.
- `handle_prepare_summaries_clicked` (`crates/harvester_core/src/update/briefing.rs`) emits
  `LoadArticlesForBriefing`, which rescans the corpus. The triage session already holds the same
  `prepared_text` (same function, same budget). `begin_briefing_article_load` re-emits
  `LoadPromptContexts`, `LoadPromptTemplateFiles` and `LoadLlmMetadata` on every start.
- The engine worker (`crates/harvester_engine/src/engine.rs`) runs one download at a time inside
  `runtime.block_on` and reads commands only between jobs. A Stop therefore reaches it after the
  current download has finished. It then drains the queue as `Cancelled`, sets
  `accept_new = false`, and cancels the parent `CancellationToken`, so every later job is refused
  or cancelled for the rest of the process.

Model concurrency, quota and dispatch:

- `DEFAULT_LLM_MAX_CONCURRENT_REQUESTS` is 3 (`crates/harvester_io/src/host_bootstrap.rs`, env
  `LLM_MAX_CONCURRENT_REQUESTS`). The reducer's `triage_max_in_flight` and `summary_max_in_flight`
  are each set to that value.
- The LLM worker (`crates/harvester_engine/src/llm/handle.rs`) clamps concurrency with a literal
  `clamp(1, 10)` and acquires a semaphore permit synchronously inside its command loop, in command
  order. Excess requests therefore queue FIFO, with Stop and usage queries behind them.
- `harvester_batch --llm-concurrency` defaults to 12, with a maximum of 12
  (`DEFAULT_LLM_CONCURRENCY` and `MAX_LLM_CONCURRENCY` in `crates/harvester_batch/src/cli.rs`),
  which exceeds the worker's cap of 10. The launch scripts (`scripts/lib/HarvesterLaunch.psm1`,
  `scripts/Start-HarvesterBatch.ps1`) pass no concurrency value; the CLI default applies.
- `--batch-api` sets both limits to the session call limit (`set_deferred_batch_max_in_flight`). That
  is a buffering allowance for requests diverted into provider batches, not a concurrency limit.
- `signal_candidate::try_enqueue` emits `RequestLlmCompletion` immediately with no in-flight limit.
  Its snapshot (`build_input_snapshot`) takes the summary from `summary_result_for_url` and the
  upstream digest from `summary_cache_key_for_url`, both of which accept summaries made under any
  prompt or model. Triage cache hits already call `try_enqueue`, possibly before any
  current-key summary exists.
- `SignalCandidateSession::enqueue` refuses a URL that has any state, Completed or Failed
  included, so a stale or failed score is never replaced within the process.
- The session call quota is 1,000 calls per process (`crates/harvester_engine/src/llm/quota.rs`).
  Warning and danger levels are 70 % and 90 % (`llm_quota_view.rs`). On `QuotaExhausted`, triage
  and summaries each fail their own remaining pending articles (`update/llm_completed.rs`).
- `sweep_eligible_after_hydration` enqueues and dispatches scoring at startup, outside any run.

Run lifecycle and settlement:

- `PipelineRunPhase` (Idle, Requested, Dispatched, AwaitingSettle, Stopping) is advanced by
  `Msg::PipelineRunAdvance`, sent by the desktop driver while the phase is not Idle
  (`crates/harvester_ui_bridge/src/driver.rs`).
- `pipeline_activity()` (`crates/harvester_core/src/state/run_progress.rs`) is the settlement query
  for both hosts (DecisionLog 2026-09-07). It counts pending and in-flight work in all three stages,
  and never counts deferred work.
- `RunProgress::stop` marks the run terminal immediately. Triaging and Summarizing finish when
  their session phase turns terminal; ScoringSignals finishes only in `settle_run`.
- The batch runner sends `PollSourcesClicked`, then loops `maybe_dispatch_batch_ai_orchestration`
  (which emits `TriageClicked` and `PrepareSummariesClicked`) until `batch_status()` is Settled.
  Import mode uses the same helper. Drain never enables orchestration.
- The batch dashboard's `classify_display_phase` (`crates/harvester_batch/src/progress/projection.rs`)
  shows one first-match phase, with Intake taking precedence.

Batch API:

- Collect-only cycles run `collect_and_rearm_batch_cycle`: `BatchResultsCollected` writes caches
  only, then `RearmDeferredBatchStages` turns Deferred entries back to Pending and re-dispatches
  (cache hits replay).
- `flush` (`crates/harvester_batch/src/batch_coordinator.rs`) answers `DeferredToBatch` for any
  `custom_id` already pending in `.batch_manifest.ron`, so re-requested work cannot be
  double-submitted.

Stop and export:

- `Msg::StopFinishClicked` sets `SessionState::Finishing` for the rest of the process, and
  `handle_poll_sources_clicked` then refuses polls. Model work keeps self-dispatching after Stop.
- The desktop Archive button is gated only on a snapshot existing. `handle_archive_clicked` and
  `handle_dialog_submitted` have no run guard. Cancelling the dialog is a host no-op.
- The archive corpus requires the triage session phase to be Complete (`select_for_archive`).

## Design

### 1. Incremental hand-off: prepare each article once per preparation budget

**Scan index (IO side).** `harvester_engine` gains a `CorpusScanIndex`: a map from article file
path to a fingerprint (length and modification time) and the small metadata a scan recovers: URL,
title, `fetched_utc`, `content_hash`, and whether the file is an article at all. A scan lists the
directory, re-reads only new or changed files, and drops entries for deleted files. It holds no
article text. `EffectRunner` owns one index for the life of the process and resets it when the
imported corpus is cleared. It is never persisted (decision 21).

**Preparation budget.** The truncation budget (`max_input_bytes` minus the active summary
template's overhead) is part of preparation validity. The loader reports the budget it applied.
The reducer stores it beside each prepared article.

**Delta loads (effect contract).** `Effect::LoadArticlesForTriage` carries the reducer's held
entries as (URL, `content_hash`, preparation budget). The loader returns:

- the full ordered membership of the window (identity, title, `fetched_utc`);
- the budget it applies now;
- prepared `LoadedArticle`s only for members whose identity is not held, or whose held budget
  differs from the current one.

A template change between runs therefore re-prepares every held article on the next load, while
their identities, and so their cache keys, stay the same. The loader stops building the
collection text on this path. Order stays filename order.

**Reducer merge.** `TriageArticlesLoaded` applies the delta to the pre-triage session:

- new identities are evaluated and appended;
- changed identities and re-budgeted articles replace their entries;
- departed identities are removed;
- existing entries keep their verdicts.

Refresh demand no longer resets pre-triage to Loading. It is reported as `intake_refresh_pending`
(section 5).

**Summaries from memory.** The summary stage takes its text from the triage session's prepared
articles instead of `LoadArticlesForBriefing`. That effect remains only for the aggregate-briefing
domain path, which needs the collection text.

**Configuration is frozen per run.** Prompt contexts, template overlays and LLM metadata load once
when a run starts, before its first load and its first model dispatch. Nothing reloads them
mid-run, so the prompt registry, and with it the preparation budget and every cache key, stays
fixed for the run. Edits apply from the next run.

### 2. Completeness and unfinished work

A pure classifier decides, per window member and per stage, whether the stage needs work under
the current key. `AppState::unfinished_work()` aggregates it. Each member lands in one of these
classes:

- **Not eligible**: hard-excluded by pre-triage, or triaged below the summary cutoff (for summary
  and scoring), or below the scoring cutoff (for scoring).
- **In progress**: admitted and pending, in flight, or deferred to the Batch API.
- **Needs triage**: no triage cache hit under the current key. This covers never triaged, failed,
  and triaged under an older prompt, model or context.
- **Needs summary**: a current-key triage result above the cutoff exists, and there is no summary
  cache hit under the current key.
- **Needs scoring**: a current-key triage result at or above the scoring cutoff and a current-key
  summary both exist, and there is no signal-candidate cache hit under the scoring key built from
  those current-key upstream results (decision 11).
- **Complete**: everything that applies is present under current keys.

Unadmitted work counts only here, never in a queue or in `pipeline_activity()`. Until metadata is
loaded the classifier reports "unknown", and the secondary action stays disabled with a reason.

The aggregate is a stored summary: counts per class, a count of articles with at least one
needed stage, and an upper-bound call estimate (3 per needs-triage article, 2 per needs-summary,
1 per needs-scoring). It is recomputed only when the window, a cache, the metadata or a session
changes, never inside `view()`. That keeps the desktop host-drain budget of 40 ms
(`crates/harvester_ui_bridge/tests/host_drain_cost.rs`).

**Export is independent of this summary** (decision 22).

**Large reprocess notice.** When a run admits its initial window, it counts the unfinished
articles that were already in the window before the run. The notice is non-blocking (decision
13): it goes to the run surface and to `engine_logging` with the run id and counts, and batch
prints one line.

### 3. One model-request budget, furthest-along first

There is one reducer-owned **synchronous request budget**, `llm_max_in_flight`. It replaces
`triage_max_in_flight` and `summary_max_in_flight`, and is clamped by one cap constant exported
from `harvester_engine`, which the LLM worker also uses instead of its literal `10`:

- desktop: `LLM_MAX_CONCURRENT_REQUESTS`, default 3;
- ordinary batch: `--llm-concurrency`, default 10, maximum 10 (decision 20).

In `--batch-api`, requests diverted into provider batches do not occupy worker permits. That mode
therefore uses a separate **deferred buffering allowance**, `llm_deferred_allowance`, set to the
session call limit as today. The allowance bounds how many requests may be outstanding for
buffering. Requests the runner still sends synchronously are bounded by the worker's own
semaphore. The two settings are named, set and logged separately (`[model-budget]`), so neither
can be mistaken for the other.

One scheduler, `dispatch_model_work`, is the only emitter of article-stage
`RequestLlmCompletion`s. After any message that can free a slot or admit work, it fills free
slots in the fixed order **scoring, then summary, then triage** (decision 10). The order is a
constant slice.

- **Ordering within a stage.** Admission order, which Phase 4 refines to wave number, then
  position in the wave. For a single wave the two orders are identical, so Phase 2's tests stay
  valid.
- **Cache hits** never take a slot. They complete in place, and the scheduler keeps filling.
- **Scoring is queue-only.** It is admitted only when its current-key upstream results exist; see
  section 4.
- **Quota exhaustion or repeated rate limiting** stops the scheduler for the rest of the process.
  It also fails every admitted pending entry across the three stages with the provider reason.
  Those entries count as unfinished.

### 4. Waves, admission and run scopes

**Accumulating sessions.** The triage, summary and scoring sessions accumulate entries for the
life of the process. Each entry records the key it was completed under. A member that leaves the
window is removed. Session phases are derived from admitted contents only:

- Triaging while anything is pending or in flight;
- AwaitingBatch while only deferred work remains;
- Complete when at least one result exists and nothing is pending;
- Failed when nothing succeeded.

Because unadmitted work is never pending, a settled or stopped run leaves the derived phase
Complete, and the archive corpus stays available (decision 22).

**Admission transitions.** When a run admits a member, the classifier's verdict decides each
stage entry independently:

| Stage entry before admission | Classifier says | Entry after admission |
|---|---|---|
| none, or Failed | needs the stage | Pending |
| Completed under a stale key | needs the stage | Pending (the stale result is dropped from the session; the cache keeps it) |
| Completed under the current key | complete | unchanged, and no request |
| Pending, in flight or Deferred | in progress | unchanged |
| any | not eligible | unchanged, and no request |

Valid results in one stage are retained while another stage is requeued. For example, a
prompt-invalidated summary is requeued while its current-key triage stays. Within a run, a
member is admitted to each stage at most once, so a failure during a run waits for the next run.
There is no cross-run retry cap (decision 22).

**Scoring depends on current upstream results.** `try_enqueue` and the scoring input snapshot are
changed to require the triage result under the current triage key and the summary under the
current summary key: the summary session's current-key entry, or `try_reuse_summary` with the
current key. They never read `summary_result_for_url` or `summary_cache_key_for_url`. The scoring
entry stores the digest of its input key. When an upstream result changes, the classifier
reports "needs scoring" and admission replaces the old scoring entry, Completed or Failed alike.
A triage cache hit therefore no longer triggers scoring against a historical summary.

**Wave ledger.** `PipelineWaves` is reducer-owned and lives for the process, not the run. Per
stage, it records released waves and their members. Per member, it records a downstream release
marker keyed by identity plus the upstream key. It gives the scheduler its order and makes every
release exactly-once, including across cycles.

**Release rules** (implementation defaults, decision 12):

- **Intake to triage.** During an armed run, the refresh coordinator no longer waits for the poll
  burst to end. It dispatches an incremental load after `QUIET_TICKS_AFTER_POLL` (about 1.2 s
  after the last completed download) or `MAX_WAIT_TICKS` (about 6 s after the first unserved
  download), and once more when downloads settle. Each load's newly admitted eligible members
  form one triage wave.
- **Size cap.** Any admission larger than 4 × the synchronous budget (12 at the desktop default)
  is split into consecutive waves of that size, in window order. In `--batch-api` the cap
  derives from the deferred allowance, so it never splits.
- **Triage to summary.** When a triage wave has no pending or in-flight member left, every
  completed, summary-eligible member without a release marker is released as one summary wave,
  in the same reducer step. Deferred members do not block the wave. They are released later
  (next item).
- **Replay waves.** `RearmDeferredBatchStages` groups the rearmed members of each stage into
  replay waves, tagged with their original wave. When a replay wave settles, its completed
  members without a release marker are released downstream. With partial collection across
  cycles, each collected member is released exactly once, in the cycle where its result replays.
  A member replayed again (duplicate collection, or a second rearm) finds its marker and is not
  released twice.
- **Summary to scoring.** A completed current-key summary admits scoring for its article when
  eligible, marked per identity and upstream key. This per-article step is existing behavior, now
  bounded by the budget and by current keys; it is not new streaming.

**Arming.** The scheduler dispatches only while a run is armed for model work (decision 19). A
run is armed only when AI is available; otherwise a Full run is intake-only.

- `sweep_eligible_after_hydration` no longer enqueues. Startup-eligible scoring shows only as
  unadmitted "needs scoring" and never enters `pipeline_activity()`. A drain with
  startup-eligible scoring therefore settles.
- `RearmDeferredBatchStages` and `BatchResultsCollected` outside an armed run only update entries
  and caches; nothing is dispatched.

**Run scopes.** `Msg::PipelineRunRequested` carries a scope:

- `Full`: poll, download, admit the unfinished window, then waves. Used by the desktop Run button
  and every batch intake cycle.
- `Resume`: admit the unfinished window from a fresh incremental load, with no fetching. Used by
  the desktop secondary action and import mode.
- `Continue`: arm the run without admitting anything. Only work already admitted and rearmed
  flows, plus its downstream releases. Used by `--batch-api` collect-only cycles (decision 15).

A request that arrives while a compatible run is active joins it, as a pipeline request already
joins an active poll run today.

**Legacy entry points during migration.** Until the batch host migrates in Phase 5,
`TriageClicked` and `PrepareSummariesClicked` start an armed `Resume` run if none is active, then
behave as before. `batch_next_action` stays as a shim over the new sessions until then. This
keeps the old batch orchestration dispatching in Phase 4. Phase 5 deletes the shim, the legacy
arming and `BatchNextAction`.

### 5. Settlement and progress under overlap

`pipeline_activity()` stays the single settlement query for both hosts. It counts admitted work
only, and gains `intake_refresh_pending`: the coordinator has unserved demand or a load in flight
while the run's intake is open. Triage-to-summary and replay releases happen in the same reducer
step as the settlement that triggers them, so they need no term of their own. Deferred work
remains non-blocking.

A run's lifecycle is Active, then optionally Stopping, then Terminal. It turns Terminal when its
intake is closed and `pipeline_activity()` is settled, via `settle_run` after normal completion,
or when a Stopping run finishes draining (section 6).

`RunProgress` under overlap:

- Several stages may be Active at once.
- ScanningSources and DownloadingArticles keep their finish rules.
- LoadingArticles counts articles admitted to this run and stays Active until intake closes.
- Triaging and Summarizing count this run's admitted members (cache hits count as done). Their
  totals grow wave by wave, and they turn Done only when the run turns Terminal.
- `StageProgress` gains `total_is_final: bool`, false while an upstream stage can still add work.
  The frontend shows no ETA while it is false.
- The view gains `run_state`: `Idle`, `Active`, or `Stopping { in_flight }`.
- Wave releases are logged as `[pipeline-wave] run_id=… stage=… wave=… released=… replay=…`. They
  are not pushed to the bounded activity feed, whose capacity stays 50 (DecisionLog 2026-09-08).

### 6. Stop halts everything new and drains the rest

An accepted Stop moves the run to **Stopping**:

- **Downloads.** Queued downloads are cancelled. The in-flight download is not cancelled: the
  engine finishes it, and its article is written and recorded as usual. Its result is admitted to
  the window but released to no stage, so it appears as unfinished. `Effect::StopFinish` with
  the Finish policy tells the engine to drop its queue and stop accepting jobs for this run. It
  no longer cancels the parent token, and the engine accepts jobs again when the next run starts
  (decision 17), using a fresh cancellation token.
- **Model work.** The scheduler is disarmed, so no new request is issued. Every admitted Pending
  entry, never dispatched, is withdrawn from the queues without being marked failed. In-flight
  model requests finish; their results are applied, cached and persisted. Completions during
  Stopping release nothing downstream.
- **Progress.** Stopping freezes admissions and releases, not the counters: completions that
  drain still count, and the view shows `Stopping { in_flight }`.
- **Finish.** When the in-flight download and in-flight model requests have drained, the run turns
  Terminal. Stages are terminalized, and the session returns to Idle instead of the permanent
  `Finishing`.

Withdrawn and never-started work is what the classifier reports, so the secondary action and the
next Run pick it up.

Drain time is bounded by the existing fetch and LLM request timeouts. Batch Ctrl-C shutdown is a
separate path and is unchanged.

### 7. Export availability

`export_available()` is true exactly when no run is Active or Stopping (decisions 9 and 22). It
never looks at unfinished work, failed articles or the classifier.

- `ArchiveClicked` while unavailable does nothing and logs `[archive-gate]`.
- `ArchiveDialogSubmitted` while unavailable is rejected with a visible checkpoint status message
  ("Export is unavailable while a run is in progress"). The dialog may have been opened before the
  run started, and cancel is invisible to the reducer.
- The snapshot carries `archive_enabled`. The Archive button and the dialog's submit button follow
  it.

### 8. Batch host and Batch API deferred mode

- An intake cycle sends `PipelineRunRequested { scope: Full }` and pumps `PipelineRunAdvance` each
  loop iteration, as the desktop driver does. It settles on the same `batch_status()` (that is,
  `pipeline_activity()`). Every cycle processes unfinished window work, new articles or not
  (decision 18).
- `--batch-api`:
  - One intake wave per cycle: the intake release waits for downloads to settle.
  - Collect-only cycles reduce `PipelineRunRequested { scope: Continue }` before
    `RearmDeferredBatchStages`. Replay waves then release collected judgments downstream exactly
    once.
  - Manifest dedup still guards against double submission.
  - Budget: the deferred buffering allowance, not the synchronous budget.
- `--drain` never requests a run. Nothing is armed, so a drain issues no model request of its own.
  Startup-eligible scoring is unadmitted, so the drain settles and exits.
- Import mode requests a `Resume` run after its import completes, replacing
  `maybe_dispatch_batch_ai_orchestration`.
- The single-shot gate `require_new_jobs_since` is removed (decision 18).
- The dashboard keeps one phase label with first-match precedence. Its per-stage rows already show
  concurrent progress. The precedence is documented, not changed.

### 9. Desktop run surface

- **Run** is the primary button (Accent Primary). It sends a Full run and is enabled whenever no
  run is Active or Stopping.
- **Process unfinished (N)** is the secondary button. It sends a new payload-free intent,
  `UiIntent::ResumeUnfinishedWork`, mapped to a Resume run. It is enabled only when no run is
  Active or Stopping, the unfinished summary is known and nonzero, and AI is available. The count
  and tooltip come from core.
- **Stop** keeps its destructive styling and separation, and reads "Stopping…" (disabled) while
  the run drains.
- **Poll Sources** is removed from the surface, and `UiIntent::PollSources` is removed from the
  vocabulary (decision 16).
- **Stage rows** (decision 14):
  - several rows may show "In progress" at once;
  - a stage whose `total_is_final` is false and that has nothing in hand shows "Waiting for
    articles";
  - no ETA is shown while `total_is_final` is false.
- **Notices.** The reprocess notice renders as a muted run-surface notice, not a modal.
- **Archive** follows `archive_enabled`.
- Every enablement and count is core-owned; the frontend renders flags and never derives them.
- **IPC contract.** The IPC schema version increases and fixtures are regenerated. New
  reducer-generated fixtures pin:
  - overlapping active stages;
  - unfinished work available;
  - Stopping with in-flight work;
  - stopped with unfinished work and export enabled;
  - export unavailable during a run;
  - the reprocess notice.

## Phases

Every Rust phase runs from the repository root, `C:\Users\larsp\src\web_page_filet_mignon`:
`cargo build`, the listed `cargo test -p …` commands, then
`cargo clippy --all-targets -- -D warnings` and `cargo fmt`. If a batch is running, build with
`cargo build --workspace --exclude harvester_batch`.

Before each phase, record the test count per touched crate. A phase that removes a test must name
the replacing test in its EngineeringDiary entry; test files are never dropped wholesale.

Agents do not run the Harvester launchers or live model calls. "Human testing" items are for the
user, through the normal launchers.

### Phase 1 — Incremental corpus hand-off

Scope: `harvester_engine`, `harvester_io`, `harvester_core`. Stage order is unchanged.

1. Add `CorpusScanIndex` and an index-backed window scan in `harvester_engine`. Split
   `prepare_loaded_articles_and_collection` so the triage path prepares without the collection
   text and returns the applied budget.
2. Own one index in `EffectRunner`. Service `LoadArticlesForTriage` as a delta against the held
   (identity, budget) entries. Reset the index on imported-corpus clear. Log
   `[corpus-index] request_id=… files=… reused=… read=… reprepared=… removed=… budget=… elapsed_ms=…`.
3. Change `Effect::LoadArticlesForTriage` and `Msg::TriageArticlesLoaded` to the delta contract.
   Merge deltas into pre-triage and store the budget per article. Stop the reset to Loading, and
   add `intake_refresh_pending` to `pipeline_activity()` in the same step. Add a full-window delta
   test helper so existing tests migrate rather than being deleted.
4. Feed summaries from the triage session. Keep `LoadArticlesForBriefing` for the
   aggregate-briefing path only. Load contexts, template overlays and metadata once per start of
   triage or summaries (the pre-run equivalent of the per-run freeze that Phase 4 formalizes),
   and never on each summary start.

Tests:

- Engine: an unchanged fingerprint is never re-read. Overwrite the file with unparseable bytes,
  restore its length and modification time with `File::set_modified`, and check the scan still
  succeeds (the diary's 2026-08-21 technique). Also cover:
  - new, changed and deleted files;
  - the `since` filter using the cached `fetched_utc`;
  - archive artifacts excluded;
  - filename order.
- Engine contract: triage-path and summary-path preparation give identical `prepared_text` and
  `content_hash`.
- `harvester_io`:
  - a second load returns prepared text only for the new file;
  - after a template change that alters the summary overhead, a load re-prepares every held
    article at the new budget, with unchanged `content_hash` and text within the new bound.
- Reducer:
  - a delta appends, replaces changed and re-budgeted entries, keeps verdicts and removes
    departed members;
  - pending or in-flight refresh keeps `pipeline_activity()` unsettled;
  - summaries start with no `LoadArticlesForBriefing` effect.

Verify: `cargo test -p harvester_engine -p harvester_io -p harvester_core -p harvester_batch
-p harvester_ui_bridge`, then clippy and fmt.

Human testing (recommended): poll and then run triage and summaries in the desktop app. In
`engine.log`, after the first load, `[corpus-index]` should report `read` equal to the new
arrivals only.

Docs: Architecture (process-lifetime scan index, delta loads with preparation budget, summaries
fed from the triage session); EngineeringDiary entry.

### Phase 2 — One model-request budget, furthest-along first, current-key scoring

Scope: `harvester_engine` (cap constant), `harvester_core`, `harvester_io` (bootstrap),
`harvester_batch` (CLI, bootstrap, dry run, import mode).

1. Export the worker cap constant and use it in `handle.rs`. Replace the two per-stage limits with
   `llm_max_in_flight`. Add `llm_deferred_allowance` for `--batch-api`, set and logged separately.
2. Set `--llm-concurrency` to default 10 and maximum 10, and update its `cli.rs` tests. The launch
   scripts pass no concurrency value, which Phase 2 re-checks. If that still holds, no launcher
   code changes, but run
   `Invoke-Pester scripts/tests/HarvesterLaunch.Tests.ps1` to confirm the launch contract; the
   coordinator runs Pester itself, since implementing sandboxes may not. If a launcher does pass
   the value, update it and its Pester tests.
3. Add `update/model_dispatch.rs` with `dispatch_model_work` and route all stage dispatch through
   it. In this phase there is no run arming. The scheduler dispatches whenever admitted work is
   pending, exactly as today's code would dispatch, and orders within a stage by admission order.
4. Make scoring queue-only and dependent on current-key upstream results. Store the input-key
   digest on the scoring entry, and let a changed upstream replace a Completed or Failed scoring
   entry.
5. Unify the quota and rate-limit halt across stages.

Tests (reducer, emitted effects):

- Budget 3 with pending work in all stages emits scoring, then summaries, then triage, and never
  more than 3 in flight across stages.
- A completion's freed slot goes to the highest-priority pending item.
- Within a stage, admission order holds.
- Cache hits take no slot.
- A triage cache hit whose only summary is under an old key does not enqueue scoring. Once the
  current-key summary completes, scoring is enqueued with that summary's digest.
- A changed summary replaces a Completed scoring entry.
- `QuotaExhausted` fails pending work in all three stages and nothing further is emitted.
- The batch CLI clamps to 10, and `--batch-api` uses the allowance, not the synchronous budget.

Verify: `cargo test -p harvester_engine -p harvester_core -p harvester_io -p harvester_batch
-p harvester_ui_bridge`, then clippy and fmt, and the Pester check above.

Human testing: none required.

Docs: Architecture (the synchronous budget, the deferred buffering allowance, the priority order,
current-key scoring inputs); EngineeringDiary entry.

### Phase 3 — Identity-based completeness

Scope: `harvester_core`. A pure classifier and a stored summary, with no UI.

1. Add the classifier and `unfinished_work()` from section 2, including "needs scoring". Resolve
   keys with the same builders dispatch uses. Report "unknown" before metadata is loaded.
2. Store the summary. Recompute it on window deltas, cache hydration or writes, metadata loads
   and session changes.
3. Add the reprocess-notice evaluation. Its defaults are named constants: more than 150
   previously-in-window articles needing work, or an estimate above 50 % of the remaining session
   quota.

Tests:

- A complete article stays complete across a reload.
- A prompt, model or context change makes it "needs triage".
- A failed triage is "needs triage".
- A summary under an old key is "needs summary".
- A current-key triage and summary with no current-key score is "needs scoring", and a
  below-cutoff article is not.
- Hard-excluded and below-cutoff articles are "not eligible".
- A deferred article is "in progress".
- Missing metadata gives "unknown".
- The notice fires just above each default and not at it.
- `host_drain_cost` still meets 40 ms with a production-scale window.

Verify: `cargo test -p harvester_core -p harvester_ui_bridge`, then clippy and fmt.

Human testing: none.

Docs: Architecture (completeness is identity-based under current keys); EngineeringDiary entry.

### Phase 4 — Waves, admission and run scopes in the reducer

Scope: `harvester_core`, `harvester_ui_bridge` (fixtures), `frontend/` (one new field only). The
desktop gains overlap through its existing buttons. Batch keeps its old orchestration through
legacy arming.

1. Make the sessions accumulate, record completion keys, and derive phases from admitted
   contents. Implement the admission transitions in section 4.
2. Add the process-scoped `PipelineWaves` with release markers, the release rules (quiet window,
   max-wait, size cap of 4 × budget, triage-wave release) and replay waves on rearm. Remove the
   poll-burst barrier for armed runs only.
3. Add scopes (Full, Resume, Continue) and per-run arming, and freeze configuration per run.
   Map the existing `UiIntent::RunPipeline` to Resume, and let it upgrade an active poll run to
   Full. Legacy `TriageClicked` and `PrepareSummariesClicked` start an armed Resume run when none
   is active, and `batch_next_action` stays as a shim.
4. Stop the startup sweep from enqueuing, so startup scoring becomes unadmitted "needs scoring".
5. Update `pipeline_activity()`, `settle_run` and `RunProgress` per section 5, including
   `total_is_final`. Record the reprocess notice at initial admission. Add `[pipeline-wave]`
   logging.

Tests (reducer walks in `update/pipeline_run/tests.rs` and `tests/triage_orchestration.rs`):

- Staggered downloads (three bursts) release three triage waves during download, and wave-1
  summaries dispatch before wave 3 is admitted.
- A 30-article Resume splits into waves of 12, 12 and 6.
- Admission across two runs:
  - a triage failed in run 1 is Pending and requested in run 2;
  - after a prompt-version change, run 2 requeues triage and summary but not other articles;
  - a current-key result is untouched and requests nothing;
  - a stale summary is requeued while its current triage stays.
- A triage wave with one member deferred releases its completed members immediately. After rearm
  and replay, the deferred member is released in a replay wave. A second replay of the same
  member releases nothing.
- The run never settles while downloads, a refresh or admitted work is pending, and settles
  exactly once. The GUI/batch parity test is updated, not removed.
- Totals grow monotonically, `total_is_final` flips when intake closes, and every stage is
  terminal at Terminal.
- After hydration with eligible scoring, no request is emitted, `pipeline_activity()` is settled,
  and `unfinished_work()` reports "needs scoring".
- Legacy `TriageClicked` with no active run arms a run and dispatches, so batch runner tests pass
  unchanged on the shim.
- Configuration loads happen once per run.

Verify: `cargo test -p harvester_core -p harvester_io -p harvester_batch -p harvester_ui_bridge`,
then clippy and fmt. Regenerate fixtures with
`$env:UPDATE_UI_FIXTURES=1; cargo test -p harvester_ui_bridge; Remove-Item Env:UPDATE_UI_FIXTURES`.
Bump `IPC_SCHEMA_VERSION` and `frontend/src/ipc/schemaVersion.ts` together, add
`total_is_final` to `frontend/src/ipc/types.ts`, then from `frontend/`: `npm run check`,
`npm run build`, `npm run fmt`. Run `cargo clippy -p harvester_ui --all-targets -- -D warnings`.

Human testing (recommended): in the desktop app, press Poll Sources and then Run triage +
summaries while downloads are running. Triaging and Summarizing should become active before
downloading finishes, and the run should finish on its own with one notice. Then run a second
recurring batch cycle and confirm it still settles (legacy arming).

Docs: Architecture, "Reducer-owned pipeline lifecycle" rewritten (waves, replay waves, admission,
scopes, arming, settlement terms); EngineeringDiary entry. The first proposed DecisionLog entry
below is appended after Phase 5 lands.

### Phase 5 — Batch host and import mode on the reducer-owned run

Scope: `harvester_batch`, `harvester_core` (delete the shim, legacy arming, `batch_next_action`
and `BatchNextAction`).

1. Intake cycles send `PipelineRunRequested { scope: Full }` and pump `PipelineRunAdvance`.
   Delete `maybe_dispatch_batch_ai_orchestration`.
2. `--batch-api`: one intake wave per cycle through a host-set wave policy. Collect-only cycles
   reduce `PipelineRunRequested { scope: Continue }` before `RearmDeferredBatchStages`.
3. `--drain`: request no run.
4. Import mode: request a Resume run after import completion.
5. Remove `require_new_jobs_since`.
6. Print the reprocess notice and the unfinished count at cycle start, in both the interactive and
   non-interactive output paths.

Tests (`crates/harvester_batch/src/runner/tests.rs`, fake LLM and fake batch transport):

- A recurring cycle with staggered downloads dispatches triage before the last download finishes,
  and settles once.
- A second cycle with no new jobs still retries a failed article, and a single-shot cycle with no
  new jobs processes unfinished work.
- `--batch-api` across three cycles:
  - intake buffers one submission after downloads settle;
  - cycle 2 collects half the triage lines, replays them and releases exactly those summaries;
  - cycle 3 collects the rest and releases the remainder;
  - a line collected twice releases nothing twice;
  - re-requested manifest work is answered `DeferredToBatch` and never uploaded twice;
  - each cycle settles with deferred work outstanding.
- `--drain` with restored jobs and startup-eligible scoring issues no `RequestLlmCompletion` of
  its own, and the loop returns and the process exits (termination is asserted, not only the
  absence of requests).
- Import mode triages and summarizes imported articles.
- Dashboard phase classification under overlapping observation keeps its documented precedence.

Verify: `cargo test -p harvester_batch -p harvester_core -p harvester_io`, then clippy and fmt.

Human testing (required, operator-run under the usual launch policy):

- one recurring cycle;
- one `--batch-api` run through collection;
- one `--drain`;
- one `--single-shot`.

Watch for overlapping download and triage, one settlement per cycle, unchanged manifest behavior,
and a drain that exits.

Docs: Architecture ("Batch API automation path": Continue runs, one intake wave, replay waves;
drain never arms); EngineeringDiary entry; append the DecisionLog entry "Pipeline stages overlap
in waves under one request budget".

### Phase 6 — Stop drains; the engine resumes; export waits only for the run

Scope: `harvester_engine`, `harvester_io`, `harvester_core`, `harvester_ui_bridge` (fixtures).

1. Engine: Stop with the Finish policy drops the queue and refuses jobs for the current run. It
   no longer cancels the parent token, and the in-flight job completes. Accepting jobs resumes on
   the next run's first enqueue, or on an explicit resume command, with a fresh token.
2. Reducer:
   - Stopping per section 6: disarm, withdraw admitted pending entries, close intake, keep
     in-flight results, release nothing during Stopping;
   - turn Terminal when downloads and model requests have drained;
   - return the session to Idle.
3. `export_available()` is false while Active or Stopping. Guard `ArchiveClicked` and
   `ArchiveDialogSubmitted`. Add `archive_enabled` and `run_state` to the snapshot.
4. Log `[run-stop] run_id=… withdrawn_triage=… withdrawn_summary=… withdrawn_scoring=…
   in_flight_llm=… in_flight_download=…`, the Terminal transition, and `[archive-gate]` refusals.

Tests:

- Engine:
  - a Stop during a job lets that job complete successfully and cancels only queued jobs;
  - jobs enqueued for the next run after a Stop are processed.
- Reducer:
  - Stop during overlapping triage and summaries emits `StopFinish` and no further
    `RequestLlmCompletion` while completions drain;
  - drained completions are cached;
  - the run is Stopping until the last in-flight completion, then Terminal;
  - withdrawn articles appear in `unfinished_work()`.
- Export:
  - an archive submit while Active and while Stopping is refused with the message and no
    `ArchiveRequested`;
  - after Terminal it succeeds, including when some summaries failed and unfinished work is
    nonzero (decision 22).
- A Full run after a Stop polls and downloads again.

Verify: `cargo test -p harvester_engine -p harvester_io -p harvester_core -p harvester_ui_bridge`
with fixture regeneration as in Phase 4, then clippy and fmt, plus
`cargo clippy -p harvester_ui --all-targets -- -D warnings`.

Human testing: covered in Phase 7.

Docs: Architecture (Stop drains, session and engine resume, export availability);
EngineeringDiary entry; append the DecisionLog entry "Stop halts new work and drains; export waits
only for the run".

### Phase 7 — Desktop one-press Run surface

Scope: `harvester_core` (intent and view fields), `harvester_ui_bridge` (fixtures, schema),
`frontend/`.

1. Add `UiIntent::ResumeUnfinishedWork`. Map `UiIntent::RunPipeline` to a Full run. Remove
   `UiIntent::PollSources`, and remove `Msg::PollSourcesClicked` if no test or host still needs it.
2. View fields: `run_enabled`, `resume_enabled`, `unfinished_work` (counts, known or unknown) and
   `reprocess_notice`. Remove `triage_can_start`, `summaries_can_start` and `poll_sources_enabled`
   from the desktop payload if nothing renders them.
3. Frontend:
   - `RunSurface.tsx`: primary Run, secondary "Process unfinished (N)", Stop and "Stopping…", the
     notice, and multi-active stage rows with "Waiting for articles" and no ETA while
     `total_is_final` is false;
   - `App.tsx`: the Archive button and modal submit follow `archive_enabled`;
   - follow the spec's button hierarchy (one primary per context, Stop separated and
     destructive).
4. Bump `IPC_SCHEMA_VERSION` and `schemaVersion.ts`, regenerate fixtures, and add the named
   fixtures listed in section 9.

Tests:

- Intent decoding round-trips, and a removed `PollSources` payload fails closed.
- Reducer enablement across Idle, Active, Stopping, Terminal-after-stop and AI unavailable. Resume
  is enabled only when unfinished is known and nonzero.
- Frontend component tests render each new fixture: primary and secondary enablement, "Stopping…",
  multiple active rows, no ETA while totals are open, the notice, and Archive disabled while
  Active or Stopping but enabled with nonzero unfinished work after the run. `App.test.tsx` is
  updated.

Verify, from the root: `cargo build`, `cargo test -p harvester_core -p harvester_ui_bridge`,
clippy, `cargo clippy -p harvester_ui --all-targets -- -D warnings`, fmt. From `frontend/`:
`npm run check`, `npm run build`, `npm run fmt`. Then the keyless IPC probe from the root,
`cargo run -p harvester_ui -- --probe-ipc`, which needs a display and the GUI lock, takes about
150 s, writes `.local/probe/ipc-report.json`, and exits non-zero on a failed gate. Report it as
not run if no display is available.

Human testing (required):

- one full desktop Run from idle to the notice;
- a Stop mid-overlap: Stop reads "Stopping…" while in-flight work finishes, Archive stays disabled
  until it has finished, then "Process unfinished" picks up the remainder, and Run works again
  without a restart;
- an export after a run with a failed summary: Archive is enabled, and the failed article is
  counted as unfinished;
- a context-file edit followed by a Run, to see the reprocess notice.

Docs: Architecture (desktop diagram: Run and Process unfinished intents replace Poll Sources and
Run Pipeline); VisualDesignSpec if the "Waiting for articles" and "Stopping…" states are judged
new patterns; EngineeringDiary entry; append the DecisionLog entry "The desktop Run is one primary
action".

### Phase 8 — Closing verification and project memory

1. Full keyless suite from the root: `cargo build`, `cargo test`,
   `cargo clippy --all-targets -- -D warnings`, `cargo clippy -p harvester_ui --all-targets -- -D warnings`,
   `cargo fmt`. From `frontend/`: `npm run check`, `npm run build`, `npm run fmt`. The IPC probe.
   `Invoke-Pester scripts/tests` if any launcher file changed.
2. Operator re-verification of batch modes: recurring, `--batch-api` through collection,
   `--drain`, `--single-shot`, and import mode.
3. Documentation sweep: Architecture consistent with the landed behavior; the three DecisionLog
   entries appended; confirm that `docs/CorpusFormat.md`, `CORPUS_SCHEMA_VERSION`,
   `docs/ArchiveExportFormat.md` and `docs/ThreatModel.md` needed no change, and say so in the
   final report.

## Documentation and project memory

| Document | Change | When |
|---|---|---|
| `docs/Architecture.md` | Scan index and delta loads; the synchronous budget and deferred allowance; current-key scoring; the lifecycle section (waves, replay waves, admission, scopes, arming, settlement); the Batch API automation path; Stop, drain and export availability; the desktop diagram | Phases 1, 2, 4, 5, 6, 7 |
| `docs/DecisionLog.md` (append-only) | Three entries, below | After Phases 5, 6 and 7 land |
| `docs/EngineeringDiary.md` | One entry per landed phase, per its "How to use" section | Every phase |
| `docs/visual_design/VisualDesignSpec.md` | Only if the new run-surface states are judged new patterns | Phase 7 |
| `docs/CorpusFormat.md`, `CORPUS_SCHEMA_VERSION`, `harvester-corpus.json` | No change: article layout is untouched, and the index is in memory | — |
| `docs/ArchiveExportFormat.md` | No change | — |
| Launch scripts and Pester tests | No change expected: the launch scripts pass no concurrency value. Pester confirms the launch contract | Phase 2 |

DecisionLog entries (append when the behavior lands; do not cite plan phases):

- **Pipeline stages overlap in waves under one request budget.**
  - Decision: A run releases articles to the next stage in waves while earlier stages continue.
    All article model work shares one synchronous request budget, dispatched scoring, then
    summary, then triage. Batch API buffering has its own allowance. Completeness is judged per
    identity under current cache keys, and scoring requires current-key upstream results. Model
    requests are issued only inside a run.
  - Context: The concurrency cap, not stage order, bounds throughput once stages overlap, and
    per-article streaming would dissolve the stage concept.
  - Consequences: Settlement counts intake-refresh demand and admitted work only. Stage sessions
    and the wave ledger live for the process. Deferred results release downstream exactly once
    through replay waves. Configuration is frozen per run. `batch_next_action` no longer exists.
    Batch synchronous concurrency is capped at the worker's limit.
- **Stop halts new work and drains; export waits only for the run.**
  - Decision: Stop cancels queued downloads and issues no new model request. The in-flight
    download and in-flight model requests finish and are kept. Never-started work becomes
    unfinished. The next run may start without a restart. Export is unavailable while a run is
    working or draining, and never because articles are unfinished or failed.
  - Context: An unattended run must be stoppable without losing paid results, and a checkpoint
    moved mid-run would split the window. Completion-time checkpoints were rejected because they
    would change the public corpus format.
  - Consequences: Failed articles are retried on the next run, with no retry cap for now.
- **The desktop Run is one primary action.**
  - Decision: Run performs poll through scoring, and the Poll Sources action is gone. A secondary
    action processes unfinished window work without fetching and is enabled only when such work
    exists.
  - Consequences: The desktop intent vocabulary loses `PollSources` and gains
    `ResumeUnfinishedWork`.

## Risks

- **Settlement regressions.** Early settlement, a run that never settles, or a drain that never
  exits would affect both hosts. Mitigations:
  - admitted-only counting and `intake_refresh_pending`;
  - releases in the same reducer step;
  - the GUI/batch parity test and termination-asserting drain tests.
- **Test churn** from the delta contract and the session shapes. Mitigations: the full-window delta
  helper, baseline test counts, and no wholesale deletions.
- **Memory with no checkpoint.** The accumulating sessions hold prepared text for the whole corpus,
  as today's sessions already do. The index adds only metadata.
- **Host-drain budget.** The unfinished summary is never recomputed in `view()`, and Phase 3
  re-measures at production scale.
- **Mass reprocess** after a prompt or context change can exhaust the 1,000-call quota. The notice,
  the priority order and the quota halt keep the result coherent.
- **Batch API shape.** One intake wave, Continue runs and manifest dedup keep one submission per
  intake and exactly-once downstream release.
- **Stop latency.** Draining waits at most for the current download's fetch timeout and the
  in-flight model requests' timeouts. The UI shows "Stopping…" meanwhile.
- **Fingerprint staleness.** A same-length rewrite within one modification-time tick would be
  missed. NTFS resolution is 100 ns, and articles are written once.
- **Behavior changes users will notice:** no automatic startup scoring; context and template edits
  apply from the next run; Stop no longer ends the session; Poll Sources is gone; batch
  single-shot works on unfinished articles even with no new downloads.

## Implementation defaults (tunable; not prerequisites)

All questions from the first draft are settled (decisions 10–22). The values below are starting
points that an implementer may tune with a recorded reason, in the phase that introduces them.
None blocks starting work.

- **Wave timing:** a quiet window of `QUIET_TICKS_AFTER_POLL` (about 1.2 s), a maximum wait of
  `MAX_WAIT_TICKS` (about 6 s), and a size cap of 4 × the synchronous budget.
- **Reprocess notice thresholds:** 150 previously-in-window articles needing work, or 50 % of the
  remaining session quota by estimated calls.
- **Stage-row wording:** "Waiting for articles" and "Stopping…".
- **Possible later addition:** a cross-run retry cap for articles that keep failing. Not planned
  now (decision 22).
