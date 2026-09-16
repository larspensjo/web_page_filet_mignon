# Plan: "Last 24h" mode for the desktop job list

## Goal

The user collects news over several days before archiving, so the default
*Since checkpoint* view mixes today's downloads with several days of older
ones. Add a third option to the desktop job-list mode switch, **Since checkpoint
/ Last 24h / Results**, that lists only jobs whose `fetched_utc` falls in the
last 24 hours, measured back from the current time.

Done means that in the Tauri desktop window:

- the mode switch has three options, and *Since checkpoint* stays the default;
- *Last 24h* lists every job fetched in the last 24 hours, whether or not it was
  fetched before the archive checkpoint;
- search works in that mode, and the search text carries over when switching
  between *Since checkpoint* and *Last 24h*;
- the same row cap and truncation hint apply;
- jobs without a fetch time are left out, and the existing
  hidden-without-fetch-time hint counts them;
- an empty result has its own message;
- the list updates as time passes, with no user action.

## Settled decisions

From the brief:

- Rolling window: `now - 24h <= fetched_utc`. Rejected: a "latest download run"
  cutoff (needs a persisted run start time, a definition of which actions count
  as a download, and batch-launcher wiring), and a "today" (local midnight)
  cutoff.
- No new persisted data, no new IO path, no `harvester_batch` or launcher
  changes, no corpus-layout change, and no `CommanDuctUI` change.
- The new mode is a third option on the existing switch, not a separate
  narrowing toggle.
- The archive checkpoint is ignored. Rejected: `max(24h, checkpoint)`.
- Search works inside the mode. Default mode stays `SinceCheckpoint`.
- Jobs without `fetched_utc` are excluded.
- The mode has its own empty-state message, separate from the existing three.

From the user's answers during plan review:

- **Label**: the button reads **"Last 24h"**. This is final, not a fallback
  depending on the width check. The internal mode name stays `Last24Hours`.
- **No window-start caption**: the view does not show when the window starts.
  The snapshot has no "now" field, and a page-side clock read could disagree
  with the core's window.
- **Search carries over**: the core keeps one search query. Switching between
  *Since checkpoint* and *Last 24h* keeps it, in both directions, and the
  query is not cleared on mode change.
- **Hint wording unchanged**: during a download, jobs that have no fetch time yet
  are counted in the existing "N jobs are hidden because their fetch time is
  missing." hint. There is no mode-specific wording.

## Constraints from the repository

- **Architecture** (`Agents.md`): input -> action -> reducer -> state -> render,
  with a pure reducer. "Now" comes from `AppState::last_observed_utc()`, which
  `Msg::Tick` sets. The view builder never reads the clock.
- **Decision log**:
  - *Desktop job list has two modes* (2026-09-06): this plan refines it with a
    new entry. The *no unbounded All mode* rule still holds, because the new mode
    is bounded by its time window and by `DESKTOP_JOB_LIST_MAX_ROWS`.
  - *Desktop IPC carries only renderable job rows*: the window filter lives in
    core, next to the scope and search filters, and never in the page.
  - *An empty desktop job list after archive is correct*: its empty states must
    stay distinct, and this plan adds a fourth.
  - *Desktop job list triage ordering* (in `Plan.TauriDesktopUi.md`
    *Information architecture*): the cap picks rows by recency, then display
    order is by priority. That ordering is unchanged.
- **Legacy host**: `JobListScope`, the frozen Win32 scope, is not touched.
- **IPC contract**: the plan says `IPC_SCHEMA_VERSION` is bumped whenever
  `SnapshotEnvelope` or `UiIntent` changes shape. A new `JobListMode` variant
  changes both (`DesktopJobListView.mode` and the `SetJobListMode` payload), so
  it goes from 5 to 6 in `crates/harvester_ui_bridge/src/ipc.rs` and
  `frontend/src/ipc/schemaVersion.ts` together. A stale bundle then shows the
  blocking "out of date" panel instead of silently mishandling an unknown mode.

## How time moves the window (investigated)

- `crates/harvester_ui/src/host.rs` sends `Msg::tick_at(Utc::now())` every
  75 ms.
- The driver loop (`crates/harvester_ui_bridge/src/driver.rs`, `run_driver`)
  rebuilds `state.desktop_view()` after each message batch. It pushes a snapshot
  only when the view differs from `last_view`, rate-limited by
  `SnapshotCoalescer`.
- Result: when a job ages out of the window, the next tick rebuilds the view,
  the rows differ, and one snapshot goes out. Ticks in which no job crosses the
  boundary produce the same view and no snapshot. **No new timer, message or
  emission logic is needed.**
- The per-tick cost is one pass over `self.jobs`. The *Since checkpoint* mode
  already pays this today.
- **Before the first tick** (`last_observed_utc() == None`), the new mode's
  scope is empty: no rows, `scoped_count = 0`, `hidden_without_fetch_time = 0`.
  - Rejected alternative: treating the time as `UNIX_EPOCH`, as the blacklist
    view does. That would put every job fetched in 1969–1970 in the window,
    which is meaningless.
  - In the host this state lasts only until the first tick, a few milliseconds.

## Phase 1: Core mode, filter and IPC contract

Smallest end-to-end slice: the reducer accepts the new mode, the view builder
filters by the window, the wire vocabulary carries it, and the frontend type
union compiles. The mode is not yet selectable in the UI.

### Changes

1. `crates/harvester_core/src/tabs.rs`: add `JobListMode::Last24Hours`. The
   default stays `#[default] SinceCheckpoint`, and the wire string is
   `"Last24Hours"`. Extend the existing default test only if needed.
2. `crates/harvester_core/src/view_model.rs`: add
   `pub const DESKTOP_JOB_LIST_RECENT_WINDOW_HOURS: i64 = 24;` next to
   `DESKTOP_JOB_LIST_MAX_ROWS`, and re-export it from `lib.rs` like the cap.
   It is a named constant, not a config flag: no user setting is proposed.
3. `crates/harvester_core/src/state/view_builder.rs`:
   - Replace the per-job `is_since_checkpoint` test in `select_desktop_job_rows`
     with a small scope predicate chosen by mode:
     - `SinceCheckpoint` behaves exactly as today.
     - `Last24Hours` uses a new free function,
       `is_within_recent_window(job, window_start: Option<DateTime<Utc>>)`.
       It returns `false` when `window_start` is `None` or `job.fetched_utc` is
       `None`, and otherwise `fetched >= window_start`. The window is closed at
       the start; future-dated fetch times are included.
     - `window_start` is `last_observed_utc() - Duration::hours(DESKTOP_JOB_LIST_RECENT_WINDOW_HOURS)`.
   - `hidden_without_fetch_time`:
     - `SinceCheckpoint` keeps today's rule (counted only when a checkpoint
       exists).
     - `Last24Hours` counts jobs with `fetched_utc == None` whenever
       `last_observed_utc` is known, and reports 0 before the first tick.
   - Search, `searched_count`, the cap and `emitted_ids` stay shared across both
     modes. Search is applied before the cap, and the cap keeps the newest rows.
   - `build_desktop_job_list_view`: the selected-job visibility `match` gets a
     `Last24Hours` arm. It shares the `SinceCheckpoint` logic (`OutsideScope` /
     `QueryMismatch` / `Capped` / `Visible`), preferably through an or-pattern
     so the two modes cannot drift apart.
   - The per-row `is_since_checkpoint` flag stays checkpoint-based in every
     mode. It describes the row, not the scope.
   - The core search query is not touched by `JobListModeSet`. This is already
     true today; a test now covers it (see below).
4. `crates/harvester_core/src/msg.rs`: fix the stale doc comment on
   `JobListModeSet` ("Filtering is wired in phase 3") so it describes the mode
   without citing a phase.
5. `crates/harvester_ui_bridge/src/ipc.rs` and
   `frontend/src/ipc/schemaVersion.ts`: bump `IPC_SCHEMA_VERSION` from 5 to 6.
6. `frontend/src/ipc/types.ts`: change the union to
   `JobListMode = "Results" | "SinceCheckpoint" | "Last24Hours"`.
7. `crates/harvester_ui_bridge/src/fixtures.rs`: add a named snapshot,
   `idle_last_24_hours`. Its state:
   - ticks at a fixed time;
   - holds a job fetched inside the window, a job fetched just outside it, a job
     fetched before the checkpoint but inside the window, and a job with no fetch
     time;
   - sets `JobListModeSet { mode: Last24Hours }`.

   Regenerate the fixtures with `UPDATE_UI_FIXTURES=1`. The existing fixtures
   change only in `schema_version`.

### Tests (regression, focused on reducer and view-builder contracts)

Add these to `crates/harvester_core/src/state/tests/mod.rs`, next to the
existing desktop job-list tests:

- **Window boundary**: with `now` ticked, a job at exactly `now - 24h` is listed,
  one at `now - 24h - 1s` is not, and one at `now` is listed.
- **Ignores the checkpoint**: with a checkpoint set after a job's fetch time, the
  job is listed in `Last24Hours` but not in `SinceCheckpoint`. A job older than
  24h but after the checkpoint shows the reverse.
- **No fetch time**: excluded from rows and counted in
  `hidden_without_fetch_time`, even with no checkpoint set.
- **Search within mode**: the query narrows rows; `scoped_count` is the
  post-search count; selected-job visibility reports `QueryMismatch`.
- **Cap**: more than `DESKTOP_JOB_LIST_MAX_ROWS` in-window jobs gives
  `truncated == true` and keeps the newest rows.
- **Search before cap**: this proves that matching articles outside the newest
  400 can still be found by searching.
  - Setup: more than `DESKTOP_JOB_LIST_MAX_ROWS` in-window jobs, where the jobs
    that match the query are all *older* than the newest
    `DESKTOP_JOB_LIST_MAX_ROWS` in-window jobs. Without the query, the matching
    jobs are absent.
  - With the query, every matching job is emitted, `scoped_count` equals the
    match count, and `truncated == false`.
- **Search survives mode switch**: set a query in `SinceCheckpoint`, dispatch
  `JobListModeSet { mode: Last24Hours }`, and check that
  `desktop_job_list.query` is unchanged and still filters. Then switch back and
  check the same.
- **Sliding window**: after a second `Msg::Tick` later than the first, a job that
  was in the window disappears from `desktop_view()`.
- **Before first tick**: `AppState` never ticked, in `Last24Hours` mode, gives
  empty rows, `scoped_count == 0` and `hidden_without_fetch_time == 0`.
- **Selected job outside window**: `OutsideScope`.
- **Mode round-trip**: `UiIntent::SetJobListMode { mode: Last24Hours }` maps to
  `Msg::JobListModeSet`. Extend the existing test in `ui_intent.rs`.

In `crates/harvester_ui_bridge`:

- The existing fixture contract test covers the new fixture.
- Add an assertion that `idle_last_24_hours` carries `mode: "Last24Hours"` and
  exactly the expected row ids.
- Add IPC decode acceptance of `{"type":"SetJobListMode","payload":{"mode":"Last24Hours"}}`
  if the decode tests enumerate modes.

### Verification (from the repository root)

- `cargo build`
- `cargo test -p harvester_core` and `cargo test -p harvester_ui_bridge`
- `$env:UPDATE_UI_FIXTURES=1; cargo test -p harvester_ui_bridge` to regenerate
  fixtures, then re-run without the variable to confirm they are stable
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt`

`harvester_ui` is not expected to change. If it does (for example, to fix a
compile error caused by the new `JobListMode` variant),
`cargo clippy -p harvester_ui --all-targets -- -D warnings` becomes mandatory.
From `frontend/`: `npm run check` (the type union and schema constant),
`npm run build`, `npm run fmt`. No human testing yet: nothing is user-visible.

## Phase 2: Mode switch, search, empty state and documents

### Changes

1. `frontend/src/components/JobList.tsx`:
   - **Mode switch**: add a third `aria-pressed` button, labelled **Last 24h**,
     between *Since checkpoint* and *Results*. Keep the existing `mode-selector`
     styling (see `docs/visual_design/VisualDesignSpec.md`). If three buttons
     don't fit the pane heading, adjust spacing inside the existing selector
     style; do not change the label or introduce a new control.
   - **Search box**: render it for both list modes (`mode !== "Results"`).
   - **Empty state**: `emptyMessage` takes `mode`. The precedence is loading,
     then "No jobs yet.", then "No matches." when a query is set, then a
     mode-specific message: "No jobs since checkpoint." or
     "Nothing fetched in the last 24 hours."
   - **Hints**: the truncation hint and the missing-fetch-time hint already
     render from `list` and stay exactly as they are, wording included.
2. `frontend/src/App.tsx`:
   - The effect that syncs `searchText` with the core query uses
     `mode !== "Results"` instead of `mode === "SinceCheckpoint"`. The search
     box then shows the carried-over query in *Last 24h* and again after
     switching back.
   - `Ctrl+F` focus behaviour is unchanged. It already re-runs on `mode`.
3. `frontend/src/App.test.tsx`:
   - Update "dispatches SetJobListMode from the two-mode selector" to cover three
     modes, including dispatching `Last24Hours` from the button labelled
     "Last 24h".
   - Add rendered tests using the `idle_last_24_hours` fixture:
     - the *Last 24h* button is pressed;
     - the search box is present;
     - the rows match the fixture;
     - the missing-fetch-time hint shows, with its existing wording.
   - Add an empty-state test: a `Last24Hours` list with no rows and no query
     shows "Nothing fetched in the last 24 hours.", and "No matches." when a
     query is set.
   - Add a search carry-over test: a snapshot in `SinceCheckpoint` with a query,
     followed by a snapshot in `Last24Hours` with the same query, keeps that
     text in the search box.

### Documents

- `docs/DecisionLog.md`: append a new entry, dated when the implementation
  lands. Do not edit the 2026-09-06 entries. Draft:

  > **Desktop job list adds a Last 24h mode**
  > **Decision:** The desktop job list offers Since checkpoint (default),
  > Last 24h and Results. Last 24h shows jobs whose fetch time is within
  > 24 hours of host-observed time, regardless of the archive checkpoint, and
  > excludes jobs without a fetch time.
  > **Context:** The user collects over several days before archiving, and
  > needs to see what recent downloads added. A "latest run" cutoff would need
  > persisted run timing and batch wiring. A rolling window needs no new
  > persisted data.
  > **Consequences:**
  > - Refines "Desktop job list has two modes". There is still no unbounded All
  >   mode: the new scope is bounded by time and by `DESKTOP_JOB_LIST_MAX_ROWS`.
  > - Scope and search remain core-owned, and one search query is shared by both
  >   list modes.
  > - The window slides by tick-driven view rebuilds, not a timer, and the view
  >   shows no window-start caption.
  > - The UI distinguishes a fourth empty state, "nothing fetched in the last
  >   24 hours".
  >
  > **Refs:** `crates/harvester_core/src/tabs.rs`,
  > `crates/harvester_core/src/state/view_builder.rs`,
  > `DESKTOP_JOB_LIST_RECENT_WINDOW_HOURS`.

- `docs/plans/Plan.TauriDesktopUi.md`, *Information architecture*: change the
  mode-selector bullet to three modes (Since checkpoint / Last 24h / Results).
  Describe Last 24h as a rolling window that ignores the checkpoint and shares
  the search box, and keep the note that *All* was retired. Also check the
  phase-summary line near "the two-mode `JobListMode` selector" and the
  `JobListMode` enum sketch (`pub enum JobListMode { Results, SinceCheckpoint }`).
  Update them where they describe current behaviour rather than history.
- `docs/visual_design/VisualDesignSpec.md`: update it only if it lists the
  mode-selector options or empty-state strings. The investigation found no such
  listing.
- `docs/EngineeringDiary.md`: add an entry only if something with a reusable
  lesson comes up, such as the selector width needing a spec change or a
  surprise with tick-driven emission. Otherwise leave it alone.
- No `docs/CorpusFormat.md` change, no `CORPUS_SCHEMA_VERSION` bump, no
  `CommanDuctUI` version or changelog change, no launcher change.

### Verification

- From `frontend/`: `npm run check`, `npm run build`, `npm run fmt`.
- From the repository root: `cargo build` and `cargo test -p harvester_ui_bridge`
  (the fixtures and schema-version pairing are unchanged since Phase 1, and this
  confirms it), then `cargo clippy --all-targets -- -D warnings` and
  `cargo fmt`. If `harvester_ui` changed, also run
  `cargo clippy -p harvester_ui --all-targets -- -D warnings`.
- The keyless IPC probe (`cargo run -p harvester_ui -- --probe-ipc`) is **not
  required**:
  - the payload shape for the new mode matches *Since checkpoint* and stays
    within the same row cap;
  - the probe's synthetic views don't exercise the mode.

  Run it only if the frontend change touches list rendering beyond the selector
  and empty state.
- **Recommended human testing** (the user launches the desktop app; agents do not
  run launchers):
  1. After a download, switch to *Last 24h* and confirm the new articles are
     listed and articles older than a day are not, including ones still after
     the checkpoint.
  2. Type a search term and confirm it narrows the list. Switch to
     *Since checkpoint* and back, and confirm the term stays. Clear it and
     confirm the list is restored.
  3. Confirm the three-option switch fits the pane heading in the dark theme.
  4. Optionally, leave the app open across the 24-hour boundary of an older
     article and confirm it drops out without interaction.
  5. Switch back to *Since checkpoint* and confirm it is unchanged.

## Open Questions

None remain. The label, window-start caption, search carry-over and hint
wording were settled during plan review and are recorded under
*Settled decisions*.
