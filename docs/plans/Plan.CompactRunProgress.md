# Plan: compact run progress in the desktop Run panel

Written 2026-10-17 from the design brief "Harvester desktop UI: minimal run-progress default".
Revised 2026-10-17 after a single-shot Codex review, which approved the plan with one advisory
recommendation. That recommendation was applied: a Phase 1 test that covers collapse, then a
new snapshot and elapsed time, then expand. The orchestrator settled all six former open
questions, and they are recorded under "Settled by the orchestrator".
This plan is ephemeral: durable documents (VisualDesignSpec, Architecture, DecisionLog, code
comments) must name behaviours, never the phase numbers used here.

## Goal

While a run is active, the desktop Run panel stays calm and one row high by default. In the
header row, where it shows "Running" today, a small block of six hairline bars appears with a
chevron beside it, and the panel shows no text. One click expands today's full view (stage
table and activity feed) in its current place, and another click collapses it.

Done means:

- During a run, by default, the panel shows the compact six-bar block, with no stage text, no
  meters and no activity feed.
- Starting or progressing a run does not by itself change the panel's height. The existing
  notices (reprocess notice at a Process-unfinished start, completion notice at run end,
  AI-unavailable notice) still add their rows as today. That is accepted.
- The toggle expands to exactly today's details (stage table and activity feed, unchanged
  markup and behaviour) and collapses again.
- The idle panel, the Run, Process unfinished and Stop buttons, and all three notices are
  unchanged.
- The keyless IPC probe still measures the heavy view (full table plus a 50-entry feed at
  20 Hz).
- The change is frontend-only. Core, reducer, snapshot IPC, fixtures and persisted formats do
  not change.

## Decisions and who made them

### User decisions (approved)

1. **Compact default.** The header row shows six hairline bars stacked vertically, roughly
   button height and a bit wider, in stage-table order: Scanning sources, Downloading
   articles, Loading articles, Triaging, Summarizing, Scoring signals. A small chevron sits
   beside them.
2. **No visible text in the compact view.** Bar length keeps its current meaning: remaining
   new work this run, from `presentStageRows`. Article stages share one scale and Scanning
   sources has its own. Bars are empty while Stopping.
3. **Disclosure.** Clicking the block or the chevron expands to today's full view in its
   current place, pushing the panes down as today. The chevron flips. Clicking again
   collapses it.
4. **Unchanged:** the Run, Process unfinished and Stop buttons, the completion notice, the
   reprocess notice, the AI-unavailable notice and the idle panel.
5. **Frontend-only.** No core, reducer, IPC or persisted-format change.

### Settled by the orchestrator (cheap to change; open to challenge)

- **Session-only state.** The expanded or compact choice is frontend-local view state for the
  app session, with the same precedent as `searchText` in `App.tsx`. Each launch starts
  compact. Nothing is written to runtime-state persistence.
- **No hover tooltip** in the compact view.
- **Inline expansion.** The expanded view pushes the panes down. It is not an overlay.
- **Track-state rule.** A remaining-work fill can be zero width, so state must not rely on
  fill colour alone. The track of each compact bar carries state:
  - warning colour when the stage has failures or status Failed, so "0 of 28 to do · 1
    failed" still shows red;
  - otherwise a subtle accent tint while the stage is Active, a sign of life even when its
    queue is empty;
  - otherwise dim while the stage is Pending with no work.

  The remaining-work fill draws on top as today. Loading articles has no remaining-work bar
  (DecisionLog 2026-10-03). In the compact view it is a track-only bar that follows the same
  rule, so six bars always show. This stays restrained: no glow and no animation.
- **Accessibility.** The toggle is a real disclosure button. It is keyboard-operable and has
  `aria-expanded`, `aria-controls` pointing at the details region, and an accessible name
  such as "Show run details" / "Hide run details". No `role="meter"` is nested inside the
  button, because ARIA makes button children presentational. The compact bars are decorative
  (`aria-hidden`). The accessible per-stage meters remain those of the full view. When the
  view is expanded, the compact block adds no meter roles.
- **Tests.**
  - Pin the baseline count and delete no tests.
  - Route the full-view tests through an expand step.
  - Add tests for: compact default, toggle, failure track with zero remaining work, active
    track, Loading track-only bar, Stopping, the unchanged idle panel, and session-only
    state.
- **Probe stays expanded.** The probe page starts expanded, based on the frontend's existing
  probe-mode detection, so the gate keeps covering the full table and feed.
- **DecisionLog.** A new entry records:
  - the compact default;
  - the track-state rule;
  - the session-only toggle;
  - that the 50-entry activity feed is still sent on every snapshot, although it is now shown
    only on demand. This puts on record the justification for the delivery-shape change
    anticipated by 2026-09-08.

  Changing feed delivery is out of scope.
- **No "Running" label in either state** (settled 2026-10-17, formerly Open Question 1). The
  disclosure toggle replaces the `Running` text both collapsed and expanded. Only the chevron
  and the accessible name change. Expanding adds exactly today's details region below the
  header.
- **Two block sizes** (settled 2026-10-17, formerly Open Question 2). About 27 px in the wide
  layout and about 22 px at narrow widths, where the header stacks. A run start then does not
  grow the panel at either width.
- **The choice carries over between runs** (settled 2026-10-17, formerly Open Question 3).
  The expanded or compact choice lasts until the app closes. It is not reset when a run
  starts.
- **No in-probe count of drawn activity entries** (settled 2026-10-17, formerly Open
  Question 4). That would need Rust changes to `ProbePageReport`, the probe evaluation and
  `harvester_ui`, which is outside the frontend-only scope. The App test with `?probe=1` and
  the comparison with the baseline probe run are the guard.
- **A FutureIdeas backlog item** (settled 2026-10-17, formerly Open Question 5). Phase 3 adds
  a docs-only `docs/FutureIdeas.md` candidate: send the activity feed only while it is shown,
  or as deltas.
- **Fixed accessible names** (settled 2026-10-17, formerly Open Question 6). The toggle reads
  "Show run details" when collapsed and "Hide run details" when expanded, with no live progress
  summary. The Stop / Stopping… button already announces the run state, and a summary that
  changes at 20 Hz would be noisy for screen readers.

### Settled by this plan (implementation-level; open to challenge)

- **One button wraps the bar block and the chevron.** Clicking either toggles. It replaces the
  `Running` span in `.run-summary` and renders only while `run_progress.run_active` is true.
  It is visible in both states, and only the chevron and the accessible name change. There is
  no "Running" label in either state (settled by the orchestrator).
- **State lives in `RunSurface`.**
  - `useState(initialDetailsExpanded)` holds it, with a new optional prop
    `initialDetailsExpanded?: boolean` that defaults to `false`.
  - `RunSurface` stays mounted for the app's lifetime once a snapshot exists (`App.tsx`
    renders `{snapshot && <RunSurface …/>}`). The choice therefore survives run end and the
    next run start within a session, and resets on launch.
  - No intent, action or reducer is involved. This is purely presentational, like
    `searchText`.
- **One shared probe-mode helper.** `frontend/src/ipc/probeMode.ts` exports
  `isProbePage(search = window.location.search): boolean`.
  - `useSnapshot.ts` and `useProbe.ts` switch to it. Today each repeats
    `new URLSearchParams(window.location.search).has("probe")`.
  - `App.tsx` passes `initialDetailsExpanded={isProbePage()}`.
  - `RunSurface` itself never reads `window.location`.
- **The details region keeps a resolvable id.**
  - While a run is active, the `.run-details` container always renders with an id from
    `useId()`.
  - When collapsed, it carries the `hidden` attribute and has no children: `StageList` and
    `ActivityFeed` are not mounted. `aria-controls` therefore always resolves, and the
    collapsed view does no table or feed render work at 20 Hz.
  - When expanded, its contents are exactly today's.
- **Rows are computed once per render.**
  - `RunSurface` calls `presentStageRows(progress.stages, isStopping)` once and passes the
    rows to both the compact block and `StageList`.
  - The muted and warning predicates `StageList` uses for row classes move into one exported
    pure function, `stageTrackState(stage): "warning" | "active" | "muted" | "neutral"`. Both
    views use it, so the full table and the compact block cannot disagree about which stage is
    in warning.
  - Precedence: warning (failed > 0 or status Failed), then active (status Active), then
    muted (Pending with total 0), then neutral.
  - Row classes keep exactly today's meaning: `--warning` is the warning state, and
    `--muted` is "Pending with total 0" whatever the precedence.
- **Distinct compact markup.** Compact bars use their own class prefix (`run-glance-*`) and
  their own attributes (`data-glance-stage`, `data-track`). They never reuse `data-stage`,
  `.stage-row`, `.stage-bar` or `.stage-bar-slot`. The test helper
  `stageRow()` takes the first `[data-stage=…]` match, and other tests count `.stage-row` and
  `.stage-bar-slot`. The compact block sits before the table in DOM order, so reused hooks
  would break the full-view tests silently.
- **The ETA clock ticks only when it is shown.** `useEtaNow(progress.run_active &&
  detailsExpanded)`. ETAs appear only in the full table, so the collapsed panel does not
  re-render every 5 s. On expansion the hook refreshes `now` at once, as it does today when a
  run starts.
- **No new global design tokens.** `tokens.css` is pinned key by key to `tokenValues.ts` by
  `tokens.test.ts`, and the spec documents those values.
  - The two track tints are local custom properties in `workspace.css`, derived from existing
    tokens with `color-mix()`: for example, Accent Primary at about 30 % over Surface Overlay
    for active, and Accent Warning at about 45 % over Surface Overlay for warning. The dim
    track uses Surface Overlay at reduced opacity, or Border Subtle.
  - Fills keep today's colours: Accent Primary, Accent Warning on warning stages, and Text
    Disabled on muted stages. A warning fill therefore stays distinguishable from its warning
    tint track.
  - The WebView2 runtime the probe records (152.x) supports `color-mix()`. No CSS in the repo
    uses it today.
- **Chevron.** Use `ChevronDown` from `lucide-react`. It is already a dependency but no
  component in `src/` imports it yet. It is rotated 180° by CSS when expanded, sized about
  14 px and `aria-hidden`. A CSS-drawn chevron is an acceptable alternative if the
  implementer prefers not to start using the icon package.
- **Sizing keeps the row height.** In the wide layout, the toggle's outer height must not
  exceed the action buttons' height. Base `button` is 8 px + 21 px + 8 px ≈ 37 px.
  - Wide layout: 2 px bars with 3 px gaps make a 27 px block; with about 4 px vertical
    padding the toggle is about 35 px. Block width is about 44–48 px, so it is a bit wider than
    tall.
  - Narrow layout (`@media (max-width: 50rem)`, where `.run-surface-header` stacks into a
    column): the summary row is only as tall as the `h2` line box, 18 px × 1.3 ≈ 23 px. There,
    the block tightens to 2 px gaps (22 px) with no vertical padding, so the stacked summary
    row does not grow when a run starts. These two sizes were settled by the orchestrator.
- **Toggle style.** The toggle uses ghost-style button styling, overriding the base accent
  background: transparent at rest, Surface Overlay on hover, the base `:focus-visible` ring,
  and Radius Standard (4 px).

## Verified facts (code survey, 2026-10-17)

### `RunSurface.tsx` today

- The header is `.run-summary` (an `h2` "Run" plus `.run-state` reading `Running`, or the
  idle line `Idle · N articles · M ready to archive`) and `.run-actions` (Run, Process
  unfinished, Stop or Stopping…).
- Below the header come, in order: the AI-unavailable `<p role="status">`, the reprocess
  `<p role="status">`, and the completion `.run-notice`. While `run_active`, the
  `.run-details` block follows, holding `StageList` and `ActivityFeed`.
- `presentStageRows` returns `bar: null` for LoadingArticles and remaining-work bars for the
  other five stages. While Stopping, every bar's `remaining` is 0.
- `StageList` sets `stage-row--muted` for `Pending && total === 0`, and `stage-row--warning`
  for `failed > 0 || status === "Failed"`. Each bar is a `div.stage-bar[role=meter]`. Loading
  has a `span.stage-bar-slot[aria-hidden]`.
- `useEtaNow` runs a 5 s interval whenever `run_active`.

### Probe

- The host loads `harvester://localhost/index.html?probe=1&case=…&durationMs=…`
  (`crates/harvester_ui/src/host.rs:597`), one fresh window per case.
- `useSnapshot.ts:12` and `useProbe.ts:10-11` detect probe mode with
  `URLSearchParams(window.location.search).has("probe")`.
- The synthetic run progress in `crates/harvester_ui_bridge/src/probe.rs`:
  - has `run_active: true`;
  - has four Done stages and two Active stages, with one failure on Downloading;
  - carries `ACTIVITY_FEED_CAPACITY` (50) entries on every generation at 20 Hz.

  The probe serves the built `frontend/dist`, so `npm run build` must precede it.

### Tests

- `RunSurface.test.tsx` has **32** `it(` blocks. The brief's "~25" undercounts.
- Most of them query stage rows, meters, ETAs or the feed. All of these depend on the
  details being rendered and need the expand step:
  - `stageRow()`;
  - `.stage-row`;
  - `.stage-bar-slot`;
  - `.stage-count`;
  - `.stage-status`;
  - `getAllByRole("meter")` (expected 5);
  - `getByRole("meter", { name })`;
  - `.activity-entry`.
- These tests do not need the expand step:
  - the AI-unavailable test (idle fixture);
  - the idle summary;
  - the completion notice;
  - the Run button;
  - the unfinished count;
  - the run and stop buttons;
  - Run after Stop;
  - the pure ETA function.
- Two replay tests `unmount()` and render again, so they must expand again after the fresh
  mount.
- The reprocess test uses `getByRole("status")` (singular), so the compact view must not add
  a `role="status"`.

### Styles

- `workspace.css`:
  - run surface styles: lines 28-263;
  - narrow-width rules: lines 571-599 (stack the header, re-template `.stage-list`, collapse
    `.activity-entry`).
- `base.css` gives every `button` the accent background, `--space-compact`/`--space-control`
  padding and a `:focus-visible` ring.
- `tokens.test.ts` pins `tokens.css` key by key against `tokenValues.ts`.

### App tests

- `App.test.tsx` mocks `invoke` and `listen` and emits fixture snapshots.
- It already imports several running fixtures (`overlapping_active_stages`,
  `export_unavailable_during_run`, `reprocess_notice`).
- No App test asserts run-surface stage content.

### Documentation

- `docs/Architecture.md` lines 109-118 describe stage bars and say Loading articles "has a
  plain done count and no bar". That stays true of the full table. The compact view's
  track-only Loading bar needs one clarifying sentence.

## Phases

Run frontend commands from `C:\Users\larsp\src\web_page_filet_mignon\frontend`. Run Cargo
commands from the repository root `C:\Users\larsp\src\web_page_filet_mignon`.

Standard frontend gate, used by every code phase:

```
npm run check   # tsc --noEmit, biome check, vitest run
npm run build   # tsc --noEmit, vite build (also required before the IPC probe)
npm run fmt     # biome format --write
```

- **No Rust changes are planned**, so `cargo build`, `cargo clippy` and `cargo fmt` gates do
  not apply. The orchestrator settled that there is no in-probe count of drawn activity entries.
  If any phase nevertheless ends up touching `crates/harvester_ui`, add
  `cargo clippy -p harvester_ui --all-targets -- -D warnings`, plus the root Rust gate for any
  shared crate.
- **IPC probe.** `cargo run -p harvester_ui -- --probe-ipc`.
  - It needs a display and the GUI lock, so the desktop app must not be running. It takes
    about 150 s, writes `.local/probe/ipc-report.json`, and exits non-zero on a failed gate.
  - If no display or no GUI lock is available, report the probe as not run. Do not substitute
    another check.
- **Tests follow `Agents.md`.** They cover rendered behaviour and public contracts: roles,
  names, `aria-expanded`, and the `data-track` and `data-glance-stage` attributes. They do not
  cover implementation details.
- **Pin the baseline before Phase 1.** Record the `it(` titles of `RunSurface.test.tsx`
  (32 at plan time) and `App.test.tsx`, and the total that `npx vitest run` reports. After
  each phase, confirm that every baseline title still exists, with any edits only adding an
  expand step. Counts may only grow.
- **No live runs.** Do not run Harvester launchers or live LLM APIs. Leave changes
  uncommitted unless authorised.

### Before Phase 1 - Baselines

1. Pin test titles and counts as described above. Run `npx vitest run` from `frontend/`.
2. Run `npm run build`, then the IPC probe on the unchanged tree. Record each gated case's
   `slow_frame_percent`, latency p95 and envelope p95 from `.local/probe/ipc-report.json` in
   the Phase 1 hand-off. If the probe cannot run (no display, or the GUI lock is held), note it
   and run it in Phase 1 only.

### Phase 1 - Compact default with a disclosure toggle (end-to-end slice)

Goal: during a run, the panel shows the six-bar block with fills only and no text, and the
toggle expands to today's details. The probe page starts expanded. Track-state colouring
comes in Phase 2. In this phase every compact track uses the neutral colour, and the Loading
bar is an empty track.

Changes:

1. **`frontend/src/ipc/probeMode.ts`.**
   - Add `isProbePage(search?: string)`.
   - Switch `useSnapshot.ts` and `useProbe.ts` to it. Their behaviour is unchanged:
     `useProbe` still reads `durationMs` from the query itself.
2. **`App.tsx`.** Pass `initialDetailsExpanded={isProbePage()}` to `RunSurface`.
3. **`RunSurface.tsx`.**
   - Add the optional `initialDetailsExpanded` prop and `detailsExpanded` state.
   - Compute `rows = presentStageRows(...)` once.
   - While `run_active`, replace the `Running` span with the disclosure button:
     - `type="button"`, class `run-glance-toggle`;
     - `aria-expanded={detailsExpanded}`, `aria-controls={detailsId}`;
     - `aria-label` "Show run details" when collapsed and "Hide run details" when expanded;
     - its content is an `aria-hidden` `span.run-glance` holding six
       `span.run-glance-bar[data-glance-stage]` elements in snapshot order, followed by the
       `aria-hidden` chevron.
   - Each bar except Loading carries `--stage-progress: <percent>%` from the shared rows, and
     its `::before` fill is drawn as in `.stage-bar`. The Loading bar has no fill.
   - The bars have no `role`, no text and no `title`.
   - Render `.run-details` with `id={detailsId}` while `run_active`. It is `hidden` and empty
     when collapsed. When expanded it holds `StageList` (now taking the shared rows) and
     `ActivityFeed`, with unchanged markup.
   - Gate `useEtaNow` on `run_active && detailsExpanded`.
   - Notices keep their order and markup.
4. **`workspace.css`.**
   - Add `.run-glance-toggle`: ghost styling, hover, and a focus ring from the base styles.
   - Add `.run-glance`: a vertical flex column with fixed bar height and gap.
   - Add `.run-glance-bar` (track) and `.run-glance-bar::before` (fill).
   - Add the chevron rotation keyed off `[aria-expanded="true"]`.
   - Make `.run-details[hidden]` show nothing. The `hidden` attribute suffices because
     `.run-details` sets no `display`. State that in a short comment, or add `display: none`
     explicitly.
   - Narrow-width rule inside the existing `@media (max-width: 50rem)` block: tighter bar gaps
     and no vertical toggle padding (see "Sizing keeps the row height").
   - Confirm the stacked `.run-summary` still wraps cleanly with the toggle beside the `h2`.

Test changes (`RunSurface.test.tsx`):

- **Expand helper.** Add `expandDetails()`, which clicks the "Show run details" button and
  asserts `aria-expanded="true"`. Add `renderExpanded(view)`, which renders and then expands
  and returns the render result.
- **Route the full-view tests through the helper** (the list under Verified facts). For
  tests that `rerender`, the expanded state persists across rerenders, so one expand after
  the first render suffices. For the two replay tests that `unmount()` and render again, call
  `renderExpanded` again for the fresh mount.
- **Change only setup.** Every existing assertion stays as written, including
  `getAllByRole("meter")` `toHaveLength(5)`, the `.stage-row` count of 6 and the
  `.stage-bar-slot` count of 1. These now prove that the compact block adds no meters, rows or
  slots.
- **Idle-surface test.** "collapses to the projected idle summary when no run is active"
  additionally asserts that no "Show run details" button exists.

New tests (`RunSurface.test.tsx`):

1. **Compact by default during a run** (`inProgress`):
   - the toggle has `aria-expanded="false"` and empty text content;
   - no stage label, count or status text is present (for example, no "Scanning sources",
     "Done", "Estimating…" or "Running");
   - `queryAllByRole("meter")` is empty;
   - no "Pipeline stages" list and no `.activity-feed` are rendered;
   - six `.run-glance-bar` elements exist, in stage order by `data-glance-stage`, inside an
     `aria-hidden` container;
   - the element named by `aria-controls` exists and is `hidden`.
2. **The toggle expands and collapses.** After a click:
   - `aria-expanded` is `"true"` and the name reads "Hide run details";
   - six stage rows, five meters and the activity entries are present;
   - the controlled region is no longer `hidden`.

   After a second click, everything returns to the compact state. The toggle is a native
   `<button type="button">` and is focusable, so keyboard operation is the browser's.
3. **Compact fills equal the shared-row percentages.** For `ownerRunEndStages()`, Triaging
   ≈ 19.4 % and Summarizing ≈ 16.1 %, the zero-remaining stages read 0 %, and Loading has no
   `--stage-progress`.
4. **Stopping in compact view** (`stopping.view`):
   - the toggle is still present;
   - every compact fill is 0 %;
   - Stop reads "Stopping…".
5. **The choice is session-only:**
   - expand, rerender an idle view (no toggle), then rerender an active view: the panel is
     still expanded;
   - `unmount()`, then render a fresh active view: it starts compact.
6. **`initialDetailsExpanded`** starts expanded (the probe path), and the toggle still
   collapses it.
7. **The reprocess and completion notices render in compact view** with their existing text,
   and `getByRole("status")` still finds exactly one status in the reprocess fixture.
8. **Expanding after a collapsed stretch shows current details.** This covers the details
   that are unmounted while collapsed and the ETA clock that is suspended while collapsed.
   - Use fake timers. Render an active view whose Triaging stage has a final total, a start
     time and settled work, so it shows an ETA, plus a first activity list.
   - Expand, and record the ETA text ("about 10 sec left" in the shape of the existing
     stalled-estimate test). Then collapse.
   - While collapsed, rerender with a new snapshot whose activity list is different, and
     advance the clock by more than one 5 s ETA interval.
   - Expand again. The feed shows exactly the new snapshot's entries and none of the old
     ones. The Triaging status shows the ETA computed from the advanced time ("about 15 sec
     left" after 5 s), not the value recorded before collapsing.

New tests (other files):

- `frontend/src/ipc/probeMode.test.ts`: `isProbePage("?probe=1&case=x")` is true, `""` and
  `"?other=1"` are false.
- `App.test.tsx`: two tests, each rendering a running fixture.
  - Without `?probe`, the run panel is compact (toggle `aria-expanded="false"`, no "Pipeline
    stages" list).
  - With `window.history.replaceState(null, "", "/?probe=1")` before render (restored in
    `afterEach`), the run panel starts expanded and the "Pipeline stages" list is present.

  This pins the probe's heavy-view coverage keylessly. `useProbe` returns early without
  `durationMs`, and the mocked `invoke` absorbs `probe_ack`.

Verification:

- The standard frontend gate passes.
- The baseline audit passes: no `it(` title is removed from `RunSurface.test.tsx` or
  `App.test.tsx`, and the totals are the baseline plus the new tests.
- IPC probe: run `npm run build`, then `cargo run -p harvester_ui -- --probe-ipc`.
  - All gates must pass.
  - Compare each gated case's `slow_frame_percent` and latency p95 with the baseline. They
    should be flat, because the probe page renders the same expanded details as before.
  - A material change indicates the probe is not measuring what it used to, so investigate
    before continuing.
- **External human testing recommended** (by-eye, the user directly). Look at the panel at the
  start of the next normal desktop run, which the user starts; the assistant does not run
  launchers. This can wait until after Phase 2 so one look covers both. Check:
  - the panel does not grow when the run starts, at full width and below about 800 px wide;
  - the block reads as six bars and the chevron flips;
  - Tab reaches the toggle and Enter or Space toggles it.

#### Phase 1 status (2026-10-18): implemented, verified, user-tested

- **Implemented** as specified above. Two small deviations, both deliberate:
  - toggle hover uses Surface Raised, not Surface Overlay, because Surface Overlay is already
    the compact track colour;
  - `useEtaNow` uses `useLayoutEffect` instead of `useEffect`, so expanding after a collapsed
    stretch shows the refreshed ETA on the first painted frame (new test 8).
- **Automated checks (2026-10-18):** `npm run check` (tsc, biome, vitest: 8 files,
  149 tests passed), `npm run build` and `npm run fmt` (no changes) all pass.
- **Baseline audit:** every baseline `it(` title remains. `RunSurface.test.tsx` went from 32
  to 40 (8 new tests), and `App.test.tsx` from 51 to 53 (the two probe and non-probe tests).
  The new `probeMode.test.ts` has 3 tests.
- **IPC probe:** no probe numbers (baseline or post-change) are on record from the session
  that implemented this phase. The committing session did not run it. Treat the probe
  comparison as outstanding. Phase 2 does not need it, but run it once before close-out if a
  display and the GUI lock are available.
- **User test (Lars, 2026-10-18):** he ran the desktop app with the compact Run progress strip
  and judged that it "looks really good". This is a by-eye approval of Phase 1. The record
  does not say whether he also checked the narrow width or keyboard toggling.

#### Handoff to Phase 2 (for a fresh session)

- Start from `main` after the Phase 1 commit. The tree should be clean apart from your own
  work.
- **Where things are in `frontend/src/components/RunSurface.tsx`:**
  - The compact bars are rendered in `RunSurface` inside the `.run-glance-toggle` button, one
    `span.run-glance-bar[data-glance-stage]` per row from the shared `rows`.
  - The muted predicate is still inline in `StageList`
    (`stage.status === "Pending" && stage.total === 0`), and so is the warning predicate on
    the row class. Phase 2 lifts both into the exported `stageTrackState(stage)`. Keep the row
    classes' exact meaning.
  - Loading currently renders an empty bar with no `--stage-progress`. Phase 2 makes it a
    track-only bar that follows the track-state rule.
- **CSS** lives in `frontend/src/styles/workspace.css` under `.run-glance*`. Today the track
  is `--surface-overlay` and the fill is `--accent-primary`. Put the per-state tint
  percentages in one block of local custom properties on `.run-glance`, and do not add
  global tokens (`tokens.test.ts` pins them).
- **Tests:** helpers `expandDetails()` and `renderExpanded()` already exist in
  `RunSurface.test.tsx`. Pin the new baseline (40 / 53 / 149 total) before changing
  anything, and add the Phase 2 tests listed below.
- **Gate:** the standard frontend gate plus the baseline audit. Then ask Lars for the by-eye
  tint verdict listed under Phase 2 verification. Leave changes uncommitted until he or the
  orchestrator authorises a commit.

### Phase 2 - Track state on compact bars

Goal: each compact bar's track shows the stage's state, so failures and activity are visible
even with an empty fill. Loading becomes a regular track-only bar under the same rule.

Changes:

1. **`RunSurface.tsx`.**
   - Export `stageTrackState(stage)` with the precedence warning, active, muted, neutral.
   - `StageList` derives its existing row classes from the same predicates, with no visible
     change: `--warning` exactly when the state is `"warning"`, and `--muted` exactly when the
     stage is Pending with total 0.
   - Compact bars get `data-track={state}` and a matching class.
2. **`workspace.css`.**
   - Track colours per state, using local `color-mix()` tints of Accent Primary and Accent
     Warning over Surface Overlay, and a dimmer track for muted.
   - Fills stay as today: Accent Primary, Accent Warning on warning bars, Text Disabled on
     muted bars.
   - Nothing animates or glows.
   - Keep the tint percentages in one place, in a short block of local custom properties on
     `.run-glance`, so the by-eye pass can tune them in one edit.

New tests (`RunSurface.test.tsx`):

1. **The pure `stageTrackState` table:**
   - Failed with 0 failures → warning;
   - Active with failures → warning;
   - Active with an empty queue → active;
   - Pending with total 0 → muted;
   - Pending with total > 0 → neutral;
   - Done with no failures → neutral.
2. **A failure track with zero remaining work** (`ownerRunEndStages()`, compact):
   - ScanningSources ("0 of 28 to do · 1 failed") and DownloadingArticles have
     `data-track="warning"` with 0 % fill;
   - Triaging, Summarizing and ScoringSignals are `neutral`.
3. **An active track with an empty queue.** An all-reused Active Summarizing (the existing
   "all-reused model stage" shape) is `active` with 0 % fill.
4. **The Loading track-only bar** follows the rule:
   - Active → `active`;
   - Failed with `failed: 1` → `warning`;
   - Pending with total 0 → `muted`;
   - it never has a fill.
5. **Stopping keeps track states while fills empty.** On the `stopping` fixture, Active stages
   keep `active` and failed stages keep `warning`, and every fill is 0 %.
6. **The full table is unchanged.** The existing "retains warning styling …" and "shows
   zero-total Pending rows as muted …" tests already pin this and must pass unchanged, apart
   from the expand step.

Verification:

- Standard frontend gate and baseline audit.
- The IPC probe is not required for a CSS and attribute change on the compact block, because
  the probe page renders expanded and the compact block's cost is six spans. Run it anyway if
  Phase 1's probe could not run.
- **External human testing recommended** (by-eye verdict, the user directly). On a normal
  desktop run, check that:
  - a failed stage's red track is visible at hairline thickness;
  - the active tint is noticeable but calm;
  - pending stages read as dim;
  - the block is not louder than the header meters (VisualDesignSpec, "Status Indicators and
    Progress": visible without dominating, no ornamental progress).

  If the tints need tuning, change only the local custom properties.

#### Phase 2 status (2026-10-18): implemented, verified, user-tested

- **Implemented** as specified above:
  - `stageTrackState(stage)` is exported. A small exported `isMutedStage(stage)` holds the
    single Pending-with-total-0 rule. Both `stageTrackState` and the `StageList` `--muted` row
    class use it, so the table and the compact block cannot drift apart.
  - Compact bars carry `data-track` and a `run-glance-bar--{state}` class, and Loading is
    track-only.
  - The track colours live in one block of local custom properties on `.run-glance`: active is
    Accent Primary at 30 % and warning is Accent Warning at 45 %, both `color-mix()` over Surface
    Overlay; muted is Border Subtle. No global tokens changed.
- **Review:** approved. Two minor findings were applied: the shared muted helper, and a
  regression test for a Pending, zero-total stage with failures. That stage gets both the
  `--muted` and `--warning` row classes and a warning track.
- **Automated checks (2026-10-18):**
  - `npm run check` passes: tsc, biome, and vitest with 8 files and 162 tests.
  - `npm run build` passes, and `npm run fmt` made no changes.
  - Baseline audit: every one of the 40 Phase 1 `RunSurface.test.tsx` titles remains and no
    test lines were removed. `App.test.tsx` is unchanged.
- **IPC probe:** not run in this phase, which does not require it. The probe comparison owed
  since Phase 1 is still outstanding; run it once before or at close-out if a display and the GUI
  lock are available.
- **User test (Lars, 2026-10-18):** he tested Phase 2 in the desktop app and approved it ("I
  tested phase 2 of the Harvester, approved"), relayed by the orchestrator. This is the by-eye
  tint verdict. The tints are unchanged from the values above.

#### Handoff to Phase 3 (for a fresh session)

- Start from `main` after the Phase 2 commit.
- Phase 3 is documentation-only. Follow the Phase 3 list below, with the DecisionLog draft
  dated on the implementation day.
- Describe the shipped tints in behaviour terms in the documents: accent tint while Active,
  warning tint for failures or Failed, dim for Pending with no work. Do not cite these
  percentages or phase numbers.
- Close-out still owes the IPC probe comparison noted under the Phase 1 status. Record it as
  run, or as not run with the reason.

### Phase 3 - Documentation and close-out

Documentation only. No build or test is needed beyond the final gate already run in Phase 2.

1. **`docs/visual_design/VisualDesignSpec.md`.**
   - "Desktop run surface": add bullets.
     - During a run, the panel shows a compact block of six hairline bars, in stage-table
       order, with a chevron. It has no text and no tooltip, and it keeps the header row
       height.
     - Bar length is remaining new work, with the same scales as the table and empty while
       Stopping.
     - The track carries state: warning tint for failures or Failed, accent tint for Active,
       dim for Pending with no work. Loading is track-only.
     - The block and chevron form one disclosure button that expands today's stage table and
       activity feed inline.
     - The expanded or compact choice lasts for the session, and each launch starts compact.
   - "Status Indicators and Progress": one line saying that the run glance follows the same
     restraint, with tinted tracks and no glow or animation.
2. **`docs/Architecture.md`.** In the stage-bar paragraph (lines 109-118), add that during a
   run the desktop shows these bars by default as a compact, text-free glance. Add that the
   full table and the activity feed are a frontend-local disclosure, and that the compact
   Loading bar is track-only. The core, snapshot and feed delivery are unchanged.
3. **`docs/DecisionLog.md`.** Append a new entry dated on the implementation day. Draft:

   ```md
   ## YYYY-MM-DD - Desktop run progress is a compact glance by default
   Decision: While a run is active the desktop Run panel shows six text-free hairline stage
   bars in its header row, in stage-table order, with a chevron; the block and chevron are one
   disclosure button that expands today's stage table and activity feed inline. Bars keep the
   2026-10-03 remaining-work meaning and scales and are empty while Stopping. Each compact
   track carries state so a zero-width fill still informs: warning when the stage has failures
   or is Failed, an accent tint while Active, dim while Pending with no work; Loading articles
   is a track-only bar under the same rule. The expanded or compact choice is frontend-local
   session state, never persisted; each launch starts compact. The IPC probe page starts
   expanded so its gate keeps measuring the full table and feed.
   Context: The full table and 50-entry feed pushed the workspace down during every run; the
   owner wanted a calm default with the detail one click away. Fill colour alone could not
   show failures or activity when remaining work is zero.
   Consequences: The 50-entry activity feed is still sent on every snapshot although it is
   now rendered only on demand; this is on record as justification for the different
   delivery shape anticipated by 2026-09-08, which remains separate work. Starting a run no
   longer changes the panel height; notices still add rows. Core, snapshot IPC and the
   command-line progress block are unchanged. Compact bars are decorative; the full view's
   meters remain the accessible per-stage values.
   Refs: frontend/src/components/RunSurface.tsx, frontend/src/ipc/probeMode.ts,
   docs/visual_design/VisualDesignSpec.md, docs/Architecture.md; 2026-09-07 "Run progress is
   accumulated in the reducer"; 2026-09-08 "Desktop activity feed holds 50 entries";
   2026-09-09 "Desktop runs never navigate the user"; 2026-10-03 "Desktop stage bars show
   remaining work this run actually does"; 2026-10-04 "The command-line host is kept with a
   compact progress block"
   ```

   The entry records intent only, with no file lists or test results beyond the Refs line,
   per the log's "How to use". It refines 2026-10-03 only in presentation; the Loading row
   still has no remaining-work bar.
4. **`docs/EngineeringDiary.md`.** Append a `Type: Implementation` entry with reusable
   lessons:
   - the compact markup must not reuse the full view's `data-stage` and class hooks, because
     DOM-order-first test helpers silently bind to the wrong element;
   - the probe page must keep the heavy view on by default, pinned by an App test, or the
     frame gate quietly measures less;
   - a disclosure keeps a stable `hidden` region so `aria-controls` resolves without rendering
     the hidden content.
5. **`docs/FutureIdeas.md`.** Add one `Status: Candidate` entry in the file's existing entry
   format, under the most fitting performance or desktop section. The entry:
   - says to send the activity feed only while the run details are shown, or as deltas,
     instead of the full 50 entries on every snapshot;
   - gives its origin in the file's usual `Origin:` form (`SourceDoc:
     Plan.CompactRunProgress.md`, as existing entries cite their source plans);
   - gives as rationale the 2026-09-08 and new DecisionLog entries;
   - sets as success criteria a smaller per-envelope payload during collapsed runs, measured
     by the IPC probe, with the expanded feed unchanged.

   This is documentation only; delivery is not changed here.

Verification: documentation-only. Read-through for accuracy against the shipped behaviour.
Confirm that no durable document cites this plan's phase numbers.

#### Phase 3 status and plan close-out (2026-10-18): complete

- **Documents updated:**
  - `VisualDesignSpec.md`: five bullets under Desktop run surface and one restraint line
    under Status Indicators;
  - `Architecture.md`: a paragraph after the stage-bar paragraph;
  - `DecisionLog.md`: entry "2026-10-18 - Desktop run progress is a compact glance by
    default";
  - `EngineeringDiary.md`: an implementation entry with lessons;
  - `FutureIdeas.md`: candidate `FI-Performance-SnapshotDelivery-0001` with a new
    `SnapshotDelivery` taxonomy row.

  None of them cites this plan's phase numbers.
- **IPC probe comparison (the item owed since Phase 1): done.** It ran on 2026-10-18 on the
  Phase 2 tree, passed every gate and exited 0. The report is saved as
  `.local/probe/ipc-report.compact-phase2.json` and compared with
  `ipc-report.compact-baseline.json` (2026-10-17, pre-change tree):

  | Case | Gated | Slow frames, baseline → now | Latency p95, baseline → now | Envelope p95 |
  |---|---|---|---|---|
  | typical-scope | yes | 0.07 % → 0.00 % | 15 ms → 13 ms | 82 809 B, unchanged |
  | capped-no-checkpoint | yes | 0.00 % → 0.08 % | 24 ms → 33 ms | 219 721 B, unchanged |

  The gated cases are flat. The capped-case latency moved within the 24–42 ms spread of the
  earlier Phase 1 runs, and envelopes are byte-identical, so the probe still measures the full
  table and feed.

  The ungated populated-results case read 17.5 % slow frames. Its earlier runs ranged from
  0.66 % to 34 % on this machine, so it is noise and not gated.
- **Plan complete.** All three phases are implemented, verified and user-approved.

## Documentation obligations (summary)

| Document | Change | Phase |
|---|---|---|
| `docs/visual_design/VisualDesignSpec.md` | Desktop run surface: compact glance, track-state rule, disclosure, session-only; Status Indicators: one restraint line | 3 |
| `docs/Architecture.md` | Stage-bar paragraph: compact default, frontend-local disclosure, track-only compact Loading bar | 3 |
| `docs/DecisionLog.md` | New append-only entry (draft above). This is a durable product commitment, so an entry is warranted | 3 |
| `docs/EngineeringDiary.md` | Implementation entry with lessons | 3 |
| `docs/FutureIdeas.md` | One candidate entry: send the activity feed only while shown, or as deltas | 3 |

No corpus-format, `CORPUS_SCHEMA_VERSION`, IPC schema, fixture, launch-script, changelog or
version change. `docs/CorpusFormat.md` is untouched.

## Risks

- **Silent test re-binding.** Reusing `data-stage` or `.stage-*` hooks in the compact block
  would make `stageRow()` and the counts read the wrong elements. Mitigation: a distinct
  prefix and attributes, and the unchanged `toHaveLength(5)` and `toHaveLength(6)`
  assertions in the expanded tests.
- **The probe measures less without failing.** If the probe flag stopped reaching
  `RunSurface`, the gate would pass on a cheap compact page. Mitigation: the App test with
  `?probe=1`, and the Phase 1 comparison of probe numbers against the baseline. A stronger
  in-probe check was deliberately left out, because it needs Rust changes (settled by the
  orchestrator).
- **Stale details after re-expanding.** Unmounting the details and suspending the ETA clock
  while collapsed could show an old feed or ETA on expansion. Mitigation: Phase 1 test 8.
- **Height invariance cannot be verified in jsdom.** Layout is not computed there.
  Mitigation: explicit sizing constraints and the external by-eye check at two widths.
- **Hairline tints may be too subtle or too loud.** Mitigation: tints live in one block of
  local properties, and the user gives a by-eye verdict.
- **Reduced default information.** ETAs, counts and failure reasons are now one click away.
  This is accepted by the user decision. The warning track keeps failures visible.

## Open Questions

None remain open. The six questions raised by the first draft were settled by the
orchestrator on 2026-10-17:

- no "Running" label in either state;
- two block sizes;
- the choice carries over between runs within a session;
- no in-probe count of drawn activity entries;
- a FutureIdeas candidate for feed delivery;
- fixed accessible names.

They are recorded under "Settled by the orchestrator". They are not user decisions and are
cheap to overturn. If one is overturned, update the matching "Settled by this plan" item, the
Phase 1 sizing, tests and verification, and the Phase 3 documentation list.

The by-eye verdicts in Phases 1 and 2 (panel height at two widths, legibility of the tints)
still need the user directly.
