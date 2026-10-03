# Plan: desktop run-stage bars show remaining work

Written 2026-10-03 from the design brief agreed with the owner that day. The brief's decisions
are settled and are not re-opened here; this plan grounds them in the code, settles the routine
details the brief left open, and names the checks and documents each phase needs. It is a small
standalone step on `feature/simplification`, done before the remaining phases of
`docs/plans/Plan.Simplification.md` and not folded into that plan. Revised the same day after a
Codex plan review and the owner's answers: the status column shows no ETA while the run is
Stopping (owner answer, superseding brief decision 8 for that state only), Scanning sources
filling at poll start is recorded as expected behaviour, and the replayed run sequence now drains
a downstream bar from a positive backlog to zero. Phase numbers are local to this plan: durable
documents, code comments and decision-log entries name behaviours, never phases.

## For the owner: what changes

While a run is going, the Run surface shows six stage rows (Scanning sources, Downloading
articles, Loading articles, Triaging, Summarizing, Scoring signals). Today each bar fills up as
the stage completes its work. After this change each bar shows **how much work is still waiting
at that stage**:

- When you press Run nothing has been handed to any stage yet, so the bars are empty. Like every
  bar, Scanning sources grows when work arrives: the poll hands it all sources at once, so it
  fills in one step and then drains as sources finish.
- A downstream bar grows as the stage before it hands work down, shrinks to nothing when it
  catches up, and grows again when more arrives.
- The four article stages (Downloading, Triaging, Summarizing, Scoring) are drawn on one shared
  scale, so you can compare their lengths directly. Scanning sources counts sources, not
  articles, so it has its own scale.
- The text next to each bar says the stage's own share in words, for example "8 of 69 to do", or
  "4 of 22 to do · 3 failed" when some items failed.
- Loading articles has no bar for now (the information needed to draw it honestly is not
  available yet; the planned pipeline rebuild will provide it). It keeps a plain count such as
  "69 done".
- When you press Stop, every bar empties at once, because nothing more will be started; the counts
  switch to plain "N done" and keep rising while in-flight work finishes. The status column stops
  counting down: instead of "about N min left" or "Estimating…" it shows the plain stage status
  ("In progress", "Done", "Pending"). The Stop button already reads "Stopping…".
- Colours, the red failure colour, the muted look of stages that have not started, and the
  "about N min left" style status column during a normal run stay as they are.

What the bars mean: **how much is waiting at each stage, not which stage is slow**. Because all
AI work shares one request budget and is handed out scoring first, then summaries, then triage,
Triaging will usually hold the longest of the AI bars. That is a true queue size, not a sign that
triage is the bottleneck.

Your action: try one desktop run after Phase 1 (human testing recommended there). No questions
are open.

## Goal and definition of done

Each run-stage bar shows remaining work per the settled decisions below; the count text matches;
regression tests cover the rules, including a replayed representative run sequence; the
documents that describe the run surface are accurate; `docs/plans/Plan.Simplification.md` carries
the two notes (already added with this plan); frontend checks pass.

## Settled inputs this plan follows

From the brief (owner decisions, not re-opened):

1. Remaining for a stage = `max(0, total - completed - failed)`, where `total` is what has been
   admitted to that stage so far this run. In-flight items count as remaining.
2. Shared article scale: Downloading articles, Triaging, Summarizing and Scoring signals are
   scaled against the largest current `total` among those four stages in the snapshot. Stage
   totals only grow during a run, so this equals "largest seen this run". It is computed from each
   snapshot; the render keeps no `useRef`/`useState` memory.
3. Scanning sources uses its own scale: its remaining over its own total.
4. Loading articles shows no bar. Its count text stays a plain done count ("69 done", plus
   failures).
5. Bar-row count text: "8 of 69 to do", with " · 3 failed" appended when failures are above zero.
6. While the run is Stopping (`view.run_state` is `{ Stopping: … }`) every bar renders empty.
   Per-stage draining display is out of scope (the snapshot has only an overall `in_flight`).
7. "All bars empty at run end" is not a criterion: `StageList` renders only while
   `run_progress.run_active`, so the rows vanish at run end.
8. Unchanged: warning bar colour for stages with failures or Failed status; muted styling for
   Pending stages with no admitted work; per-stage ETA logic (`estimateActiveStageEtas`); the
   status labels. Refined by owner answer 12 for the Stopping state only.
9. Frontend-only: no engine, reducer or IPC change; `IPC_SCHEMA_VERSION` stays.
10. Standalone step on this branch, ahead of the remaining Plan.Simplification phases.
11. A test replays a representative run's snapshot sequence and asserts bar lengths and text.

Owner answers after the plan review (2026-10-03), settled:

12. While the run is Stopping, the status column shows no ETA: no "about N min left" and no
    "Estimating…"/"Finishing…" countdown fallback. Each row shows its plain stage status instead.
    This supersedes decision 8 for the Stopping state only; outside Stopping the ETA logic and
    status labels are unchanged. It stays a pure function of the snapshot and is covered by a
    test.
13. Scanning sources is empty when Run is pressed and fills in one step when the poll hands it
    all sources at once (`PollStarted { total }` in `update/pipeline_run.rs`), then drains. This
    is expected behaviour, consistent with "bars grow as work arrives", and is documented as
    such.

Repository constraints (`Agents.md`): input -> action -> reducer -> state -> render is preserved,
because the bars are presentation arithmetic over the current snapshot, the same kind of
derivation as the existing per-stage ETA, and they do not classify work. UI follows
`docs/visual_design/VisualDesignSpec.md`. Frontend checks run from `frontend/`. No launchers, API
keys or live LLM calls. `docs/DecisionLog.md` is append-only.

Relevant decision-log entries: 2026-09-07 "Run progress is accumulated in the reducer" (the
source of `total`, `completed`, `failed`); 2026-09-27 "Pipeline stages overlap in waves under one
request budget" (scoring, then summary, then triage, which explains why Triaging usually holds
the longest bar); 2026-09-27 "Stop halts new work and drains; export waits only for the run"
(why remaining work would stay large forever after Stop); 2026-09-27 "The desktop Run is one
primary action" (the run surface this changes).

## Verified facts (grounding)

- `frontend/src/components/RunSurface.tsx`: `progressPercent` returns
  `(completed + failed) / total`; `stageCount` returns "N done" with ", M failed"; `StageList`
  renders every stage with a `div.stage-bar` carrying `role="progressbar"`,
  `aria-valuemax={total}`, `aria-valuenow={min(total, completed + failed)}` and the CSS variable
  `--stage-progress`. `RunSurface` already computes `isStopping` from `view.run_state` for the
  Stop button but does not pass it to `StageList`. `RunSurface` computes `stageEtas` whenever
  `run_active` is true, including while Stopping, and `stageStatus` turns an Active stage with a
  final total into "about N min left", "Estimating…" or "Finishing…". Because `begin_stopping`
  marks every total final, a Stopping run shows those countdown labels today: in
  `stopping_with_in_flight_work.json` the Triaging row (Active, total 1, none settled, total
  final) reads "Estimating…".
- `crates/harvester_core/src/run_progress.rs`: `activate` and `counts` grow `total`,
  `completed` and `failed` with `max()`; `begin_stopping` sets `total_is_final` on every stage
  and returns empty Active stages to Pending; `PipelineActivity::pre_triage_loading` (the real
  loading backlog) is not part of `StageProgress`.
- `crates/harvester_core/src/update/waves.rs` (end of the wave progress update) records Loading
  articles with `completed == total` (the triage admission count), so its remaining is always
  zero. A load failure records `counts(LoadingArticles, 0, 1, 1)` in `update/pipeline_run.rs`.
- `crates/harvester_core/src/update/pipeline_run.rs`: `PollStarted { total }` activates Scanning
  sources with the number of sources at once, so its bar fills in one step when the poll starts
  (empty before that); each
  `SourcePollCompleted`/`SourcePollFailed` settles one. `handle_stop_for_pipeline` withdraws
  never-dispatched triage, summary and scoring entries without counting them completed or failed.
- `crates/harvester_core/src/update/model_dispatch.rs`: `STAGE_PRIORITY` is scoring, summary,
  triage.
- `frontend/src/styles/workspace.css`: `.stage-list` is a four-column grid (name, count 9rem, bar,
  status 9rem; under 50rem: 7rem count column) and each `.stage-row` is a subgrid. Removing the
  bar element from one row would shift its status into the bar column. `.stage-count` is
  `white-space: nowrap` with no overflow handling, so the longer "N of M to do · K failed" text
  can overflow the 9rem (and 7rem) count column.
- `frontend/src/components/StatusMeters.tsx` (the header budget meters) also uses
  `role="progressbar"`; it is not touched by this plan.
- `aria-query` 5.3.0 in `frontend/node_modules` includes the `meter` role, so Testing Library's
  `getByRole("meter")` works.
- Frontend tests decode fixtures generated by `crates/harvester_ui_bridge` (for example
  `run_in_progress_with_failures.json`, `stopping_with_in_flight_work.json`); no fixture changes
  are needed because the snapshot shape does not change.

## Design

### Presentation model (one pure function of the snapshot)

Add an exported pure helper in `RunSurface.tsx`, next to `estimateActiveStageEtas`, for example
`presentStageRows(stages: StageProgress[], stopping: boolean)`, returning per stage: the count
text and either no bar (Loading articles) or `{ remaining, scale, percent }`. `StageList`
receives `isStopping` from `RunSurface` and renders only from this helper's output. No hook,
ref or state is added. The existing `progressPercent` and `stageCount` are replaced by it.

Rules:

| Row | Remaining (bar value) | Scale | Count text (Active run) | Count text (Stopping) |
|---|---|---|---|---|
| Scanning sources | `max(0, total - completed - failed)` | its own `total` | "N of M to do" | "N done" |
| Downloading, Triaging, Summarizing, Scoring signals | same | largest `total` among these four | "N of M to do" | "N done" |
| Loading articles | no bar | none | "N done" | "N done" |

- `percent = scale > 0 ? min(100, remaining / scale * 100) : 0`; while Stopping the displayed
  value and percent are 0 for every bar row.
- A bar row whose `total` is 0 reads "0 to do" (not "0 of 0 to do"). Settled here as routine:
  it stays aligned with the other rows, and the status column already says "Pending" or
  "Waiting for articles".
- Failures append " · K failed" when `failed > 0`, on every row and in every state. The separator
  changes from today's ", " to " · " so all rows read alike (the Run summary line already uses
  " · ").
- While Stopping, every row uses the plain done count. Remaining no longer means anything once
  pending work is withdrawn, while the done count is accurate and still advances as in-flight
  work finishes. This matches the Loading row wording.
- A Done stage mid-run (for example Scanning sources once the poll has ended) reads "0 of 12 to
  do" with an empty bar; its status column says "Done".

### Status column while Stopping (owner answer 12)

- While Stopping, every row's status is its plain stage status label (`statusLabel`: "Pending",
  "In progress", "Done", "Failed"). No "about N min left", "Estimating…", "Finishing…" or
  "Waiting for articles" is shown in that state.
- `stageStatus` takes the stopping flag and returns the plain label first when it is set, and
  `RunSurface` passes an empty ETA list while Stopping (it computes ETAs only when the run is
  active and not Stopping). `estimateActiveStageEtas` itself is unchanged, and outside Stopping
  `stageStatus` behaves exactly as today. Both stay pure functions of the snapshot plus the
  existing clock; the existing `useEtaNow` timer is not changed.

### Accessibility (settled here)

The bar becomes `role="meter"`, not `role="progressbar"`: it shows a quantity on a scale, not
progress toward completion, and the meter role is the ARIA 1.2 fit for that. Attributes:
`aria-label="<Stage> work to do"`, `aria-valuemin=0`, `aria-valuemax=<scale>`,
`aria-valuenow=<displayed remaining>` (0 while Stopping), and `aria-valuetext=<count text>`
so assistive technology reads "8 of 69 to do" rather than a number on the shared scale. This
does not change what a sighted user sees, so it is not raised as an owner question. If Biome's
`useSemanticElements` rule asks for a native `<meter>` element, keep the styled `div` with a
scoped `biome-ignore` comment explaining that the bar is drawn through the `--stage-progress`
custom property (native meter styling relies on vendor pseudo-elements); do not disable the rule
globally.

### Layout

- Loading articles renders an empty, `aria-hidden` placeholder cell in the bar column (for
  example `<span className="stage-bar-slot" aria-hidden="true" />`) so the four-column subgrid
  stays aligned. No bar background is drawn for it.
- Widen the count column in `.stage-list` (and its under-50rem variant) so "999 of 999 to do"
  fits without overlapping the bar, and let `.stage-count` wrap under the narrow layout so a
  " · K failed" suffix moves to a second line instead of overflowing. Final widths are an
  implementation choice, checked visually by the owner (see Phase 1).
- `.stage-bar`, `--stage-progress`, the muted and warning colour rules are reused unchanged.

### Expected behaviour over a run (for the documentation and the replay test)

1. Run pressed: no stage has admitted work; every bar is empty, every bar row reads "0 to do"
   and Loading articles reads "0 done".
2. Poll started: Scanning sources grows as work arrives, like every bar; the poll hands it all
   sources at once, so it fills in one step and then drains as sources settle. Article bars are
   still empty.
3. Downloads and model work arrive in waves: Downloading grows first; Triaging grows as articles
   load; Summarizing and Scoring follow. The article bars share the largest article total as
   their scale, so as more work is admitted the scale itself grows and earlier bars shorten in
   proportion; this is expected.
4. A downstream stage that catches up shows "0 of M to do" and an empty bar, then grows again
   when the next wave arrives.
5. Triaging usually holds the longest model-stage bar, because the shared request budget is
   handed out scoring first, then summary, then triage. That is a queue size, not a slowness
   signal.
6. Stop: every bar empties at once, the counts read "N done", and the status column shows plain
   stage statuses with no countdown.
7. Run end: the stage rows disappear.

## Phases

### Phase 1: Remaining-work bars, count text and tests

Work:

1. Add the pure presentation helper and route `StageList` through it as described in "Design";
   pass `isStopping` from `RunSurface` to `StageList`. Remove `progressPercent` and
   `stageCount`.
2. Apply "Status column while Stopping": `stageStatus` returns the plain status label while
   Stopping, and `RunSurface` computes no ETAs while Stopping.
3. Switch the bar to `role="meter"` with the attributes in "Accessibility"; render the Loading
   placeholder cell.
4. `frontend/src/styles/workspace.css`: count-column width and narrow-layout wrapping per
   "Layout"; no change to bar colours.
5. Update the existing tests in `frontend/src/components/RunSurface.test.tsx` that pinned the old
   meaning: "renders the six projected stages, failure counts and waiting stages" expects
   `/1 done, 1 failed/` and six `.stage-bar` elements; rewrite it to the new text (the fixture's
   Scanning sources row is total 2, completed 1, failed 1, so "0 of 2 to do · 1 failed") and
   five meters plus one Loading placeholder. Tests are edited, never deleted.

Regression tests (add to `RunSurface.test.tsx`; assert rendered output through `RunSurface` and,
where exact numbers are clearer, through the exported helper):

- Remaining, not completed: a Triaging row with total 69, completed 61 renders "8 of 69 to do"
  and a meter with `aria-valuenow` 8.
- Shared article scale: with Downloading total 69 and Summarizing total 22 (15 completed,
  3 failed), Summarizing renders "4 of 22 to do · 3 failed", `aria-valuemax` 69 and a
  `--stage-progress` of about 5.8%; the largest article total sets the scale even when that
  stage is Done.
- Scanning sources uses its own scale: total 12, completed 8 renders about 33.3% regardless of
  article totals.
- Loading articles has no meter, keeps a placeholder in the bar column, and reads "69 done" (and
  "0 done · 1 failed" after a load failure).
- Zero-total row reads "0 to do" with an empty bar and stays muted when Pending.
- Stopping bars and counts: with `run_state: { Stopping: { in_flight: 1 } }` every meter has
  `aria-valuenow` 0 and a 0% bar, and every count reads "N done" (use
  `stopping_with_in_flight_work.json`, whose Triaging row has total 1 and completed 0).
- Stopping status (owner answer 12): in `stopping_with_in_flight_work.json` the Triaging row
  reads "In progress", not "Estimating…"; and a Stopping view whose Active, final-total stage
  would otherwise yield an ETA (for example Triaging total 4, 2 settled, started 30 seconds
  earlier under fake timers) reads "In progress", with no row matching `/about .* left/`,
  "Estimating…" or "Finishing…". The same stages with `run_state: "Active"` still show the ETA,
  pinning that only the Stopping state changed.
- Warning colour retained: a row with failures keeps `stage-row--warning`.
- Replayed run sequence: one test rerenders `RunSurface` through a hand-built sequence of stage
  snapshots and asserts, at every step, each row's bar percent (to one decimal) and count text:
  1. Run pressed: all bars empty, bar rows "0 to do", Loading "0 done".
  2. Poll started: Scanning sources "12 of 12 to do", full bar; article bars empty.
  3. First wave: Downloading total 30 with 10 done (66.7%), Triaging total 10 (33.3%), Loading
     "10 done".
  4. Scoring backlog: Downloading total 69 with 40 done, Triaging total 40 with 20 done,
     Summarizing total 15 with 10 done, Scoring total 5 with 1 done ("4 of 5 to do", bar 4/69,
     about 5.8%).
  5. Later wave, Scoring drained: Downloading 69 all done, Triaging "8 of 69 to do",
     Summarizing "4 of 22 to do · 3 failed", Scoring "0 of 5 to do" with an empty bar (the same
     downstream bar has gone from a positive backlog to zero).
  6. Regrowth: Scoring total 12 with 5 done, "7 of 12 to do", bar 7/69.
  7. Stopping: all bars empty, counts "N done", statuses plain.

  The test ends by mounting the regrowth snapshot fresh and asserting identical output, pinning
  that the bars are a pure function of the snapshot with no render memory.

Existing ETA and status tests must keep passing unchanged: they render non-Stopping views, where
decision 8 still holds.

Verification, from `C:\Users\larsp\src\web_page_filet_mignon\frontend`:

1. Record the `vitest` total before the change (from `npm run check`).
2. `npm run check` (type check, Biome, Vitest), `npm run build`, `npm run fmt`; if `fmt` changed
   files, re-run `npm run check`.
3. The `vitest` total rises by exactly the added tests; no test was removed.
4. `git status` shows changes only under `frontend/src/` (and this plan's docs in Phase 2). No
   Rust file changes, so no Cargo checks; no IPC change, so no fixture regeneration and no IPC
   probe.

Human testing recommended (owner only; agents do not run launchers): one desktop Run on a
normal morning corpus, watching that Scanning sources fills then drains, downstream bars grow,
shrink to empty and regrow, the count text fits beside the bars at the usual window width and at
a narrow width, then Stop mid-run to see every bar empty, the counts switch to "N done" and the
status column drop its countdown.

### Phase 2: Documentation and project memory

Documentation only; no builds or tests needed.

1. `docs/Architecture.md`, desktop run-surface paragraph (the one that describes Run, Process
   unfinished, Stop and "Waiting for articles"): add that each stage row's bar shows the work
   still waiting at that stage (admitted minus completed and failed, in-flight counted as
   waiting); article stages share the largest article-stage total as their scale while Scanning
   sources uses its own; the count reads "N of M to do"; Loading articles has no bar because the
   snapshot carries no loading backlog; every bar is empty while Stopping, counts read "N done"
   and the status column shows plain stage statuses without an ETA; the page computes this from
   the current snapshot alone. Also extend the existing sentence "Open stage totals have no
   ETA" with "and no stage shows an ETA while the run is Stopping".
2. `docs/visual_design/VisualDesignSpec.md`, "Desktop run surface": add a bullet stating the same
   meaning in visual terms (including that a Stopping run shows empty bars and plain statuses
   with no ETA), and that the count text and bar are deliberately not redundant: the
   bar compares queues across stages on one scale, the text gives the stage's own fraction. This
   reconciles the change with "Status Indicators and Progress", which warns against redundant
   numeric and bar treatments.
3. `docs/DecisionLog.md`: append a new entry, suggested text:

   ```md
   ## YYYY-MM-DD - Desktop stage bars show remaining work
   Decision: Each desktop run-stage bar shows the work still waiting at that stage this run:
   admitted minus completed and failed, with in-flight work counted as waiting. The article
   stages (downloading, triage, summary, scoring) share one scale, the largest current
   article-stage total; source scanning, which counts sources, uses its own total. Count text
   reads "N of M to do", plus failures. While the run is Stopping every bar is empty, counts
   show completed work, and no stage shows a time-left estimate or estimating/finishing
   countdown, only its plain status; outside Stopping the per-stage estimates are unchanged. The
   article-loading row has no bar until the snapshot carries a real loading backlog. Bars and
   statuses are a pure function of the current snapshot.
   Context: Completed-fraction bars all trended to full and did not show where work was waiting.
   Stop withdraws pending model work without counting it, so neither remaining work nor a
   time-left estimate can be shown honestly during the drain, and the snapshot has only an
   overall in-flight count.
   Consequences: Bars show queue size, not a bottleneck; triage usually holds the longest
   model-stage bar because model work is dispatched scoring, then summary, then triage under one
   budget. Any rebuild of run progress keeps this meaning and its tests, and should add a
   per-stage loading backlog so that row can regain a bar. The count text now carries the
   denominator, reversing the earlier compact-row choice that left the fraction to the bar.
   Refs: frontend/src/components/RunSurface.tsx, docs/Architecture.md,
   docs/visual_design/VisualDesignSpec.md; 2026-09-07 "Run progress is accumulated in the
   reducer"; 2026-09-27 "Pipeline stages overlap in waves under one request budget"; 2026-09-27
   "Stop halts new work and drains; export waits only for the run".
   ```

4. `docs/EngineeringDiary.md`: append an implementation entry ("Desktop run-stage bars show
   remaining work") at the end of the file, noting that it reverses the 2026-09-10 "Compact
   desktop run stage rows" reasoning (the count no longer omits the denominator, because the bar
   no longer conveys the stage's own fraction) and recording the reusable lesson: a
   remaining-work display, and any estimate derived from it, must define what happens to
   withdrawn work, or Stop leaves the backlog large and the time-left estimate counting forever.
5. `docs/FutureIdeas.md`, Performance / LlmProcessing: add a Candidate entry
   `[FI-Performance-LlmProcessing-0003] Queue-aware model-stage priority` in the file's entry
   format (Origin: SourceDoc `Plan.RemainingWorkBars.md`, Captured 2026-10-03): a stage whose
   remaining-work bar stays long relative to the others over time could eventually motivate a
   shift in scheduler priorities away from the fixed scoring, summary, triage order. Future
   direction only, not work in this plan.

Verification: re-read each edited section against the Phase 1 behaviour; confirm no durable
document cites this plan's phase numbers; `git diff` shows the decision-log and diary changes as
pure appends.

## Plan.Simplification notes (done with this plan)

Added to `docs/plans/Plan.Simplification.md` on 2026-10-03, as the owner asked:

- Phase 12, "Run progress" bullet: the stage bars show remaining work (shared article scale,
  "N of M to do", empty and without ETAs while Stopping); the rebuild keeps that meaning and its tests, and should
  send a real per-stage loading backlog so the Loading articles row can regain a bar.
- Phase 9, compact command-line progress: an open item to decide whether the block should also
  report what is left per stage, consistent with the desktop.

## Documents to update (summary)

- `docs/Architecture.md`: Phase 2.
- `docs/visual_design/VisualDesignSpec.md`: Phase 2.
- `docs/DecisionLog.md`: Phase 2, one new entry (the remaining-work meaning is a contract the
  pipeline rebuild must keep).
- `docs/EngineeringDiary.md`: Phase 2, implementation entry.
- `docs/FutureIdeas.md`: Phase 2, queue-aware priority idea.
- `docs/plans/Plan.Simplification.md`: done.
- Not affected: `docs/CorpusFormat.md` and `CORPUS_SCHEMA_VERSION` (no corpus change),
  `IPC_SCHEMA_VERSION` and fixtures (no snapshot change), launch scripts, `README.md` (does not
  describe the stage bars), `docs/ApplicationDescription.md` (does not describe run progress).

## Risks

- **Count text overflow.** The longer wording can overflow the count column, especially in the
  narrow layout. Mitigated by the Layout changes and the owner's visual check; jsdom tests
  cannot catch it.
- **Biome a11y rule on `role="meter"`.** Handled by a scoped, explained `biome-ignore` if it
  fires (see Accessibility).
- **Scale jumps.** When a larger wave is admitted the shared scale grows and every article bar
  shortens at once. This follows from decision 2 and is documented as expected.
- **Stopping status leaking into normal runs.** The no-ETA rule must apply only while
  `run_state` is Stopping. The paired Active/Stopping status test pins this, and the existing
  ETA tests (all non-Stopping) must pass unchanged.

## Open questions

None. The two questions from the first draft were settled by the owner on 2026-10-03 (see
"Settled inputs", answers 12 and 13).
