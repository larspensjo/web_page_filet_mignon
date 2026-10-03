# Plan: desktop run-stage bars show remaining work this run actually does

Phase numbers are local to this plan: durable documents, code comments and decision-log
entries name behaviours, never phases.

## Revision history

- **2026-10-03, first version.** Written from the design brief agreed with the owner that day.
  A small standalone step on `feature/simplification`, done before the remaining phases of
  `docs/plans/Plan.Simplification.md` and not folded into that plan.
- **2026-10-03, review revision.** After a Codex plan review and the owner's answers: the status
  column shows no ETA while the run is Stopping (settled input 12, superseding settled input 8
  for that state only), Scanning sources filling at poll start is recorded as expected
  behaviour (settled input 13), and the replayed run sequence drains a downstream bar from a
  positive backlog to zero.
- **2026-10-03, reuse revision, after the owner's desktop run.** Phase 1 was
  implemented (uncommitted) and the owner ran it on the desktop. Only about 25 articles were
  new, but the Triaging, Summarizing and Scoring rows read "6 of 158 to do", "5 of 140 to do" and
  "0 of 135 to do". The shared scale was 158, so the bars were slivers. The cause is that the run
  admits every article in the current window, and results reused from earlier runs count toward
  each model stage's total and completed counts. The owner decided (reuse decisions R1-R10
  below) that bars measure only the work this run actually does. This revision:
  - keeps Phase 1 as the record of what was built and marks it implemented;
  - adds Phase 2 (engine: reuse resolved at admission, a per-stage reused count in the
    snapshot, `IPC_SCHEMA_VERSION` 14, completion-notice count) and Phase 3 (frontend: new-work
    scale, count text, Stopping counts and ETA);
  - moves the documentation phase to Phase 4 and rewrites it for the new meaning;
  - marks settled inputs 1, 2, 5, 6, 8, 9 and 11 as narrowed or superseded where the reuse
    decisions change them. They are marked in place, not silently rewritten.
- **2026-10-03, Codex review of the reuse revision, plus the owner's answer.**
  - The owner chose Option A for the former Open question 1. It is recorded as R11: a result
    that appears only after admission is used without a request but stays new work, as a
    documented exception to R6. Codex's alternative was to keep such items on the request path;
    it was not applied because it conflicts with the owner's answer. No open questions remain.
  - Run progress now keeps per-run settlement sets for the model stages, so completed and
    reused accumulate across pruning. Recounting the surviving session members could let reused
    exceed completed.
  - The classifier-agreement test compares verdicts at the moment each stage admits an
    article, not at run start. It also covers an article that needs fresh triage but already
    has a current summary.
  - Phase 2 also updates `frontend/src/App.test.tsx`, which pins schema version 13 and lists
    the review fixtures.
  - Timing-sensitive tests keep their downstream enqueue, wave release, Stop and
    request-priority assertions.

## For the owner: what changes

While a run is going, the Run surface shows six stage rows (Scanning sources, Downloading
articles, Loading articles, Triaging, Summarizing, Scoring signals). Each bar shows **how much
work this run still has to do at that stage**:

- When you press Run nothing has been handed to any stage yet, so the bars are empty. Scanning
  sources fills in one step when the poll hands it all sources, then drains as sources finish.
- A downstream bar grows as the stage before it hands work down, shrinks to nothing when it
  catches up, and grows again when more arrives.
- **Results the app already has from earlier runs do not count as work.** When an article
  reaches Triaging, Summarizing or Scoring and a current saved result can be reused, it is
  settled at once and left out of that stage's numbers. In your run on 2026-10-03, Triaging
  would read about "6 of 31 to do" instead of "6 of 158 to do". The four article bars would
  share a scale of about 31 instead of 158, so they get real length.
- The four article stages (Downloading, Triaging, Summarizing, Scoring) share one scale, so you
  can compare their lengths directly. Scanning sources counts sources, not articles, so it has
  its own scale.
- The text next to each bar gives the stage's own share, for example "8 of 31 to do", or
  "4 of 22 to do · 3 failed" when some items failed. It does not mention reused results. A stage
  whose admitted articles were all reused reads "0 to do".
- Loading articles keeps a plain count such as "158 done" and has no bar. It really does load
  every window article from disk.
- When you press Stop, every bar empties at once. The counts switch to plain "N done", where N
  counts only this run's new work, and keep rising while in-flight work finishes. The status
  column shows plain statuses ("In progress", "Done", "Pending") with no countdown.
- The "about N min left" estimate is based only on new work. Reused results no longer make a
  stage look faster than it is.
- The end-of-run notice "Run finished - N articles scored" counts only articles actually scored
  by the AI during this run, not saved scores that were reused.
- Colours, the red failure colour, the muted look of stages that have not started, and the
  command-line progress do not change.

What the bars mean: **how much new work is waiting at each stage, not which stage is slow**. All
AI work shares one request budget and is handed out scoring first, then summaries, then triage,
so Triaging will usually hold the longest of the AI bars. That is a true queue size.

Your action: after Phase 3, one desktop run (human testing recommended there). No questions are
open.

## Goal and definition of done

Each run-stage bar shows the remaining work this run actually does, per the settled inputs and
reuse decisions below. For Triaging, Summarizing and Scoring signals, results settled without a
model request this run do not count toward the stage total used for the bar scale and the
"N of M to do" text. The one deliberate exception is in R11. The count text, the Stopping counts, the per-stage ETA and the end-of-run
notice follow the same meaning. Reducer tests cover the reused count, including a pre-filled
cache with too few request slots, a second run in the same process, prune and Stop. Frontend
tests cover the presentation, including a replayed run sequence with reuse and a fixture decoded
from a reducer-built snapshot. The documents that describe the run surface, run progress and
the scheduler are accurate. The Rust and frontend checks pass.

## Settled inputs this plan follows

### From the first brief (owner decisions, 2026-10-03)

1. Remaining for a stage = `max(0, total - completed - failed)`, where `total` is what has been
   admitted to that stage so far this run. In-flight items count as remaining.
   **Narrowed by R1/R2 for the model stages:** the bar and text total for Triaging, Summarizing
   and Scoring signals is admitted minus reused. Remaining is unchanged numerically, because
   reused work is settled at admission (see "Settled from the code", item 2).
2. Shared article scale: Downloading articles, Triaging, Summarizing and Scoring signals are
   scaled against the largest current `total` among those four stages in the snapshot. It is
   computed from each snapshot; the render keeps no `useRef`/`useState` memory.
   **Narrowed by R1/R7:** the scale is the largest current *new-work* total (admitted minus
   reused; Downloading has no reuse). The new-work total also only grows during a run, because
   reuse is resolved at admission (R7). So "largest current" still equals "largest seen this
   run".
3. Scanning sources uses its own scale: its remaining over its own total.
4. Loading articles shows no bar. Its count text stays a plain done count ("69 done", plus
   failures). Unchanged by R8.
5. Bar-row count text: "8 of 69 to do", with " · 3 failed" appended when failures are above zero.
   **Narrowed by R4:** M is the new-work total, and the text never mentions reused results.
6. While the run is Stopping every bar renders empty. Per-stage draining display is out of
   scope. **Narrowed by R8:** the Stopping "N done" count is completed minus reused.
7. "All bars empty at run end" is not a criterion: `StageList` renders only while
   `run_progress.run_active`.
8. Unchanged: the warning colour for stages with failures or Failed status, muted styling for
   Pending stages with no admitted work, the per-stage ETA logic (`estimateActiveStageEtas`), and
   the status labels. Refined by settled input 12 for the Stopping state.
   **Narrowed by R7:** the ETA uses new-work settlements over the new-work total; otherwise its
   logic is unchanged.
9. ~~Frontend-only: no engine, reducer or IPC change; `IPC_SCHEMA_VERSION` stays.~~
   **Superseded by R2 for the model stages:** the engine reports a per-stage reused count, the
   snapshot gains the field and `IPC_SCHEMA_VERSION` goes from 13 to 14.
10. Standalone step on this branch, ahead of the remaining Plan.Simplification phases.
11. A test replays a representative run's snapshot sequence and asserts bar lengths and text.
    **Extended by R10:** a second replay includes reuse. The frontend replay alone is not
    sufficient evidence; reducer tests carry the reused-count contract.

### Owner answers after the plan review (2026-10-03)

12. While the run is Stopping, the status column shows no ETA: no "about N min left" and no
    "Estimating…"/"Finishing…" countdown fallback. Each row shows its plain stage status. This
    supersedes settled input 8 for the Stopping state only and stays a pure function of the
    snapshot.
13. Scanning sources is empty when Run is pressed and fills in one step when the poll hands it
    all sources at once (`PollStarted { total }`), then drains. This is expected behaviour.

### Reuse decisions (owner, 2026-10-03, after the desktop run)

Numbered as in the reuse brief:

- **R1.** Bars measure only work this run actually does. Reused results are excluded from each
  model stage's total for scale and count text. (Chosen over scaling everything to the
  new-download count. That option pins bars full when a backlog exceeds new downloads and has no
  scale when nothing new was downloaded.)
- **R2.** The engine reports the reused count as an additional per-stage figure next to the
  existing counts. The existing `total`, `completed` and `failed` meanings are not redefined,
  so command-line progress and other consumers are unaffected. The desktop derives the new-work
  total by subtracting reused. The snapshot gains the field, `IPC_SCHEMA_VERSION` is bumped
  and bridge fixtures are regenerated.
- **R3.** Unchanged: Scanning sources and Downloading articles (downloads are always new work);
  Loading articles' plain "N done" count with no bar; Stopping behaviour (empty bars, "N done",
  plain statuses, no ETA); colours and the muted and warning rules.
- **R4.** Count text does not mention reused results ("6 of 31 to do", never "· 127 reused").
- **R5.** Phase 1 as implemented stays as the base. This revision adds phases, and the
  documentation phase follows them and describes the new meaning.
- **R6.** Definition: "reused" means **settled without a model request this run**. It is not
  limited to cache-hit paths. It covers same-process reuse, where `TriageSession::admit` or
  `BriefingSession::admit` leaves an already-current Completed article untouched
  (`triage.rs:88-95`, `briefing.rs:195-202`), and scoring's `SignalCandidateSession::enqueue`
  refusal. It is tested with two runs in one process.
- **R7.** Reuse is resolved **when an article is admitted to a stage**, not when its turn comes
  in dispatch. Use the pure reducer-state lookups and reconcile them with, preferably derive
  them from, the reuse-aware classifier (`UnfinishedWork` / `classify_window_article` in
  `crates/harvester_core/src/state/unfinished_work.rs`). That classifier powers "Process
  unfinished (N)" and the reprocess notice, so the bars and those counts cannot disagree.
  Consequences that must hold: the new-work total only grows during a run, remaining excludes
  work that will be reused, and the per-stage ETA uses new-work settlements (in scope; the ETA
  logic is otherwise unchanged).
- **R8.** While Stopping, the per-row "N done" counts only this run's new work (completed minus
  reused). The end-of-run notice "Run finished - N articles scored" counts only articles actually
  scored this run. Loading articles keeps its full count.
- **R9.** Durable wording: the decision-log entry and the Plan.Simplification Phase 12 note state
  the meaning ("settled without a model request this run"), not cache-hit counters. Phase 12
  deletes the wave ledger and the process-lifetime stage sessions; the snapshot field is the
  contract the rebuild keeps.
- **R10.** Verification: reducer tests start from a pre-filled cache with a mix of reusable and
  new work and too few request slots, so new work queues, plus a second run in the same
  process. They assert per-stage reused counts, reused ≤ completed at all times, survival of
  `prune`/`retain_members`, and a new-work total that never shrinks. The frontend replay test
  alone is not sufficient evidence.

### Owner answer to the reuse revision's open question (2026-10-03)

- **R11. A result that appears only after admission stays new work.** Example: the same
  article content arrives under two URLs in one run. Both are new when admitted. The first is
  processed by a model request, and the second then settles from that freshly saved result in
  dispatch, at no cost and without a request of its own. It stays counted as new work:
  - no extra model request is made;
  - the reused count does not change;
  - the new-work total never shrinks.

  Rationale: the owner prefers a small imprecision in the bookkeeping and the ETA, about one
  item, over either alternative:
  - paying for duplicate model calls by keeping items judged new at admission on the request
    path (Codex's suggested policy, not adopted);
  - letting the new-work total shrink by counting the item as reused.

  This is a **deliberate, documented exception to the literal R6 definition**. It is bounded to
  items judged new work when admitted whose result appears later in the same run. A Phase 2
  test asserts this behaviour.

### Repository constraints

From `Agents.md`:

- Input -> action -> reducer -> state -> render is preserved. Reuse is decided and applied in
  the reducer from reducer state, with no new effect. The frontend only subtracts two snapshot
  numbers.
- Runtime logging uses `engine_logging` with run and stage context.
- UI follows `docs/visual_design/VisualDesignSpec.md`.
- Rust changes finish with `cargo clippy --all-targets -- -D warnings` and `cargo fmt`. Root
  Cargo commands stay Node-free.
- Frontend checks run from `frontend/`.
- No launchers, API keys or live LLM calls.
- `docs/DecisionLog.md` is append-only.

### Relevant decision-log entries

- 2026-09-07 "Run progress is accumulated in the reducer": the reused count is accumulated
  there like the other counts.
- 2026-09-08 "Desktop activity feed holds 50 entries": envelope size matters. One small integer
  per stage is negligible, but the probe is re-run because the snapshot shape changes.
- 2026-09-27 "Pipeline stages overlap in waves under one request budget": this explains why
  Triaging usually holds the longest bar. Its "stage sessions and the wave ledger live for the
  process" is the source of same-process reuse.
- 2026-09-27 "Stop halts new work and drains; export waits only for the run".
- 2026-09-27 "The desktop Run is one primary action".

None of these commits to *when* a cached result is applied. Moving that point from dispatch to
admission contradicts no entry, but it changes the scheduler description in
`docs/Architecture.md`. Phase 4 updates that description, and the new decision-log entry
records it.

## Settled from the code in this revision

The reuse brief left these open for the plan to settle:

1. **When reuse is knowable: at admission, for all three model stages.** The cache key is
   already computed at each admission point:
   - `waves::admit_triage` computes `current_triage_cache_key` before calling
     `TriageSession::admit`.
   - `admit_summary_wave` computes `current_summary_cache_key` before `BriefingSession::admit`.
   - `signal_candidate::try_enqueue` computes the scoring input key (`input_key`) before
     `scoring_admitted`.

   Prompt metadata is loaded for every run by `ProcessingConfigurationLoaded` before
   `pipeline_ready()` is true. The saved-result caches are hydrated at startup
   (`host_bootstrap.rs`, `TriageCacheHydrated` / `SummaryCacheHydrated` /
   `SignalCandidateCacheLoaded`). So the bars never need a "starts at the admitted total and
   collapses as hits drain" phase.

   This also explains why dispatch-time hits did not drain instantly. A hit takes no request
   slot, but dispatch takes the next article in admission order. `dispatch_triage` and
   `dispatch_summary` return as soon as that article is a miss and no slot is free. Reusable
   articles queued behind new work therefore waited for their turn.
2. **Reuse is settled at admission, not only counted there.** Counting at admission while still
   applying the result at dispatch cannot satisfy R10:
   - Reused would exceed completed until the hits drained.
   - Remaining would include work that will be reused.
   - Counting reuse only once it settles would make the new-work total shrink.

   So when an admitted article has a reusable result, the admission code applies it right away,
   through the same completion code dispatch uses today (complete the article, mark the triage
   wave changed, try to admit scoring), and records the identity as reused. All three
   completion paths are pure state changes with no effect, so this stays inside the reducer.
   Consequences:
   - Reused ≤ completed holds in every snapshot.
   - The new-work total (admitted minus reused) only grows.
   - Remaining (`total - completed - failed`) already excludes reused work.
   - Reuse no longer waits behind queued new work.
   - Priority between the stages for *model requests* is unchanged.

   The dispatch-time cache-hit branches stay as a fallback for results that appear after
   admission. Those items stay new work (R11).
3. **Derivation from the classifier.** Admission decides reuse with the same lookup that applies
   the result: `try_reuse_triage`, `try_reuse_summary`, `try_reuse_signal_candidate`, plus the
   session check below. The classifier gives the verdicts through the same lookups and keys. It
   is not called at admission, for two reasons:
   - Counting must use the exact key that applies the result, or reused could exceed completed.
   - The classifier derives the scoring key from cached upstream results, while admission uses
     the frozen scoring input snapshot.

   A reducer test pins agreement instead. Run-start verdicts cannot be the reference: the
   classifier deliberately reports a downstream verdict as `Unknown` until the current upstream
   result exists. An article can need fresh triage yet already have a current summary; once
   its triage finishes, summary admission correctly reuses that summary even though its
   run-start summary verdict was `Unknown`.

   The test therefore compares verdicts under the current keys at the moment each stage admits
   the article. For each article newly admitted to a stage by a message, reused at that stage ⇔
   the classifier's verdict for that stage, evaluated right after that message, is Complete.

   Evaluating right after the admitting message is equivalent to evaluating just before
   admission, for three reasons:
   - the classifier reads the saved-result caches;
   - admission-time reuse only reads those caches;
   - no model result for that stage can arrive for an article in the message that admits it
     to the stage.

   The classifier's semantics are not changed (see Phase 2).
4. **Same-process reuse (R6).**
   - *Triage and summaries:* in a second run in the same process, `admit` leaves an
     already-current Completed article untouched. Admission counts it reused because, right
     after `admit`, the article is Completed under the current key and has no request this run.
   - *Scoring:* a same-digest admission is refused by `SignalCandidateSession::enqueue`, so
     `try_enqueue` returns before `scoring_admitted`. The article never enters the scoring stage
     total, completed count or reused count (see the existing test
     `rerun_counts_current_triage_and_summary_hits_without_readmitting_scoring`). Nothing is
     inflated, so its reused contribution is zero by construction. A test pins this.
5. **ETA (open item 2): skewed today, in scope.** Reuse settles at stage start in zero time.
   `estimateActiveStageEtas` divides all settled items by elapsed time, so it overstates the
   rate and understates time left. It changes to new-work figures (Phase 3). The status
   fallbacks ("Waiting for articles", "Estimating…", "Finishing…") compare `completed + failed`
   with `total`, and that comparison is unchanged by subtracting reused from both sides. They
   keep their current checks, including the zero-total check on the raw total.
6. **Field name and location (open item 3):** `reused: u32` on the snapshot's `StageProgress`
   (`view.run_progress.stages[]`), next to `completed`, `failed` and `total`, mirrored on
   `StageRecord`. It is always zero for Scanning sources, Downloading articles and Loading
   articles. Its documented meaning is "items of this stage settled without a model request this
   run; always ≤ completed". The run's reused identities are kept per model stage in
   `PipelineAdmission` (`reused: [HashSet<Identity>; 3]`, next to `admitted`). That struct is
   created fresh for every run and is not touched by `waves::prune`.
9. **Settlements accumulate per run; they are not recounted from surviving session members.**
   Today `record_progress` recounts the Completed and Failed states of admitted members still
   present in the sessions, and `RunProgress::counts` keeps the maximum seen. That is not
   accumulation.

   Example: two articles are reused, then a window reload prunes one, then another reusable
   article is admitted. The reused set reaches three while the recount, and therefore the
   maximum, stays at two, so reused > completed. In the same way, completions by model request
   that are later pruned disappear from completed minus reused once later completions arrive.

   So `PipelineAdmission` also keeps `completed: [HashSet<Identity>; 3]` and
   `failed: [HashSet<Identity>; 3]`, filled from two sources:
   - every reused identity is inserted into the completed set when it is reused;
   - every `record_progress` pass inserts each admitted member it observes in a Completed or
     Failed session state.

   The sets are never pruned, so each model stage's `completed` and `failed` are the sizes of
   these per-run sets. Reused ⊆ completed holds by construction, and the counts never fall.
   Within one run an admitted identity cannot move between Completed and Failed, because
   `admit_once` blocks re-admission and failed work is retried only in a later run. A
   `debug_assert!` checks that the two sets stay disjoint.

   This follows the 2026-09-07 decision that run progress is accumulated in the reducer. It
   does not redefine `completed` or `failed` (R2): both still mean this run's admitted items
   that completed or failed. They just no longer lose members to pruning.
7. **End-of-run notice.** `settle_run` computes
   `new_result_count = completed_count - signal_completed_at_reset`. Scoring reuse increments
   the session's completed counter (`SignalCandidateSession::complete`), so it becomes that
   value minus the run's scoring reused count (saturating).
8. **No corpus change.** Nothing on disk changes: no `docs/CorpusFormat.md` edit and no
   `CORPUS_SCHEMA_VERSION` bump.

## Verified facts (grounding)

### Frontend, as implemented in Phase 1

- `frontend/src/components/RunSurface.tsx` exports `presentStageRows(stages, stopping)` and
  `estimateActiveStageEtas`.
  - `ARTICLE_SCALE_STAGES` lists the four article stages.
  - The scale is `max(0, ...totals)`. Remaining is `max(0, total - completed - failed)`.
  - The count is "N done" while Stopping or for Loading, "0 to do" for a zero total, and
    "N of M to do" otherwise, plus " · K failed".
  - `stageStatus(stage, eta, stopping)` returns the plain label while Stopping.
  - `RunSurface` computes ETAs only when the run is active and not Stopping.
  - The bar is `role="meter"` with a scoped `biome-ignore`. Loading renders an `aria-hidden`
    `.stage-bar-slot`.
- `frontend/src/components/RunSurface.test.tsx` holds the Phase 1 tests. `makeStages` and two
  ETA tests build `StageProgress` literals that will need the new field.
- `frontend/src/ipc/types.ts` is hand-written; `StageProgress` has no `reused` field yet.
  `frontend/src/ipc/schemaVersion.ts` holds `IPC_SCHEMA_VERSION = 13`.
- `frontend/src/App.test.tsx`:
  - The test "pins the reduced IPC 13 snapshot in every bridge fixture" hard-codes
    `expect(fixture.schema_version).toBe(13)` (line 153).
  - It iterates the hand-listed `reviewFixtures` array, which also drives an `it.each` render
    test per fixture. A new bridge fixture is not covered unless it is added to that array.

### Engine

- `crates/harvester_core/src/run_progress.rs`:
  - `StageRecord` and `StageProgress` carry `completed`, `failed`, `total`, `total_is_final`
    and the timestamps.
  - `activate` and `counts` grow values with `max()`.
  - `begin_stopping` marks every total final.
  - `RunCompletionNotice.new_result_count` is a `usize`.
- `crates/harvester_core/src/update/waves.rs`:
  - `admit_once` inserts the identity into the per-run `PipelineAdmission.admitted[stage]` set.
  - `admit_triage` calls `record_reprocess_notice` first, then admits the articles, releases
    the triage wave and admits ready summaries.
  - `scoring_admitted` is reached only after `SignalCandidateSession::enqueue` accepts.
  - `record_progress` sets each model stage's total to the admitted set size and its completed
    and failed to the admitted members' current session states, then applies `counts()` (max).
    It records Loading articles as the triage admission count.
  - `prune` retains the sessions and the wave ledger to the window members, but not
    `PipelineAdmission`. So a pruned admitted member drops out of the recount, and only the
    `max()` in `counts` hides the drop until later completions overtake it ("Settled from the
    code", item 9).
- `crates/harvester_core/src/update/model_dispatch.rs`:
  - The triage, summary and scoring cache-hit branches complete the article without a slot.
  - Triage calls `record_triage_cache_hit`, `triage_changed` and `try_enqueue`; summary calls
    `record_summary_cache_hit` and `try_enqueue`; scoring zeroes the tokens and clears the
    input snapshot.
  - On a miss with no free slot, the stage returns `false`.
  - The loop runs `release_ready` and then scoring, summary and triage in priority order until
    nothing progresses.
  - `dispatch_model_work` runs after every reducer message (`update/mod.rs`).
- `crates/harvester_core/src/update/triage.rs::start_triage_from_pretriage` calls
  `admit_triage` and *then* `start_triage_cache_run()`, which resets the triage cache run
  metrics. Hits recorded at admission would be erased from the run's cache summary log unless
  the reset moves before admission.
- `crates/harvester_core/src/update/pipeline_run.rs`:
  - `handle_pipeline_requested` creates a fresh `PipelineAdmission` per run.
  - `settle_run` computes `new_result_count`.
  - `handle_stop_for_pipeline` withdraws pending work; it never touches completed items.
- `crates/harvester_core/src/state/unfinished_work.rs`: `classify_window_article` uses
  `triage_cache.lookup` under the current triage key, `try_reuse_summary` and
  `try_reuse_signal_candidate`. `unfinished_stage_verdicts(url, hash)` exposes per-stage
  verdicts.
- Existing reducer tests touching reuse:
  - `update/pipeline_run/tests.rs`: `rerun_counts_current_triage_and_summary_hits_without_readmitting_scoring`,
    `scoring_stays_active_from_triage_cache_hit_through_the_last_summary_wave`, and the
    `assert_progress_does_not_regress` helper.
  - `update/model_dispatch_tests.rs`: `cache_hits_in_all_stages_take_no_slot`,
    `summary_cache_completion_yields_to_scoring_before_next_summary`.
  - `update/pipeline_run/wave_tests.rs`: one test asserts `new_result_count == 3`.

### Bridge, command line and docs

- `crates/harvester_ui_bridge`:
  - `IPC_SCHEMA_VERSION = 13` in `src/ipc.rs`, and the test
    `schema_version_matches_frontend_constant` pins it to the frontend constant.
  - Snapshot fixtures are reducer-built in `src/fixtures.rs` and regenerated with
    `UPDATE_UI_FIXTURES=1 cargo test -p harvester_ui_bridge`.
  - `src/probe.rs` builds `StageProgress` literals for the synthetic probe cases.
- Command-line progress (`crates/harvester_batch/src/progress/projection.rs`) reads session
  counts from `BatchObservation`, not `StageProgress`. R2 leaves it unaffected in meaning.
- `docs/Architecture.md` lines 82-83 say `RunProgress` totals include triage and summary cache
  hits. Lines 189-194 say cache hits complete without a slot and priority is reconsidered after
  each completion. Both need updating.
- ~~No fixture changes are needed because the snapshot shape does not change.~~
  **Superseded by R2.** Every snapshot fixture gains `reused`.

## Design

### Engine: reuse resolved at admission (Phase 2)

1. **One completion path per stage.** Move the bodies of the three dispatch cache-hit branches
   into small reducer helpers, for example in a new `crates/harvester_core/src/update/reuse.rs`:
   - `reuse_triage(state, index) -> bool`
   - `reuse_summary(state, index) -> bool`
   - `reuse_score(state, url) -> bool`

   Each looks up the current-key result. On a hit it applies it exactly as dispatch does today:
   the same session completion, the same cache-hit metric, the same `[triage-cache]`,
   `[summary-cache]` and `[signal-cache]` hit log lines, `triage_changed` and `try_enqueue` as
   today. Dispatch calls the helpers, and its behaviour on a hit is unchanged. Admission calls
   them as well.
2. **Admission resolves reuse and records it.**
   - In `admit_triage`, after `release(..., Triaging, members)`, so the wave bookkeeping exists
     for `triage_changed`: for each newly admitted member, either the article is already
     Completed under the current key (left untouched by `admit`), or it is Pending and
     `reuse_triage` hits. In both cases insert the identity into
     `PipelineAdmission.reused[0]`.
   - Do the same in `admit_summary_wave` (after the wave release) with `reuse_summary` and
     `reused[1]`.
   - In `try_enqueue`, after `scoring_admitted` and the input snapshot are stored, call
     `reuse_score` and insert into `reused[2]` on a hit.
   - Only identities that are also in `admitted[stage]` this run are ever inserted.
3. **Progress, accumulated per run** ("Settled from the code", item 9).
   - `StageRecord` and `StageProgress` gain `reused: u32` (default 0), and `RunProgress::view`
     copies it.
   - `PipelineAdmission` gains per-stage `completed` and `failed` identity sets beside
     `admitted` and `reused`.
   - Reuse inserts the identity into both `reused` and `completed`.
   - `record_progress` no longer derives the model stages' completed and failed from a
     recount. It inserts every admitted member it observes as Completed or Failed into the run
     sets, then reports:
     - `total` = admitted;
     - `completed` and `failed` = the sizes of the run sets;
     - `reused` = the size of the reused set.

     All of these still go through `counts()` (with a `reused` parameter or a sibling setter),
     so the existing `max()` stays as a belt-and-braces guard.
   - The `total_is_final` and Loading-row logic is unchanged.
   - `debug_assert!`s document reused ⊆ completed, completed ∩ failed = ∅, and
     completed ∪ failed ⊆ admitted. Release builds report the counts as they are; tests carry
     the invariants.
4. **Notice.** `settle_run` subtracts the run's scoring reused count from `new_result_count`
   (saturating).
5. **Metrics order.** In `start_triage_from_pretriage`, call `start_triage_cache_run()` (and the
   metadata-ready mark) before `admit_triage`. The run's triage cache summary log then includes
   hits recorded at admission. Prompt metadata is already ready from
   `ProcessingConfigurationLoaded`, so the keys are unchanged.
6. **Logging.** When an admission call reuses at least one item, log once per call:
   `engine_info!("[pipeline-reuse] run_id={} stage={:?} admitted={} reused={}", ...)`. Per-item
   detail stays in the existing cache-hit lines.
7. **Snapshot contract.** `StageProgress.reused` is serialized in the desktop snapshot.
   `IPC_SCHEMA_VERSION` goes from 13 to 14 in `crates/harvester_ui_bridge/src/ipc.rs` and
   `frontend/src/ipc/schemaVersion.ts` together. `frontend/src/ipc/types.ts` gains
   `reused: number`, and `probe.rs` sets it in its synthetic stages.

### Presentation model (Phase 3; one pure function of the snapshot)

Phase 3 adds a small exported helper next to `presentStageRows`, for example
`newWork(stage) -> { total, done }`, with `total = max(0, stage.total - stage.reused)` and
`done = max(0, stage.completed - stage.reused)`. Clamping keeps the display sane if a snapshot
ever violated the invariant. `presentStageRows` and `estimateActiveStageEtas` use it. No hook,
ref or state is added.

| Row | Remaining (bar value) | Scale | Count text (Active run) | Count text (Stopping) |
|---|---|---|---|---|
| Scanning sources | `max(0, total - completed - failed)` | its own `total` | "N of M to do" | "N done" |
| Downloading, Triaging, Summarizing, Scoring signals | `max(0, newTotal - newDone - failed)` (equals `max(0, total - completed - failed)`) | largest `newTotal` among these four | "N of newTotal to do" | "newDone done" |
| Loading articles | no bar | none | "N done" | "N done" |

- Reused is zero for Scanning, Downloading and Loading, so their rows are unchanged.
- `percent = scale > 0 ? min(100, remaining / scale * 100) : 0`. While Stopping the displayed
  value and percent are 0 for every bar row.
- A bar row whose new-work total is 0 reads "0 to do". That includes a stage whose admitted
  articles were all reused.
- " · K failed" is appended when `failed > 0`, on every row and in every state. Reused items
  never fail, so failures are new work.
- Muted styling stays on the raw `total` (R3): a stage with admitted but all-reused work is
  Active or Done, not muted.
- `estimateActiveStageEtas` uses `newTotal` and `newSettled = newDone + failed` in place of
  `total` and `completed + failed`. Its other checks are unchanged (Active, final total, start
  time, nothing settled yields no ETA, all settled yields no ETA).
- `stageStatus` is unchanged; see "Settled from the code", item 5.

### Status column while Stopping (settled input 12)

While Stopping, every row's status is its plain stage status label. `RunSurface` computes ETAs
only when the run is active and not Stopping. Implemented in Phase 1; unchanged.

### Accessibility (implemented in Phase 1)

The bar is `role="meter"` with `aria-label="<Stage> work to do"`, `aria-valuemin=0`,
`aria-valuemax=<scale>`, `aria-valuenow=<displayed remaining>` (0 while Stopping) and
`aria-valuetext=<count text>`. It keeps a scoped `biome-ignore` for `useSemanticElements`.
Phase 3 only changes the numbers it is fed.

### Layout (implemented in Phase 1)

Loading articles renders an `aria-hidden` placeholder cell in the bar column. The count column
is widened and wraps under the narrow layout. With smaller new-work totals the text gets
shorter, so no layout change is needed.

### Expected behaviour over a run (for the documentation and the replay tests)

1. Run pressed: no stage has admitted work. Every bar is empty, every bar row reads "0 to do"
   and Loading articles reads "0 done".
2. Poll started: Scanning sources fills in one step, then drains as sources settle.
3. Window admitted: Triaging receives every window article. Those with a current saved result
   settle at once and are left out of the stage's numbers. Only the rest show as "N of M to do".
   Summaries and scoring behave the same as articles reach them. Loading articles reads the full
   window count, for example "135 done".
4. Downloads and model work arrive in waves. The article bars share the largest new-work total
   as their scale, so earlier bars shorten in proportion as more new work is admitted.
5. A downstream stage that catches up shows "0 of M to do" and an empty bar, then grows again
   when the next wave arrives.
6. Triaging usually holds the longest model-stage bar (scoring, then summary, then triage share
   one budget).
7. Stop: every bar empties at once, the counts read "N done" for this run's new work, and the
   status column shows plain statuses.
8. Run end: the stage rows disappear. The notice counts only articles scored by a model request
   this run.

## Phases

### Phase 1: Remaining-work bars, count text and tests (implemented)

**Status:** implemented 2026-10-03 and uncommitted, in
`frontend/src/components/RunSurface.tsx`, `frontend/src/components/RunSurface.test.tsx` and
`frontend/src/styles/workspace.css`. The owner ran it on the desktop the same day; that run
revealed the reuse inflation this revision addresses. This plan does not record Phase 1's check
results; Phase 2 and Phase 3 re-run the frontend checks on top of it. The record of what was
built follows.

Work (as built):

1. Added the pure helper `presentStageRows` and routed `StageList` through it, with `isStopping`
   passed from `RunSurface`. Removed `progressPercent` and `stageCount`.
2. "Status column while Stopping": `stageStatus` returns the plain label while Stopping, and
   `RunSurface` computes no ETAs while Stopping.
3. Switched the bar to `role="meter"` with the accessibility attributes and rendered the
   Loading placeholder cell.
4. `workspace.css`: count-column width and narrow-layout wrapping.
5. Edited "renders the six projected stages, failure counts and waiting stages" to the new
   text ("0 of 2 to do · 1 failed") and to five meters plus one Loading placeholder.

Regression tests (as built, in `RunSurface.test.tsx`):

- remaining-not-completed;
- shared article scale (including a Done stage supplying it);
- Scanning sources' own scale;
- Loading placeholder and done count;
- zero-total muted row;
- floor at zero;
- Stopping meters and counts;
- the paired Active/Stopping status test;
- warning styling;
- the replayed run sequence (Run pressed, poll, first wave, scoring backlog, scoring drained,
  regrowth, Stopping), ending with a fresh-mount equality check.

Human testing: the owner's desktop run on 2026-10-03. Its remaining checks (count text width at
a narrow window, Stop mid-run) are folded into Phase 3's human test.

### Phase 2: Engine reports reused work, resolved at admission

Work:

1. Extract the three cache-hit completion paths from `model_dispatch.rs` into the reuse helpers
   ("Engine", item 1). Dispatch uses them unchanged.
2. `PipelineAdmission` gains `reused: [HashSet<Identity>; 3]`. Admission resolves and records
   reuse in `admit_triage`, `admit_summary_wave` and `try_enqueue` ("Engine", item 2).
3. `StageRecord`/`StageProgress` gain `reused`. `PipelineAdmission` gains the per-run
   `completed` and `failed` sets, and `record_progress` reports the model stages from the run
   sets instead of a recount ("Engine", item 3). The field's doc comment states the meaning:
   "settled without a model request this run; always ≤ completed; zero for stages without model
   work; an item judged new work at admission stays new work even if it later settles from a
   result produced earlier in the same run".
4. `settle_run` subtracts scoring reuse from `new_result_count`.
5. Move the triage cache-run reset before admission in `start_triage_from_pretriage`.
6. Add the `[pipeline-reuse]` log line.
7. IPC contract:
   - Bump `IPC_SCHEMA_VERSION` to 14 in `crates/harvester_ui_bridge/src/ipc.rs` and
     `frontend/src/ipc/schemaVersion.ts`.
   - Add `reused: number` to `StageProgress` in `frontend/src/ipc/types.ts`.
   - Set `reused` in `crates/harvester_ui_bridge/src/probe.rs`.
   - Add a reducer-built fixture `run_in_progress_with_reused_results` to
     `crates/harvester_ui_bridge/src/fixtures.rs`. Build it from hydrated caches covering some
     articles, with `llm_max_in_flight` 1 so new work queues and the snapshot carries reused > 0
     on the model stages.
   - Regenerate all snapshot fixtures. Review the diff: every stage gains `"reused"`. If
     `run_finished_with_notice`'s `new_result_count` changes, confirm the fixture's scoring was a
     reuse and keep the new value.
8. Frontend type follow-through, with no behaviour change:
   - Add `reused: 0` to `makeStages` and to the `StageProgress` literals in the two ETA tests
     in `RunSurface.test.tsx`.
   - In `frontend/src/App.test.tsx`, edit "pins the reduced IPC 13 snapshot in every bridge
     fixture": the title becomes "IPC 14" and line 153 expects `14`. Add the new fixture
     `run_in_progress_with_reused_results` to the `reviewFixtures` array, so it is pinned and
     rendered like the others.
   - The presentation still ignores `reused` until Phase 3.
9. Existing reducer tests that pinned *when* a cache hit is applied (for example
   `scoring_stays_active_from_triage_cache_hit_through_the_last_summary_wave`,
   `summary_cache_completion_yields_to_scoring_before_next_summary`,
   `cache_hits_in_all_stages_take_no_slot`) are edited to the admission-time behaviour where
   they fail. Only their timing changes. They keep every assertion about:
   - downstream enqueue (a reused triage or summary still admits scoring);
   - wave release (summaries of a wave release only when its triage members have settled);
   - Stop (withdrawal and drain);
   - request priority (scoring, then summary, then triage for model requests);
   - no slot taken by reuse, and no model request for current results.

   Tests are edited, never deleted.

Regression tests (new module `crates/harvester_core/src/update/pipeline_run/reused_work_tests.rs`,
registered next to `wave_tests` in `pipeline_run/tests.rs`; reuse the existing helpers
`prepare_pipeline`, `run_to_completion` and `assert_progress_does_not_regress`, extending the
last one with reused):

- **Mixed reuse, too few slots.** A window of about six articles. Two are new and placed first
  in window order. Three have current triage, summary and scoring results. One has a current
  triage result only, with priority high enough for a summary. Set `llm_max_in_flight` to 1 and
  start a Resume run.
  - Immediately after admission, before any model completion: Triaging total 6, reused 4,
    completed 4, and exactly one model request in flight. With one slot, a released summary
    may take it before triage, because summaries outrank triage. This shows reuse does not wait
    behind the queued new work.
  - Summarizing and Scoring reused counts reach 3 as those stages admit. Triage waves are
    chunked at four times the slot count, so summaries of a wave holding new work are released
    only after that work settles.
  - Drive every model completion. After every message assert:
    - reused ≤ completed on every stage;
    - reused is 0 on Scanning, Downloading and Loading;
    - neither `total - reused` nor `completed` decreases.
  - Final per-stage reused is (4, 3, 3). The model request count equals new work only.
    `new_result_count` equals the number of articles scored by a model request.
- **Second run in the same process.** Run again after that run settles.
  - Triaging and Summarizing reused equal their totals (articles left untouched by `admit`).
  - The scoring total and scoring reused are 0 (enqueue refusal; nothing admitted).
  - No model request is issued, and the notice reports 0.
  - Also extend `rerun_counts_current_triage_and_summary_hits_without_readmitting_scoring` to
    assert reused (1, 1) on Triaging and Summarizing and 0 on scoring (edit, not replacement).
- **Prune survival, followed by further admissions and completions.** This pins
  settlement accumulation, not only the post-prune snapshot.
  1. Start a run in which two articles are reused at triage and one new article is triaged by
     a model request.
  2. Mid-run, deliver a `TriageArticlesLoaded` delta that drops one reused article and the
     model-triaged article from the window (which runs `prune`/`retain_members`).
  3. Then deliver a delta that adds one more reusable article and one more new article, and
     drive the new article's triage completion.

  Assert after every message:
  - Triaging reused goes 2, 2, 3 and never drops;
  - completed goes 3, 3, 4, then 5 after the new completion;
  - completed minus reused goes 1, 1, 1, then 2, so the pruned model completion is still
    counted;
  - reused ≤ completed;
  - the new-work total never shrinks.

  Repeat the drop for a summary-stage member, so the summary path is covered too.
- **Stop.** Stop mid-run with new work queued. Reused counts are unchanged by the withdrawal,
  reused ≤ completed holds through the drain, and the settled run's counts keep the reused
  values.
- **Classifier agreement at each stage's admission.**
  - Drive a run message by message. After each message, take the identities newly admitted to
    each stage by that message, and evaluate `unfinished_stage_verdicts` for each in the
    resulting state.
  - The article is counted reused at that stage ⇔ its verdict for that stage is Complete.
    "Settled from the code", item 3, explains why this equals evaluating just before
    admission.
  - The window includes a **partially cached** article: no current triage result, but a
    current summary result (and scoring result) for its content. Its run-start summary verdict
    is `Unknown`. After its triage completes by model request, summary admission reuses the
    saved summary, and the agreement assertion holds at that moment.
  - The test also asserts that the run-start verdict was `Unknown`, documenting why run-start
    verdicts are not the reference. The classifier itself is not changed.
- **Late hit stays new work (R11).**
  - Two URLs with identical content are both new at admission. The first is triaged by a model
    request, and the second then settles from the freshly saved result in dispatch.
  - Exactly one triage model request is made for the content, and Triaging reused stays 0.
  - `total - reused` is unchanged, and completed rises by two: both count as new work.
  - This asserts the documented exception to the literal R6 definition.
- **Contract.** The bridge fixture test (`checked_in_snapshot_fixtures_match_core_projection`)
  covers serialization of the new field. `schema_version_matches_frontend_constant` covers the
  version pairing.

Verification, from `C:\Users\larsp\src\web_page_filet_mignon`:

1. Before changing anything, record the passing test counts of
   `cargo test -p harvester_core`, `cargo test -p harvester_ui_bridge` and
   `cargo test -p harvester_batch`, and the `vitest` total from `npm run check` in `frontend/`.
2. `cargo build`. If a batch is running, use `cargo build --workspace --exclude harvester_batch`
   and run `harvester_batch` tests after it ends.
3. Regenerate fixtures (PowerShell):
   `$env:UPDATE_UI_FIXTURES=1; cargo test -p harvester_ui_bridge; Remove-Item Env:UPDATE_UI_FIXTURES`.
   Then run `cargo test -p harvester_ui_bridge` again without the variable.
4. `cargo test -p harvester_core` and `cargo test -p harvester_batch`. The command-line progress
   tests must pass unchanged in meaning: reused results now settle earlier, but `total`,
   `completed` and `failed` mean the same. If the driver snapshot test with the 20 ms timeout
   fails under full-suite load, rerun it alone before blaming the change.
5. `cargo clippy --all-targets -- -D warnings`, then `cargo fmt`. No `harvester_ui` source
   changes, so `cargo clippy -p harvester_ui` is not required.
6. From `C:\Users\larsp\src\web_page_filet_mignon\frontend`: `npm run check`, `npm run build`,
   `npm run fmt` (re-run `check` if `fmt` changed files).
7. Test counts:
   - Rust totals rise by exactly the added tests, and the fixture list gains one snapshot.
   - `vitest` rises only by the `it.each` case(s) the new fixture adds through
     `reviewFixtures` in `App.test.tsx`.
   - No `#[test]` or `it(` was removed: audit `git diff` for deleted test functions.
8. IPC probe, because the snapshot shape changes: `cargo run -p harvester_ui -- --probe-ipc`
   (needs a display and the GUI lock, about 150 s). Keep the report as
   `.local/probe/ipc-report.reused-work.json` and compare it with the previous report; all gates
   must pass. If no display is available, report the probe as not run. The owner can then run
   it; agents still do not run launchers.
9. `git status` shows no corpus-layout change and no launch-script change.

Human testing: not needed for this phase alone. The display meaning is unchanged until Phase 3,
apart from the end-of-run notice count.

### Phase 3: Desktop bars, text, Stopping counts and ETA use new work

Work, in `frontend/src/components/RunSurface.tsx`:

1. Add the exported `newWork` helper. Use it in `presentStageRows` for the article scale, the
   count text and the Stopping "N done" (Presentation model).
2. Use new-work figures in `estimateActiveStageEtas`.
3. No change to `stageStatus`, the muted and warning rules, accessibility or CSS.

Regression tests (add to `RunSurface.test.tsx`):

- **Owner's run end state** (reused values on the model stages):
  - Scanning sources: total 28, completed 27, failed 1.
  - Downloading: Done, total 25, completed 23, failed 2.
  - Loading: total 158, completed 158.
  - Triaging: total 158, reused 127, completed 152.
  - Summarizing: total 140, reused 118, completed 135.
  - Scoring: total 135, reused 110, completed 135.

  Expected rows: "0 of 28 to do · 1 failed", "0 of 25 to do · 2 failed", "158 done",
  "6 of 31 to do" (19.4%), "5 of 22 to do" (16.1%), "0 of 25 to do" (0%).
  `aria-valuemax` is 31 on all four article meters, and no row text matches `/reused/`.
- **All-reused stage.** Summarizing total 118, reused 118, completed 118, Active: "0 to do",
  empty bar, not muted.
- **Stopping counts are new work.** The same end state under
  `run_state: { Stopping: { in_flight: 1 } }` reads "27 done · 1 failed",
  "23 done · 2 failed", "158 done", "25 done", "17 done" and "25 done", with every meter at 0.
- **ETA uses new work.** Under fake timers, Triaging is Active with a final total of 40,
  reused 30, completed 35, started 50 seconds earlier.
  - `estimateActiveStageEtas` returns 50 seconds and the row reads "about 50 sec left". The
    formula before this phase would give 8 seconds.
  - With completed 30 (nothing new settled) the row reads "Estimating…" and no ETA is
    returned.
- **Clamping.** A stage with reused greater than completed or total renders no negative number
  ("0 to do" or "0 done").
- **Fixture decode.** `run_in_progress_with_reused_results.json` renders its model rows from new
  work. Assert the exact strings from the regenerated fixture's known reducer state.
- **Replayed run sequence with reuse.** Rerender `RunSurface` through these steps and assert each
  row's count text and bar percent (to one decimal):
  1. Run pressed: all "0 to do", Loading "0 done".
  2. Window admitted with reuse settled:
     - Scanning: Done, 28 total, 27 completed, 1 failed.
     - Downloading: total 25, completed 5.
     - Loading: 135.
     - Triaging: total 135, reused 127, completed 127.
     - Summarizing: total 118, reused 118, completed 118.
     - Scoring: total 110, reused 110, completed 110.

     Expected rows: "0 of 28 to do · 1 failed" (0), "20 of 25 to do" (80.0), "135 done",
     "8 of 8 to do" (32.0), "0 to do" (0), "0 to do" (0).
  3. The owner's end state above.
  4. Stopping: the new-work done counts above, all bars empty, plain statuses.

  End by mounting step 3 fresh and asserting identical output (no render memory).

Existing Phase 1 tests must pass unchanged apart from the `reused: 0` literals added in Phase 2.
They use zero reuse, where the new rules reduce to the old ones.

Verification, from `C:\Users\larsp\src\web_page_filet_mignon\frontend`:

1. Record the `vitest` total before the change.
2. `npm run check`, `npm run build`, `npm run fmt` (re-run `check` if `fmt` changed files).
3. The `vitest` total rises by exactly the added tests, and none was removed.
4. `git status` shows changes only under `frontend/src/` for this phase. No Rust change, so no
   Cargo checks; no snapshot change, so no fixture regeneration or probe.

Human testing recommended (owner only; agents do not run launchers): one desktop Run on a normal
morning corpus with mostly already-processed articles. Check that:

- the Triaging, Summarizing and Scoring rows show "N of M to do" with M close to the new work
  (roughly the new-download count), not the window size;
- the bars have visible length and shrink as work completes;
- the "about N min left" estimates look plausible;
- the count text fits at the usual and a narrow window width;
- after Stop mid-run, every bar empties, the counts read "N done" for new work only and the
  status column drops its countdown;
- after a full run, "Run finished - N articles scored" matches the articles actually scored this
  time.

### Phase 4: Documentation and project memory

Documentation only; no builds or tests needed.

1. `docs/Architecture.md`:
   - **Desktop run-surface paragraph** (the one about Run, Process unfinished, Stop and
     "Waiting for articles"). Add:
     - Each stage row's bar shows the work this run still has to do at that stage: admitted
       minus completed and failed, with in-flight counted as waiting.
     - For the model stages, results settled without a model request this run (the snapshot's
       per-stage reused count) are left out of the total.
     - Article stages share the largest new-work total as their scale, while Scanning sources
       uses its own.
     - The count reads "N of M to do" and never mentions reused results.
     - Loading articles has no bar.
     - While Stopping, every bar is empty, counts read this run's new work as "N done", and no
       stage shows an ETA.
     - The ETA uses new-work settlements, and the page computes all of this from the current
       snapshot alone.

     Extend "Open stage totals have no ETA" with "and no stage shows an ETA while the run is
     Stopping".
   - **`RunProgress` paragraph** (lines 82-83). State:
     - Totals still include reused results.
     - Each model stage also carries a reused count: items settled without a model request
       this run, always ≤ completed, resolved when the item is admitted to the stage.
     - An item judged new work at admission stays new work, even if a result produced earlier
       in the same run later lets it settle without its own request.
     - Completed and failed accumulate per run, so they do not fall when a window reload prunes
       admitted articles.
     - Same-digest scoring is not readmitted, so it is neither counted nor reused.
     - The completion notice counts only articles scored by a model request this run.
   - **"Shared article-model request budget"** (lines 189-194). Replace the cache-hit sentence:
     a current saved result is applied when the article is admitted to its stage, without a
     request slot and before any queued new work. Priority for model requests is unchanged.
     A result that appears only after admission is still applied in dispatch without a slot,
     and the item stays counted as new work.
2. `docs/visual_design/VisualDesignSpec.md`, "Desktop run surface". Add a bullet in visual
   terms:
   - Bars show the remaining new work; reused results are not shown, neither as length nor in
     text.
   - A Stopping run shows empty bars, new-work done counts and plain statuses with no ETA.
   - The count text and bar are deliberately not redundant: the bar compares queues across
     stages on one scale, and the text gives the stage's own fraction. This reconciles the
     bullet with "Status Indicators and Progress".
3. `docs/DecisionLog.md`: append one entry, dated with the implementation date. Suggested text:

   ```md
   ## YYYY-MM-DD - Desktop stage bars show remaining work this run actually does
   Decision: Each desktop run-stage bar shows the work this run still has to do at that stage:
   admitted minus completed and failed, with in-flight work counted as waiting. For triage,
   summary and scoring, results settled without a model request this run are excluded: the
   snapshot carries a per-stage reused count next to admitted, completed and failed, whose
   meanings are unchanged, and the desktop subtracts it. Reuse is resolved when an article is
   admitted to a stage, so reused work settles at once, never waits behind queued requests,
   and the new-work total only grows. An item judged new work at admission stays new work even
   if a result produced earlier in the same run later lets it settle without its own request;
   this deliberate exception trades about one item of bookkeeping and ETA precision for never
   paying for a duplicate model call and never shrinking the total. Completed and failed
   accumulate per run and do not fall when a window reload prunes admitted articles. The
   article stages share one scale, the largest current
   new-work total; source scanning, which counts sources, uses its own. Count text reads
   "N of M to do", plus failures, and never mentions reused results. While the run is Stopping
   every bar is empty, counts show this run's new work done, and no stage shows a time-left
   estimate or countdown, only its plain status. Time-left estimates use new-work settlements.
   The end-of-run notice counts only articles scored by a model request this run. The
   article-loading row keeps a plain done count and has no bar until the snapshot carries a
   real loading backlog. Bars and statuses are a pure function of the current snapshot.
   Context: Completed-fraction bars all trended to full. A first remaining-work version still
   counted every window article reused from earlier runs, so on a morning run with about 25 new
   articles the shared scale was 158 and the bars were slivers. Stop withdraws pending model
   work without counting it, so neither remaining work nor a time-left estimate can be shown
   honestly during the drain.
   Consequences: Bars show queue size, not a bottleneck; triage usually holds the longest
   model-stage bar because model work is dispatched scoring, then summary, then triage under one
   budget. The per-stage reused count, meaning "settled without a model request this run", is a
   snapshot contract that any rebuild of run progress keeps along with these tests; it does not
   depend on cache-hit counters, the wave ledger or process-lifetime stage sessions. A rebuild
   should add a per-stage loading backlog so that row can regain a bar. Command-line progress
   keeps its own counts. The count text now carries the denominator, reversing the earlier
   compact-row choice that left the fraction to the bar.
   Refs: frontend/src/components/RunSurface.tsx, crates/harvester_core/src/run_progress.rs,
   crates/harvester_core/src/update/waves.rs, docs/Architecture.md,
   docs/visual_design/VisualDesignSpec.md; 2026-09-07 "Run progress is accumulated in the
   reducer"; 2026-09-27 "Pipeline stages overlap in waves under one request budget"; 2026-09-27
   "Stop halts new work and drains; export waits only for the run".
   ```

4. `docs/EngineeringDiary.md`: append one entry at the end ("Desktop run-stage bars show
   remaining work this run actually does", Type: Implementation) in the file's template. Cover:
   - It reverses the 2026-09-10 "Compact desktop run stage rows" reasoning: the count carries
     the denominator because the bar no longer conveys the stage's own fraction.
   - The owner's desktop run exposed the reuse inflation that the synthetic replay could not,
     because its snapshots had no reuse.
   - Reuse moved from dispatch to admission.

   Lessons:
   - A remaining-work display must define what happens to withdrawn work, or Stop leaves the
     backlog large and the estimate counting forever.
   - A progress denominator must say whose work it counts. Work satisfied from saved results
     belongs to an earlier run.
   - Lazily applied cache hits queue behind slot-bound work, so "instant" reuse is not instant
     unless it is resolved at admission.
   - Replay tests need representative data, including reuse, to catch this.
   - Run counts recomputed from long-lived sessions lose members when the sessions are pruned.
     Per-run counters must accumulate identities themselves.
5. `docs/FutureIdeas.md`, Performance / LlmProcessing: add a Candidate entry
   `[FI-Performance-LlmProcessing-0003] Queue-aware model-stage priority` in the file's entry
   format (Origin: SourceDoc `Plan.RemainingWorkBars.md`, Captured 2026-10-03). A stage whose
   new-work bar stays long relative to the others over time could eventually motivate a shift in
   scheduler priorities away from the fixed scoring, summary, triage order. Future direction
   only.
6. `docs/plans/Plan.Simplification.md`, per "Plan.Simplification notes" below.

Verification:

- Re-read each edited section against the Phase 2 and Phase 3 behaviour.
- Confirm no durable document cites this plan's phase numbers, and that the decision-log entry
  and the Phase 12 note state the meaning, not cache-hit counters (R9).
- `git diff` shows the decision-log and diary changes as pure appends.

## Plan.Simplification notes

Added to `docs/plans/Plan.Simplification.md` on 2026-10-03 with the first version of this plan.
Phase 4 updates them to the reuse meaning:

- **Phase 12, "Run progress" bullet note.** Replace with: the stage bars show the remaining work
  this run actually does, not completed work.
  - For the model stages, items settled without a model request this run are excluded, using
    the snapshot's per-stage reused count, which keeps that meaning.
  - Article stages share one scale (the largest new-work total), Scanning sources uses its own,
    and the count reads "N of M to do".
  - While Stopping every bar is empty, counts show this run's new work and no stage shows an
    ETA.
  - The end-of-run notice counts only articles scored by a model request this run.
  - The rebuild keeps that meaning, the reused field and the tests (see the decision-log entry
    on remaining-work bars). Its per-article pipeline reports reused when a step needs no
    request at admission. A step judged new work at admission stays new work even if a result
    produced earlier in the same run later satisfies it without a request (the documented
    exception). Completed, failed and reused accumulate per run. It should also send a real
    per-stage loading backlog so the Loading articles row can regain a bar.
- **Phase 12, IPC item.** "if it does, bump to 14" becomes "bump to 15". This plan takes 14.
- **Phase 9, open item.** Extend it: decide whether the compact command-line block should also
  report what is left per stage, and whether its totals should exclude work settled without a
  model request this run, consistent with the desktop. The command line keeps its own counts
  until then.

## Documents to update (summary)

- `docs/Architecture.md`: Phase 4 (run surface, `RunProgress`, scheduler cache-hit paragraph).
- `docs/visual_design/VisualDesignSpec.md`: Phase 4.
- `docs/DecisionLog.md`: Phase 4, one new entry. The remaining-new-work meaning, admission-time
  reuse and the reused snapshot field are a contract the pipeline rebuild must keep.
- `docs/EngineeringDiary.md`: Phase 4, implementation entry with lessons.
- `docs/FutureIdeas.md`: Phase 4, queue-aware priority idea.
- `docs/plans/Plan.Simplification.md`: Phase 4, notes updated.
- IPC: `IPC_SCHEMA_VERSION` 13 to 14 (`crates/harvester_ui_bridge/src/ipc.rs`,
  `frontend/src/ipc/schemaVersion.ts`), `frontend/src/ipc/types.ts`, regenerated fixtures plus
  one new fixture, `probe.rs`: Phase 2.
- Not affected:
  - `docs/CorpusFormat.md` and `CORPUS_SCHEMA_VERSION` (no corpus change);
  - launch scripts (no launch-policy change);
  - `README.md` (does not describe the stage bars or the notice);
  - `docs/ApplicationDescription.md` (does not describe run progress).

## Risks

- **Scheduler timing change.** Reused results now settle at admission instead of in dispatch.
  Model-request priority is unchanged, but tests that pinned the old timing may fail and must be
  edited to keep their behavioural point (Phase 2, item 9). Command-line progress sees reused
  results complete sooner; its counts keep their meaning.
- **Admission cost.** Each admitted item gets one extra cache lookup. Triage and summary keys
  are already computed there, so this is negligible at window sizes in the hundreds.
- **Per-run settlement sets.** Each run keeps identity sets of the size of its admitted work,
  which is a few hundred entries. That is negligible next to the existing admitted sets, and
  they are dropped when the next run creates a fresh `PipelineAdmission`. If the
  `record_progress` change is skipped or done partly, pruning can make reused exceed completed.
  The prune-then-admit test pins it.
- **Late hits (R11).** Duplicate content within one run makes a bar and the ETA count about
  one item more than the requests actually made. This is accepted by the owner and pinned by a
  test.
- **Caches not yet hydrated.** Hydration is a startup message. A run admitted before it landed
  would see no reuse at admission; dispatch would still apply those results, counted as new
  work. This is conservative and believed not to occur in practice; no extra handling.
- **Triage cache summary log.** If the metrics reset is not moved before admission, hits
  recorded at admission vanish from the run summary log (Phase 2, item 5).
- **Fixture drift.** `run_finished_with_notice` may change its `new_result_count` if its scoring
  was a reuse. This is expected under R8; review it in the regeneration diff.
- **Probe.** The envelope grows by six small integers. The probe is re-run because the shape
  changed; a failing gate is fixed in delivery, not by loosening it.
- **Count text overflow** (from Phase 1). Mitigated by the Phase 1 layout and the owner's visual
  check. New-work totals are smaller, so the text is shorter.
- **Scale jumps.** When a larger wave of new work is admitted, the shared scale grows and every
  article bar shortens at once. This is expected.
- **Stopping status leaking into normal runs.** Pinned by the paired Active/Stopping test from
  Phase 1.

## Open questions

None. The former Open question 1 (a result that appears only after admission) was settled by
the owner on 2026-10-03 as Option A; see R11.
