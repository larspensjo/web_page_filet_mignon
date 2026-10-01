# Plan: Harvester simplification

Written 2026-09-28 from the design brief agreed with the owner that day (three read-only
audits plus a blindspot pass). The brief's decisions are settled and are not re-opened here;
this plan turns them into phases, grounds each phase in the code, and names the checks,
documents and decision-log entries each phase needs. Revised the same day after a Codex plan
review: crash-safe result-store and runtime-state migrations, a reconciliation of Batch API
results before removal, a saved-results index that also covers Last 24h, summary-body export
through that index, and a cross-phase carry-over fixture. The owner's answers to the open
questions are folded in. Revised again after a second review
(`docs/plans/Plan.Simplification.Review.2026-09-28.md`): pinned temporary-file names, carry-over
assertions scoped to paid results, a visible store refusal, the pending-intake field, the
estimate test and the view-state save cadence. Phase numbers are local to this plan:
durable documents, code comments and decision-log entries name behaviours, never phases.

## For the owner: what changes and where to stop

Harvester keeps doing the same job: each morning it collects AI-company news, downloads the
articles, asks OpenAI to triage, summarise and score them, and lets you read the results and
export `archive.md`. The desktop app looks and behaves the same. What changes:

- **Paid results are never thrown away again.** Today the summary store would start deleting
  old paid summaries in about three weeks, and a crash mid-run loses every result of that run.
  After the second phase, every result is saved within a few seconds of arriving and nothing
  is deleted. One visible change comes with this: if a saved-results file is ever damaged,
  Harvester no longer quietly starts over with an empty file (which would overwrite what you
  paid for). It says which file failed and keeps the AI steps off until you restore the file
  from a backup or move it aside.
- **The desktop remembers everything after a restart.** Priorities, summaries, the Results
  list, the archive meter, the list tab you had open and the article you were reading are all
  there when you reopen the app, without pressing Run. Export works straight after a restart.
- **The morning command-line run gets faster and simpler.** The Batch API is removed (about
  $5 more per month, as agreed), so the run no longer waits in five-minute provider cycles.
  The runtime-state file shrinks from 78 MB, so saving no longer takes 10 seconds per
  download. The busy console dashboard becomes a short per-stage progress block.
- **Unused features go:** aggregate briefing, trends, stale-summary refresh, dry-run, the
  repeating batch mode, linked-page download, the indirect-link pool, the old concatenated
  export and the never-built Script source type. Browser-page import, checkpoint editing on
  the command line, "open a link found in the article", the signal-candidate exclusion toggle
  and replay records stay.
- **Several small bugs are fixed:** articles a feed offered beyond the per-poll limit are no
  longer silently skipped forever, polls that finish after Stop no longer lose their articles,
  the desktop and command-line runs can no longer write to the same folder at the same time,
  and an empty API key is treated as "no AI" everywhere.

**Natural stopping point.** Phases 1 to 9 are the cleanup stage. Each is useful on its own
and leaves a working app. After Phase 9 the plan stops at an explicit checkpoint with a short
report (code size, test counts, speed measurements), and you decide whether to go on. The
later phases replace the internal machinery that schedules the AI work with a simpler
per-article pipeline. They make the code much smaller and easier to change (for example for
the planned extra "standing-lens" AI step), but they are not needed for speed: the cleanup
stage is expected to remove most of the morning run's wall time.

Your actions during the work are few: confirm before the Batch API code goes (Phase 4), try
the app or the morning run where a phase says "human testing recommended", and decide at the
checkpoint. Old files that Harvester stops using are listed for you to delete or archive; the
plan never deletes your data.

## Goal and definition of done

The same user-visible product (brief section 3) on a much smaller codebase: the reducer-owned
wave, admission and session orchestration replaced by a straightforward per-article pipeline;
the Batch API and dormant features gone; the bugs in brief section 5 fixed with regression
tests; documentation accurate; all standing checks green. The cleanup stage alone must deliver
the restart-state requirement, export from saved results, paid-result safety, and the measured
speed-ups.

## Settled inputs this plan follows

- Brief sections 2 and 9 (owner decisions, including the blindspot pass). In particular:
  policy stays in the pure reducer and every effect is one unit of I/O (brief 9.1, which
  supersedes the brief's earlier "pipeline on the effect side" sentence); export and view
  after a restart use saved results under the current keys only (9.2); every paid result is
  kept and saved in small batches (9.3); a keyless replay benchmark comes first and a go/no-go
  checkpoint follows the cleanup phases (9.4); screen behaviours are restated per article and
  look the same (9.5); the exclusion toggle stays and `.sources.ron` keeps parsing (9.6).
- `Agents.md`: input -> action -> reducer -> state -> render; pure reducers; `engine_logging`
  with job, URL or operation context; launch scripts change only with launch policy; corpus
  layout changes update `docs/CorpusFormat.md`, `CORPUS_SCHEMA_VERSION` and the marker; UI
  follows `docs/visual_design/VisualDesignSpec.md`; keyless verification only.
- Decision log entries that stay as behaviour: 2026-09-04 (restricted `UiIntent`, local assets,
  rfd only for pre-window lock refusal), 2026-09-06 (job-list bounds and cap), 2026-09-07 (run
  progress accumulated in the reducer), 2026-09-08 (50-entry feed; summary-only reading pane),
  2026-09-09 (runs never navigate), 2026-09-11 (channel-2 fixtures), 2026-09-19 (archive
  contract), 2026-09-27 (Stop drains; one primary Run action). Entries this plan refines or
  supersedes are listed in "Decision-log entries" below.
- `docs/plans/Plan.ArchiveExportContract.md` Phase 3: 3b is independent and may land at any
  time; 3c, 3d and 3e wait for the new pipeline (see "Sequencing with the archive contract").
- Owner answers after the plan review (2026-09-28), settled:
  - **No dual-write after the result-store switch.** From Phase 2 on only the new
    append-friendly files are written; the old RON cache files stay untouched as a backup.
    Returning to a pre-switch build is an emergency-only step via git; results made since the
    switch could be converted back by hand or by a small tool if ever needed. That tool is not
    part of this plan.
  - **Jobs whose fetch time cannot be recovered stay hidden with the note.** Restored jobs with
    no article file keep no fetch time, stay hidden from the desktop list, keep blocking
    re-download of their URL, and are never deleted. The "N jobs are hidden because their fetch
    time is missing" note stays and shows the remaining count. Fetch times are still recovered
    from existing article files wherever possible (investigated in Phase 1, done in Phase 7).
  - **`archive.md`, and everything that predicts it, keeps today's summary rule.** Today the
    places that resolve summaries for the archive use the newest saved summary for the
    article's content hash under any prompt, model or context key
    (`lookup_any_by_content_hash`):
    - the exported summary body (`build_summary_map`, `update/archive.rs:330`);
    - the dialog's summary-mode token estimate (`archive_token_estimates`,
      `state/batch.rs:280`), and the header meter's estimate, which uses the same any-key
      fallback through the view's summary lookup (`view_builder.rs:323-372`);
    - `summary_result_for_url` (`state/signal_candidate_access.rs:154-167`): live summary
      session first, then the any-key cache. Its callers are at `signal_candidate_access.rs:243`
      and `job_access.rs:152` and `230`. The `:243` and `:152` callers go with the aggregate
      briefing (Phase 5) and the preview pipeline (Phase 6); any caller that remains keeps the
      same semantics.

    All of these keep their semantics, so `archive.md` stays byte-identical to a post-run
    export today and the estimates keep predicting it. The only change is where the content
    hash comes from: the saved-results index instead of the live sessions. Current-key-only
    results (brief 9.2) apply to triage and priority, selection, annotations and coverage
    counters. The reading pane shows only current-key summaries: today it reads only the live
    summary session (`view_builder.rs:746-748`), which holds current-key results.

## Verified facts (grounding)

Each claim was checked in the code on 2026-09-28 unless marked "to verify", in which case the
named phase verifies it first.

**Bugs from the brief**

- Bug 1 (`--dry-run` marks entries seen): `crates/harvester_batch/src/runner/dry_run.rs`,
  `crates/harvester_engine/src/rss_seen_set.rs:39-53`,
  `crates/harvester_core/src/update/polling.rs:52-59`. Resolved by removal (Phase 4).
- Bug 2 (excess entries marked seen): verified for RSS and Brave. `filter_unseen_entries` marks
  every entry seen (`rss_seen_set.rs:39-53`) before `poll_rss_source` applies `take(limit)`
  (`crates/harvester_engine/src/source_poll.rs:131-144`); the production caller is
  `crates/harvester_io/src/effect_helpers.rs:104`, so the `#[allow(dead_code)]` on
  `poll_rss_source` is stale. Brave does the same: `BraveSeenSet::filter_unseen` marks all
  (`crates/harvester_engine/src/brave_seen_set.rs:56-65`) before `take(limit)`
  (`effect_helpers.rs:687-692`). The existing test `poll_rss_source_applies_max_after_dedup`
  (`source_poll.rs:293`) passes while the fourth entry is lost.
- Bug 3 (polls after Stop): verified. `handle_source_poll_completed` drops URLs when intake is
  closed, with a comment claiming the next run will collect them (`polling.rs:52-59`), but the
  seen-sets were already saved by the poll thread (`effect_helpers.rs:122-129`, `694-701`).
  To verify in Phase 3: whether downloads cancelled by Stop before they started are
  re-enqueued by the next run and survive a restart (only completed jobs are persisted,
  `crates/harvester_io/src/persistence.rs:9-37`), which would be the same class of loss.
- Bug 4 (results saved late): verified. Triage persists only when the session settles
  (`crates/harvester_core/src/update/triage.rs:220-241`); summaries only in
  `settle_summaries` and aggregate completion (`update/briefing.rs:285-333`,
  `update/llm_completed.rs:423-456`); the signal cache is cloned and rewritten after every
  completion (`update/signal_candidate.rs:502-521`). Every persist effect carries a full clone
  of its cache (`crates/harvester_core/src/effect.rs:119-130`). Additional hazard, verified for
  all three stores: a cache file that fails to parse ends up as an empty cache, and the next
  save overwrites the paid results with it. The summary store returns an empty cache on parse
  failure (`crates/harvester_io/src/summary_cache_store.rs:64-92`, pinned by the test
  `load_corrupt_file_returns_empty_cache` at `:230`). The triage store does the same, pinned
  by `corrupt_file_returns_empty_and_warns` (`triage_cache_store.rs:227`). The signal store's
  `load` returns an error (`signal_candidate_cache_store.rs:70-86`); `host_bootstrap.rs:259-268`
  logs it and continues without hydrating, so the next full-clone save replaces the file. An
  unknown signal-cache version is also discarded as empty (`:77-84`). Found by the Phase 1
  benchmark: each persist-cache effect writes its full clone on its own freshly spawned thread
  (`crates/harvester_io/src/effect_runner/dispatch.rs`, `PersistSignalCandidateCache` and
  siblings), with nothing ordering the writes. The baseline run issued 36 concurrent rewrites of
  the 9.5 MB signal store, so an older clone can finish last and briefly replace newer paid
  results until the next save. Phase 2's single ordered result sink removes this path.
- Bug 5 (two lock files): verified. `.harvester_gui.lock`
  (`crates/harvester_io/src/run_lock.rs:18-23`) and `.harvester_batch.lock`
  (`crates/harvester_batch/src/runner.rs:28-33`).
- Bug 6 (export needs a live session): verified. `handle_archive_clicked` uses
  `state.archive_corpus()` (`update/archive.rs:16`), which is
  `CurrentWorkingCorpus::select_for_archive` over the live triage session
  (`state/batch.rs:162-164`) and is Unavailable unless triage is Complete
  (`working_corpus.rs:639-657`). Annotations and the priority snapshot also read only the live
  sessions (`update/archive.rs:229-265`).
- Bug 7 (empty API key on desktop): verified. The desktop checks only `is_err()`
  (`crates/harvester_ui/src/host.rs:783-788`) and `OpenAiProvider::from_env` accepts an empty
  string (`crates/openai_provider_kit/src/openai.rs:23-28`); the batch host treats empty as
  unavailable (`runner.rs:35-38`).
- Bug 8 (iteration cap): verified. `MAX_ITERATIONS = 10_000` with a 100 ms receive timeout in
  `crates/harvester_batch/src/runner/dispatch_loop.rs:214-240` and the same pattern in
  `crates/harvester_batch/src/import_mode.rs:230-247`.
- Bug 9 (performance): verified. Whole-state clone per message in the batch loop
  (`dispatch_loop.rs:269`, `304`) and import loop (`import_mode.rs:262`, `313`). Every
  successful download emits `PersistRuntimeState` (`update/mod.rs:55-65`, `659-663`) whose
  snapshot copies all completed jobs with their links (`effect.rs:19-24`). The desktop driver
  rebuilds and deep-compares the full view after every message batch (`crates/harvester_ui_bridge/src/driver.rs:233-240`)
  on a 75 ms tick (`host.rs:806-810`), and every view build walks the entire summary cache
  (`state/view_builder.rs:323-356`).

**Restart state and export**

- The reading pane reads `self.briefing.summary_for_url(url)` (`view_builder.rs:746-748`) and
  row priorities read the live triage session (`view_builder.rs:269-276`), so both are empty
  after a restart. The archive meter instead uses the cache-derived index
  (`state/batch.rs:166-248`) built from current-metadata lookups
  (`TriageCache::lookup_current_priority_parts`, `triage_cache.rs:165-181`). The current-key
  lookup the restart fix needs therefore already exists and works at startup, and
  `TriageCache::lookup` returns the stored key for provenance (`triage_cache.rs:125-141`).
- The exported summary bodies come from `build_summary_map` (`update/archive.rs:321-336`), which
  takes the content hash from the live triage session and then calls
  `lookup_any_by_content_hash` (newest summary under any key). After a restart with no run the
  live session holds no hashes, so no summary bodies are exported.
- Batch API collection writes the collected record to `.batch_manifest.ron` before the cache
  effects are applied, and keeps it there until the caches confirm it
  (`runner/batch_runtime.rs:351-395`, `remove_collected_with_persisted_cache_confirmation`;
  test `collected_snapshot_replays_after_restart_until_cache_confirmation` in
  `batch_coordinator.rs:1769`). A batch in state `Collected` can therefore still hold the only
  copy of a paid result.
- "22 jobs are hidden because their fetch time is missing": the count covers every job without
  a fetch time, regardless of the list scope (`view_builder.rs:392-402`), and a persisted job's
  `fetched_utc` is optional with a serde default (`persistence.rs:16-17`). Likely cause: jobs
  persisted before fetch times were recorded. To verify in Phase 1; fixed in Phase 7.

**Inputs and contracts**

- `.sources.ron` is parsed as one strict `SourceRegistry`; any parse or validation error yields
  an empty registry with a warning (`crates/harvester_io/src/source_loader.rs:10-39`). An
  unknown source type today therefore silently disables every source, which is worse than the
  brief assumed. `SourceType::Script` is at `crates/harvester_engine/src/source_config.rs:84-90`
  and is polled as a failure at `crates/harvester_io/src/effect_runner/poll.rs:115-126`.
- The summary store maps persisted `AggregateBriefing` prompt ids (`summary_cache_store.rs:97-109`);
  after the aggregate briefing is removed such entries must be skipped, not fail the load.
- Launch policy: `scripts/lib/HarvesterLaunch.psm1:12` passes `--single-shot --batch-api`;
  pinned by `scripts/tests/HarvesterLaunch.Tests.ps1:149`, `266`, `352`.
- IPC: `IPC_SCHEMA_VERSION = 12` (`crates/harvester_ui_bridge/src/ipc.rs:4`) and
  `frontend/src/ipc/schemaVersion.ts`. The probe gates synthetic snapshots pushed at
  `PROBE_RATE_HZ = 20` (`crates/harvester_ui_bridge/src/probe.rs:18-24`, `385-393`); it does
  not depend on the 75 ms tick. `crates/harvester_ui_bridge/tests/host_drain_cost.rs` holds
  view-cost timing tests that track state shape.
- Batch API manifest states: `Created`, `Submitted`, `Collected`, `Failed`
  (`crates/harvester_batch/src/batch_manifest.rs:47-53`).
- `MockLlmProvider` (`crates/openai_provider_kit/src/test_support.rs:14-80`) answers from a
  FIFO queue regardless of the request. With concurrent triage, summary and scoring calls the
  order is nondeterministic, so the benchmark needs a small routing provider that answers by
  prompt (Phase 1).
- LLM output is validated in the worker and again in the reducer
  (`crates/harvester_engine/src/llm/handle.rs:1025-1033`; `update/llm_completed.rs:164`,
  `297`; `update/signal_candidate.rs:70`) and a third time for Batch API collection
  (`runner/batch_runtime.rs:244-251`).
- Frontend checks: `npm run check` runs `tsc`, `biome check` and `vitest`
  (`frontend/package.json:7-9`).

## Target design

### One saved-results store per result kind, one implementation (Phase 2)

- The three result caches share one generic in-memory store and one I/O store. On disk each
  becomes an append-only JSON Lines file: `.triage_cache.jsonl`, `.summary_cache.jsonl`,
  `.signal_candidate_cache.jsonl`. Each line is one `(key, entry)` record in today's persisted
  DTO shape, so cache keys and entry meanings are unchanged. On load, later lines win for equal
  keys; an unparseable earlier line is skipped and counted in the log.
- Tail recovery before any append: when the store is opened, a final line without a
  terminating newline (a torn write) is copied to a sidecar such as
  `.summary_cache.torn-<timestamp>.jsonl` for forensics (covered by the marker's `.*.jsonl`
  internal-state pattern) and the file is truncated back to the last complete line. Only then does the sink
  append. This keeps the next record from joining the fragment and becoming unreadable.
- Migration with atomic publication: when a `.jsonl` file is absent and the `.ron` file
  exists, the RON file is read once and the complete result is written to a temporary file in
  the output folder, flushed to disk, verified by re-reading (same key count as the RON load),
  and only then renamed to `.jsonl`. A `.jsonl` file therefore exists only when it holds the
  whole migration; an interrupted migration leaves at most a stale temporary file, which the
  next start deletes before migrating again. The temporary file's name is pinned to the
  marker's `internal_state` patterns, for example `.summary_cache.migrating-<timestamp>.jsonl`,
  so a leftover never sits unclassified in the corpus root. The RON file is never modified and
  stays as the backup.
- Refusal, not an empty store: if the RON file exists but fails to parse (or has an unknown
  format version), nothing is written, AI stages stay unavailable, and the owner is told which
  file failed and why. Paid results are never replaced by an empty store. The desktop shows the
  existing `ai_unavailable_message` on the Run surface. Today that field reaches the page only as
  the tooltip of the disabled Process unfinished button (`view_builder.rs:165-177`); the page
  never reads the field itself. The command line prints the file and reason at startup and in
  its final summary, still polls and downloads, and exits non-zero. The owner recovers
  deliberately: restore the file from a backup, or move it aside to start that store empty
  (its results are then paid for again when needed).
- After the switch only the `.jsonl` files are written; there is no dual-write period (owner
  answer, see "Settled inputs").
- No eviction: `DEFAULT_CACHE_CAPACITY` and the eviction paths in `summary_cache.rs` and
  `triage_cache.rs` go.
- Saving: on each completion the reducer inserts the result into state and emits
  `Effect::SaveResults` carrying only the new records (never a clone of the whole store). The
  effect runner's injected result sink appends them in coalesced batches: at most about two
  seconds after the first unsaved record, at run end and Stop (the reducer emits a flush
  request), on runner drop, and on desktop window close. A crash loses at most a few seconds.
  This follows the 2026-09-18 pattern (reducer-emitted effect, runner-owned injected sink).
- Why JSON Lines rather than coalesced RON rewrites: appends cost only the new records, while
  a rewrite costs the whole store (9,345 summaries today and growing without eviction). JSON
  Lines also makes a torn write local to one line.

### Slim runtime state and a per-article link store (Phase 7)

- `.harvester_state.ron` keeps, per completed job, the URL, tokens, bytes and fetch time, plus
  the pending-intake list (Phase 3), the desktop window size and the remembered desktop list
  tab and selected article URL (Phase 8). All new fields are optional with serde defaults, so
  old files still load and older builds can still read new files.
- Extracted links move to a per-article link store under `output/.article_links/`, one small
  JSON file per article named by a hash of the canonical archive URL key. Records keep the URL,
  anchor text and link kind, which Archive contract step 3c needs. Paths are derived from the
  hash only and confined to the output folder.
- "Open a link found in the article" loads the selected article's links through a
  reducer-emitted effect when the selection changes; the snapshot shape for the selected job's
  links is unchanged.
- One-time migration on startup (I/O side), ordered so the original bytes are never lost:
  1. Preserve: copy `.harvester_state.ron` to a temporary file, flush, verify it is
     byte-identical to the original (length and hash), then rename it to
     `.harvester_state.pre-slim.ron`. An existing verified backup is never overwritten.
  2. Write the link files from the original (idempotent; each file written atomically).
  3. Replace: write the slim state to a temporary file, verify it loads, then atomically
     replace `.harvester_state.ron`.

  Both temporary files have pinned names that match the marker's `.*.ron` pattern, for example
  `.harvester_state.pre-slim-partial-<timestamp>.ron` and
  `.harvester_state.slim-partial-<timestamp>.ron`, so recovery can recognise them and a
  leftover is never unclassified. Link-file temporaries stay inside `.article_links/`.

  Recovery at each interruption point: a leftover temporary backup is deleted and step 1
  restarts; a verified backup with an old-shape active state resumes at step 2; a verified
  backup with a slim active state means the migration is complete. The active state is never
  replaced before a verified backup exists.

### Saved-results index: one mechanism for view, archive and export (Phase 8)

- The reducer keeps, per article in the display scope, its content hash and the saved triage,
  summary and signal-candidate result under the current keys, with the stored triage key's
  model for provenance. The display scope is the union of the two time-bounded list modes:
  articles fetched since the archive checkpoint (the window) and articles fetched in the last
  24 hours, which DecisionLog 2026-09-16 makes independent of the checkpoint. Each entry records
  whether it is in the window: processing, unfinished work and archive selection stay
  restricted to the window, while the view uses the whole scope so an archived article that is
  still in Last 24h keeps its priority, ordering and summary. "Current keys" means the keys a run under the current
  configuration would compute, including dated-alias compatibility and the scoring key that
  embeds the upstream summary digest. It is built at startup from the stores, the corpus scan's
  content hashes and the loaded prompt metadata and contexts; it is rebuilt when a run freezes
  its configuration and updated on each saved completion.
- The desktop view (priority badges, ordering, Results rows, reading pane summary, archive
  meter and readiness) and the archive dialog, selection, annotations, coverage counters and
  priority snapshot all read this index under current keys. Stale-key results stay in the
  stores but are invisible there, exactly as a run would treat them. The reading pane shows
  only current-key summaries.
- Archive-predicting summary lookups keep today's any-key rule, settled with the owner (see
  "Settled inputs"). The exported summary body, the dialog's summary-mode token estimate, the
  header meter's summary-token estimate and any surviving caller of `summary_result_for_url`
  resolve the summary as today: the newest saved summary for the content hash under any key,
  with `summary_result_for_url` still preferring a live session summary first. The index only
  supplies the content hash, which removes today's dependence on the live triage and
  pre-triage sessions, so these work after a restart with no run and export output is
  unchanged.
- In the pipeline stage this index becomes the "done" side of the per-article stage table.

### Per-article pipeline (Phases 10 to 13)

- Policy lives in a new pure module, `crates/harvester_core/src/pipeline/`: a per-article
  stage-status table (triage, summary, score: not needed, needed, pending, in flight, done,
  failed), eligibility (pre-triage verdict first, then priority cutoffs; scoring requires
  priority at least 2 and current-key upstream results), the cache hit or miss decision under
  the run's frozen configuration, one shared cap on in-flight model calls, cross-stage priority
  (scoring, then summary, then triage, as today, so articles finish sooner and Stop leaves fewer
  half-done ones), the 1,000-call session quota, halts (out of credits; three consecutive rate
  limits), Stop semantics, and one definition of "run done" used by every host.
- Effects are single units of I/O: load and prepare one article, make one validated model call
  (the message carries the validated result and the replay record is written in one place),
  save a batch of results, plus the existing poll, download, import and export effects. No
  worker owns policy or a second copy of results.
- One shared host loop in `crates/harvester_io/src/host_loop.rs` (Tauri-free, Node-free) drives
  the reducer for the command-line run, import and the desktop driver. It moves state rather
  than cloning it and needs no periodic tick for pipeline pacing; the desktop adds only a
  coarse (about one minute) tick for the Last 24h window.

## Conventions for every phase

**Where to run checks.** All Cargo and Pester commands run from the repository root
`C:\Users\larsp\src\web_page_filet_mignon`; frontend commands run from `frontend/`.

**Standard Rust checks** (every phase that touches Rust):

1. `cargo build` (while a batch run is active: `cargo build --workspace --exclude harvester_batch`)
2. `cargo test` for the crates the phase touches, then the full root `cargo test`
3. `cargo clippy --all-targets -- -D warnings`
4. `cargo fmt` (Codex reports `cargo fmt --check`)
5. When `harvester_ui` changes: `cargo clippy -p harvester_ui --all-targets -- -D warnings`

Codex runs these with `--offline` against pre-warmed caches.

**Frontend checks** (phases that touch `frontend/` or IPC), from `frontend/`: `npm run check`,
`npm run build`, `npm run fmt`.

**Owner or Claude only** (Codex cannot run them): Pester
(`Invoke-Pester -Path scripts/tests/HarvesterLaunch.Tests.ps1`), the IPC probe
(`cargo run -p harvester_ui -- --probe-ipc`, needs a display and the output-folder lock, about
150 s, writes `.local/probe/ipc-report.json`), `scripts/project-stats.ps1`, and any git
operation.

**Benchmark** (from Phase 1 on): run before and after each phase and record the numbers in
"Benchmark and size log" below:
`cargo run -p harvester_batch --example replay_bench -- --host batch` and
`cargo run -p harvester_batch --example replay_bench -- --host desktop`.

**Test-count protocol.** Before a phase, record each crate's `cargo test` totals (the
`test result:` lines), the frontend `vitest` total, and the Rust test-attribute count
(the regex in `scripts/project-stats.ps1`, `Count-RustTests`; 1,509 attributes on
2026-09-28). After the phase:

- Only files on the phase's "may be deleted wholesale" list may disappear. Every other test file
  is edited, not deleted.
- The change in counts must equal the named removals plus the added regression tests. The phase
  report lists every removed test by name with the feature it covered.
- Tests that pin preserved behaviour (quota, halts, Stop, cutoffs, cache keys, export bytes,
  ordering, job-list modes, IPC decoding) are rewritten when their harness goes, never dropped.

**Regression tests.** Every bug fix gets a test at the reducer, emitted-effect or
public-contract level. The archive fixtures under
`crates/harvester_engine/tests/fixtures/archive_export/` must pass byte for byte in every phase.

**Carry-over fixture** (from Phase 1 on, run in every later phase): a checked-in pre-change
output folder in today's formats, containing an old-shape `.harvester_state.ron` (with links,
`downloaded_path` values and both window-size pairs), RSS and Brave seen-sets, and all three
paid-result stores in RON with current-key and stale-key entries. The test copies it to a
temporary folder, runs one unchanged command-line cycle through the benchmark harness (canned
polls return only already-seen entries and URLs of restored jobs), and asserts:

- no `EnqueueUrl` for already-downloaded or seen work;
- no model call for any article whose completed work exists under the current keys;
- every paid result still loadable afterwards;
- the three paid-result RON stores (`.triage_cache.ron`, `.summary_cache.ron`,
  `.signal_candidate_cache.ron`) byte-identical to the fixture's copies;
- the RSS and Brave seen-sets lose no entry (content, not bytes: they are rewritten whole
  whenever a poll saves them);
- the runtime state still holds every restored job with its URL and fetch time (content, not
  bytes: the runtime state is rewritten by design).

Later phases may extend the assertions (the Phase 7 backup and migration checks, restored view
state in Phase 8) but never weaken them.

**IPC changes** bump `IPC_SCHEMA_VERSION` in `crates/harvester_ui_bridge/src/ipc.rs` and
`frontend/src/ipc/schemaVersion.ts` together and regenerate fixtures with
`UPDATE_UI_FIXTURES=1 cargo test -p harvester_ui_bridge`.

**IPC probe baselines.** The probe's gates stay as they are: they measure delivery of synthetic
snapshots at 20 Hz, which is independent of the tick and of pipeline pacing. The probe is run
at the Phase 1 baseline, after each phase that changes the snapshot shape or its pacing
(Phases 6, 8 and 12), and at close-out. Each report is kept as
`.local/probe/ipc-report.<label>.json`. When the view shape changes, the synthetic cases in
`crates/harvester_ui_bridge/src/probe.rs` are updated to the new shape and the new numbers
become the baseline. A failing gate is fixed in delivery, following the 2026-09-08 precedent.
Loosening a gate needs its own decision-log entry.

**Engineering diary.** Every bug fix gets a `docs/EngineeringDiary.md` entry with lessons and
prevention; noteworthy implementations get one too (appended at the end of the file).

## Cleanup stage

### Phase 1: Baseline, replay benchmark and pre-flight checks

No product behaviour changes. Everything later is measured against this phase.

Work:

1. Split `harvester_batch` into a library plus a thin `main.rs` (argument parsing and exit code
   only), so examples and tests can drive the real runner. Add an effect-sink seam to the
   dispatch loop (today it calls `effect_runner.enqueue` directly, `dispatch_loop.rs:310-320`)
   so the benchmark can intercept network effects.
2. Add `crates/harvester_batch/examples/replay_bench.rs`:
   - Copies a real output folder (default `output`) into a work folder (default
     `.local/bench/<timestamp>`). It refuses any work path inside the source and never writes to
     the source. `--reuse-copy` skips the copy.
   - Holds back the newest N articles (default 40, like this morning's run) from the copy's
     article files, runtime state and result stores so the run discovers them.
   - Answers `PollAllSources` with canned `SourcePollCompleted` messages carrying the held-back
     URLs spread over 29 sources (24 Brave-like, 5 RSS-like), and answers each `EnqueueUrl` by
     writing the held-back article file into place and sending the job messages with its links
     and a fetch time. No network, no keys, no launcher.
   - Serves model calls from a routing canned provider built on
     `openai_provider_kit`'s `LlmProvider` trait. It returns valid triage, summary and
     signal-candidate JSON by prompt, spreads priorities deterministically by URL hash so the
     cutoffs are exercised, and applies `--llm-latency-ms` (default 0; the log also records a
     1500 ms run to approximate provider latency).
   - Hosts: `--host batch` runs the real batch cycle; `--host desktop` runs
     `harvester_ui_bridge::driver::run_driver` with a 75 ms synthetic tick and a counting
     snapshot sink (`harvester_ui_bridge` is Tauri-free, so it can be a dev-dependency).
   - Reports to `report.json` and stdout: wall time to run completion; reducer time per message
     kind (count, p50, p95, max, total); effects by kind; bytes written and write latency per
     private file; view builds and their cost (desktop host); snapshots emitted.
   - A smoke test in `cargo test -p harvester_batch` runs the harness on a generated
     five-article folder and asserts the run completes and results are saved.
3. Add the carry-over fixture and its test (see "Conventions for every phase") under
   `crates/harvester_batch/tests/`. The fixture is small and synthetic but uses the real
   on-disk shapes; it never contains the owner's data.
4. Add `/.local/bench/` to `.gitignore`.
5. Record baselines: benchmark (both hosts, both latencies), the IPC probe, per-crate test
   totals, and `scripts/project-stats.ps1` output, in the log below.
6. Pre-flight checks, each reported in the phase report:
   - Read `output/.batch_manifest.ron` (keyless, read-only) and list every batch not in
     `Collected` or `Failed`, every entry without a collected record, and every successful
     collected record that is still in the manifest. A remaining collected record means its
     cache confirmation has not happened yet (see Verified facts), so `Collected` alone is not
     treated as safe. For each such record, report whether its key is present in the current
     result stores. As of 2026-09-28 the only batch is `Collected`; if anything is outstanding
     at the provider, the owner runs one `--drain` before Phase 4; unconfirmed records are
     reconciled at the start of Phase 4.
   - Investigate the 22 hidden jobs: count restored jobs without a fetch time in
     `output/.harvester_state.ron`, and check whether each has an article file whose
     frontmatter carries `fetched_utc`. Record the finding for Phase 7.
   - Confirm the benchmark reproduces the known costs (state save of about 10 s per download
     on the copied 78 MB state; the per-message clone).

Verification: standard Rust checks; `cargo test -p harvester_batch` count unchanged apart from
the new smoke test and carry-over test (plus 2). Benchmark runs complete on a copy of
`output/`. Pester and the IPC probe run once to confirm the baseline is green.

Expected test counts: +2 (smoke test, carry-over test). Nothing deleted.

Docs: none required beyond this plan's log. Diary: implementation entry for the replay
benchmark (what it measures and how to run it). Decision log: none.

### Phase 2: Keep every paid result

Fixes bug 4 and the eviction risk (brief 9.3) before anything else changes behaviour.

Work:

1. Refusal instead of an empty store for all three stores (the hazard is verified for each; see
   Verified facts), with the owner-facing behaviour in "Target design": the desktop shows
   `ai_unavailable_message` on the Run surface, following `docs/visual_design/VisualDesignSpec.md`.
   The page starts reading an existing field, so the IPC shape does not change. The command line
   reports the file and exits non-zero.
2. Introduce the generic result store (in-memory in `harvester_core`, I/O in `harvester_io`)
   and move the three caches onto it without changing `TriageCacheKey`, `SummaryCacheKey`,
   the signal-candidate key or their entry types. The triage alias index for current-metadata
   lookups stays and is rebuilt on load.
3. JSON Lines persistence, tail recovery before appending, atomic migration publication with
   the RON file kept, and the refuse-to-overwrite rule, as in "Target design". Every reader of
   the caches moves to the new store in this phase, including the Batch API cache confirmation
   (`runner/batch_runtime.rs:351-395`), which would otherwise keep reading the frozen RON files
   and never confirm newly collected records.
4. Remove `DEFAULT_CACHE_CAPACITY` and the eviction paths, including the code that refreshes
   derived indexes after eviction.
5. Replace `PersistTriageCache`, `PersistSummaryCache` and `PersistSignalCandidateCache` (and
   their full clones) with `SaveResults { records }` emitted on each completion and cache-hit
   provenance change, plus a flush request at run end and Stop. Add the coalescing result sink
   to `EffectRunner` next to the runtime-persistence sink; flush on drop and on desktop close.
6. Summary entries with the retired `AggregateBriefing` prompt id keep loading for now (removed
   with the briefing in Phase 5, where they are skipped with a warning).

Regression tests:

- Reducer: a triage, a summary and a signal completion each emit `SaveResults` with exactly
  that record, before the session settles.
- Result sink: records are written within the coalescing window, on flush and on drop; a
  simulated crash after the window loses nothing earlier.
- Store: more than 10,000 entries are all kept; for each of the three stores, a corrupt RON
  file (and, for the signal store, an unknown format version) is not overwritten and AI stages
  report unavailable with the file named; RON-to-JSONL migration preserves every key and entry
  and leaves the RON file byte-identical; the real-shape fixtures in `summary_cache_store.rs`
  (V3 without entities) still load.
- Refusal is visible: the view carries the store failure in `ai_unavailable_message`, and the
  frontend renders it on the Run surface (a `vitest` case against the existing
  `ai_unavailable.json` fixture); the command-line bootstrap reports the file and ends with a
  non-zero exit code.
- Marker: the pinned migration temporary name and the torn-tail sidecar name each match a
  declared `internal_state` pattern (`corpus_manifest.rs` tests).
- Crash sequences: (a) migration interrupted before publication (temporary file present, no
  `.jsonl`), restart, append, restart: every RON record and the appended record load, no
  temporary file remains; (b) torn final line, restart, append, restart: the fragment is in the
  sidecar, every complete record and the new record load, and the file ends with a newline;
  (c) interrupted append of a multi-record batch: the complete records survive and the next
  append is readable.
- The Batch API cache confirmation finds records saved in the new store.
- Keys: existing key-construction tests unchanged (cache keys must not move).

Expected test counts: removed: `capacity_enforcement_evicts_oldest`,
`evict_to_limit_removes_oldest`, `evict_to_limit_does_nothing_when_below_limit`
(`summary_cache.rs`), `capacity_guard_evicts_oldest_entry` (`triage_cache.rs`); TTL and
older-than eviction tests go only if their functions have no remaining caller. Rewritten to the
new rule, not removed: `load_corrupt_file_returns_empty_cache` (`summary_cache_store.rs:230`)
and `corrupt_file_returns_empty_and_warns` (`triage_cache_store.rs:227`), which pin the hazard
being removed and now assert refusal; effect assertions on the old persist effects. Added: the
regression tests above (about 19, including a new signal-store refusal test). No file deleted
wholesale.

Verification: standard Rust checks; frontend checks (the Run surface now renders
`ai_unavailable_message`); benchmark (expect far fewer bytes written per completion). Human
testing recommended: the owner's next run after this phase; afterwards the `.jsonl` files
exist, the `.ron` files are unchanged, and the run's results are present after closing and
reopening. What the owner would see if a store ever refuses: the desktop Run surface names the
file and says AI is off; the command line prints the same and exits with an error. The AI
steps stay off until the owner restores that file from a backup, or moves it aside to start
that store empty.

Docs: `docs/Architecture.md` (persistence rules under "Key rules"); `docs/CorpusFormat.md` and
the marker: `internal_state` gains `.*.jsonl` (a compatible addition, no schema bump; update
`build_corpus_manifest` and its tests in `crates/harvester_engine/src/corpus_manifest.rs`,
including the pinned temporary and sidecar names). `docs/visual_design/VisualDesignSpec.md` if
the Run surface message needs a new element.
Decision log: "Paid model results are kept and saved incrementally" (see list), including the
owner's no-dual-write rule and the emergency-only rollback. Diary: bug-fix entry (results saved
only at settlement; empty-store overwrite hazard).

### Phase 3: Intake and host fixes

Fixes bugs 2, 3, 5 and 7, the per-message clone in the command-line loops, and brief 9.6.

Work:

1. Bug 2: `poll_rss_source` marks seen only the entries it emits plus entries without a URL
   (the latter as today); `handle_brave_source_poll` marks seen only emitted URLs. Drop the
   stale `#[allow(dead_code)]`.
2. Bug 3: when a poll completes after Stop, the reducer records its URLs in a persisted
   pending-intake list instead of dropping them. The list is a new optional field with a serde
   default on the persisted runtime state (`.harvester_state.ron`), carried in the existing
   `PersistRuntimeState` snapshot, so both hosts write it through the same effect and old state
   files still load. Any Full run, from the command line or the desktop Run button, ingests
   the list before polling and clears ingested entries. Process unfinished (Resume) does not
   fetch them. First verify cancelled-before-start downloads (see Verified facts); if they are
   lost too, they join the same list. Phase 7's slim state keeps the field.
3. Bug 5: one lock per output folder, `.harvester.lock`, for both hosts (and the IPC probe). The
   lock metadata records which host holds it; a second start refuses and names the holder
   ("the Harvester window" or "a command-line run", with PID and start time). The desktop still
   refuses through the pre-window rfd dialog; the command line prints the message and exits
   non-zero; `--force-unlock` keeps its meaning.
4. Bug 7: one helper in `crates/harvester_io/src/host_bootstrap.rs` decides AI availability
   from the environment (missing or empty means unavailable, with the existing messages), used
   by every host; `OpenAiProvider::from_env` also rejects an empty key.
5. Replace `update(state.clone(), msg)` with a move in `dispatch_loop.rs` and `import_mode.rs`.
6. `.sources.ron` (brief 9.6): remove `SourceType::Script`, `SourceKind::Script` and
   `--allow-unsupported-sources`. Load the registry entry by entry: an entry whose type is
   unknown or removed is skipped with an `engine_logging` warning naming its position and, where
   readable, its id; valid entries load as today; File and CuratedList stay parseable; other
   existing validation errors behave as today. First step: verify the chosen parsing approach
   (per-entry `ron::Value`, an untagged fallback, or span splitting) against a copy of the real
   file (24 BraveNews and 5 Rss entries) plus a Script entry and a misspelt type, because RON's
   enum handling in untyped values is the known risk.

Regression tests:

- RSS and Brave: with a limit of 1 and three unseen entries, the first poll emits one and the
  next poll emits the next one (replaces the loose assertion in
  `poll_rss_source_applies_max_after_dedup`).
- Reducer: `SourcePollCompleted` after Stop stores the URLs and the next `PersistRuntimeState`
  carries them; after a simulated restart the next Full run emits `EnqueueUrl` for them before
  polling and the list is empty afterwards; Resume does not fetch them. A state file without the
  field loads with an empty list.
- Lock: command-line identity held, desktop acquisition refused with the holder named, and the
  reverse; release on drop.
- AI availability: empty and missing key both unavailable on both hosts' bootstrap paths.
- Sources: real-shape registry loads all 29 entries; a Script entry and an unknown type are
  skipped with a warning and the rest load; File and CuratedList entries still parse.

Expected test counts: `source_config.rs` and `cli.rs` tests that construct Script sources or
`--allow-unsupported-sources` are removed and named; about 11 added. No file deleted wholesale.

Test disposition: no tests were removed. The source registry tests and
`explicit_values_are_parsed_and_applied` remain, with the latter no longer passing or asserting
the removed `--allow-unsupported-sources` flag. No test file was deleted wholesale.

Verification: standard Rust checks, plus `cargo clippy -p harvester_ui --all-targets -- -D warnings`
(the desktop host changes). Benchmark (command-line reducer time per message should fall).
Human testing recommended: start the desktop, then the command-line launcher, and confirm the
second refuses and names the first; then the reverse.

Docs: `docs/Architecture.md` (crates section: one lock); `docs/FutureIdeas.md`: close
FI-Architecture-HostConcurrency-0001; `README.md` if it mentions separate locks (it does not
today). Decision log: "One lock per output folder for both hosts"; "Source registry entries of
unknown type are skipped". Diary: bug-fix entries for bugs 2, 3, 5 and 7.

### Phase 4: Remove the Batch API and the batch-only modes

Removes the Batch API, `--drain`, `--dry-run` (bug 1), `--single-shot`, the recurring mode and
`--poll-interval`, and fixes bug 8.

Entry condition: the Phase 1 manifest check is repeated at the start of this phase and shows
nothing outstanding at the provider, every successful collected record is confirmed in the
durable result stores (step 1 below), and the owner confirms before the code is removed.

Work:

1. Reconcile collected results before anything is removed. `Collected` does not prove a result
   reached the caches (see Verified facts). While the batch code still exists, add a keyless
   one-shot reconciliation (a `harvester_batch` example, run by the owner or by Claude with the
   owner's consent because it writes to `output/`): it loads the manifest and the migrated
   result stores, validates every successful collected record with the same validators the
   collection path uses, appends each record whose key is missing through the result store,
   flushes, and re-checks. The key is the entry's frozen key from the manifest
   (`PendingEntry.key: FrozenBatchKey`, `batch_manifest.rs`), never recomputed under the current
   configuration, since dated-alias keys may differ. It makes no provider call and needs no key. It reports confirmed,
   appended and invalid counts; invalid records are listed for the owner and are not appended.
   The removal proceeds only when the re-check finds every successful record in the stores. The
   reconciliation example is deleted together with the batch code later in this phase.
2. Delete `crates/harvester_batch/src/batch_coordinator.rs`, `batch_manifest.rs`,
   `runner/batch_runtime.rs`, `runner/drain_control.rs`, `runner/dry_run.rs`,
   `crates/openai_provider_kit/src/batch.rs`, `crates/harvester_core/src/update/batch_results.rs`,
   and the reconciliation example from step 1.
3. Remove from core: `LlmResultKind::DeferredToBatch`, `Msg::RearmDeferredBatchStages`, frozen
   batch keys, `llm_deferred_allowance`, the `AfterDownloadsSettle` and `Disabled` wave
   policies, Continue scope if it has no remaining caller, `AwaitingBatch` phases, deferred
   counters in observations and progress, and the `[model-budget]` deferred line.
4. The command-line host always runs one cycle: poll, download, process, exit. Remove
   `--batch-api`, `--drain`, `--dry-run`, `--single-shot` and `--poll-interval`.
5. Launch policy: `RuntimeArguments` for Batch becomes empty in
   `scripts/lib/HarvesterLaunch.psm1`; update the three assertions in
   `scripts/tests/HarvesterLaunch.Tests.ps1`.
6. Bug 8: the command-line and import loops stop counting idle timeouts toward an iteration
   cap; a wall-clock no-progress watchdog (no message and no in-flight work for a named
   duration) replaces it and logs the stuck operation.
7. Replay records are now written in one place (the synchronous path); `persist_replay_record`
   stays.

Regression tests: the reconciliation (before its deletion) confirms records already in the
stores, appends a missing successful record so a second check confirms it, and refuses an
invalid record; a command-line cycle with no flags polls, processes and exits; a loop idle for
longer than the old cap does not fail while downloads are in flight; the watchdog fires only
without progress; the carry-over fixture test still passes. Pester: the Batch policy has no
runtime arguments.

Expected test counts. May be deleted wholesale (test attributes): `batch_coordinator.rs` (13),
`batch_manifest.rs` (4), `runner/batch_runtime.rs` (2), `openai_provider_kit/src/batch.rs` (4),
`update/tests/batch_api_tests.rs` (5), the reconciliation example's tests, plus
`drain_control.rs`, `dry_run.rs` and `batch_results.rs` (no tests). About 28 in total, plus
the reconciliation tests added and removed within the phase. Edited, not deleted: `runner/tests.rs`
(50 today; about 17 tests named for batch, drain, dry-run, deferral or recurring behaviour go),
`cli.rs` (21; removed-flag tests go), `update/pipeline_run/wave_tests.rs` (12),
`update/pipeline_run/tests.rs` (28), `update/model_dispatch_tests.rs` (12). Expected total
removal: roughly 45 to 65 attributes, every one named in the report.

Verification: the reconciliation report (every successful collected record confirmed in the
stores) is attached to the phase report before step 2 starts; standard Rust checks; Pester
(owner or Claude); benchmark with `--host batch` (the Batch API wait cycles disappear). Human testing recommended: the first morning run
without the Batch API; the progress lines should reach the end without "awaiting batch".

Docs: `docs/Architecture.md` (remove the Batch API and drain paths, the deferred allowance
paragraph and `AfterDownloadsSettle`/`Disabled`); `README.md` (launch description);
`docs/ThreatModel.md` (confirm no Batch API collection path remains; replay records stay as
forensics); `docs/FutureIdeas.md` (retire Batch API items). Decision log: "The Batch API and the
batch-only modes are removed". Diary: bug-fix entries for bug 1 (by removal) and bug 8.

### Phase 5: Remove the aggregate briefing, stale-summary refresh and the concatenated export

No IPC change in this phase: view-model fields that fed removed features stay and read
`false` or empty until Phase 6 removes them with the single IPC bump.

Work:

1. Aggregate briefing and stream: remove `crates/harvester_core/src/briefing_snapshot.rs`,
   `state/briefing_snapshot_access.rs`, `state/briefing_orchestration.rs`, the aggregate parts
   of `briefing.rs`, `update/briefing.rs` and `update/llm_completed.rs`, engine
   `llm/prompts/briefing.rs` and `llm/prompts/briefing_stream.rs`, briefing validation, briefing
   history persistence and `contexts/aggregate_briefing.toml`. `PromptId::AggregateBriefing`
   goes; persisted summary entries with that id are skipped with a warning.
   - Trap: `BriefingSession` is also the live summary stage. `settle_summaries` must never
     issue an aggregate call; remove the `skip_aggregate_briefing` switch rather than defaulting
     it, and keep `briefing_metadata_state` only as far as live summary dispatch needs it.
   - Keep (live, misnamed): `.briefing_checkpoint.ron`, `briefing_since_utc`,
     `SaveBriefingCheckpoint`, `TriageSelectionPolicy`, and the engine corpus loader
     (`CorpusScanIndex`, `TriageArticleDelta`, `summary_preparation_budget`).
   - Rewrite the roughly 40 live-summary tests that drive through `Msg::ArticlesLoaded` or
     `request_briefing_orchestration()` to drive the triage-to-summary path through
     `update/test_support.rs`. Their count stays the same.
2. Engine loaders used only by briefing, refresh or tests in
   `crates/harvester_engine/src/briefing.rs`, and `LoadArticlesForBriefing`.
3. Stale-summary refresh: `crates/harvester_batch/src/summary_refresh.rs`,
   `progress/stale_reporter.rs` (move `format_elapsed` to `import_reporter.rs` first), the
   refresh CLI flag, and writing `summary_refresh_reports/` and `.summary_refresh_last.json`.
4. Concatenated export (`export.txt`, `manifest.json`) in `crates/harvester_engine/src/export.rs`,
   separating `ExportOptions` from the archive exporter without changing archive output.
5. Replay provider lookup (`replay_cache` is always `None`); keep `persist_replay_record`.
6. Prompt Lab remnants (`Msg::RequestLlmCompletion`, template and model overrides,
   `max_input_chars`); keep prompt overlay loading (2026-09-18).
7. Test-only production code: `CurrentWorkingCorpus::select`, `summaries_follow_triage`,
   `PipelineRunPhase::AwaitingSettle`, and Msg variants with no production sender that are not
   reachable from `UiIntent`. The manual pre-triage decisions API moves to test support because
   15 or more tests use it as a fixture tool.
8. Corpus marker: drop `export.txt`, `manifest.json`, `summary_refresh_reports/` and
   `.summary_refresh_last.json` from `generated_artifacts`. Not a schema bump: none of those
   patterns can match an article record (`*.md` in the root or `linked/*.md`), so no reader's
   classification of articles changes (`docs/CorpusFormat.md`, "Versioning Rules").

Regression tests: summaries complete and are saved with no aggregate request emitted, whatever
the settle path; a summary store containing `AggregateBriefing` entries loads the rest; the
marker lists exactly the remaining artifacts; the archive fixtures pass byte for byte.

Expected test counts. May be deleted wholesale: `briefing_snapshot.rs` (9),
`state/briefing_snapshot_access.rs` (1), `update/tests/briefing_stream_tests.rs` (9),
`update/tests/briefing_history_tests.rs` (7), `llm/prompts/briefing.rs` (6),
`llm/prompts/briefing_stream.rs` (4), `summary_refresh.rs` (4), `progress/stale_reporter.rs`
(27, less any `format_elapsed` tests moved with it), `state/briefing_orchestration.rs` (0).
About 67 in total. Edited, not deleted: `briefing.rs` (24), `update/tests/mod.rs`,
`state/tests/mod.rs`, `tests/triage_orchestration.rs`,
`crates/harvester_engine/tests/briefing_loader_integration.rs` (23; loader tests for removed
loaders go, scan-exclusion tests stay), `tests/output.rs` (26; concatenated-export tests go),
`tests/llm_replay.rs` (10; lookup tests go, record tests stay), `working_corpus.rs` (15;
`select` tests go), `llm/validation.rs` (briefing validation tests go).

Verification: standard Rust checks; benchmark. No human testing needed beyond the next normal
run.

Docs: `docs/Architecture.md` (remove "Executive briefing" from planned evolution and the
`LoadArticlesForBriefing` sentence); `docs/CorpusFormat.md` marker example and text;
`docs/ThreatModel.md` (replay records are write-only forensics); `docs/FutureIdeas.md` (retire
briefing and Prompt Lab items); `docs/PromptContextFiles.md` if it lists
`aggregate_briefing.toml`. Decision log: "The aggregate briefing is removed" (supersedes
2026-09-18 "Prompt Lab is deleted while briefing domain state remains" for the briefing part);
"Retired generated artifacts leave the corpus marker without a schema bump". Diary:
implementation entry.

### Phase 6: Slim the desktop surface (IPC 13)

Work:

1. Trends and the entity index: `crates/harvester_core/src/trends.rs`, `entity_index.rs`,
   `crates/harvester_io/src/entity_index_store.rs`, the worker upsert path (so
   `.entity_index.ron` is no longer rewritten on every result), 6 Msg, 3 Effect and 2 UiIntent
   variants (`SetTrendCategory`, `TrendsViewOpened`).
2. `WorkspaceView` and `UiIntent::SetWorkspaceView` if the page does not use it (verify against
   `frontend/src`), unread `AppViewModel` fields (the page reads 22 of 48; verify the list
   against `frontend/src` and the bridge projection before removing;
   `ai_unavailable_message` is read by the page from Phase 2 on and stays), the preview pipeline
   (`content_preview` up to 40 KB per job, `crates/harvester_engine/src/preview.rs`), keeping
   `format_summary_for_preview` used by the summary-only reading pane; remove the unread
   `format_triage_for_preview` and its tests,
   `crates/harvester_core/src/url_age.rs`, and unused body keys (Preview, TriageMarkdown,
   PollStatsMarkdown).
3. Linked-page download and delete (`Effect::DownloadLinkedPage`, `Effect::DeleteLinkedPage`,
   `LinkDownloadState`), the indirect-link pool (`state/indirect_links.rs`,
   `UiIntent::PollIndirectLinks`, `JobOrigin::Indirect`). The per-link `downloaded_path` field
   stays readable in old state files and is ignored.
4. Legacy window size (`PersistWindowSize`, `load_window_size`); the desktop geometry fields stay
   (2026-09-04 entry).
5. Bump IPC to 13 in Rust and TypeScript, regenerate fixtures, update the frontend and its
   tests.

Regression tests: IPC decoding rejects the removed intents; the desktop snapshot fixture pins
the reduced field set; the selected job still carries its extracted links; old state files with
`downloaded_path` values load.

Expected test counts. May be deleted wholesale: `trends.rs` (24), `entity_index_store.rs` (7),
`update/tests/entity_index_tests.rs` (3), `url_age.rs` (7), engine `preview.rs` (6) if nothing
else uses it, `state/indirect_links.rs` (0). About 47 in total. Edited: core `preview.rs` (8),
`update/mod.rs` (the trends contract test), `crates/harvester_ui_bridge/src/probe.rs` synthetic
views, `frontend/src/App.test.tsx` and component tests for removed fields.

Verification: standard Rust checks; `cargo clippy -p harvester_ui --all-targets -- -D warnings`;
frontend checks; IPC probe (owner or Claude; new baseline recorded); benchmark `--host desktop`.
Human testing recommended: open the desktop app and walk the actions in brief section 3 (Run,
Process unfinished, Stop, Add URLs, archive dialog, list modes, search, reading pane, open in
browser, open extracted link, exclusion toggle, meters, activity feed).

Docs: `docs/Architecture.md` (crates section, view projection); `README.md` (drop
`.entity_index.ron` from the output list); `docs/visual_design/VisualDesignSpec.md` only if it
names a removed surface. Decision log: "Trends, the entity index, linked-page download and the
indirect-link pool are removed". Diary: implementation entry.

### Phase 7: Slim the runtime state and move links to a link store

Work:

1. Link store and slim state as in "Target design", with the one-time startup migration in its
   stated order: preserve and verify `.harvester_state.pre-slim.ron` first, then write link
   files, then atomically replace the active state. The active state is never replaced before a
   verified byte-identical backup exists, an existing verified backup is never overwritten, and
   each interruption point has the recovery rule given there.
2. The persistence snapshot captures only slim job records, so a finished download no longer
   copies every job's links.
3. Selecting an article emits a load for its links; the reply fills the selected job's links
   in state. "Open extracted link" keeps resolving through the core index
   (`update/mod.rs:733-775` test stays green).
4. Hidden-jobs fix, per the Phase 1 finding: recover missing fetch times from article
   frontmatter through the corpus scan index at startup. Jobs that still lack a fetch time
   (no article file) stay hidden from the desktop list, keep blocking re-download of their URL
   and are never deleted; the existing "N jobs are hidden because their fetch time is missing"
   note stays and shows the remaining count (owner answer). The phase report states how many
   remain.
5. New link records keep anchor text and link kind for Archive contract step 3c.

Regression tests: an old-shape state file (links, `downloaded_path`, both window-size pairs)
loads, migrates, produces link files and a slim state, and leaves a backup byte-identical to the
original; a second start does not migrate again; interruption at each point (during the backup
copy, after the backup but before or during link writing, during the slim-state write) followed
by a restart completes the migration with the backup still byte-identical and all links present;
an existing verified backup is not overwritten; the link-store path cannot escape the output
folder; a restored job's links open after selection; recovery fills fetch times from
frontmatter and the hidden count drops accordingly; a job without an article file stays hidden,
is counted in the note, is kept after a restart, and still blocks re-download of its URL; the
pinned temporary names match the marker's `.*.ron` pattern and a leftover of each is recognised
and handled by recovery; the pending-intake list survives the migration. The carry-over fixture
test gains the Phase 7 assertions: `.harvester_state.pre-slim.ron` is byte-identical to the
fixture's original state file, the active state is slim and loads, every restored job keeps its
URL and fetch time, and a second run does not migrate again.

Expected test counts: `crates/harvester_io/src/persistence.rs` (19) edited; about 14 added; no
file deleted wholesale.

Verification: standard Rust checks; benchmark on a fresh copy of `output/` (expect the per-
download state save to drop from about 10 s to well under a second, and a much smaller state
file). Human testing recommended: first desktop start after this phase (migration runs once),
then open an extracted link on an old article and a new one.

Docs: `docs/CorpusFormat.md` and the marker (`internal_state` gains `.article_links/`; no schema
bump); `docs/Architecture.md` (persistence and link handling); `docs/ThreatModel.md` (path
confinement for the link store). Decision log: "Extracted links live in a per-article link
store". Diary: implementation entry for the slimming, bug-fix entry for missing fetch times if
the investigation confirms a data gap. No decision-log entry is needed for the hidden-jobs rule:
it keeps today's visible behaviour (the note and exclusion from time-scoped lists, per
2026-09-16).

### Phase 8: Same state after restart; export from saved results

Fixes bug 6 and delivers the restart-state requirement with one mechanism.

Work:

1. First step: confirm that startup hydration provides content hashes for the whole window and
   loads prompt metadata and contexts without a run (the archive meter working at startup
   indicates it does), and that the signal-candidate key can be built from saved current-key
   upstream results without a live session. Also establish how content hashes are obtained for
   Last 24h articles fetched before the checkpoint, which the window-bound pre-triage hydration
   does not cover (the corpus scan index already holds a hash per article file). And confirm
   how the summary-token callback that `archive_token_estimates_from_parts`
   (`state/batch.rs:385-414`) receives counts a summary's tokens. The composition itself is
   known: per selected article it adds the resolved summary's token count, or the article's full
   token count when no summary resolves. The estimate test below asserts that rule, not a raw
   sum over exported bodies.
2. Build the saved-results index (see "Target design") over the display scope (window plus
   Last 24h), marking window membership, and keep it current on saved completions, when a run
   freezes its configuration, when the checkpoint moves, and when the coarse clock moves an
   article out of Last 24h.
3. Point the view at it: row priority and ordering, Results rows, reading-pane summary
   (current-key only, as today), archive meter and readiness. The meter's summary-token
   estimate keeps today's any-key rule (step 5). The view uses the whole display scope, so
   Since checkpoint and Last 24h both show saved results. This replaces the per-build walk of
   the whole summary cache (`view_builder.rs:323-356`) and the separate cache-derived archive
   index: each index entry also records the summary today's any-key rule resolves for its
   content hash, kept current as summaries are saved, so no view build walks the cache. The
   live sessions still drive run progress and the "triage running" ordering rule (2026-09-09)
   until the pipeline stage.
4. Point the archive at it, restricted to window entries: dialog counts and defaults (no longer
   gated on the live triage phase, `update/archive.rs:35-45`), the pinned corpus selection by
   the same policy, annotations with `triage_model` from the stored key, and the priority
   snapshot for coverage counters (every window article with a current-key triage result). Field
   meanings in `docs/ArchiveExportFormat.md` do not change; `export_schema` stays 2.
5. Keep every archive-predicting summary lookup on today's any-key rule, taking content hashes
   from the index instead of the live sessions (owner answer; see "Settled inputs"):
   - `build_summary_map` (`update/archive.rs:321-336`): the exported body is the newest saved
     summary for the content hash under any key, via `lookup_any_by_content_hash`, so
     `archive.md` stays byte-identical to a post-run export today;
   - `archive_token_estimates` (`state/batch.rs:273-283`), the dialog's summary-mode estimate,
     and the header meter's summary-token estimate: same any-key resolution;
   - `summary_result_for_url` (`state/signal_candidate_access.rs:154-167`): live summary session
     first, then the any-key cache, for whichever of its callers survive Phases 5 and 6
     (`job_access.rs:230`).

   All of these work after a restart with no run.
6. Remember the job-list tab and the selected article URL in the runtime state; restore them at
   startup (selection only if that article is in the restored tab, otherwise none). Search text
   and scroll position are not remembered. Cadence: a tab or selection change emits the existing
   `PersistRuntimeState`. The persistence worker already coalesces bursts (350 ms debounce, 2 s
   maximum flush interval, `crates/harvester_io/src/persistence_worker.rs:11-12`), and the state
   is slim since Phase 7, so rapid clicking produces a few small writes, not one per click.
   Window close flushes. A crash mid-session loses at most about two seconds of view state,
   which reverts to the previous tab and selection.
7. The reading-pane placeholder distinguishes "not summarised under the current settings" from
   AI being unavailable, using existing wording where it exists.

Regression tests:

- Reducer (the brief's required test): hydrate saved stores and completed jobs as at startup,
  with no run, and assert priority, priority ordering, reading-pane summary, Results rows and
  the archive meter for current-key results, and nothing for stale-key results.
- Last 24h across the checkpoint: with the checkpoint newer than an article fetched within the
  last 24 hours, a simulated restart shows that article in Last 24h with its priority, ordering
  and current-key summary, while it is absent from Since checkpoint, from unfinished work and
  from archive selection.
- Snapshot builder: after hydration only, `ArchiveRequested` carries annotations and a priority
  snapshot from current-key results only, including a compatible-alias hit that exports the
  stored model; a stale-key triage result is absent and counts as `unavailable`.
- Summary bodies: after hydration only, the summary map contains bodies for selected articles
  (content hash taken from the index, not the live session). An article whose only saved
  summary is under a stale key still exports that summary, exactly as today, while the reading
  pane shows no summary for it; when both exist, the export uses the newest by today's rule and
  the reading pane uses the current-key one.
- Estimate predicts export: after a restart with no run, the dialog's summary-mode token
  estimate equals the estimate a post-run dialog shows over the same saved results and
  selection. For every selected article the estimate counts the same summary the exporter
  exports (including an article whose only summary is under a stale key), or the article's full
  token count when none resolves, per the estimator's own rule (`state/batch.rs:385-414`). The
  header meter's summary-token estimate agrees with the dialog's.
- Any-key lookup: after hydration only, `summary_result_for_url` returns the any-key cached
  summary for a stale-key-only article, as today, and prefers a live session summary when one
  exists.
- Equivalence: an export after a run and an export after a restart over the same saved results
  produce identical `ArchiveRequested` effects and byte-identical `archive.md` output (checked
  through the exporter, not only the effect).
- Tab and selection survive a simulated restart; an out-of-tab selection is dropped; a tab or
  selection change emits `PersistRuntimeState` carrying the new values.
- The carry-over fixture test additionally asserts restored priorities and summaries with no
  run.
- Archive fixtures pass byte for byte.

Expected test counts: `update/tests/archive_tests.rs` (71) edited, not deleted; tests that
asserted export was unavailable without a completed session are rewritten to the new rule and
named; about 19 added.

Verification: standard Rust checks; `cargo clippy -p harvester_ui --all-targets -- -D warnings`;
frontend checks if any wording changes; IPC probe (view changes); benchmark `--host desktop`
(view build cost should drop). Human testing recommended: close and reopen the desktop app and
check priorities, summaries, Results, the meter, the remembered tab and article; export straight
after a restart and compare with a post-run export.

Docs: `docs/Architecture.md` (archive availability and the saved-results index);
`docs/ArchiveExportFormat.md` (state that annotations and the priority snapshot come from saved
current-key results and that summary bodies are the newest saved summary for the content hash;
no field change). Decision log: refinement of 2026-09-20 (the priority snapshot and annotations
come from saved current-key results, not only the live session); "The desktop view and export
read one saved-results index" (current keys for triage and priority, selection, annotations,
coverage counters and the reading pane; the exported summary body and every estimate or lookup
that predicts it keep the newest-summary-for-content-hash rule). Diary: bug-fix entry for bug 6 and the
restart symptom.

### Phase 9: Compact command-line progress

Work:

1. Replace the console dashboard (`crates/harvester_batch/src/progress/dashboard.rs`,
   `progress/projection.rs`, `runner/live_progress.rs` and the dashboard parts of
   `runner/reporting.rs` and `progress.rs`) with a compact per-stage block that reads the
   reducer-owned `RunProgress`: one line each for poll, download, triage, summary and score with
   done and total counts and "Waiting for articles" while intake is open. It redraws in place on
   a terminal and prints periodic plain lines otherwise.
2. Remove `--verbose-progress` and `--ascii-progress`. Keep `--import-saved-web-dir`,
   `--llm-concurrency`, `--signal-candidate-threshold`, `--force-unlock`, `--sources`,
   `--output-dir`, `--contexts-dir`, `--prompts-dir`.
3. Rename the checkpoint flags to `--set-checkpoint`, `--set-checkpoint-now`,
   `--clear-checkpoint` and `--show-checkpoint`, keeping the old `--*-briefing-since` spellings
   as hidden aliases. The file stays `.briefing_checkpoint.ron`.

Regression tests: the block's text for a representative `RunProgress` (golden strings); old and
new checkpoint flag spellings parse to the same command.

Expected test counts. May be deleted wholesale: `progress/dashboard.rs` (6),
`progress/projection.rs` (16), `runner/live_progress.rs` (8). Edited: `runner/reporting.rs`
(20), `progress.rs` (3), `cli.rs`. About 30 to 40 removed, about 6 added.

Verification: standard Rust checks; benchmark `--host batch`. The launch policy does not change
(the launcher passes no flags since Phase 4), so Pester is not needed. Human testing
recommended: one morning run to judge the block's readability.

Docs: `README.md` (command-line description). Decision log: "The command-line host is kept with
a compact progress block". Diary: implementation entry.

### Checkpoint: go/no-go (owner decision)

The plan stops here. The checkpoint report, written in plain language, contains:

- production and test line counts per crate (`scripts/project-stats.ps1`) against the Phase 1
  baseline, plus test totals;
- the benchmark table (both hosts, both latencies) for every phase so far;
- IPC probe results against the Phase 1 baseline;
- what the remaining orchestration still costs in code (the wave, admission and session
  machinery with its tests, about 13k production and 10-12k test lines per the brief) and what
  the pipeline stage would remove;
- the list of files the owner may delete or archive (see "Files on disk");
- any regressions or open issues found by the owner's use since Phase 2.

The owner decides whether to continue. Stopping here leaves a complete, documented product:
every decision-log entry written so far describes the code as it stands.

## Pipeline replacement stage

### Phase 10: Per-article pipeline core (not yet wired)

Work:

1. First step: inventory the three quota mechanisms and the double validation, and list which
   are live, so the new module owns exactly one session quota and one validation point.
2. Add `crates/harvester_core/src/pipeline/` as in "Target design", seeded from the
   saved-results index. It is not yet used by any host.
3. Define the effect vocabulary: `LoadArticle` (one article: read, prepare with the frozen
   budget, content hash; text kept only while the article has pending model steps),
   `CallModel` (one request; the reply carries the validated result or a classified failure
   and writes the replay record once), `SaveResults` (from Phase 2). Implement them in
   `EffectRunner` alongside the existing effects, with bounded concurrency for article loads.
4. Port behaviour tests from the legacy reducer to the new module, by behaviour rather than
   harness: pre-triage before any call; cutoffs; scoring needs priority at least 2 and
   current-key upstream results; configuration frozen per run; one shared cap and cross-stage
   priority; the 1,000-call quota; out-of-credits and three rate limits halt the run and the next
   run clears them; Stop issues no new call, in-flight calls finish and are saved, never-started
   work stays unfinished; unfinished count and reprocess notice (more than 150 articles, or an
   estimate above 50 percent of remaining quota; equality does not trigger); model calls only
   inside a run; one "run done" definition.

Expected test counts: additions only (the ported behaviour tests, roughly 60 to 90). No
deletions.

Verification: standard Rust checks. No human testing.

Docs: none yet (not wired). Decision log: none yet. Diary: none.

### Phase 11: Shared host loop; the command line runs on the new pipeline

Work:

1. Add `crates/harvester_io/src/host_loop.rs`: receive, reduce by move, dispatch effects,
   optional per-iteration hook for the host, no iteration cap, no pacing tick.
2. The command-line run and import use the loop and the new pipeline. Import becomes an input:
   imported pages are written as article files and enter the stage table like downloads.
3. The compact progress block reads `RunProgress` fed by pipeline messages.
4. The desktop still runs the legacy orchestration; both write the same stores.

Regression tests: a full command-line cycle through the shared loop with the routing provider
(poll, download, triage, summary, score, saved, exit); import adds articles and processes them
without polling; Stop through the loop drains in-flight calls.

Expected test counts: additions for the loop and host paths; command-line tests that drove the
old loop are rewritten, not deleted.

Verification: standard Rust checks; benchmark `--host batch` on the new path compared with the
checkpoint numbers. Human testing recommended: at least three morning runs on the command line
before Phase 12, checking counts and results in the desktop afterwards.

Docs: `docs/Architecture.md` (host loop, noting the desktop still uses its own driver). Decision
log: none in this phase; the shared-host-loop entry is written in Phase 12, once the desktop also
uses the loop. Diary: implementation entry.

### Phase 12: The desktop runs on the new pipeline

Work:

1. The desktop driver in `crates/harvester_ui_bridge/src/driver.rs` uses the shared loop; Run,
   Process unfinished and Stop go through the new pipeline.
2. Screen behaviours restated per article (brief 9.5), looking the same:
   - Job-list ordering: arrival order while the current run still has triage work; priority
     order once it has none.
   - Last 24h slides on a coarse refresh (about once a minute); an article leaves the view
     within about a minute of turning 24 hours old. The 75 ms tick goes; the view is rebuilt
     only when state changed or the minute tick fires.
   - Run progress: same stage lines and wording; totals grow as articles arrive; "Waiting for
     articles" while intake is open; model stages show Done when the run ends.
   - Reprocess notice computed at run start with the same thresholds.
3. IPC: the snapshot shape should not change; if it does, bump to 14 and regenerate fixtures.

Regression tests: ordering switches exactly when the run has no triage work left; Last 24h
removes an article after the minute refresh that follows its 24-hour mark; progress wording and
Done-at-end; reprocess notice at run start; the desktop concurrency default of 3 and maximum of
10; the UI never navigates during a run.

Expected test counts: desktop driver and view tests edited; `host_drain_cost.rs` timing tests
updated to the new state shape, not deleted.

Verification: standard Rust checks; `cargo clippy -p harvester_ui --all-targets -- -D warnings`;
frontend checks; IPC probe (pacing changed; new baseline); benchmark `--host desktop`. Human
testing recommended: a desktop Run, a Stop mid-run followed by Process unfinished, a restart,
and an export.

Docs: `docs/Architecture.md` (run surface, progress, list behaviour). Decision log: "All hosts
drive the reducer through one shared host loop"; supersede 2026-09-27 "Pipeline stages overlap in waves under one request budget"; refine 2026-09-07 "One
completion query serves both hosts"; refine 2026-09-09 ordering; refine 2026-09-16 Last 24h;
refine 2026-09-27 run surface for progress and the notice (see list). Diary: implementation
entry.

### Phase 13: Delete the legacy orchestration

Work: remove `PipelineWaves` (`pipeline_waves.rs`, `update/waves.rs`), `PipelineAdmission`, run
phases and scopes, wave policies, `PreTriageRefreshCoordinator` and quiet-tick scheduling
(`pre_triage_coordinator.rs`), delta corpus loading, `TriageSession`, `BriefingSession` and
`SignalCandidateSession` as orchestration, the unfinished-work aggregate and its revision
counters, the derived indexes that only served them, `BatchObservation` and
`pipeline_activity()`, frozen key snapshots, re-admission bookkeeping, the remaining duplicate
validation and quota mechanisms, and Msg and Effect variants left without a sender. Rewrite
`docs/Architecture.md` around the per-article pipeline.

Expected test counts. May be deleted wholesale (after Phase 10 ported their behaviours):
`update/pipeline_run/wave_tests.rs`, `update/pipeline_run/tests.rs`,
`update/model_dispatch_tests.rs`, `pre_triage_coordinator.rs`,
`update/tests/pre_triage_refresh_tests.rs`, `update/tests/delta_tests.rs`,
`update/tests/unfinished_work_tests.rs`, `crates/harvester_core/tests/triage_orchestration.rs`,
and session-internal tests in `triage.rs`, `briefing.rs` and `signal_candidate.rs`. The report
maps each deleted file to the Phase 10 tests that cover its behaviours. The brief estimates
10-12k test lines here.

Verification: standard Rust checks; `cargo clippy -p harvester_ui --all-targets -- -D warnings`;
frontend checks; IPC probe; benchmark on both hosts. Human testing recommended: one desktop day
and one morning run.

Docs: `docs/Architecture.md` rewritten. Decision log: none new if Phase 12's entries hold;
otherwise a refinement. Diary: implementation entry.

### Phase 14: Close-out

Final line counts and benchmark table against the Phase 1 baseline; reassess the IPC probe
(keep or retire, with a decision-log entry if retired); sweep `README.md`, `docs/ThreatModel.md`,
`docs/FutureIdeas.md` (stale Win32, Prompt Lab, briefing and Batch API items) and
`docs/ApplicationDescription.md`; present the final "Files on disk" list to the owner.

## Decision-log entries

All appended, never edited. Titles are suggestions; each follows the log's template.

| When | Entry | Relation |
|---|---|---|
| Phase 2 | Paid model results are kept and saved incrementally: no eviction; append-only result stores; results saved through reducer-emitted incremental effects that the runner's result sink coalesces; after the switch only the new format is written (no dual-write), the old RON files stay untouched as a backup, and returning to a pre-switch build is an emergency-only step via git (any conversion back is manual or a separate tool) | New; states how 2026-09-18 "Host persistence is a reducer-emitted effect" applies to result stores (under brief 9.1 there is no pipeline worker that saves results) |
| Phase 3 | One lock per output folder for both hosts; a second start refuses and names the holder | New; resolves FI-Architecture-HostConcurrency-0001 |
| Phase 3 | Source registry entries of unknown or removed type are skipped with a warning; the rest load | New (external input also edited by the portfolio) |
| Phase 4 | The Batch API and the batch-only modes (drain, dry-run, recurring, single-shot) are removed; all model calls are synchronous | New; retires the "Batch API buffering has its own allowance" clause of 2026-09-27 |
| Phase 5 | The aggregate briefing is removed | Supersedes the briefing half of 2026-09-18 "Prompt Lab is deleted while briefing domain state remains" |
| Phase 5 | Retired generated artifacts leave the corpus marker without a schema bump | New (corpus contract) |
| Phase 6 | Trends, the entity index, linked-page download and the indirect-link pool are removed | New |
| Phase 7 | Extracted links live in a per-article link store, not in runtime state | New |
| Phase 8 | The archive priority snapshot and annotations come from saved current-key results | Refines 2026-09-20 "Archive coverage counters measure the exporter's window" |
| Phase 8 | The desktop view and export read one saved-results index covering the Since checkpoint and Last 24h scopes; current keys for triage and priority, selection, annotations, coverage counters and the reading pane; the exported summary body, the summary-mode token estimates and `summary_result_for_url` keep today's newest-summary-for-content-hash rule so `archive.md` and its predictions are unchanged; processing and archive selection stay window-bound | New |
| Phase 9 | The command-line host is kept with a compact progress block | New |
| Phase 12 | All hosts drive the reducer through one shared host loop | New (written once the desktop uses the loop) |
| Phase 12 | Articles move through the pipeline individually under one shared request budget; the wave ledger, process-lifetime stage sessions and replay waves are gone | Supersedes 2026-09-27 "Pipeline stages overlap in waves under one request budget" (overlap, one budget, scoring-first priority, per-run frozen configuration and current-key completeness remain) |
| Phase 12 | One definition of run completion, owned by the shared pipeline, serves both hosts | Refines 2026-09-07 "One completion query serves both hosts" |
| Phase 12 | Job-list ordering per article | Refines 2026-09-09 "Desktop job list triage ordering" |
| Phase 12 | Last 24h slides on a coarse refresh | Refines 2026-09-16 "Desktop job list adds a Last 24h mode" |
| Phase 12 | Run progress and the reprocess notice per article | Refines 2026-09-27 "The desktop Run is one primary action" (progress and notice parts) |
| Phase 14 | Only if the IPC probe is retired | New |

2026-09-27 "Stop halts new work and drains; export waits only for the run" and "The desktop Run
is one primary action" stay as behaviour. No purity-rule refinement is needed (brief 9.1).

## Documents to update (summary)

- `docs/Architecture.md`: Phases 2, 3, 4, 5, 6, 7, 8, 11, 12; rewritten in Phase 13.
- `docs/CorpusFormat.md`, marker generation and tests: Phases 2, 5, 7 (no schema bump).
- `docs/ArchiveExportFormat.md`: Phase 8 (source of annotations and summary bodies; no field
  change).
- `docs/ThreatModel.md`: Phases 4, 5, 7, 14.
- `README.md`: Phases 4, 6, 9, 14.
- `docs/FutureIdeas.md`: Phases 3, 4, 5, 14.
- `docs/PromptContextFiles.md`: Phase 5 if it lists the briefing context.
- `docs/visual_design/VisualDesignSpec.md`: only if a removed surface is named (Phase 6).
- `docs/EngineeringDiary.md`: every bug fix and noteworthy implementation, as listed per phase.
- `scripts/lib/HarvesterLaunch.psm1` and `scripts/tests/HarvesterLaunch.Tests.ps1`: Phase 4.
- `.gitignore`: Phase 1 (`/.local/bench/`).

## Files on disk for the owner

Harvester stops using these; the plan never deletes them. The owner deletes or archives them
after the phase named:

- After Phase 4, once the reconciliation has confirmed every successful collected record in
  the result stores: `output/.batch_manifest.ron`.
- After Phase 5: `output/.briefing_history.ron`, `output/summary_refresh_reports/`,
  `output/.summary_refresh_last.json`, `output/export.txt`, `output/manifest.json`.
- After Phase 6: `output/.entity_index.ron`.
- Anytime: `output/knowledge_base/` (empty), `output/logs/mcp.log*`, the empty root `src/`.
- Backups created by migrations, once the owner is satisfied: `output/.triage_cache.ron`,
  `output/.summary_cache.ron`, `output/.signal_candidate_cache.ron` (after Phase 2),
  `output/.harvester_state.pre-slim.ron` (after Phase 7), old `.harvester_batch.lock` or
  `.harvester_gui.lock` files if any remain (after Phase 3). Returning to a pre-switch build
  after Phase 2 is an emergency-only step via git (owner answer), so keep the RON backups until
  the owner no longer wants that option.
- Any `*.torn-<timestamp>.jsonl` sidecars written by tail recovery, after the owner has seen the
  warning that created them.
- `.local/bench/` copies of the output folder made by the benchmark.

## Sequencing with the archive contract

- 3b (code-only event clustering inside the exporter) is independent and can land at any time;
  if it lands during Phase 8, rebase on whichever lands first, since both touch the archive
  path but not the same code.
- 3c waits for the pipeline stage. Phase 7 keeps extracted links with anchor text in the link
  store, which gives 3c its frozen candidate list without the runtime-state file. The lead
  excerpt comes from the article file.
- 3d is not decided by removing the dormant linked-page download; it designs its own harvest.
- 3e (a fourth per-article call) becomes one more stage in the per-article table after Phase 13.

## Risks

- **Codex deleting whole test files** during removals: mitigated by the per-phase wholesale lists
  and the test-count protocol; Claude or the owner audits removed test names.
- **Result-store migration** is the first touch of paid data: the RON files are never modified,
  migration refuses on parse failure, the migrated file is published atomically only when
  complete, torn tails are recovered before any append, crash sequences are tested, and the
  owner's first run after Phase 2 is a human check.
- **Batch API results held only in the manifest**: removal waits until the reconciliation
  confirms every successful collected record in the result stores.
- **Runtime-state migration**: the active state is replaced only after a verified byte-identical
  backup exists; each interruption point has a tested recovery.
- **Carry-over drift**: the carry-over fixture test runs in every phase, so a change that would
  re-download seen work or re-pay completed current-key work fails a test before it reaches the
  owner's folder.
- **Sources loader leniency** depends on RON's handling of enums in untyped values; Phase 3
  verifies the approach on the real file first.
- **Two orchestrations coexist** between Phases 11 and 13: both write the same stores through the
  same result sink, and the single lock prevents concurrent runs.
- **Timing tests** in `host_drain_cost.rs` may flake as state changes; they are updated with the
  state shape, and the benchmark is the primary cost measure.

## Benchmark and size log

Filled in as phases land (debug build, copy of `output/`, 40 held-back articles).

| Phase | Host | LLM latency | Wall time | Reducer time total | Slowest message kind (p95) | Bytes written per download | Prod lines | Test lines | Tests |
|---|---|---|---|---|---|---|---|---|---|
| 1 (baseline) | batch | 0 ms | 327 s | 8.7 s + 158.5 s state clones | TriageArticlesLoaded 201 ms | 27.3 MB | 90,667 (all Rust) | n/a | 1,509 passing (1,511 attributes) |
| 1 (baseline) | batch | 1500 ms | 339 s | 17.7 s + 158.1 s state clones | TriageArticlesLoaded 232 ms | 26.7 MB | | | |
| 1 (baseline) | desktop | 0 ms | 36 s | 10.8 s | JobDone 149 ms | 16.0 MB | | | |
| 1 (baseline) | desktop | 1500 ms | 46 s | 24.5 s | JobDone 135 ms | 16.4 MB | | | |
| 2 (0 model calls, see notes) | batch | 0 ms | 276 s | 5.0 s + 130.4 s state clones | JobDone 99 ms | 10.1 MB | 91,841 (all Rust) | n/a | 1,527 passing (1,537 attributes) |
| 2 (0 model calls) | batch | 1500 ms | 266 s | 5.0 s + 127.5 s state clones | JobDone 105 ms | 10.1 MB | | | |
| 2 (0 model calls) | desktop | 0 ms | 28 s | 3.8 s | JobDone 129 ms | 4.1 MB | | | |
| 2 (0 model calls) | desktop | 1500 ms | 28 s | 3.8 s | JobDone 147 ms | 4.1 MB | | | |

Phase 1 notes (2026-09-28, 40 held-back articles, synchronous model-call path, not `--batch-api`):

- The batch host copies the whole state 1,270 times per cycle at about 125 ms each; that is
  about half the wall time. The runtime state (`.harvester_state.ron`, 79 MB) is written 5
  times per cycle at about 10 s each, which confirms the known cost. Other large writers per
  cycle: `.entity_index.ron` (about 38 s of write time) and `.signal_candidate_cache.ron`
  (about 34 full rewrites, about 37 s).
- Desktop: "wall time" runs from startup to run completion. The driver loop accounts for
  14.2 s (0 ms latency) and 34.3 s (1500 ms); the rest is desktop startup and hydration. At
  0 ms latency the loop ran only 17 iterations, because each iteration reduces a large queued
  batch (p95 6 s), so the 75 ms ticks coalesce. At 1500 ms: 139 iterations, view builds
  6.4 s in total.
- `scripts/project-stats.ps1` reports total Rust lines only, so production and test lines are
  not split here; later phases compare the same total.
- Model-call counts vary between runs (summary and scoring: 30 to 33 of 38 triaged) although
  canned priorities are deterministic. Later phases should compare wall time per model call as
  well as totals, and Phase 10's per-article table should make this deterministic.
- The canned poll skipped 1 persisted RSS source (39 seen entries) beyond the five RSS-like
  slots. Reports are in `.local/bench/reports/`. IPC probe baseline:
  `.local/probe/ipc-report.simplification-phase1-baseline.json` (all gates passed). Pester: 26
  passed.

Pre-flight results (2026-09-28, read-only): the Batch API manifest holds one `Collected`
signal-candidate batch with 33 successful collected records, none outstanding at the provider,
and all 33 full keys already present in `.signal_candidate_cache.ron`. The Phase 4
reconciliation should therefore confirm 33 and append 0. The 22 hidden jobs are all from
February 2026: 9 have an article file whose frontmatter carries `fetched_utc` (recoverable in
Phase 7), and 13 have no article file (they stay hidden, per the owner's answer).

Phase 2 notes (2026-09-29):

- The Phase 2 benchmark rows are not comparable with the baseline: every run made 0 model
  calls. The owner's desktop run that morning had already processed the 40 newest articles
  the harness holds back, and the copied briefing checkpoint was later than their original
  fetch times. The pre-phase code (b293945) on the same source copy also made 0 model calls;
  the checkpoint filtered the held articles from processing. The expected fall in bytes written per completion is therefore not yet shown
  by the benchmark. The sink tests and the file-write report show it instead: no
  `.*_cache.*` file is rewritten, and results are appended only as new records. Follow-up
  before Phase 3's benchmark: clear the checkpoint in the work copy after selecting held
  articles, alongside dropping their saved results and completed-job records.
- The runs used a copy of `output/` without the new `.jsonl` files, so each run also migrated
  the RON stores at startup. The RON files stayed byte-identical.
- Migration of the owner's data (a scratch copy made during review, then the live desktop run
  on 2026-09-29): all 2,999 triage, 9,411 summary and 7,022 signal entries migrated. The
  `.ron` files were unchanged afterwards. The live run then appended about 60 results of each
  kind. Reading the live `.jsonl` files back with the final code loaded 3,062, 9,470 and 7,081
  entries, with no skipped or damaged lines.
- Test counts: root `cargo test` 1,501 → 1,527 (32 added, 6 removed); `harvester_ui` 8 → 8;
  Rust test attributes 1,511 → 1,537; vitest 95 → 96. The six removed tests all cover eviction,
  which this phase removes: `capacity_enforcement_evicts_oldest`,
  `evict_to_limit_removes_oldest`, `evict_to_limit_does_nothing_when_below_limit`,
  `evict_older_than_removes_old_entries` and `evict_by_ttl_removes_expired_entries`
  (`summary_cache.rs`; the TTL and older-than helpers had no other callers and went with
  them), and `capacity_guard_evicts_oldest_entry` (`triage_cache.rs`). The two corrupt-file
  tests were rewritten to assert refusal. No test file was deleted. The carry-over and archive
  fixtures are unchanged.
- The IPC probe and Pester were not run. The owner's desktop app held the GUI lock throughout,
  and this phase changes no launch script.

Phase 3 notes (2026-09-29):

- Regression coverage now includes the repeat-poll limit for RSS and Brave, pending intake
  across Stop and simulated restart, Resume without fetching, persisted compatibility with old
  state files, both lock-holder directions and release, host AI availability, lenient registry
  entries, and benchmark replays from a source folder that already has saved results and a
  briefing checkpoint later than the held articles. The
  carry-over replay and archive-export fixtures pass unchanged.
- Cancellation finding: `EngineHandle::stop(false)` drains queued downloads and emits cancelled
  completion events. Runtime persistence stores successful completed jobs only, so a download
  cancelled before its first progress event was not re-enqueued after restart. Accepted Stop now
  adds those queued URLs to the same pending-intake list as late poll results. Review follow-up:
  Full run retries a pending URL only when it has no job history or all its jobs ended cancelled;
  success during Stop removes that URL immediately. Batch cycle persistence writes the reducer's
  current list, including an empty list after consumption. Stop emits a full state snapshot only
  when it added pending URLs.
- Test counts: root `cargo test --offline` passed 1,540 tests (2 ignored), from the 1,527-pass
  baseline; `harvester_ui` passed 8 tests, unchanged; Rust test attributes are 1,550, from
  1,537 (13 added, 0 removed); Script-source tests had no constructed Script source to remove.
  Frontend Vitest remains at its 96-test baseline and was not rerun
  because frontend files were untouched. No test functions or test files were removed. The existing
  `full_run_after_stop_polls_again_and_enqueues_the_returned_url` regression was rewritten as
  `post_stop_poll_urls_persist_and_full_run_reingests_before_polling_after_restore`; the
  `poll_rss_source_applies_max_after_dedup` regression was strengthened in place.
- The replay smoke test now uses the first run's completed work folder as the next source, sets
  its briefing checkpoint later than the held articles, then verifies held-back articles still
  trigger model calls while the source tree stays byte-identical. The zero-call failure came from
  the copied checkpoint filtering articles by their original fetch times; the harness clears that
  checkpoint in its work copy after selecting held articles. This test covers the newer-checkpoint
  case with five held articles.
- The source-registry fixture now uses synthetic feeds, names, and queries with the same 24 Brave
  and 5 RSS entry shape, plus the two unsupported types.
  Benchmark table rows are left for the owner to run.
- Build, touched-crate tests, full root tests, both Clippy gates, and formatting checks passed.
  The IPC probe and Pester were not run; the real output-folder benchmark was not run.

Phase 4 notes — Stage A (2026-09-30; removal has not started):

- Added `harvester_batch/examples/reconcile_batch.rs`, with its nine tests in
  `examples/reconcile_support/tests.rs`. Default invocation is read-only, uses the
  existing JSONL DTOs/parser, ignores incomplete trailing lines without recovery,
  and never migrates RON backups or acquires a lock. Missing JSONL stores count as
  empty; a missing or unreadable manifest refuses the check. `--apply` takes the
  command-line host's `.harvester.lock`, appends only validated missing records
  through the coalescing result sink, flushes, and re-checks. Keys and timestamps
  come from the frozen manifest entries, including dated model aliases.
- Reports confirmed, missing, invalid and appended counts per kind, missing full
  key dimensions, invalid batch/custom ids and reasons, and Created/Submitted
  batches. Exit 0 means all successful collected records are confirmed, with no
  invalid or outstanding work; exit 1 reports discrepancies; exit 2 reports an
  I/O or manifest error. Temporary-folder CLI smoke checks verified all three exits.
- Test counts: root `cargo test --offline` 1,540 → 1,549 passing (2 ignored
  unchanged); Rust test attributes 1,550 → 1,559: 9 added, 0 removed. Removed-test
  list: none. Per-crate root totals (before → after, including integrations):
  batch 199 → 208 (the example has `test = true`), core 677 → 677, engine
  462 → 462 (1 ignored), IO 135 → 135, UI bridge 33 → 33, provider kit
  34 → 34 (1 ignored), logging 0 → 0. UI source and frontend are unchanged;
  their tests were not rerun (prior baselines: UI 8, frontend Vitest 96).
- The nine additions cover existing records in all three stores; apply plus a
  second check with exact keys/provenance; invalid records alongside a valid
  missing result; byte-identical folders with torn tails; no legacy migration or
  stale-migration deletion; outstanding states versus failed/line-error records;
  every frozen key dimension including model aliases; lock refusal naming the
  desktop holder while check still works; and missing/unreadable inputs.
- `cargo build --offline`, example build, touched-crate tests, full root tests,
  `cargo clippy --offline --all-targets -- -D warnings`, and formatting checks
  passed. The carry-over replay and archive-export golden fixtures passed
  unchanged. Logs are in `.local/checks/phase4-stage-a/`. Pester and IPC probe
  were not run; no launch policy, frontend or UI source changed. Benchmark table
  rows remain for the owner.
- The real `output/` was not read or written. Mandatory gate: owner/Claude runs
  the default read-only command from the repository root and returns its report:
  `cargo run --offline -p harvester_batch --example reconcile_batch -- "C:\Users\larsp\src\web_page_filet_mignon\output"`.
  The earlier pre-flight expected 33 confirmed signal-candidate records; the new
  check must establish the current count. Stage B waits for that report and
  proceeds on a clean pass under the owner's settled authorization. Missing,
  invalid or outstanding work stops for owner review. Stage B also removes the
  example's test module, Cargo target, temporary manifest re-exports, and snapshot
  reader if it has no remaining caller.

Phase 4 notes — Stage B (2026-09-30; implementation complete):

- Owner/Claude reconciliation, run read-only on 2026-09-30 against
  `C:\Users\larsp\src\web_page_filet_mignon\output`: exit **0**.

  ```text
  Mode: read-only
  triage: confirmed=0 missing=0 invalid=0 appended=0
  summary: confirmed=0 missing=0 invalid=0 appended=0
  signal_candidate: confirmed=33 missing=0 invalid=0 appended=0
  outstanding=0
  PASS: every successful collected record is confirmed; nothing invalid or outstanding.
  ```

  Claude checked SHA-256 hashes: the manifest and all three RON backups stayed
  byte-identical; no sidecar or temporary file was created. The 33 confirmed
  signal-candidate records match the Phase 1 pre-flight. This clean report met the
  owner's settled option A authorization; Stage B proceeded without another
  confirmation. Codex did not read or write the real output folder.
- Removed Batch API transport, coordinator, manifest, collection/runtime and
  batch-only core contracts, deferral state, counters, allowance and Continue
  scope. The temporary reconciliation example/support module, Cargo target,
  manifest re-export and unused read-only snapshot helper were removed too.
  Every remaining article call uses the synchronous request budget, with scoring,
  summary, then triage priority and overlapping waves. Replay records remain;
  the synchronous path is their sole production writer.
- Default command-line invocation performs one Full cycle and exits; the removed
  flags and repeating/provider-wait loops are gone. Import and checkpoint editing
  remain. The fixed Batch launcher supplies an empty runtime-argument array. Its
  four assertions now check count zero, and the policy-copy test replaces the
  empty argument array instead of indexing its nonexistent first element.
- Bug 8: dispatch, browser import and startup hydration use
  `NO_PROGRESS_TIMEOUT` (60 seconds, monotonic time). A received message or
  in-flight effect resets the deadline; pending, undispatched model work alone
  does not. A failure logs the stuck operation and pipeline/import phase through
  `engine_logging`. The operation diagnostic is built only when the watchdog
  expires. Quiet downloads do not exhaust an iteration cap. Bug 1 is resolved by
  removing dry-run. Stop still drains already in-flight work.
- Review fixes preserve synchronous contracts: the budget clamp and triage-wave
  summary barrier tests are rewritten below. All dashboard paints and resumes
  receive the worker's synchronous session cost; a regression checks both terminal
  and plain output. Import has its own quiet-loop regression. Removed dead host
  runtime data, unused dispatch sender parameters and the single-variant
  `PipelineWavePolicy`; run admission now directly controls overlapping intake.
  Test-only wall time and sleep live on a clock extension trait. The provider
  changelog retains 0.3.0 verbatim and records API/codec removal in 0.4.0;
  multipart is removed and the lockfile resolves offline. Probe copy no longer
  mentions deferred batch cycles; no IPC fixture embeds that string, so shape and
  schema are unchanged. The empty reconciliation support directory is removed.
- Review verification passed: `cargo build --offline`; touched-crate tests,
  then full root `cargo test --offline` (**1,461 passed, 2 ignored**);
  `cargo clippy --offline --all-targets -- -D warnings`;
  `cargo clippy --offline -p harvester_ui --all-targets -- -D warnings`;
  `cargo fmt` and `cargo fmt --check`. The desktop binary was neither built nor
  run; its host wiring was checked by Clippy. Carry-over/archive fixtures and IPC
  snapshot fixtures passed unchanged. Frontend files did not change. Logs are
  under `.local/phase4-review-*.log`. Pester remains for Claude to run.
- Both `replay_bench` hosts build and run offline against a temporary copy of the
  synthetic carry-over fixture (one held-back article), with `completed=true`
  and `model_call_path=synchronous`. This is a harness smoke check, not a
  corpus-sized performance comparison. Benchmark table rows remain for the owner.
  Logs/reports are under `.local/checks/phase4-stage-b/`.
- The crates/scripts/README scan contains no removed Batch API/mode usage. The
  remaining `drain` matches concern Stop, queue/message draining or host cost
  fixtures; the ignored engine `real_corpus_dry_run` is a content-preparation
  diagnostic, unrelated to the removed CLI mode, and was not run. Corpus marker
  generation contains no explicit Batch artifact; public layout is unchanged.
  Pester was not run by Codex. Follow-ups: Claude runs
  `Invoke-Pester -Path scripts/tests/HarvesterLaunch.Tests.ps1`; owner records
  the full-corpus benchmark rows and observes the first normal morning cycle.

Phase 4 test-count reconciliation (baseline before Stage A):

| Count | Baseline | Stage A | Stage B/final |
| --- | ---: | ---: | ---: |
| Rust test attributes | 1,550 | 1,559 | 1,471 |
| Root passing tests | 1,540 | 1,549 | 1,461 |
| Root ignored tests | 2 | 2 | 2 |

- Stage A: 9 additions, 0 removals. Stage B: 7 additions, 95 removals,
  including those same 9 temporary reconciliation tests. Across the phase:
  **1,550 + 16 - 95 = 1,471 attributes** and
  **1,540 + 16 - 95 = 1,461 passing root tests**. The baseline loses 86
  feature-specific tests; 17 renamed/reworked tests retain preserved behavior and have
  no count effect. Unrenamed mixed tests were also edited in place. No preserved
  test file was deleted. The removal exceeds the plan's rough estimate because
  the existing provider progress, wait scheduling, reporting and flag-conflict
  regressions also belonged exclusively to the retired path.

| Crate (root totals, integrations included) | Before Stage A | After Stage A | Final passed | Ignored |
| --- | ---: | ---: | ---: | ---: |
| harvester_batch | 199 | 208 | 135 | 0 |
| harvester_core | 677 | 677 | 667 | 0 |
| harvester_engine | 462 | 462 | 461 | 1 |
| harvester_io | 135 | 135 | 135 | 0 |
| harvester_ui_bridge | 33 | 33 | 33 | 0 |
| openai_provider_kit | 34 | 34 | 30 | 1 |
| engine_logging | 0 | 0 | 0 | 0 |

Added Stage B regressions (7):

- `dispatch_loop_survives_more_than_ten_thousand_quiet_download_iterations` (`crates/harvester_batch/src/runner/tests.rs`).
- `cli_with_no_flags_polls_one_cycle_persists_and_exits` (`crates/harvester_batch/tests/result_store_refusal.rs`).
- `quiet_downloads_can_outlast_the_old_iteration_cap` (`crates/harvester_batch/src/no_progress.rs`).
- `messages_restart_the_no_progress_deadline` (`crates/harvester_batch/src/no_progress.rs`).
- `watchdog_fires_only_after_a_full_idle_duration_without_in_flight_work` (`crates/harvester_batch/src/no_progress.rs`).
- `import_loop_survives_more_than_ten_thousand_quiet_import_iterations` (`crates/harvester_batch/src/import_mode.rs`).
- `painted_cost_reflects_synchronous_session_usage` (`crates/harvester_batch/src/runner/live_progress.rs`).

Renamed/reworked tests with preserved behavior (17; no count change):

- `deferred_allowance_is_separate_from_clamped_synchronous_budget` → `synchronous_budget_is_clamped_and_zero_dispatches_one_triage_request` (`crates/harvester_core/src/update/model_dispatch_tests.rs`): both clamp limits remain, plus dispatch at the clamped minimum; only the removed deferred allowance assertions are dropped.
- `deferred_triage_replays_once_and_does_not_block_completed_wave_members` → `in_flight_triage_member_blocks_wave_summaries_until_last_completion` (`crates/harvester_core/src/update/pipeline_run/wave_tests.rs`): a two-member synchronous wave holds summaries until the last triage completion, then emits both requests in that reducer step; only deferred/replay assertions are dropped.
- `formatter_exact_wide_dashboard_for_provider_wait_replay_complete_and_interrupted` → `formatter_exact_wide_dashboard_for_complete_and_interrupted` (`crates/harvester_batch/src/progress/dashboard.rs`).
- `formatter_zero_totals_and_stale_provider_counts_never_make_fake_or_overfull_bars` → `formatter_zero_totals_and_overfull_counts_never_make_fake_or_overfull_bars` (`crates/harvester_batch/src/progress/dashboard.rs`).
- `rearm_transition_cannot_shrink_latched_stage_total` → `settlement_cannot_shrink_latched_stage_total` (`crates/harvester_batch/src/progress/projection.rs`).
- `rearm_can_replace_pending_urls_without_shrinking_latched_total` → `pending_window_can_shrink_without_shrinking_latched_total` (`crates/harvester_batch/src/progress/projection.rs`).
- `synchronous_batch_defaults_to_worker_cap_without_deferred_allowance` → `synchronous_batch_defaults_to_worker_cap` (`crates/harvester_batch/src/runner/bootstrap.rs`).
- `terminal_progress_stays_live_across_collection_passes_and_finishes_once` → `terminal_progress_stays_live_across_stages_and_finishes_once` (`crates/harvester_batch/src/runner/live_progress.rs`).
- `recurring_cycle_retries_a_failed_article_without_new_jobs` → `later_cycle_retries_a_failed_article_without_new_jobs` (`crates/harvester_batch/src/runner/tests.rs`).
- `single_shot_cycle_processes_unfinished_work_without_new_jobs` → `default_cycle_processes_unfinished_work_without_new_jobs` (`crates/harvester_batch/src/runner/tests.rs`).
- `recurring_staggered_downloads_dispatch_triage_and_settle_once` → `synchronous_staggered_downloads_dispatch_triage_and_settle_once` (`crates/harvester_batch/src/runner/tests.rs`).
- `changed_digest_preserves_pending_scoring_and_deferred_admission` → `changed_digest_preserves_pending_and_scoring_admission` (`crates/harvester_core/src/signal_candidate.rs`).
- `observation_counts_include_accumulated_rearmed_members` → `observation_counts_include_current_members_after_readmission` (`crates/harvester_core/src/signal_candidate.rs`).
- `failed_triage_needs_triage_and_deferred_triage_is_in_progress` → `failed_triage_needs_triage_and_active_triage_is_in_progress` (`crates/harvester_core/src/update/tests/unfinished_work_tests.rs`).
- `rearmed_pending_triage_with_old_snapshot_remains_in_progress` → `admitted_pending_triage_with_old_snapshot_remains_in_progress` (`crates/harvester_core/src/update/tests/unfinished_work_tests.rs`).
- `admitted_pending_and_deferred_summaries_are_in_progress` → `admitted_pending_and_active_summaries_are_in_progress` (`crates/harvester_core/src/update/tests/unfinished_work_tests.rs`).
- `deferred_scoring_is_in_progress_under_its_current_key` → `active_scoring_is_in_progress_under_its_current_key` (`crates/harvester_core/src/update/tests/unfinished_work_tests.rs`).
- The existing shutdown predicate test now drives the real dispatch boundary and checks exit 130; the existing progress flag, formatter, source/model diagnostic and synchronous quota tests remain. Pending/in-flight identity classification, admission snapshots, signal counters, latch totals, rendering and wave overlap remain covered.

Every removed test (95, including the 9 added-then-removed reconciliation tests):

**`crates/harvester_batch/examples/reconcile_support/tests.rs` (9)**

| Removed test | Feature covered |
| --- | --- |
| `records_already_in_each_store_are_confirmed` | Temporary collected-result reconciliation: records already in each store are confirmed |
| `apply_appends_missing_records_then_second_check_confirms_exact_frozen_keys_and_provenance` | Temporary collected-result reconciliation: apply appends missing records then second check confirms exact frozen keys and provenance |
| `invalid_successful_records_are_listed_and_never_appended_even_if_key_is_present` | Temporary collected-result reconciliation: invalid successful records are listed and never appended even if key is present |
| `read_only_check_leaves_entire_folder_byte_identical_including_torn_tails` | Temporary collected-result reconciliation: read only check leaves entire folder byte identical including torn tails |
| `read_only_check_neither_migrates_legacy_stores_nor_removes_stale_migrations` | Temporary collected-result reconciliation: read only check neither migrates legacy stores nor removes stale migrations |
| `created_and_submitted_batches_block_pass_but_failed_and_line_errors_do_not` | Temporary collected-result reconciliation: created and submitted batches block pass but failed and line errors do not |
| `every_frozen_key_dimension_must_match_exactly_including_model_alias` | Temporary collected-result reconciliation: every frozen key dimension must match exactly including model alias |
| `apply_refuses_desktop_lock_with_holder_named_and_check_can_read_while_locked` | Temporary collected-result reconciliation: apply refuses desktop lock with holder named and check can read while locked |
| `missing_or_unreadable_manifest_and_unreadable_store_fail_without_creating_files` | Temporary collected-result reconciliation: missing or unreadable manifest and unreadable store fail without creating files |

**`crates/harvester_batch/src/batch_coordinator.rs` (13)**

| Removed test | Feature covered |
| --- | --- |
| `fake_transport_batch_drain_reports_the_full_multistage_progress_lifecycle` | Batch API submission, collection and recovery: fake transport batch drain reports the full multistage progress lifecycle |
| `submit_reserves_attaches_and_defers_the_request` | Batch API submission, collection and recovery: submit reserves attaches and defers the request |
| `oversized_stage_group_is_chunked_by_line_cap` | Batch API submission, collection and recovery: oversized stage group is chunked by line cap |
| `submission_budget_stops_new_batches_and_leaves_remainder_deferred` | Batch API submission, collection and recovery: submission budget stops new batches and leaves remainder deferred |
| `duplicate_custom_id_uploads_once_and_replies_to_every_request` | Batch API submission, collection and recovery: duplicate custom id uploads once and replies to every request |
| `upload_failure_fails_group_without_manifest_reservation` | Batch API submission, collection and recovery: upload failure fails group without manifest reservation |
| `create_failure_releases_reservation_and_fails_group` | Batch API submission, collection and recovery: create failure releases reservation and fails group |
| `stage_partition_submits_one_batch_per_stage` | Batch API submission, collection and recovery: stage partition submits one batch per stage |
| `reconciliation_paginates_until_reserved_file_is_found` | Batch API submission, collection and recovery: reconciliation paginates until reserved file is found |
| `cancelled_batch_salvages_paid_output_and_releases_unreturned_requests` | Batch API submission, collection and recovery: cancelled batch salvages paid output and releases unreturned requests |
| `cancelled_batch_without_output_releases_every_entry` | Batch API submission, collection and recovery: cancelled batch without output releases every entry |
| `terminal_batch_is_retained_when_the_salvage_download_fails` | Batch API submission, collection and recovery: terminal batch is retained when the salvage download fails |
| `collected_snapshot_replays_after_restart_until_cache_confirmation` | Batch API submission, collection and recovery: collected snapshot replays after restart until cache confirmation |

**`crates/harvester_batch/src/batch_manifest.rs` (4)**

| Removed test | Feature covered |
| --- | --- |
| `round_trip_and_reserve_without_attach_are_durable` | Batch API manifest durability and lifecycle: round trip and reserve without attach are durable |
| `manifest_stage_labels_round_trip_through_typed_stage_keys` | Batch API manifest durability and lifecycle: manifest stage labels round trip through typed stage keys |
| `corrupt_manifest_fails_closed` | Batch API manifest durability and lifecycle: corrupt manifest fails closed |
| `failed_and_line_error_batches_are_pruned_without_losing_attempt_count` | Batch API manifest durability and lifecycle: failed and line error batches are pruned without losing attempt count |

**`crates/harvester_batch/src/cli.rs` (6)**

| Removed test | Feature covered |
| --- | --- |
| `single_shot_flag_is_parsed` | Removed CLI mode/interval flags and their conflicts: single shot flag is parsed |
| `single_shot_conflicts_with_dry_run` | Removed CLI mode/interval flags and their conflicts: single shot conflicts with dry run |
| `batch_api_parses_and_conflicts_with_excluded_modes` | Removed CLI mode/interval flags and their conflicts: batch api parses and conflicts with excluded modes |
| `drain_implies_batch_api_and_conflicts_with_excluded_modes` | Removed CLI mode/interval flags and their conflicts: drain implies batch api and conflicts with excluded modes |
| `poll_interval_is_clamped` | Removed CLI mode/interval flags and their conflicts: poll interval is clamped |
| `refresh_stale_summaries_limit_conflicts_with_dry_run` | Removed CLI mode/interval flags and their conflicts: refresh stale summaries limit conflicts with dry run |

**`crates/harvester_batch/src/import_mode.rs` (2)**

| Removed test | Feature covered |
| --- | --- |
| `import_saved_web_dir_conflicts_with_dry_run` | Import conflicts with removed CLI modes: import saved web dir conflicts with dry run |
| `import_saved_web_dir_conflicts_with_single_shot` | Import conflicts with removed CLI modes: import saved web dir conflicts with single shot |

**`crates/harvester_batch/src/progress/dashboard.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `formatter_shows_preparing_and_partially_submitted_provider_scopes` | Batch API provider-scope dashboard: formatter shows preparing and partially submitted provider scopes |

**`crates/harvester_batch/src/progress/projection.rs` (7)**

| Removed test | Feature covered |
| --- | --- |
| `provider_progress_is_provisional_without_replacing_local_denominator` | Batch API provider progress and wait presentation: provider progress is provisional without replacing local denominator |
| `budget_capped_provider_scope_keeps_unsubmitted_work_visible` | Batch API provider progress and wait presentation: budget capped provider scope keeps unsubmitted work visible |
| `same_stage_peeks_aggregate_by_typed_stage_after_chunking` | Batch API provider progress and wait presentation: same stage peeks aggregate by typed stage after chunking |
| `custom_id_text_cannot_change_typed_provider_grouping` | Batch API provider progress and wait presentation: custom id text cannot change typed provider grouping |
| `provider_lookup_failure_is_indeterminate` | Batch API provider progress and wait presentation: provider lookup failure is indeterminate |
| `deferred_work_without_peeks_is_preparing_not_zero_request_wait` | Batch API provider progress and wait presentation: deferred work without peeks is preparing not zero request wait |
| `local_wait_timestamps_keep_fixed_offsets` | Batch API provider progress and wait presentation: local wait timestamps keep fixed offsets |

**`crates/harvester_batch/src/runner/batch_runtime.rs` (3)**

| Removed test | Feature covered |
| --- | --- |
| `batch_confirmation_reads_new_jsonl_records_for_all_three_kinds` | Batch API collection cache confirmation and replay index: batch confirmation reads new jsonl records for all three kinds |
| `replay_line_index_is_built_from_filenames_without_reading_contents` | Batch API collection cache confirmation and replay index: replay line index is built from filenames without reading contents |
| `missing_replay_dir_yields_empty_index` | Batch API collection cache confirmation and replay index: missing replay dir yields empty index |

**`crates/harvester_batch/src/runner/bootstrap.rs` (2)**

| Removed test | Feature covered |
| --- | --- |
| `batch_api_sets_session_allowance_without_overwriting_sync_budget` | Batch API allowance and collect-only bootstrap: batch api sets session allowance without overwriting sync budget |
| `drain_bootstrap_disables_unarmed_intake_refreshes` | Batch API allowance and collect-only bootstrap: drain bootstrap disables unarmed intake refreshes |

**`crates/harvester_batch/src/runner/live_progress.rs` (5)**

| Removed test | Feature covered |
| --- | --- |
| `provider_lookup_failure_retains_last_successful_counts_until_a_successful_retry` | Batch API provider retry/wait scheduling: provider lookup failure retains last successful counts until a successful retry |
| `provider_wait_uses_second_heartbeats_without_extra_peeks_between_deadlines` | Batch API provider retry/wait scheduling: provider wait uses second heartbeats without extra peeks between deadlines |
| `provider_wait_shutdown_is_observed_at_the_half_second_local_poll_boundary` | Batch API provider retry/wait scheduling: provider wait shutdown is observed at the half second local poll boundary |
| `provider_wait_marks_checking_before_a_blocking_peek_and_observes_shutdown_after_it_returns` | Batch API provider retry/wait scheduling: provider wait marks checking before a blocking peek and observes shutdown after it returns |
| `local_heartbeat_wait_observes_shutdown_within_half_a_second` | Batch API provider retry/wait scheduling: local heartbeat wait observes shutdown within half a second |

**`crates/harvester_batch/src/runner/reporting.rs` (6)**

| Removed test | Feature covered |
| --- | --- |
| `format_awaiting_batch_line_is_absent_when_nothing_deferred` | Batch API deferred, collection, cost and drain reporting: format awaiting batch line is absent when nothing deferred |
| `format_awaiting_batch_line_reports_per_stage_and_total_counts` | Batch API deferred, collection, cost and drain reporting: format awaiting batch line reports per stage and total counts |
| `verbose_wait_timestamp_uses_injected_local_offset` | Batch API deferred, collection, cost and drain reporting: verbose wait timestamp uses injected local offset |
| `batch_api_final_summary_distinguishes_intake_from_collection_passes_and_cost_scope` | Batch API deferred, collection, cost and drain reporting: batch api final summary distinguishes intake from collection passes and cost scope |
| `drain_summary_reports_batches_left_pending_for_a_later_run` | Batch API deferred, collection, cost and drain reporting: drain summary reports batches left pending for a later run |
| `batch_drain_bailout_prints_remaining_stage_counts_without_changing_exit_code` | Batch API deferred, collection, cost and drain reporting: batch drain bailout prints remaining stage counts without changing exit code |

**`crates/harvester_batch/src/runner/tests.rs` (22)**

| Removed test | Feature covered |
| --- | --- |
| `collect_only_replay_dispatches_and_settles_without_pipeline_advance` | Batch API routing/collection or removed CLI mode control: collect only replay dispatches and settles without pipeline advance |
| `empty_collect_only_cycle_settles_without_admitting_the_window` | Batch API routing/collection or removed CLI mode control: empty collect only cycle settles without admitting the window |
| `batch_api_collects_replays_and_releases_summary_waves_once_across_three_cycles` | Batch API routing/collection or removed CLI mode control: batch api collects replays and releases summary waves once across three cycles |
| `test_dry_run_exits_successfully_without_api_key` | Batch API routing/collection or removed CLI mode control: dry run exits successfully without api key |
| `dry_run_returns_130_when_shutdown_is_already_requested` | Batch API routing/collection or removed CLI mode control: dry run returns 130 when shutdown is already requested |
| `test_dry_run_does_not_modify_state_files` | Batch API routing/collection or removed CLI mode control: dry run does not modify state files |
| `buffered_batch_requests_are_quiescent_but_other_pending_requests_are_not` | Batch API routing/collection or removed CLI mode control: buffered batch requests are quiescent but other pending requests are not |
| `batch_custom_id_changes_when_model_changes` | Batch API routing/collection or removed CLI mode control: batch custom id changes when model changes |
| `signal_custom_id_prefix_does_not_control_provider_stage_grouping` | Batch API routing/collection or removed CLI mode control: signal custom id prefix does not control provider stage grouping |
| `batch_routing_partition_keeps_briefing_synchronous` | Batch API routing/collection or removed CLI mode control: batch routing partition keeps briefing synchronous |
| `batch_render_failure_replies_failed_exactly_once` | Batch API routing/collection or removed CLI mode control: batch render failure replies failed exactly once |
| `collected_replay_audit_is_idempotent_and_uses_discounted_cost` | Batch API routing/collection or removed CLI mode control: collected replay audit is idempotent and uses discounted cost |
| `test_should_stop_after_cycle_for_single_shot` | Batch API routing/collection or removed CLI mode control: should stop after cycle for single shot |
| `test_should_continue_after_cycle_when_not_single_shot_and_no_shutdown` | Batch API routing/collection or removed CLI mode control: should continue after cycle when not single shot and no shutdown |
| `drain_makes_the_first_cycle_collect_only_so_no_sources_are_polled` | Batch API routing/collection or removed CLI mode control: drain makes the first cycle collect only so no sources are polled |
| `batch_api_intake_waits_for_downloads_and_hands_off_once` | Batch API routing/collection or removed CLI mode control: batch api intake waits for downloads and hands off once |
| `drain_with_restored_startup_work_issues_no_model_request_and_terminates` | Batch API routing/collection or removed CLI mode control: drain with restored startup work issues no model request and terminates |
| `batch_wait_keeps_waiting_when_all_peeked_batches_are_nonterminal` | Batch API routing/collection or removed CLI mode control: batch wait keeps waiting when all peeked batches are nonterminal |
| `batch_wait_runs_collect_cycle_when_a_peeked_batch_is_terminal` | Batch API routing/collection or removed CLI mode control: batch wait runs collect cycle when a peeked batch is terminal |
| `batch_wait_runs_collect_cycle_when_no_batches_can_be_peeked` | Batch API routing/collection or removed CLI mode control: batch wait runs collect cycle when no batches can be peeked |
| `batch_drain_progress_compares_manifest_and_deferred_work` | Batch API routing/collection or removed CLI mode control: batch drain progress compares manifest and deferred work |
| `batch_drain_exits_after_second_consecutive_no_progress_cycle` | Batch API routing/collection or removed CLI mode control: batch drain exits after second consecutive no progress cycle |

**`crates/harvester_core/src/pre_triage_coordinator.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `after_downloads_policy_never_uses_max_wait_to_release_a_partial_intake` | Removed wait-for-downloads wave policy: after downloads policy never uses max wait to release a partial intake |

**`crates/harvester_core/src/update/pipeline_run/tests.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `deferred_signal_rearm_starts_a_new_continue_run` | Batch API rearm and Continue lifecycle: deferred signal rearm starts a new continue run |

**`crates/harvester_core/src/update/pipeline_run/wave_tests.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `batch_buffering_keeps_a_single_admission_wave` | Batch API buffering and deferred replay waves: batch buffering keeps a single admission wave |

**`crates/harvester_core/src/update/tests/batch_api_tests.rs` (5)**

| Removed test | Feature covered |
| --- | --- |
| `deferred_triage_settles_and_rearm_redispatches` | Batch API deferral, collected-result validation and rearm: deferred triage settles and rearm redispatches |
| `deferred_summary_settles_without_failing_article` | Batch API deferral, collected-result validation and rearm: deferred summary settles without failing article |
| `collected_successes_insert_frozen_keys_coalesce_persistence_and_do_not_complete_articles` | Batch API deferral, collected-result validation and rearm: collected successes insert frozen keys coalesce persistence and do not complete articles |
| `collected_line_error_and_invalid_output_do_not_write_cache` | Batch API deferral, collected-result validation and rearm: collected line error and invalid output do not write cache |
| `collected_summary_rearm_cache_hits_and_runs_post_processing_once` | Batch API deferral, collected-result validation and rearm: collected summary rearm cache hits and runs post processing once |

**`crates/harvester_core/src/update/tests/signal_candidate_tests.rs` (2)**

| Removed test | Feature covered |
| --- | --- |
| `deferred_only_signal_work_is_settled` | Batch API scoring deferral and collected-result rearm: deferred only signal work is settled |
| `deferred_signal_round_trip_rearms_directly_and_completes_from_collected_cache` | Batch API scoring deferral and collected-result rearm: deferred signal round trip rearms directly and completes from collected cache |

**`crates/harvester_engine/src/llm/pricing.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `batch_price_is_half_of_standard_without_cached_input_tier` | Batch API discounted pricing: batch price is half of standard without cached input tier |

**`crates/openai_provider_kit/src/batch.rs` (4)**

| Removed test | Feature covered |
| --- | --- |
| `jsonl_round_trip` | OpenAI Batch API transport and JSONL codecs: jsonl round trip |
| `completed_batch_parses` | OpenAI Batch API transport and JSONL codecs: completed batch parses |
| `mixed_output_jsonl_parses` | OpenAI Batch API transport and JSONL codecs: mixed output jsonl parses |
| `unknown_lifecycle_is_rejected` | OpenAI Batch API transport and JSONL codecs: unknown lifecycle is rejected |

Phase 5 notes (2026-09-30; implementation complete):

- Removed aggregate executive briefing, stream/history, stale-summary refresh and concatenated export. Per-article summaries remain in the live BriefingSession; settlement has no aggregate branch or switch. Removed replay provider lookup and Prompt Lab one-off overrides, keeping replay writes and saved overlay loading. Retained checkpoint names and the indexed corpus loader. Manual pre-triage decisions are fixture support.
- Review follow-up: the linked-page effect/runner/completion/marking chain became unreachable when LinkToggleRequested was removed. It remains unchanged here and is removed in Phase 6 work item 3 together with LinkDownloadState and the IPC bump.
- Phase 6 must regenerate run_finished_with_notice.json and ai_unavailable.json and remove the retired-field blanking workaround in the snapshot projection comparison; no fixture bytes or IPC fields change here.
- Polling fixtures now drive PipelineRunRequested { scope: Full } and consume its configuration effect. The refresh-coordinator isolation tests keep a local helper inside the cfg(test) module; fixture_poll_sources and the unreachable corpus-clear handler, fixture and two tests are removed. Obsolete allow(deprecated) attributes are gone.
- JSONL and legacy RON summary stores skip AggregateBriefing entries with engine_logging warnings naming the file and skipped count. JSONL retirement/unknown-prompt counts are aggregated separately from malformed-line counts; the regression checks both classification and absence of per-line warnings. Other paid entries load without refusing the store; RON backups and existing retired artifacts remain untouched.
- The marker lists exactly archive.md and archive-*.md. CORPUS_SCHEMA_VERSION remains 1. IPC_SCHEMA_VERSION remains 12; retired controls remain present and false/empty. Snapshot fixture bytes are unchanged: their decoder inputs remain, while the projection test asserts the three retired values and compares every other field unchanged.
- Backlog review: added the missing FI-LLM-Briefing-0003 retirement line, restored FI-Storage-ExportArtifacts-0003 for per-article cycle results, and recorded FI-Storage-CorpusScanning-0001 for skip-and-warn scanning of unreadable articles. The live scanner remains unchanged. Corrected the removed selected-URL loader test label.
- Archive golden fixtures, carry-over fixture and IPC decoding/projection tests pass. Summary resolution keeps live-session-first then newest content-hash entry under any key; archive and estimate lookups remain lookup_any_by_content_hash. Frontend files and launch policy are unchanged; frontend checks were not rerun (prior Vitest baseline: 96).
- Verification: cargo build --offline; touched-crate tests followed by full root cargo test --offline; cargo clippy --offline --all-targets -- -D warnings; cargo clippy --offline -p harvester_ui --all-targets -- -D warnings; cargo fmt and cargo fmt --check. All passed. Pester, the GUI IPC probe and owner-only project-stats.ps1 were not run. No keys, live LLM APIs or real output folder were used. Changes remain uncommitted.
- Both replay_bench host harnesses built and completed using temporary synthetic carry-over copies, one held-back article, zero synthetic model latency, synchronous requests. Batch wall time 35.5 ms; desktop 157.3 ms. These are smoke checks, not corpus-sized performance comparisons or before/after speed claims. Reports are .local/bench/phase5-review-batch/report.json and .local/bench/phase5-review-desktop/report.json; check logs are under .local/bench/phase5-checks/. Owner may record full-corpus benchmark/size rows on the next normal run.

Phase 5 test-count reconciliation:

| Count | Baseline | Final |
| --- | ---: | ---: |
| Rust test attributes | 1,471 | 1,319 |
| Root passing tests | 1,461 | 1,309 |
| Root ignored tests | 2 | 2 |

**1,471 + 4 - 156 = 1,319 attributes** and **1,461 + 4 - 156 = 1,309 root passing tests**. Twenty-two renamed/reworked tests have no count effect. The eight unchanged harvester_ui test attributes remain outside the default-member root test run. Of the removals, 67 are in the permitted wholesale deletions and 89 are edited out of surviving files. The extra removals beyond the plan's rough estimate are aggregate domain/readiness/validation tests, retired corpus selector tests, override/replay lookup tests and unreachable link-toggle tests. Preserved cache, quota, halt, Stop, ordering, cutoff, scan exclusion, archive bytes and IPC contracts retain their tests.

| Crate | Root passed before | Root passed after | Ignored after |
| --- | ---: | ---: | ---: |
| harvester_batch | 135 | 104 | 0 |
| harvester_core | 667 | 596 | 0 |
| harvester_engine | 461 | 417 | 1 |
| harvester_io | 135 | 129 | 0 |
| harvester_ui_bridge | 33 | 33 | 0 |
| openai_provider_kit | 30 | 30 | 1 |
| engine_logging | 0 | 0 | 0 |

Phase 5 added regressions (4):

| File | Added test | Contract |
| --- | --- | --- |
| `crates/harvester_core/src/update/tests/summary_settlement_tests.rs` | `every_summary_settle_path_saves_successes_and_emits_only_article_requests` | Success, partial/all failure, both quota origins, rate limit, Stop drain and cache-hit settlement retain saved successes and emit only article requests |
| `crates/harvester_io/tests/retired_summary_entries.rs` | `jsonl_skips_aggregate_entries_warns_with_file_and_count_and_keeps_paid_results` | Mixed retired/current prompt entries load, warn with store/count, retain paid results and do not refuse hydration |
| `crates/harvester_io/tests/retired_summary_entries.rs` | `ron_migration_skips_aggregate_entries_warns_with_file_and_count_and_keeps_paid_results` | Mixed retired/current prompt entries load, warn with store/count, retain paid results and do not refuse hydration |
| `crates/harvester_batch/src/progress/import_reporter.rs` | `format_elapsed_formats_minutes_and_padded_seconds` | Zero, sub-minute, minute rollover, long durations and sub-second truncation preserve elapsed formatting |

The removed concatenated_export_writes_corpus_manifest test is restored as triage_archive_writes_and_refreshes_corpus_manifest: archive production writes and refreshes the marker with format harvester-corpus, schema 1 and exactly archive.md/archive-*.md. This adds one back to the pre-review count; the new elapsed test adds one and the two unreachable corpus-clear tests subtract two, leaving the final count unchanged.

The existing manifest_records_current_schema_version_and_layout regression now pins the exact two generated artifacts and schema 1. The existing migration test was rewritten for skip semantics; no fixture bytes changed.

Phase 5 renamed/reworked tests (22; no count effect):

| Before | After |
| --- | --- |
| `concatenated_export_writes_corpus_manifest` | `triage_archive_writes_and_refreshes_corpus_manifest` |
| `test_dispatch_loop_reduces_queued_poll_before_settling` | `test_dispatch_loop_drains_full_run_poll_effects_before_settling` |
| `cli_summary_refresh_refuses_any_damaged_store_before_model_setup` | `cli_one_cycle_refuses_any_damaged_store_before_model_setup` |
| `briefing_complete_then_job_selected_shows_the_selected_summary` | `summaries_complete_then_job_selected_shows_the_selected_summary` |
| `resume_run_sets_current_working_corpus_to_unavailable_until_triage_completes` | `resume_run_makes_archive_corpus_available_when_triage_completes` |
| `generate_briefing_loads_archive_final_selection` | `archive_selection_loads_archive_final_selection` |
| `generate_briefing_loads_signal_filtered_archive_final_selection` | `archive_selection_loads_signal_filtered_archive_final_selection` |
| `generate_briefing_preserves_signal_order_and_honors_exclusions` | `archive_selection_preserves_signal_order_and_honors_exclusions` |
| `generate_briefing_cache_hit_reuses_summary_for_aligned_selection` | `archive_selection_reuses_cached_summaries_under_any_key` |
| `prepare_summaries_loads_base_corpus_skip_aggregate` | `prepare_summaries_loads_base_corpus` |
| `summary_completion_advances_and_generates_briefing` | `summary_completion_advances_and_saves_without_aggregate` |
| `aggregate_briefing_success_records_usage_for_status_bar` | `summary_success_records_usage_for_status_bar` |
| `briefing_generate_enabled_true_when_summaries_settled_and_signal_idle` | `retired_briefing_controls_are_empty_when_summaries_settled` |
| `briefing_quota_exhaustion_reclassifies_all_pending_article_work` | `triage_quota_exhaustion_reclassifies_all_pending_article_work` |
| `update_is_noop` | `advance_without_a_run_is_noop` |
| `resume_run_enters_the_pipeline_after_briefing_readiness_failure` | `resume_run_enters_the_pipeline_with_retired_briefing_controls_disabled` |
| `path_based_loading_preserves_same_url_duplicate_entries` | `imported_same_url_duplicate_entries_remain_separate_files` |
| `single_article_is_loaded_and_in_collection` | `single_article_is_loaded_and_prepared` |
| `triage_delta_and_summary_collection_use_identical_preparation` | `triage_delta_preserves_summary_hash_and_preparation_budget` |
| `stage_model_wins_over_default_when_override_is_none` | `stage_model_wins_over_default` |
| `triage_loader_shared_scanning_matches_briefing` | `triage_loader_shared_scanning_matches_archive_metadata` |
| `retired_aggregate_briefing_summary_entries_still_migrate` | `retired_aggregate_briefing_summary_entries_are_skipped_during_migration` |

Phase 5 tests reworked in place (including live-summary and manual-fixture harness changes; no count effect):

- `crates/harvester_batch/src/runner/tests.rs`: `synchronous_staggered_downloads_dispatch_triage_and_settle_once`.
- `crates/harvester_core/src/briefing.rs`: `briefing_progress_text_shows_failure_reason`.
- `crates/harvester_core/src/pre_triage_filter.rs`: `borrowed_included_article_queries_follow_verdict_and_delta_changes`.
- `crates/harvester_core/src/state/batch.rs`: `cache_derived_archive_display_tracks_delta_verdict_cache_and_metadata_changes`.
- `crates/harvester_core/src/state/tests/mod.rs`: `pre_triage_actionability_is_ready_with_pending_review_when_reviewing`, `unrequested_review_work_does_not_enter_a_stage_queue`, `batch_status_is_running_when_summary_article_load_is_in_flight`, `selecting_job_sets_preview_mode_to_selected_job_summary`, `ai_warning_banner_present_for_missing_api_key`, `resolve_preview_prefers_summary_over_triage`, `resolve_preview_returns_correct_kind`, `desktop_view_carries_rich_state_for_the_filtered_job_list`.
- `crates/harvester_core/src/update/model_dispatch_tests.rs`: `synchronous_budget_is_clamped_and_zero_dispatches_one_triage_request`, `shared_budget_dispatches_scoring_summary_triage_and_never_exceeds_three`, `completion_gives_slot_to_new_highest_priority_work`, `summary_cache_completion_yields_to_scoring_before_next_summary`, `changed_in_flight_score_settles_and_caches_old_request_before_readmission`, `every_stage_preserves_admission_order`, `cache_hits_in_all_stages_take_no_slot`, `triage_cache_hit_rejects_old_summary_then_current_completion_admits_scoring`, `quota_halts_all_stages_and_future_admissions_with_provider_reason`, `existing_three_rate_limit_threshold_halts_all_stages`.
- `crates/harvester_core/src/update/pipeline_run/tests.rs`: `total_source_failure_marks_scanning_failed`, `poll_only_run_becomes_terminal_when_source_poll_settles`, `accepted_stop_during_poll_drains_the_poll_without_ingesting_its_urls`, `pipeline_request_joins_an_active_poll_run_without_resetting`.
- `crates/harvester_core/src/update/pipeline_run/wave_tests.rs`: `failed_triage_is_requested_in_the_next_run_without_an_advance_message`, `hydration_with_eligible_scoring_is_settled_and_unadmitted`, `run_requested_during_continuous_downloads_starts_triage_before_downloads_end`, `three_download_bursts_release_overlapping_waves_and_monotonic_totals`.
- `crates/harvester_core/src/update/tests/archive_tests.rs`: `archive_clicked_with_triage_complete_and_pre_triage_ready_sets_pending_count`, `parity_a_pre_triage_ready_archive_count_is_zero_pending_count_is_nonzero`, `parity_b_triage_complete_corpus_count_dialog_count_urls_match`, `checkpoint_set_does_not_reduce_corpus_count_to_zero`, `summary_failed_for_url_returns_true_for_failed_summary`, `summaries_can_start_false_when_briefing_active`, `archive_counts_derive_from_triage_cache_at_startup_without_running_triage`.
- `crates/harvester_core/src/update/tests/delta_tests.rs`: `delta_appends_replaces_rebudgets_preserves_verdicts_and_removes_departed_members`, `triage_waits_for_matching_budget_then_summaries_reuse_snapshot_and_prepared_text`, `duplicate_url_delta_keeps_first_identity_and_manual_decision`.
- `crates/harvester_core/src/update/tests/import_tests.rs`: `poll_stats_cleared_when_new_poll_starts`, `poll_started_sets_total`, `poll_complete_increments_progress`, `poll_failed_increments_progress`, `poll_ended_preserves_the_current_desktop_workspace`.
- `crates/harvester_core/src/update/tests/mod.rs`: `articles_loaded_dispatches_first_summary`, `summary_store_uses_run_frozen_metadata_when_completion_model_differs`, `summary_persisted_with_dated_model_variant_is_cache_hit_after_reload`, `second_run_reuses_cached_summary_with_configured_model_key`, `summary_completion_emits_exact_record_before_settlement`.
- `crates/harvester_core/src/update/tests/pre_triage_refresh_tests.rs`: `poll_burst_multiple_job_dones_yields_exactly_one_triage_load`, `poll_burst_waits_for_engine_jobs_to_drain_before_dispatching`, `poll_burst_zero_urls_no_triage_load_dispatched`.
- `crates/harvester_core/src/update/tests/provider_alert_tests.rs`: `stale_quota_completion_after_new_run_start_does_not_raise_banner`.
- `crates/harvester_core/src/update/tests/signal_candidate_tests.rs`: `summary_completion_enqueues_signal_scoring`, `summary_cache_hit_reuses_signal_candidate_cache_without_snapshot_leak`.
- `crates/harvester_core/src/update/tests/ui_state_tests.rs`: `missing_api_key_blocks_triage_and_briefing_actions`.
- `crates/harvester_core/src/update/tests/unfinished_work_tests.rs`: `admitted_pending_and_active_summaries_are_in_progress`.
- `crates/harvester_core/src/working_corpus.rs`: `triage_complete_but_all_below_cutoff_yields_unavailable`, `fingerprint_changes_on_membership_and_order_change`, `select_for_archive_ignores_pre_triage`.
- `crates/harvester_core/tests/brave_integration.rs`: `brave_source_poll_completed_enqueues_urls`, `brave_source_dedup_skips_already_seen_urls`.
- `crates/harvester_core/tests/jobs.rs`: `link_download_failed_sets_failed_state`.
- `crates/harvester_core/tests/pre_triage_filter.rs`: `manual_include_overrides_hard_exclude`, `manual_exclude_overrides_auto_include`, `corpus_fingerprint_changes_when_decisions_change`.
- `crates/harvester_core/tests/reducer_behaviour.rs`: `llm_completed_success_updates_state`, `request_ids_monotonically_increase`.
- `crates/harvester_engine/src/corpus_manifest.rs`: `manifest_records_current_schema_version_and_layout`.
- `crates/harvester_engine/src/llm/handle.rs`: `extra_template_vars_not_in_context_block`, `worker_request_matches_prepare_completion`, `signal_candidate_falls_back_to_summary_model_then_default`.
- `crates/harvester_engine/src/llm/prompts/mod.rs`: `register_defaults_activates_exported_default_aliases`, `register_defaults_keeps_older_exported_versions_addressable`.
- `crates/harvester_engine/src/llm/template_validation.rs`: `both_fields_reported_independently`.
- `crates/harvester_engine/tests/briefing_loader_integration.rs`: `empty_directory_returns_no_articles`, `non_md_files_are_skipped`, `linked_directory_is_not_scanned`, `files_without_frontmatter_are_skipped`, `valid_files_do_not_prevent_others_from_loading`, `prepared_text_is_within_summary_budget`, `filtered_loader_includes_only_selected_urls`, `filtered_loader_preserves_caller_order`, `filtered_loader_missing_selected_url_is_skipped`, `filtered_loader_empty_selection_returns_empty_result`, `filtered_loader_matches_www_and_eu_host_variants`, `filtered_loader_matches_normalized_url_shape`, `filtered_loader_matches_mobile_and_query_variants`, `filtered_loader_matches_http_https_and_edition_variants`, `filtered_loader_matches_cisco_content_path_alias`, `archive_format_file_in_output_dir_does_not_block_article_scan`, `filtered_loader_selected_urls_older_than_since_utc_produce_empty_result`, `filtered_loader_with_progress_reports_scan_progress`.
- `crates/harvester_engine/tests/llm_handle.rs`: `llm_handle_dispatches_completion_event`, `llm_handle_emits_usage_update_after_completion`, `concurrent_requests_never_exceed_cap`, `retry_under_concurrency_pressure`.
- `crates/harvester_engine/tests/llm_pricing.rs`: `default_pricing_covers_configured_default_models`.
- `crates/harvester_engine/tests/llm_prompt.rs`: `prompt_id_from_str_round_trips`, `registry_with_defaults_exposes_active_latest_template_for_each_prompt`.
- `crates/harvester_engine/tests/output.rs`: `triage_archive_uses_ordered_urls_and_preserves_full_markdown`, `triage_archive_since_filter_excludes_old_docs_but_keeps_malformed_timestamps`, `triage_archive_ignores_existing_archive_md_artifact`, `triage_archive_uses_summary_body_when_provided`, `triage_archive_falls_back_to_full_body_when_no_summary`, `triage_archive_summary_mode_with_empty_map_uses_fallback_format`, `triage_archive_truncates_large_fallback_body_safely`, `triage_archive_schema2_since_matches_shared_golden_fixture`, `triage_archive_since_coverage_counts_window_remainders_and_zero_export`, `triage_archive_boundary_forgery_matches_shared_multi_document_fixture`, `triage_archive_sanitizes_header_injection_and_json_escapes_tags`, `triage_archive_index_offsets_follow_escaped_lines_and_skip_bad_timestamp_bounds`, `triage_archive_unparseable_only_timestamp_has_dash_bounds`, `triage_archive_zero_documents_matches_fixture_and_is_excluded_from_next_export`.
- `crates/harvester_engine/tests/triage_loader_integration.rs`: `triage_loader_respects_shared_summary_budget`, `triage_loader_truncates_at_utf8_boundary`, `triage_loader_mapping_preserves_fields`.
- `crates/harvester_io/src/effect_helpers.rs`: `every_prompt_id_has_a_context_filename`.
- `crates/harvester_io/src/effect_runner/dispatch.rs`: `loaded_context_pairs_are_sorted_by_key`.
- `crates/harvester_ui_bridge/src/driver.rs`: `driver_flushes_the_trailing_snapshot_without_another_message`.
- `crates/harvester_ui_bridge/src/fixtures.rs`: `checked_in_snapshot_fixtures_match_core_projection`.

All retained tests using the old ArticlesLoaded/orchestration harness now release completed triage through update/test_support.rs and the live downstream scheduler. Boundary tests continue to use the raw reducer and inspect emitted effects. The independent template-field validator and link-failure handler tests remain, and the old generic-selector cutoff test now uses the archive selector.

Phase 5 removed tests (156), by file:

**`crates/harvester_batch/src/cli.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `refresh_stale_summaries_limit_is_parsed` | Stale-summary refresh mode: refresh stale summaries limit is parsed |

**`crates/harvester_batch/src/progress/stale_reporter.rs` (27)**

| Removed test | Feature covered |
| --- | --- |
| `format_eta_zero_completed_returns_dashes` | Stale-summary refresh terminal reporting: format eta zero completed returns dashes |
| `format_eta_partial_progress_returns_minutes_seconds` | Stale-summary refresh terminal reporting: format eta partial progress returns minutes seconds |
| `format_eta_all_completed_returns_zero` | Stale-summary refresh terminal reporting: format eta all completed returns zero |
| `format_eta_clamps_to_zero_when_completed_exceeds_selected` | Stale-summary refresh terminal reporting: format eta clamps to zero when completed exceeds selected |
| `format_eta_zero_selected_returns_zero` | Stale-summary refresh terminal reporting: format eta zero selected returns zero |
| `startup_line_emits_expected_fields` | Stale-summary refresh terminal reporting: startup line emits expected fields |
| `startup_line_disabled_writes_nothing` | Stale-summary refresh terminal reporting: startup line disabled writes nothing |
| `completed_ok_increments_counts_and_redraws` | Stale-summary refresh terminal reporting: completed ok increments counts and redraws |
| `request_dispatched_alone_does_not_paint_status` | Stale-summary refresh terminal reporting: request dispatched alone does not paint status |
| `completed_ok_disabled_writes_nothing` | Stale-summary refresh terminal reporting: completed ok disabled writes nothing |
| `completed_fail_writes_sticky_stderr_and_redraws_status` | Stale-summary refresh terminal reporting: completed fail writes sticky stderr and redraws status |
| `completed_fail_balances_pending_with_request_dispatched` | Stale-summary refresh terminal reporting: completed fail balances pending with request dispatched |
| `completed_fail_truncates_long_reason` | Stale-summary refresh terminal reporting: completed fail truncates long reason |
| `completed_fail_normalizes_control_chars_in_reason` | Stale-summary refresh terminal reporting: completed fail normalizes control chars in reason |
| `completed_fail_clears_active_status_row_before_failure` | Stale-summary refresh terminal reporting: completed fail clears active status row before failure |
| `completed_fail_disabled_writes_nothing` | Stale-summary refresh terminal reporting: completed fail disabled writes nothing |
| `unloadable_target_increments_fail_without_touching_pending` | Stale-summary refresh terminal reporting: unloadable target increments fail without touching pending |
| `unloadable_target_does_not_underflow_when_pending_already_zero` | Stale-summary refresh terminal reporting: unloadable target does not underflow when pending already zero |
| `unloadable_target_clears_active_status_row_before_failure` | Stale-summary refresh terminal reporting: unloadable target clears active status row before failure |
| `unloadable_target_disabled_writes_nothing` | Stale-summary refresh terminal reporting: unloadable target disabled writes nothing |
| `finish_writes_done_line_with_path_and_no_cr` | Stale-summary refresh terminal reporting: finish writes done line with path and no cr |
| `finish_disabled_writes_nothing` | Stale-summary refresh terminal reporting: finish disabled writes nothing |
| `drop_cleanup_emits_newline_when_status_was_painted_and_finish_not_called` | Stale-summary refresh terminal reporting: drop cleanup emits newline when status was painted and finish not called |
| `drop_cleanup_is_silent_when_no_status_was_painted` | Stale-summary refresh terminal reporting: drop cleanup is silent when no status was painted |
| `drop_cleanup_is_silent_when_disabled` | Stale-summary refresh terminal reporting: drop cleanup is silent when disabled |
| `finish_marks_cleanup_done_so_drop_is_silent` | Stale-summary refresh terminal reporting: finish marks cleanup done so drop is silent |
| `full_disabled_walkthrough_writes_nothing` | Stale-summary refresh terminal reporting: full disabled walkthrough writes nothing |

**`crates/harvester_batch/src/summary_refresh.rs` (4)**

| Removed test | Feature covered |
| --- | --- |
| `summary_refresh_exit_code_is_zero_for_partial_success` | Stale-summary refresh mode: summary refresh exit code is zero for partial success |
| `summary_refresh_exit_code_is_nonzero_when_all_attempts_fail` | Stale-summary refresh mode: summary refresh exit code is nonzero when all attempts fail |
| `select_stale_summary_targets_prefers_missing_current_cache_key_and_respects_limit` | Stale-summary refresh mode: select stale summary targets prefers missing current cache key and respects limit |
| `select_stale_summary_targets_deduplicates_by_content_hash` | Stale-summary refresh mode: select stale summary targets deduplicates by content hash |

**`crates/harvester_core/src/briefing.rs` (17)**

| Removed test | Feature covered |
| --- | --- |
| `can_generate_allows_streaming_but_can_start_does_not` | Aggregate briefing, stream or history: can generate allows streaming but can start does not |
| `restart_bumps_epoch_and_clears_stream` | Aggregate briefing, stream or history: restart bumps epoch and clears stream |
| `append_and_exhaust_stream_items` | Aggregate briefing, stream or history: append and exhaust stream items |
| `stream_preview_has_exec_summary_numbered_items_and_session_info` | Aggregate briefing, stream or history: stream preview has exec summary numbered items and session info |
| `stream_preview_indents_multiline_item_body_under_list_item` | Aggregate briefing, stream or history: stream preview indents multiline item body under list item |
| `stream_preview_session_info_reports_truncation_and_dropped` | Aggregate briefing, stream or history: stream preview session info reports truncation and dropped |
| `stream_preview_shows_exhausted_note` | Aggregate briefing, stream or history: stream preview shows exhausted note |
| `stream_preview_none_before_exec_summary` | Aggregate briefing, stream or history: stream preview none before exec summary |
| `briefing_format_preview_none_when_not_complete` | Aggregate briefing, stream or history: briefing format preview none when not complete |
| `briefing_format_preview_shows_failure_reason` | Aggregate briefing, stream or history: briefing format preview shows failure reason |
| `briefing_format_preview_truncates_at_limit` | Aggregate briefing, stream or history: briefing format preview truncates at limit |
| `format_briefing_time_window_label_formats_checkpoint_and_all_time` | Aggregate briefing, stream or history: format briefing time window label formats checkpoint and all time |
| `format_empty_history_returns_sentinel` | Aggregate briefing, stream or history: format empty history returns sentinel |
| `format_single_entry_contains_timestamp_summary_and_top_stories` | Aggregate briefing, stream or history: format single entry contains timestamp summary and top stories |
| `format_three_entries_all_present` | Aggregate briefing, stream or history: format three entries all present |
| `from_result_rejects_empty_summary` | Aggregate briefing, stream or history: from result rejects empty summary |
| `truncation_is_safe_on_multibyte_characters` | Aggregate briefing, stream or history: truncation is safe on multibyte characters |

**`crates/harvester_core/src/briefing_snapshot.rs` (9)**

| Removed test | Feature covered |
| --- | --- |
| `includes_duplicates_in_corpus_order_with_stable_labels` | Aggregate briefing, stream or history: includes duplicates in corpus order with stable labels |
| `skips_in_window_articles_without_summary` | Aggregate briefing, stream or history: skips in window articles without summary |
| `excludes_articles_before_coverage_window` | Aggregate briefing, stream or history: excludes articles before coverage window |
| `malformed_or_missing_fetched_utc_is_included` | Aggregate briefing, stream or history: malformed or missing fetched utc is included |
| `drops_whole_entries_over_budget_and_marks_truncated` | Aggregate briefing, stream or history: drops whole entries over budget and marks truncated |
| `exact_fit_budget_includes_separator_bytes` | Aggregate briefing, stream or history: exact fit budget includes separator bytes |
| `utf8_multibyte_entries_are_never_split` | Aggregate briefing, stream or history: utf8 multibyte entries are never split |
| `oversized_first_entry_is_emitted_whole_and_marked_truncated` | Aggregate briefing, stream or history: oversized first entry is emitted whole and marked truncated |
| `empty_when_no_completed_summaries` | Aggregate briefing, stream or history: empty when no completed summaries |

**`crates/harvester_core/src/state/briefing_snapshot_access.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `snapshot_uses_full_base_corpus_including_duplicates` | Aggregate briefing, stream or history: snapshot uses full base corpus including duplicates |

**`crates/harvester_core/src/state/tests/mod.rs` (3)**

| Removed test | Feature covered |
| --- | --- |
| `starts_empty` | Aggregate briefing, stream or history: starts empty |
| `push_adds_newest_first` | Aggregate briefing, stream or history: push adds newest first |
| `push_caps_at_three` | Aggregate briefing, stream or history: push caps at three |

**`crates/harvester_core/src/update/tests/archive_tests.rs` (4)**

| Removed test | Feature covered |
| --- | --- |
| `briefing_generate_readiness_triage_or_corpus_not_ready_when_empty` | Aggregate briefing, stream or history: briefing generate readiness triage or corpus not ready when empty |
| `briefing_generate_readiness_summaries_not_settled` | Aggregate briefing, stream or history: briefing generate readiness summaries not settled |
| `briefing_generate_readiness_ready_when_failed_summary_does_not_block` | Aggregate briefing, stream or history: briefing generate readiness ready when failed summary does not block |
| `briefing_generate_readiness_signal_scoring_in_progress` | Aggregate briefing, stream or history: briefing generate readiness signal scoring in progress |

**`crates/harvester_core/src/update/tests/briefing_history_tests.rs` (7)**

| Removed test | Feature covered |
| --- | --- |
| `briefing_aggregate_not_dispatched_until_all_articles_settled` | Aggregate briefing, stream or history: briefing aggregate not dispatched until all articles settled |
| `startup_hydration_emits_load_briefing_history` | Aggregate briefing, stream or history: startup hydration emits load briefing history |
| `briefing_history_loaded_sets_state` | Aggregate briefing, stream or history: briefing history loaded sets state |
| `briefing_completion_appends_history_and_emits_save` | Aggregate briefing, stream or history: briefing completion appends history and emits save |
| `format_block_contains_history_content` | Aggregate briefing, stream or history: format block contains history content |
| `aggregate_briefing_effect_includes_previous_briefings_extra_var` | Aggregate briefing, stream or history: aggregate briefing effect includes previous briefings extra var |
| `aggregate_briefing_effect_includes_checkpoint_time_window_extra_var` | Aggregate briefing, stream or history: aggregate briefing effect includes checkpoint time window extra var |

**`crates/harvester_core/src/update/tests/briefing_stream_tests.rs` (9)**

| Removed test | Feature covered |
| --- | --- |
| `generate_without_hydration_defers_then_dispatches_after_hydration` | Aggregate briefing, stream or history: generate without hydration defers then dispatches after hydration |
| `next_item_clicked_before_exec_summary_is_noop` | Aggregate briefing, stream or history: next item clicked before exec summary is noop |
| `exec_completion_enters_streaming_and_writes_no_history` | Aggregate briefing, stream or history: exec completion enters streaming and writes no history |
| `next_item_emits_item_call_with_already_shown_suffix` | Aggregate briefing, stream or history: next item emits item call with already shown suffix |
| `item_completion_appends_then_exhausts` | Aggregate briefing, stream or history: item completion appends then exhausts |
| `item_failure_keeps_next_enabled_and_does_not_append` | Aggregate briefing, stream or history: item failure keeps next enabled and does not append |
| `stale_next_item_completion_from_discarded_stream_is_ignored` | Aggregate briefing, stream or history: stale next item completion from discarded stream is ignored |
| `streaming_with_item_in_flight_counts_as_active_work` | Aggregate briefing, stream or history: streaming with item in flight counts as active work |
| `view_exposes_next_item_enabled_and_keeps_generate_enabled_mid_stream` | Aggregate briefing, stream or history: view exposes next item enabled and keeps generate enabled mid stream |

**`crates/harvester_core/src/update/tests/delta_tests.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `imported_corpus_clear_emits_index_reset` | Unreachable corpus-clear message: emitted index reset |

**`crates/harvester_core/src/update/tests/import_tests.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `imported_corpus_cleared_resets_state` | Unreachable corpus-clear message: reset imported corpus state |

**`crates/harvester_core/src/update/tests/mod.rs` (3)**

| Removed test | Feature covered |
| --- | --- |
| `generate_briefing_defensive_fail_when_summaries_not_settled` | Aggregate briefing, stream or history: generate briefing defensive fail when summaries not settled |
| `generate_briefing_defensive_fail_when_signal_scoring_in_progress` | Aggregate briefing, stream or history: generate briefing defensive fail when signal scoring in progress |
| `aggregate_briefing_failure_records_reason_in_session_phase` | Aggregate briefing, stream or history: aggregate briefing failure records reason in session phase |

**`crates/harvester_core/src/update/tests/provider_alert_tests.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `generate_briefing_start_clears_provider_alert` | Aggregate briefing, stream or history: generate briefing start clears provider alert |

**`crates/harvester_core/src/update/tests/triage_tests.rs` (2)**

| Removed test | Feature covered |
| --- | --- |
| `briefing_blocked_when_triage_in_progress` | Aggregate briefing, stream or history: briefing blocked when triage in progress |
| `triage_click_blocked_when_briefing_owns_triage` | Aggregate briefing, stream or history: triage click blocked when briefing owns triage |

**`crates/harvester_core/src/update/tests/ui_state_tests.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `briefing_generate_enabled_false_while_briefing_is_running` | Aggregate briefing, stream or history: briefing generate enabled false while briefing is running |

**`crates/harvester_core/src/working_corpus.rs` (9)**

| Removed test | Feature covered |
| --- | --- |
| `pre_triage_ready_wins_over_stale_triage` | Retired pre-triage-first corpus selector: pre triage ready wins over stale triage |
| `pre_triage_ready_wins_over_empty_triage` | Retired pre-triage-first corpus selector: pre triage ready wins over empty triage |
| `pre_triage_reviewing_takes_precedence_over_complete_triage` | Retired pre-triage-first corpus selector: pre triage reviewing takes precedence over complete triage |
| `pre_triage_loading_falls_through_to_triage_complete` | Retired pre-triage-first corpus selector: pre triage loading falls through to triage complete |
| `triage_complete_used_when_pre_triage_unavailable` | Retired pre-triage-first corpus selector: triage complete used when pre triage unavailable |
| `both_unavailable_yields_unavailable` | Retired pre-triage-first corpus selector: both unavailable yields unavailable |
| `failed_pre_triage_and_idle_triage_yields_unavailable` | Retired pre-triage-first corpus selector: failed pre triage and idle triage yields unavailable |
| `loaded_empty_pre_triage_yields_unavailable` | Retired pre-triage-first corpus selector: loaded empty pre triage yields unavailable |
| `reviewing_phase_all_excluded_still_reports_pre_triage_reviewing` | Retired pre-triage-first corpus selector: reviewing phase all excluded still reports pre triage reviewing |

**`crates/harvester_core/tests/jobs.rs` (4)**

| Removed test | Feature covered |
| --- | --- |
| `link_toggle_requested_emits_download_effect` | Unreachable per-link toggle action: link toggle requested emits download effect |
| `link_toggle_unchecked_emits_delete_effect_when_downloaded` | Unreachable per-link toggle action: link toggle unchecked emits delete effect when downloaded |
| `link_toggle_unchecked_can_delete_during_stop_drain` | Unreachable per-link toggle action: link toggle unchecked can delete during stop drain |
| `link_toggle_unchecked_without_download_generates_no_effect` | Unreachable per-link toggle action: link toggle unchecked without download generates no effect |

**`crates/harvester_engine/src/llm/dto.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `next_item_variants_constructable` | Aggregate briefing, stream or history: next item variants constructable |

**`crates/harvester_engine/src/llm/handle.rs` (3)**

| Removed test | Feature covered |
| --- | --- |
| `validate_model_override_rejects_wrong_provider_with_small_error_type` | Prompt Lab overrides or replay lookup: validate model override rejects wrong provider with small error type |
| `validate_model_override_rejects_unknown_model_name_with_small_error_type` | Prompt Lab overrides or replay lookup: validate model override rejects unknown model name with small error type |
| `briefing_stream_ids_resolve_to_briefing_model` | Prompt Lab overrides or replay lookup: briefing stream ids resolve to briefing model |

**`crates/harvester_engine/src/llm/prompt.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `briefing_stream_prompt_ids_round_trip` | Aggregate briefing, stream or history: briefing stream prompt ids round trip |

**`crates/harvester_engine/src/llm/prompts/briefing.rs` (6)**

| Removed test | Feature covered |
| --- | --- |
| `v5_template_validates_briefing_variables` | Aggregate briefing, stream or history: v5 template validates briefing variables |
| `v6_template_validates_briefing_variables` | Aggregate briefing, stream or history: v6 template validates briefing variables |
| `v7_template_validates_briefing_variables` | Aggregate briefing, stream or history: v7 template validates briefing variables |
| `v7_expected_format_captures_top_story_schema` | Aggregate briefing, stream or history: v7 expected format captures top story schema |
| `v8_template_validates_briefing_variables` | Aggregate briefing, stream or history: v8 template validates briefing variables |
| `v8_expected_format_captures_top_story_schema` | Aggregate briefing, stream or history: v8 expected format captures top story schema |

**`crates/harvester_engine/src/llm/prompts/briefing_stream.rs` (4)**

| Removed test | Feature covered |
| --- | --- |
| `both_templates_validate` | Aggregate briefing, stream or history: both templates validate |
| `ids_and_versions_are_set` | Aggregate briefing, stream or history: ids and versions are set |
| `rendered_system_prefix_is_byte_identical` | Aggregate briefing, stream or history: rendered system prefix is byte identical |
| `next_item_user_template_carries_suffix_only_vars` | Aggregate briefing, stream or history: next item user template carries suffix only vars |

**`crates/harvester_engine/src/llm/template_validation.rs` (2)**

| Removed test | Feature covered |
| --- | --- |
| `aggregate_briefing_supports_briefing_specific_variables` | Aggregate briefing, stream or history: aggregate briefing supports briefing specific variables |
| `briefing_stream_templates_validate_with_synthetic_vars` | Aggregate briefing, stream or history: briefing stream templates validate with synthetic vars |

**`crates/harvester_engine/src/llm/validation.rs` (7)**

| Removed test | Feature covered |
| --- | --- |
| `validate_executive_summary_accepts_valid` | Aggregate briefing, stream or history: validate executive summary accepts valid |
| `validate_executive_summary_rejects_blank` | Aggregate briefing, stream or history: validate executive summary rejects blank |
| `validate_next_item_accepts_item` | Aggregate briefing, stream or history: validate next item accepts item |
| `validate_next_item_accepts_exhausted_and_ignores_extra_fields` | Aggregate briefing, stream or history: validate next item accepts exhausted and ignores extra fields |
| `validate_next_item_rejects_blank_headline_or_body` | Aggregate briefing, stream or history: validate next item rejects blank headline or body |
| `validate_next_item_fails_closed_on_unknown_status` | Aggregate briefing, stream or history: validate next item fails closed on unknown status |
| `validate_next_item_truncates_long_body_to_word_limit` | Aggregate briefing, stream or history: validate next item truncates long body to word limit |

**`crates/harvester_engine/tests/briefing_loader_integration.rs` (4)**

| Removed test | Feature covered |
| --- | --- |
| `collection_text_respects_collection_budget` | Aggregate collection-text loader: collection text respects collection budget |
| `collection_limits_articles_when_budget_tight` | Aggregate collection-text loader: collection limits articles when budget tight |
| `filtered_loader_budget_trimming_drops_tail_only` | Aggregate collection-text loader: filtered loader budget trimming drops tail only |
| `filtered_loader_single_selection_ignores_unrelated_invalid_markdown` | Retired selected-URL loader: skipped unreadable unselected Markdown before loading |

**`crates/harvester_engine/tests/llm_handle.rs` (7)**

| Removed test | Feature covered |
| --- | --- |
| `llm_handle_skips_provider_when_cache_hit` | Prompt Lab overrides or replay lookup: llm handle skips provider when cache hit |
| `llm_handle_inserts_cache_after_successful_response` | Prompt Lab overrides or replay lookup: llm handle inserts cache after successful response |
| `override_model_wins_over_stage_and_default` | Prompt Lab overrides or replay lookup: override model wins over stage and default |
| `unsupported_model_wrong_provider_fires_before_provider_call` | Prompt Lab overrides or replay lookup: unsupported model wrong provider fires before provider call |
| `unsupported_model_unknown_name_fires_before_provider_call` | Prompt Lab overrides or replay lookup: unsupported model unknown name fires before provider call |
| `valid_override_cache_miss_records_override_in_metadata` | Prompt Lab overrides or replay lookup: valid override cache miss records override in metadata |
| `valid_override_cache_hit_records_override_in_metadata` | Prompt Lab overrides or replay lookup: valid override cache hit records override in metadata |

**`crates/harvester_engine/tests/llm_prompt.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `briefing_stream_ids_have_active_default_prompts` | Aggregate briefing, stream or history: briefing stream ids have active default prompts |

**`crates/harvester_engine/tests/llm_replay.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `replay_provider_loads_and_finds_record` | Replay provider lookup: replay provider loads and finds record |

**`crates/harvester_engine/tests/llm_validation.rs` (3)**

| Removed test | Feature covered |
| --- | --- |
| `executive_summary_over_limit_is_truncated_with_notice` | Aggregate briefing, stream or history: executive summary over limit is truncated with notice |
| `briefing_story_body_is_truncated_to_150_words` | Aggregate briefing, stream or history: briefing story body is truncated to 150 words |
| `legacy_theme_briefing_is_mapped_to_story_schema` | Aggregate briefing, stream or history: legacy theme briefing is mapped to story schema |

**`crates/harvester_engine/tests/output.rs` (4)**

| Removed test | Feature covered |
| --- | --- |
| `concatenated_export_builds_delimited_output_and_manifest` | Concatenated export and manifest: concatenated export builds delimited output and manifest |
| `concatenated_export_creates_missing_output_dir` | Concatenated export and manifest: concatenated export creates missing output dir |
| `concatenated_export_includes_linked_pages_and_dedupes_urls` | Concatenated export and manifest: concatenated export includes linked pages and dedupes urls |
| `concatenated_export_ignores_custom_archive_artifacts_by_content` | Concatenated export and manifest: concatenated export ignores custom archive artifacts by content |

**`crates/harvester_io/src/effect_helpers.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `briefing_stream_ids_reuse_aggregate_context_file` | Aggregate briefing, stream or history: briefing stream ids reuse aggregate context file |

**`crates/harvester_io/src/effect_runner/tests.rs` (2)**

| Removed test | Feature covered |
| --- | --- |
| `load_articles_for_briefing_with_empty_ordered_urls_dispatches_empty_articles_loaded` | Aggregate loader or Prompt Lab override error: load articles for briefing with empty ordered urls dispatches empty articles loaded |
| `map_llm_event_unsupported_model_has_none_metadata` | Aggregate loader or Prompt Lab override error: map llm event unsupported model has none metadata |

**`crates/harvester_io/src/persistence.rs` (4)**

| Removed test | Feature covered |
| --- | --- |
| `round_trip_empty` | Briefing history persistence: round trip empty |
| `round_trip_three_entries` | Briefing history persistence: round trip three entries |
| `missing_file_returns_empty` | Briefing history persistence: missing file returns empty |
| `malformed_ron_returns_empty` | Briefing history persistence: malformed ron returns empty |

**`crates/harvester_io/src/runtime_paths.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `briefing_history_path_is_in_output_dir` | Briefing history persistence: briefing history path is in output dir |


Phase 6 notes (2026-09-30; implementation complete):

- Removed trends, entity-index state/store/worker writes, workspace selection, linked-page download/delete and their unreachable runner/completion/marking chain, indirect-link intake, article preview payloads/quality metadata, URL-age guesses and legacy window-size effects/loading. No entity sidecar is read or written. The selected job retains its extracted links and reducer-resolved browser opening.
- Frontend consumer and bridge projection audit found **23 of 48** top-level fields consumed, rather than the plan's earlier 22. `ai_unavailable_message` is read and stays. The page's rendered workflow and styles are unchanged. The frontend types now state the exact contract, without a permissive index signature. ReadingPaneMode/SetReadingPaneMode and the AppTab/LeftTab jumps were already absent from both the page and surviving hosts; WorkspaceView/SetWorkspaceView had no consumer and are removed.
- The reading pane keeps summary formatting and live-session-first, then newest content-hash result under any key. `format_summary_for_preview` remains; the unread triage formatter and its five tests are removed alongside the raw article-preview pipeline and fallback/exclusion presentation. RightPaneView carries only summary_markdown; SummaryMarkdown is the sole body key. Link rows no longer contain download state or URL-age hints.
- IPC_SCHEMA_VERSION is 13 in Rust and TypeScript. Regenerated all 13 snapshot fixtures, including run_finished_with_notice.json and ai_unavailable.json, using `UPDATE_UI_FIXTURES=1 cargo test --offline -p harvester_ui_bridge`. The projection comparison no longer blanks retired fields and compares the entire projected fixture. Updated synthetic probe views without changing delivery gates. Removed intents and body keys are rejected by decoding; the surviving frontend command decoder and components needed no behavioral changes.
- Saved-state layout remains unchanged. Old downloaded_path values and both geometry pairs still deserialize. Runtime links ignore downloaded paths, and saves no longer carry old paths forward. Saves deserialize only pending intake and geometry settings, skipping old completed-job records without allocating their jobs/links or normalizing URLs. Only desktop geometry has a host loader/writer. Carry-over assertions retain saved jobs, fetch times, link URLs, geometry, seen entries and paid results; rewritten snapshots assert downloaded paths are absent, as specified; the legacy geometry assertion uses a test-local reader after removal of the production loader. Archive golden fixtures and carry-over fixture files are unmodified; archive output and paid-result fixture byte comparisons pass.
- Preserved tests now observe scoped/selected job records, consumed dirty state, per-job tokens, batch session observations and model-usage accessors instead of removed view fields. AI availability tests assert the real message/quota state; provider-credit, rate-limit and session-budget tests assert halt reasons, dispatch effects and pending work. Removed presentation helpers are not retained as test-only implementations. Blacklist coverage asserts actual records and cooldown expiry. The Stop regression opens an extracted link during draining, rejects new intake until drain completes, then accepts intake; duplicate Resume and in-run selection preserve a non-default list mode.
- Updated Architecture, README, article-search design guidance, DecisionLog, EngineeringDiary and FutureIdeas. Retired backlog entries specific to linked-page downloads, trend insights and the article-preview pipeline. VisualDesignSpec names no removed surface and needs no change. The marker already lists only archive.md/archive-*.md, so neither it nor CORPUS_SCHEMA_VERSION changes. Launch scripts are unchanged. Runtime-state restructuring and the link store remain Phase 7 work.
- Verification passed: `cargo build --offline`; touched-crate tests followed by full root `cargo test --offline`; `cargo test --offline -p harvester_ui`; `cargo clippy --offline --all-targets -- -D warnings`; `cargo clippy --offline -p harvester_ui --all-targets -- -D warnings`; `cargo fmt` and `cargo fmt --check`. From frontend/: `npm run check`, `npm run build`, `npm run fmt` passed, including 97 Vitest tests. Pester, the GUI IPC probe, scripts/project-stats.ps1 and git writes cannot run in this environment; Claude/owner must run the probe and desktop walkthrough. Test attributes were counted directly using Count-RustTests' regex. No keys, live model APIs or real output folder were used. Changes remain uncommitted.
- Both replay harnesses completed on temporary synthetic carry-over copies with one held-back article, zero model latency and synchronous requests: batch 37.6 ms, desktop 82.1 ms. These are smoke checks, not corpus-sized performance comparisons; no fresh pre-change benchmark was run. The prior Phase 5 smoke numbers are recorded above, but are not a matched before/after comparison. Reports: .local/bench/desktop-slim-batch/report.json and .local/bench/desktop-slim-desktop/report.json. Check logs: .local/bench/desktop-slim-checks/. Full-corpus size/performance rows remain owner follow-up.

- Review follow-up: removed the unread PollQuotaWarning builder/formatting branch, ProviderAlert storage/getter/setters, MetricsState.total_tokens, domain_from_url and their feature-only tests after workspace-wide caller searches including both hosts and frontend/src. The rate-limit failure counter, owned-success reset, model halt reasons and session quota remain covered. `scan_archive_article_metadata` survives because `harvester_batch/examples/replay_support/mod.rs:read_article_files` uses it for replay corpus metadata; its comment now names that caller. `JobOrigin::Direct` remains because JobRowView, JobListRowView and SelectedJobView serialize it into the desktop JSON contract; no persisted-format change or origin migration is introduced.
- Removed RevealJobsSearch from UiIntent and JobsSearchRevealRequested from Msg/mapping. Ctrl+F focuses/selects the search input locally without dispatch; its frontend test now pins that contract. There was no separate TypeScript UiIntent union or intent fixture carrying it (dispatchIntent accepts a string). Added RevealJobsSearch to removed_desktop_intents_fail_closed. Snapshot fixture content is unchanged in this review pass; full Rust byte comparison and Vitest fixture checks pass at IPC 13.
- Deleted the four reveal-only reducer tests instead of renaming them into new no-ops. The burst test now changes actual search queries and pins the final query, mode and unique deterministic rows. Removed duplicated AI-message assertions and the meaningless unselected summary-settlement pane assertion. Added a state-level pasted-input clearing regression with a nonempty precondition and a second empty submit.
- Review verification: full root `cargo test --offline` passes 1,218 tests with 2 ignored; separate `cargo test --offline -p harvester_ui` passes 8; Rust test attributes total 1,228. `cargo build --offline`, both offline Clippy commands with all targets and warnings denied, `cargo fmt`, and frontend check/build/fmt pass (97 Vitest tests). The first test run exposed the obsolete carry-forward expectation in the replay carry-over test; it now asserts preserved link URLs and absent retired paths, with all paid-result byte assertions intact. Final logs are under `.local/review-fixes/`. No network, keys, launchers, Pester, GUI probe or git writes were used. Existing line-ending noise is left for Claude.

Phase 6 audited snapshot fields:

- Kept (23): `job_count`, `desktop_job_list`, `last_paste_stats`, `token_limit`, `archive_token_estimate`, `archive_filtered_count`, `archive_partial_coverage`, `raw_unprocessed_count`, `stop_finish_button`, `signal_candidate_rows`, `ai_unavailable_message`, `run_progress`, `archive_enabled`, `run_state`, `run_completion_notice`, `run_enabled`, `resume_enabled`, `resume_disabled_reason`, `unfinished_work`, `reprocess_notice`, `checkpoint_status_message`, `llm_quota`, `right_pane`.
- Removed (25): `ai_warning_banner`, `blacklist`, `briefing_blocked_reason`, `briefing_generate_enabled`, `dirty`, `indirect_link_summary`, `is_pre_triage_reviewing`, `job_list_mode`, `left_pane`, `llm_usage_by_model`, `next_item_enabled`, `poll_indirect_links_enabled`, `preview_context`, `preview_header`, `preview_source`, `preview_text`, `queued_urls`, `selected_job_id`, `selected_url`, `session`, `signal_candidate_preview`, `total_tokens`, `triage_blocked_reason`, `triage_results_reorder_suppressed`, `workspace_view`.
- Consumers: frontend/src/App.tsx, components/, ipc/useSnapshot.ts and components/ReadingPane.tsx; bridge projection: crates/harvester_ui_bridge/src/snapshot.rs. Selected job identity/URL and list mode remain inside desktop_job_list, rather than duplicate top-level fields. The activity feed remains capped at 50 entries and the desktop list at 400 rows plus one independent selected record.

Phase 6 test-count reconciliation (baseline commit feab6cd):

| Count | Baseline | Added | Removed | Final |
| --- | ---: | ---: | ---: | ---: |
| Rust test attributes | 1,319 | 7 | 98 | 1,228 |
| Root passing tests | 1,309 | 7 | 98 | 1,218 |
| Root ignored tests | 2 | 0 | 0 | 2 |
| harvester_ui passing / attributes (outside default members) | 8 | 0 | 0 | 8 |
| Vitest passing tests | 96 | 1 | 0 | 97 |

**1,319 + 7 - 98 = 1,228 attributes** and **1,309 + 7 - 98 = 1,218 root passing tests**.
The difference remains eight desktop-host tests plus two ignored tests. Forty-seven removals
are in the previously permitted wholesale deletions; 51 are edited out of surviving files.
No additional source or test file was deleted in the review pass. The renamed/moved tests
below have no count effect. No surviving quota, halt, Stop, cutoff, cache-key, export-byte,
ordering, job-list-mode, IPC-decoding, link-opening or saved-state-loading contract was dropped.

| Crate | Root passed before | Added | Removed | Root passed after | Ignored before / after |
| --- | ---: | ---: | ---: | ---: | --- |
| harvester_batch | 104 | 0 | 0 | 104 | 0 / 0 |
| harvester_core | 596 | 2 | 79 | 519 | 0 / 0 |
| harvester_engine | 417 | 0 | 6 | 411 | 1 / 1 |
| harvester_io | 129 | 2 | 13 | 118 | 0 / 0 |
| harvester_ui_bridge | 33 | 3 | 0 | 36 | 0 / 0 |
| openai_provider_kit | 30 | 0 | 0 | 30 | 1 / 1 |
| engine_logging | 0 | 0 | 0 | 0 | 0 / 0 |

Phase 6 review-pass reconciliation (baseline: the uncommitted implementation reviewed):

| Count | Review baseline | Added | Removed | Final |
| --- | ---: | ---: | ---: | ---: |
| Rust test attributes | 1,246 | 2 | 20 | 1,228 |
| Root passing tests | 1,236 | 2 | 20 | 1,218 |
| Root ignored tests | 2 | 0 | 0 | 2 |
| harvester_ui passing tests | 8 | 0 | 0 | 8 |
| Vitest passing tests | 97 | 0 | 0 | 97 |

The review pass adds one core and one I/O regression, removes 20 core tests, and renames
four Rust tests plus the Ctrl+F frontend test without a count change. Its root per-crate
change is core 538 + 1 - 20 = 519 and I/O 117 + 1 = 118; all other counts are unchanged.

Phase 6 added regressions (7 Rust, 1 frontend):

| File | Added test | Contract |
| --- | --- | --- |
| `crates/harvester_core/tests/persistence.rs` | `selected_job_keeps_extracted_links_after_completion` | Selected completed record retains indexed extracted links and browser opening resolves the chosen link |
| `crates/harvester_io/src/persistence.rs` | `old_link_paths_load_but_runtime_saves_drop_them_and_preserve_geometry` | Old paths load and runtime links ignore them; runtime saves drop paths while both geometry pairs survive and desktop geometry stays distinct |
| `crates/harvester_ui_bridge/src/ipc.rs` | `removed_desktop_intents_fail_closed` | Reject every removed workspace/trend/indirect intent, RevealJobsSearch, plus retired reading-pane and tab jumps |
| `crates/harvester_ui_bridge/src/ipc.rs` | `removed_body_keys_fail_closed` | Reject Preview, TriageMarkdown and PollStatsMarkdown body-key requests |
| `crates/harvester_ui_bridge/src/snapshot.rs` | `desktop_snapshot_pins_only_rendered_fields` | Exact 23-field contract, including AI unavailability and summary-only right pane |
| `frontend/src/App.test.tsx` | `pins the reduced IPC 13 snapshot in every bridge fixture` | All 13 fixtures have schema 13, the exact retained field set and only summary_markdown in right_pane |
| `crates/harvester_core/src/update/mod.rs` | `urls_submitted_clears_the_pasted_input_buffer` | Nonempty buffer becomes empty after submit; trimmed URLs enqueue once and a second empty submit emits no effects |
| `crates/harvester_io/src/persistence.rs` | `runtime_save_ignores_old_completed_payload_and_preserves_settings` | Save does not depend on old completed payload decoding; preserves pending intake and desktop geometry while replacing jobs/links |

Phase 6 renamed/moved baseline tests (23; no count effect):

| File | Original test | Surviving test |
| --- | --- | --- |
| `crates/harvester_core/src/state/tests/mod.rs` | `selecting_job_with_preview_updates_view_model` | `selecting_completed_job_updates_selected_record` |
| `crates/harvester_core/src/state/tests/mod.rs` | `selecting_job_without_preview_only_sets_header` | `selecting_in_progress_job_exposes_its_stage` |
| `crates/harvester_core/src/state/tests/mod.rs` | `selecting_job_sets_preview_mode_to_selected_job_summary` | `reselecting_job_keeps_its_summary` |
| `crates/harvester_core/src/state/tests/mod.rs` | `ai_warning_banner_present_for_missing_api_key` | `missing_api_key_disables_ai_and_explains_why` |
| `crates/harvester_core/src/state/tests/mod.rs` | `ai_warning_banner_absent_when_ai_available` | `available_ai_has_no_unavailable_message` |
| `crates/harvester_core/src/state/tests/mod.rs` | `ai_warning_banner_absent_for_non_key_ai_unavailability` | `missing_triage_model_disables_ai_and_explains_why` |
| `crates/harvester_core/src/state/tests/mod.rs` | `resolve_preview_prefers_summary_over_triage` | `summary_resolution_prefers_live_summary_over_triage` |
| `crates/harvester_core/src/state/tests/mod.rs` | `preview_metadata_for_selected_done_job_exposes_source_and_status` | `selected_done_job_exposes_url_stage_and_outcome` |
| `crates/harvester_core/src/state/tests/mod.rs` | `trends_workspace_keeps_selected_article_context` | `changing_list_mode_keeps_selected_article_context` |
| `crates/harvester_core/src/update/tests/import_tests.rs` | `poll_ended_preserves_the_current_desktop_workspace` | `poll_ended_preserves_the_current_list_and_selection` |
| `crates/harvester_core/src/update/tests/provider_alert_tests.rs` | `three_consecutive_rate_limited_summaries_stop_run_and_raise_banner` | `three_consecutive_rate_limited_summaries_stop_run_and_retain_reason` |
| `crates/harvester_core/src/update/tests/provider_alert_tests.rs` | `provider_quota_exhausted_summary_raises_credits_banner_immediately` | `provider_quota_exhausted_summary_halts_immediately_with_credit_reason` |
| `crates/harvester_core/src/update/tests/provider_alert_tests.rs` | `session_budget_quota_exhausted_stops_run_with_session_limit_banner` | `session_budget_quota_exhausted_stops_run_with_session_limit_reason` |
| `crates/harvester_core/src/update/tests/provider_alert_tests.rs` | `session_quota_halt_persists_across_triage_start_with_visible_reason` | `session_quota_halt_persists_across_triage_start_with_original_reason` |
| `crates/harvester_core/src/update/tests/ui_state_tests.rs` | `duplicate_resume_request_keeps_desktop_workspace_stable_during_run` | `duplicate_resume_request_keeps_list_mode_stable_during_run` |
| `crates/harvester_core/src/update/tests/ui_state_tests.rs` | `job_selection_during_run_preserves_the_current_workspace` | `job_selection_during_run_preserves_list_mode` |
| `crates/harvester_core/src/view_model.rs` | `blacklist_view_marks_active_and_cooldown` | `blacklist_records_active_cooldown_and_failure` |
| `crates/harvester_core/tests/desktop_job_list_mode_integration.rs` | `burst_updates_with_mode_and_workspace_switches_keep_desktop_rows_unique` | `burst_updates_with_mode_and_search_changes_keep_desktop_rows_unique` |
| `crates/harvester_core/tests/persistence.rs` | `restore_completed_job_records_downloaded_link_paths` | `restore_completed_job_ignores_downloaded_paths_and_keeps_links` |
| `crates/harvester_core/tests/reducer_behaviour.rs` | `indirect_links_are_not_polled_during_stop_drain` | `extracted_links_do_not_bypass_stop_intake_drain` |
| `crates/harvester_core/src/state/provider_alert.rs` | `rate_limit_threshold_raises_alert_after_three_consecutive_failures` | `rate_limit_threshold_stops_after_three_consecutive_failures` |
| `crates/harvester_core/src/state/provider_alert.rs` | `clear_provider_alert_resets_alert_and_counter` | `reset_provider_rate_limit_failures_resets_counter` |
| `crates/harvester_core/src/update/tests/provider_alert_tests.rs` | `stale_quota_completion_after_new_run_start_does_not_raise_banner` | `stale_quota_completion_after_new_run_start_does_not_halt_dispatch` |

Review-pass renames without count effect: the three surviving baseline tests above;
`old_link_paths_and_legacy_geometry_load_and_survive_without_runtime_use` to
`old_link_paths_load_but_runtime_saves_drop_them_and_preserve_geometry` (an added regression,
now pins the approved drop-on-save contract); and frontend `Ctrl+F dispatches RevealJobsSearch
and focuses the search box locally` to `Ctrl+F focuses the search box locally without
dispatching an intent`. The four reveal-only tests and the token-total test are removals,
not renames or moves.

Phase 6 removed tests (98), each named with the retired feature it covered:

The original 78 removals are listed below, followed by the 20 additional review-pass removals.

**`crates/harvester_core/src/preview.rs` (2)**

| Removed test | Feature covered |
| --- | --- |
| `exclusion_formatter_includes_decision_source` | Retired article-preview payload, quality or fallback presentation: exclusion formatter includes decision source |
| `fallback_formatter_provides_guidance` | Retired article-preview payload, quality or fallback presentation: fallback formatter provides guidance |

**`crates/harvester_core/src/state/tests/mod.rs` (13)**

| Removed test | Feature covered |
| --- | --- |
| `job_done_success_stores_preview` | Retired article-preview payload, quality or fallback presentation: job done success stores preview |
| `job_done_failure_clears_preview` | Retired article-preview payload, quality or fallback presentation: job done failure clears preview |
| `job_progress_with_preview_updates_selected_preview` | Retired article-preview payload, quality or fallback presentation: job progress with preview updates selected preview |
| `job_progress_with_preview_stores_content_when_not_selected` | Retired article-preview payload, quality or fallback presentation: job progress with preview stores content when not selected |
| `job_done_after_inprogress_promotes_preview_to_available` | Retired article-preview payload, quality or fallback presentation: job done after inprogress promotes preview to available |
| `preview_quality_counts_headings_and_skips_nav_indicator_when_low_density` | Retired article-preview payload, quality or fallback presentation: preview quality counts headings and skips nav indicator when low density |
| `preview_quality_marks_nav_heavy_when_link_density_high` | Retired article-preview payload, quality or fallback presentation: preview quality marks nav heavy when link density high |
| `collect_indirect_links_filters_navigation_and_share_noise` | Indirect-link collection, filtering, deduplication or admission: collect indirect links filters navigation and share noise |
| `indirect_link_pool_dedupes_normalized_urls` | Indirect-link collection, filtering, deduplication or admission: indirect link pool dedupes normalized urls |
| `resolve_preview_uses_triage_when_summary_missing` | Retired article-preview payload, quality or fallback presentation: resolve preview uses triage when summary missing |
| `resolve_preview_uses_fallback_when_nothing_available` | Retired article-preview payload, quality or fallback presentation: resolve preview uses fallback when nothing available |
| `resolve_preview_returns_correct_kind` | Retired article-preview payload, quality or fallback presentation: resolve preview returns correct kind |
| `ingest_indirect_links_skips_blacklisted_domain` | Indirect-link collection, filtering, deduplication or admission: ingest indirect links skips blacklisted domain |

**`crates/harvester_core/src/tabs.rs` (2)**

| Removed test | Feature covered |
| --- | --- |
| `trend_category_round_trip` | Trend category, weekly aggregation or ranking: trend category round trip |
| `trend_category_from_index_out_of_range_returns_none` | Trend category, weekly aggregation or ranking: trend category from index out of range returns none |

**`crates/harvester_core/src/trends.rs` (24)**

| Removed test | Feature covered |
| --- | --- |
| `normalize_trims_and_lowercases` | Trend category, weekly aggregation or ranking: normalize trims and lowercases |
| `normalize_collapses_whitespace` | Trend category, weekly aggregation or ranking: normalize collapses whitespace |
| `normalize_empty_stays_empty` | Trend category, weekly aggregation or ranking: normalize empty stays empty |
| `display_label_most_frequent` | Trend category, weekly aggregation or ranking: display label most frequent |
| `display_label_tie_resolves_lexically` | Trend category, weekly aggregation or ranking: display label tie resolves lexically |
| `display_label_empty_input` | Trend category, weekly aggregation or ranking: display label empty input |
| `trends_week_count` | Trend category, weekly aggregation or ranking: trends week count |
| `trends_current_week_is_last` | Trend category, weekly aggregation or ranking: trends current week is last |
| `trends_oldest_week_is_window_minus_1_weeks_back` | Trend category, weekly aggregation or ranking: trends oldest week is window minus 1 weeks back |
| `trends_buckets_article_in_correct_week` | Trend category, weekly aggregation or ranking: trends buckets article in correct week |
| `trends_article_outside_window_is_skipped` | Trend category, weekly aggregation or ranking: trends article outside window is skipped |
| `trends_missing_fetched_utc_skipped_no_panic` | Trend category, weekly aggregation or ranking: trends missing fetched utc skipped no panic |
| `trends_counts_at_most_once_per_article` | Trend category, weekly aggregation or ranking: trends counts at most once per article |
| `trends_top_n_tie_breaking_deterministic` | Trend category, weekly aggregation or ranking: trends top n tie breaking deterministic |
| `trends_month_year_boundary_week` | Trend category, weekly aggregation or ranking: trends month year boundary week |
| `trends_total_entity_count_is_full_population` | Trend category, weekly aggregation or ranking: trends total entity count is full population |
| `recent_mover_outranks_stale_spike` | Trend category, weekly aggregation or ranking: recent mover outranks stale spike |
| `identical_latest_falls_back_to_weighted_recent` | Trend category, weekly aggregation or ranking: identical latest falls back to weighted recent |
| `ordering_is_deterministic_on_equal_scores` | Trend category, weekly aggregation or ranking: ordering is deterministic on equal scores |
| `short_window_ranks_deterministically` | Trend category, weekly aggregation or ranking: short window ranks deterministically |
| `recency_score_weights_latest_most` | Trend category, weekly aggregation or ranking: recency score weights latest most |
| `recency_score_short_slice_rewards_previous_week_activity` | Trend category, weekly aggregation or ranking: recency score short slice rewards previous week activity |
| `recency_score_single_element_increases_with_latest_activity` | Trend category, weekly aggregation or ranking: recency score single element increases with latest activity |
| `recency_score_empty_slice` | Trend category, weekly aggregation or ranking: recency score empty slice |

**`crates/harvester_core/src/update/mod.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `trends_view_opened_loads_the_entity_index` | Entity-index loading, upsert, rebuild or worker writes: trends view opened loads the entity index |

**`crates/harvester_core/src/update/tests/entity_index_tests.rs` (3)**

| Removed test | Feature covered |
| --- | --- |
| `entity_index_loaded_populates_trend_data` | Entity-index loading, upsert, rebuild or worker writes: entity index loaded populates trend data |
| `entity_index_load_failed_triggers_rebuild` | Entity-index loading, upsert, rebuild or worker writes: entity index load failed triggers rebuild |
| `trend_category_selected_updates_active_category_no_effects` | Trend category, weekly aggregation or ranking: trend category selected updates active category no effects |

**`crates/harvester_core/src/update/tests/import_tests.rs` (1)**

| Removed test | Feature covered |
| --- | --- |
| `window_resize_completed_emits_persist_effect` | Legacy window resize/persistence workflow: window resize completed emits persist effect |

**`crates/harvester_core/src/update/tests/ui_state_tests.rs` (4)**

| Removed test | Feature covered |
| --- | --- |
| `briefing_generate_enabled_false_when_triage_incomplete_or_corpus_empty` | Unread retired aggregate-briefing control projection: briefing generate enabled false when triage incomplete or corpus empty |
| `briefing_generate_enabled_false_when_summaries_not_settled` | Unread retired aggregate-briefing control projection: briefing generate enabled false when summaries not settled |
| `briefing_generate_enabled_false_when_signal_scoring_in_progress` | Unread retired aggregate-briefing control projection: briefing generate enabled false when signal scoring in progress |
| `retired_briefing_controls_are_empty_when_summaries_settled` | Unread retired aggregate-briefing control projection: retired briefing controls are empty when summaries settled |

**`crates/harvester_core/src/url_age.rs` (7)**

| Removed test | Feature covered |
| --- | --- |
| `guess_age_from_url_parses_slash_pattern` | URL-derived link age estimation: guess age from url parses slash pattern |
| `guess_age_from_url_parses_slash_pattern_with_extension` | URL-derived link age estimation: guess age from url parses slash pattern with extension |
| `guess_age_from_url_parses_hyphen_pattern` | URL-derived link age estimation: guess age from url parses hyphen pattern |
| `guess_age_from_url_parses_hyphen_pattern_with_time_suffix` | URL-derived link age estimation: guess age from url parses hyphen pattern with time suffix |
| `guess_age_from_url_parses_compact_pattern` | URL-derived link age estimation: guess age from url parses compact pattern |
| `guess_age_from_url_ignores_invalid_date` | URL-derived link age estimation: guess age from url ignores invalid date |
| `guess_age_from_url_requires_digit_boundaries` | URL-derived link age estimation: guess age from url requires digit boundaries |

**`crates/harvester_core/tests/jobs.rs` (2)**

| Removed test | Feature covered |
| --- | --- |
| `link_download_completed_updates_state` | Linked-page download/delete effect and completion chain: link download completed updates state |
| `link_download_failed_sets_failed_state` | Linked-page download/delete effect and completion chain: link download failed sets failed state |

**`crates/harvester_engine/src/preview.rs` (6)**

| Removed test | Feature covered |
| --- | --- |
| `short_content_kept_as_is` | Raw article-preview truncation/frontmatter pipeline: short content kept as is |
| `truncated_content_appends_marker` | Raw article-preview truncation/frontmatter pipeline: truncated content appends marker |
| `exact_max_content_is_unmodified` | Raw article-preview truncation/frontmatter pipeline: exact max content is unmodified |
| `strips_frontmatter_and_trims_blank_line` | Raw article-preview truncation/frontmatter pipeline: strips frontmatter and trims blank line |
| `malformed_frontmatter_is_ignored` | Raw article-preview truncation/frontmatter pipeline: malformed frontmatter is ignored |
| `strips_frontmatter_with_crlf` | Raw article-preview truncation/frontmatter pipeline: strips frontmatter with crlf |

**`crates/harvester_io/src/effect_runner/tests.rs` (4)**

| Removed test | Feature covered |
| --- | --- |
| `download_link_page_rejects_disallowed_scheme_before_request` | Linked-page download/delete effect and completion chain: download link page rejects disallowed scheme before request |
| `download_link_page_effect_is_rejected_by_authorization` | Linked-page download/delete effect and completion chain: download link page effect is rejected by authorization |
| `delete_linked_page_effect_is_rejected_on_unsafe_path` | Linked-page download/delete effect and completion chain: delete linked page effect is rejected on unsafe path |
| `entity_index_worker_writes_a_queued_burst_once` | Entity-index loading, upsert, rebuild or worker writes: entity index worker writes a queued burst once |

**`crates/harvester_io/src/entity_index_store.rs` (7)**

| Removed test | Feature covered |
| --- | --- |
| `upsert_entry_partial_summary_preserves_existing_themes` | Entity-index loading, upsert, rebuild or worker writes: upsert entry partial summary preserves existing themes |
| `upsert_entry_partial_themes_preserves_existing_entities` | Entity-index loading, upsert, rebuild or worker writes: upsert entry partial themes preserves existing entities |
| `upsert_entry_idempotent_same_patch_twice` | Entity-index loading, upsert, rebuild or worker writes: upsert entry idempotent same patch twice |
| `upsert_entry_deduplicates_company_names_in_patch` | Entity-index loading, upsert, rebuild or worker writes: upsert entry deduplicates company names in patch |
| `load_missing_file_returns_default` | Entity-index loading, upsert, rebuild or worker writes: load missing file returns default |
| `load_corrupt_file_returns_default` | Entity-index loading, upsert, rebuild or worker writes: load corrupt file returns default |
| `roundtrip_save_and_load` | Entity-index loading, upsert, rebuild or worker writes: roundtrip save and load |

**`crates/harvester_io/src/persistence.rs` (2)**

| Removed test | Feature covered |
| --- | --- |
| `persist_and_load_window_size_roundtrips` | Legacy window resize/persistence workflow: persist and load window size roundtrips |
| `persist_window_size_preserves_existing_jobs` | Legacy window resize/persistence workflow: persist window size preserves existing jobs |


Phase 6 additional review-pass removed tests (20):

| File at review baseline | Removed test at review baseline | Original baseline name when different | Reason |
| --- | --- | --- | --- |
| `crates/harvester_core/src/llm_quota_view.rs` | `poll_warning_rules_follow_remaining_quota` | Same name | Unread PollStatsMarkdown quota-warning builder retired |
| `crates/harvester_core/src/poll_stats_fmt.rs` | `can_prepend_quota_warning` | Same name | Unread PollStatsMarkdown quota-warning formatting retired |
| `crates/harvester_core/src/preview.rs` | `triage_formatter_produces_stable_markdown` | Same name | Unread triage-preview formatter retired by owner decision |
| `crates/harvester_core/src/preview.rs` | `triage_formatter_no_json_leakage` | Same name | Unread triage-preview formatter retired by owner decision |
| `crates/harvester_core/src/preview.rs` | `triage_formatter_sanitizes_backticks_in_tag_text` | Same name | Unread triage-preview formatter retired by owner decision |
| `crates/harvester_core/src/preview.rs` | `triage_formatter_sanitizes_newlines_in_tag_text` | Same name | Unread triage-preview formatter retired by owner decision |
| `crates/harvester_core/src/preview.rs` | `triage_formatter_drops_empty_tags_after_sanitization` | Same name | Unread triage-preview formatter retired by owner decision |
| `crates/harvester_core/tests/desktop_job_list_mode_integration.rs` | `search_reveal_preserves_list_mode` | `mode_toggling_across_desktop_workspaces_preserves_mode` | Exercised only the retired search-reveal reducer no-op |
| `crates/harvester_core/src/state/provider_alert.rs` | `out_of_credits_raises_alert_immediately` | Same name | Write-only ProviderAlert state retired; rate-limit counter and halt behavior remain tested |
| `crates/harvester_core/src/state/provider_alert.rs` | `out_of_credits_retains_provider_detail` | `banner_text_for_out_of_credits_mentions_credits_and_detail` | Write-only ProviderAlert state retired; rate-limit counter and halt behavior remain tested |
| `crates/harvester_core/src/state/provider_alert.rs` | `rate_limit_alert_remains_until_explicit_reset` | `rate_limited_banner_describes_best_effort_stop` | Write-only ProviderAlert state retired; rate-limit counter and halt behavior remain tested |
| `crates/harvester_core/src/update/mod.rs` | `jobs_search_reveal_preserves_list_mode_and_selection` | `jobs_search_reveal_changes_only_the_desktop_workspace` | Exercised only the retired search-reveal reducer no-op |
| `crates/harvester_core/src/update/tests/provider_alert_tests.rs` | `prepare_summaries_start_clears_provider_alert` | Same name | Only tested setting/clearing unread ProviderAlert state; real run halt/reset tests remain |
| `crates/harvester_core/src/update/tests/provider_alert_tests.rs` | `triage_start_clears_provider_alert` | Same name | Only tested setting/clearing unread ProviderAlert state; real run halt/reset tests remain |
| `crates/harvester_core/src/update/tests/ui_state_tests.rs` | `job_list_mode_persists_across_search_reveal` | `job_list_mode_persists_across_workspace_switches` | Exercised only the retired search-reveal reducer no-op |
| `crates/harvester_core/src/update/tests/ui_state_tests.rs` | `jobs_search_query_persists_across_search_reveal` | `jobs_search_query_persists_across_workspace_switch` | Exercised only the retired search-reveal reducer no-op |
| `crates/harvester_core/src/state/tests/mod.rs` | `domain_from_url_handles_various_inputs` | Same name | Test-only URL domain helper retired |
| `crates/harvester_core/src/state/tests/mod.rs` | `provider_alert_retains_detail_without_changing_ai_setup` | `provider_alert_banner_shown_when_ai_available` | Unread provider-alert detail/setup combination retired; AI setup/message tests remain |
| `crates/harvester_core/src/state/tests/mod.rs` | `missing_api_key_message_survives_provider_alert` | `missing_api_key_banner_takes_priority_over_provider_alert` | Unread alert/message precedence retired; real missing-key message and action gates remain |
| `crates/harvester_core/src/state/tests/mod.rs` | `token_totals_accumulate_and_replace_previous_values` | Same name; moved from `tests/jobs.rs` | Write-only MetricsState.total_tokens retired; per-job token and archive-estimate contracts remain |


## Open questions

None open. The two questions from the first draft (going back to an older build after the
result-store switch, and jobs with no article file) and the exported-summary-text question
raised by the plan review were answered by the owner on 2026-09-28; the answers are recorded
under "Settled inputs this plan follows" and applied in Phases 2, 7 and 8.
