# Design: "Since Checkpoint" Left-Pane Tab

**Date:** 2026-03-01
**Status:** Revised after second review

---

## Context

The left pane currently has two tabs: **Jobs** (all fetched articles) and **Prompt Lab**. Users need a focused view of only the articles fetched since the last briefing checkpoint — the same subset that would appear in the next Archive export or Briefing. A dedicated **Since Checkpoint** tab provides a quick way to see what's new without scrolling through older jobs.

---

## Shared Algorithm

All three features (Archive, Briefing, Since Checkpoint tab) use the same rule:

> Include a job/article if `fetched_utc >= briefing_since_utc`, or if `fetched_utc` is missing/malformed (include by default).

Archive and Briefing apply this by reading `fetched_utc` from article frontmatter on disk (in `harvester_engine/src/export.rs:passes_since_filter` and `harvester_engine/src/briefing.rs:scan_and_prepare_articles`). The new tab applies the same rule using `fetched_utc` stored on `JobState` in memory, populated at job-completion time from the same source value (`(config.fetched_utc)()` in `engine.rs:435`).

A shared helper is **required** to prevent semantic drift across the three implementations:

```rust
pub fn passes_since_filter_dt(
    fetched_utc: Option<DateTime<Utc>>,
    since: Option<DateTime<Utc>>,
) -> bool {
    match (fetched_utc, since) {
        (_, None) => true,
        (None, Some(_)) => true,        // include if missing (consistent with archive/briefing)
        (Some(t), Some(s)) => t >= s,
    }
}
```

Place this in `harvester_core/src/since_filter.rs` (new file) and import it from all three call sites: the UI view path, and optionally the export/briefing engine paths as a follow-up.

**Design note — `None` semantics:** Jobs with `fetched_utc = None` are included in Since Checkpoint by the fallback rule. This covers three cases: (a) in-flight jobs (no timestamp yet), (b) failed jobs (engine never wrote frontmatter), and (c) legacy jobs restored from session files that predate this field. All three are treated as "include by default" for safety — the same behavior as Archive/Briefing. This is intentional and consistent. Users who want to exclude failed jobs can filter by status separately; that is out of scope here.

**Design note — legacy persisted jobs:** On first startup after this change, all historical jobs restored from existing session files will have `fetched_utc = None` and will appear in Since Checkpoint. This is acceptable: it matches the archive/briefing fallback, and correct timestamps will propagate from the next fetch cycle onward. No backfill from article frontmatter is needed.

**Design note — selection behavior when filtered:** If the currently selected job is not in the Since Checkpoint view, the selection is preserved but no preview is shown (or first visible item is selected, depending on existing behavior). This is out of scope for this change.

---

## Data Flow

```
engine.rs:(config.fetched_utc)()  ΓåÆ  JobOutcome.fetched_utc (String)
    ΓåÆ Msg::JobDone.fetched_utc (String)
    ΓåÆ update.rs: destructured in Msg::JobDone arm; passed to state.apply_done()
    ΓåÆ JobState.fetched_utc (Option<DateTime<Utc>>, parsed in apply_done())
    ΓåÆ PersistedJob.fetched_utc (persisted as Option<String>)
    ΓåÆ JobRowView.is_since_checkpoint (bool, computed in AppState::view())
    ΓåÆ render.rs: filtered job list when LeftTab::SinceCheckpoint is active
```

The same `fetched_utc` value the engine writes into the article frontmatter (read by briefing/archive) is now also returned in `JobOutcome`, so both paths always agree.

---

## Implementation Steps

### Step 1 — Thread `fetched_utc` from engine through to `JobState`

**`crates/harvester_engine/src/types.rs`**
- Add `fetched_utc: Option<String>` to `JobOutcome`

**`crates/harvester_engine/src/engine.rs`** (around line 435 / 457ΓÇô466)
- The `(config.fetched_utc)()` call already produces the RFC3339 string written to frontmatter
- Capture it into a local variable and include it in `JobOutcome`

**`crates/harvester_core/src/msg.rs`** (lines 51ΓÇô56, `Msg::JobDone`)
- Add `fetched_utc: Option<String>` to `Msg::JobDone`

**`crates/harvester_io/src/effect_runner.rs`** (lines 1118ΓÇô1138, `EngineEvent::JobCompleted` handler)
- Pass `outcome.fetched_utc` into `Msg::JobDone`
- Failed jobs: `fetched_utc: None` (already handled by existing `Err` branch)

**`crates/harvester_core/src/update.rs`** (around line 130, `Msg::JobDone` match arm)
- Destructure the new field: `Msg::JobDone { ..., fetched_utc } =>`
- Pass `fetched_utc` into `state.apply_done(...)`
- **Update all `Msg::JobDone` constructors repo-wide** (see fixture checklist below)

**`crates/harvester_core/src/state.rs`**
- Add `fetched_utc: Option<DateTime<Utc>>` to `JobState` (around lines 2181ΓÇô2190)
- Add or confirm `chrono` import: `use chrono::{DateTime, Utc};`
- In `apply_done()` (around line 1842), parse the RFC3339 string:
  ```rust
  job.fetched_utc = msg_fetched_utc
      .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
      .map(|dt| dt.with_timezone(&Utc));
  ```

**Fixture update checklist — `Msg::JobDone`:** Add `fetched_utc: None` (or a test value) to every `Msg::JobDone { ... }` construction in:
- `crates/harvester_core/src/update.rs` (inline tests)
- `crates/harvester_core/src/state.rs` (inline tests)
- `crates/harvester_core/tests/update_jobs.rs`
- `crates/harvester_core/tests/triage_orchestration.rs`
- `crates/harvester_app/src/platform/app.rs` (test section around line 885)
- `crates/harvester_app/src/platform/ui/render.rs` (test section around line 1712)

### Step 2 — Persist `fetched_utc` across restarts

Without persistence, all historical jobs reloaded from a previous session will have `fetched_utc = None`, causing them all to appear in the Since Checkpoint tab (due to the missing-value fallback). Persisting the timestamp prevents this for jobs fetched after this change ships.

**`crates/harvester_core/src/state.rs`** — `CompletedJobSnapshot`
- Add `fetched_utc: Option<String>` (store as RFC3339 string for serialization simplicity)
- Populate from `job.fetched_utc.map(|dt| dt.to_rfc3339())` when building the snapshot

**`crates/harvester_io/src/persistence.rs`** — `PersistedJob`
- Add `fetched_utc: Option<String>` with `#[serde(default)]` for backward compatibility with existing session files
- Restore by mapping back to `CompletedJobSnapshot.fetched_utc`

**`crates/harvester_core/src/state.rs`** — `AppState::restore_completed_jobs`
- Parse `snapshot.fetched_utc` back into `Option<DateTime<Utc>>` and assign to `job.fetched_utc`:
  ```rust
  job.fetched_utc = snapshot.fetched_utc
      .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
      .map(|dt| dt.with_timezone(&Utc));
  ```

**Fixture update checklist — `CompletedJobSnapshot`:** Add `fetched_utc: None` to every `CompletedJobSnapshot { ... }` construction in:
- `crates/harvester_core/tests/persistence.rs` (around line 39)
- `crates/harvester_io/src/persistence_worker.rs` (test section around line 225)
- Any other inline tests in `state.rs`

### Step 3 — Extract shared since-filter helper

**`crates/harvester_core/src/since_filter.rs`** (new file)
- Define `pub fn passes_since_filter_dt(fetched_utc: Option<DateTime<Utc>>, since: Option<DateTime<Utc>>) -> bool` as shown above
- Add unit tests covering: `since=None` (always true), `fetched_utc=None` with `since=Some` (true), `fetched_utc=Some(t >= s)` (true), `fetched_utc=Some(t < s)` (false)

**`crates/harvester_core/src/lib.rs`**
- Declare `pub mod since_filter;`

### Step 4 — Compute `is_since_checkpoint` in the view model

`JobState` does not have access to `self.briefing_since_utc()`, so the boolean must be computed at the `AppState` level where that method is available, and passed into the view row.

**`crates/harvester_core/src/view_model.rs`** (around line 937, `JobRowView`)
- Add `is_since_checkpoint: bool`

**`crates/harvester_core/src/state.rs`** — `JobState::to_view` (or equivalent row-building function)
- Extend the signature to accept `is_since_checkpoint: bool` as a parameter
- Assign it directly to `JobRowView.is_since_checkpoint`

**`crates/harvester_core/src/state.rs`** — `AppState::view()` function (around lines 537ΓÇô687)
- In the `jobs.iter()` map loop, compute `is_since_checkpoint` before calling `to_view`:
  ```rust
  let since = self.briefing_since_utc();
  // inside the loop:
  let is_since = passes_since_filter_dt(job.fetched_utc, since);
  job.to_view(is_since, ...)
  ```

### Step 5 — Add `SinceCheckpoint` tab variant

**`crates/harvester_core/src/tabs.rs`**
- Add `SinceCheckpoint` to `LeftTab` between `JobList` and `PromptLab`
- Update `to_index()`:
  - `JobList` ΓåÆ 0
  - `SinceCheckpoint` ΓåÆ 1
  - `PromptLab` ΓåÆ 2
- Update `from_index()` and any exhaustive match arms

### Step 6 — Fix reducer tab-selection logic (P0)

**`crates/harvester_core/src/update.rs`** (around line 1048, `Msg::LeftTabSelected`)
- Current behavior: any non-`PromptLab` tab calls `close_prompt_lab()`, which resets the active tab to `LeftTab::JobList`. This will silently override `SinceCheckpoint` back to `JobList`.
- **Fix:** Change the `Msg::LeftTabSelected` arm to set the selected tab directly, and only use `close_prompt_lab()` as a side-effect toggle for Prompt Lab open/close state — not to determine which tab is active. The selected tab should always be set to the value from the message.

**`crates/harvester_core/src/state.rs`** — `close_prompt_lab()` (around line 1989)
- Verify this function does not unconditionally set `left_tab = LeftTab::JobList`. If it does, split the Prompt Lab close side-effect from the tab-selection assignment so both can be controlled independently.

### Step 7 — Register tab in the UI and fix layout

**`crates/harvester_app/src/platform/ui/layout.rs`** (lines 144ΓÇô149)
- Add `"Since Checkpoint".to_string()` as index-1 item in the tab bar vec
- **Fix layout collapse:** The `left_tab_dock` and `left_tab_size` closures (around line 1109) currently only show `PANEL_LEFT_JOBS` for `LeftTab::JobList`. Update them so the panel remains visible for both `JobList` and `SinceCheckpoint`:
  ```rust
  let show_jobs = matches!(left_tab, LeftTab::JobList | LeftTab::SinceCheckpoint);
  let jobs_dock = if show_jobs { DockStyle::Fill } else { DockStyle::Top };
  let jobs_size = if show_jobs { None } else { Some(0) };
  ```
  This also preserves the URL input bar for both job-related tabs, which is correct UX — Since Checkpoint is an alternate view of the same core list.

**`crates/harvester_app/src/platform/ui/render.rs`**
- Handle `LeftTab::SinceCheckpoint` in tab-selection render (index 1)
- In job list rendering (around line 1344, `build_job_tree`): when `left_tab == LeftTab::SinceCheckpoint`, filter `view.jobs` to rows where `is_since_checkpoint == true` before building tree items

### Step 8 — Engineering diary entry

Add a draft diary entry to `docs/EngineeringDiary.md` as part of this change:

**Draft entry:**

```
## Since Checkpoint Tab (2026-03-01)

**Context:** Users needed a way to see only articles fetched since the last briefing
checkpoint without scrolling through older jobs. Archive and Briefing already had
this filter; the UI had no equivalent.

**Change:** Added a new `SinceCheckpoint` left-pane tab. Threaded `fetched_utc`
from the engine through `JobOutcome` ΓåÆ `Msg::JobDone` ΓåÆ `JobState` ΓåÆ `JobRowView`.
Persisted via `PersistedJob` for restart correctness. Fixed reducer tab-selection
logic that was overriding any non-PromptLab tab to `JobList`. Extracted shared
`passes_since_filter_dt` helper to keep semantics consistent across Archive,
Briefing, and the new tab.
```

Finalize with **Evidence** (manual verification results) and **Lessons Learned** once implementation is complete.

---

## Files Modified

| File | Change |
|------|--------|
| `crates/harvester_engine/src/types.rs` | Add `fetched_utc: Option<String>` to `JobOutcome` |
| `crates/harvester_engine/src/engine.rs` | Populate `fetched_utc` in `JobOutcome` |
| `crates/harvester_core/src/msg.rs` | Add `fetched_utc: Option<String>` to `Msg::JobDone` |
| `crates/harvester_io/src/effect_runner.rs` | Thread `fetched_utc` into `Msg::JobDone` |
| `crates/harvester_core/src/update.rs` | Destructure `fetched_utc` in `Msg::JobDone` arm; fix `Msg::LeftTabSelected` to preserve selected tab without overriding to `JobList`; update all test fixtures constructing `Msg::JobDone` |
| `crates/harvester_core/src/state.rs` | Add `fetched_utc` to `JobState`; parse in `apply_done()`; compute `is_since_checkpoint` in `view()` and pass into `to_view()`; add chrono import; update `restore_completed_jobs`; add `fetched_utc` to `CompletedJobSnapshot`; fix `close_prompt_lab()` to not reset active tab |
| `crates/harvester_io/src/persistence.rs` | Add `fetched_utc: Option<String>` with `#[serde(default)]` to `PersistedJob` |
| `crates/harvester_core/src/since_filter.rs` | New file: `passes_since_filter_dt` helper + unit tests |
| `crates/harvester_core/src/lib.rs` | Declare `pub mod since_filter` |
| `crates/harvester_core/src/view_model.rs` | Add `is_since_checkpoint: bool` to `JobRowView` |
| `crates/harvester_core/src/tabs.rs` | Add `SinceCheckpoint` to `LeftTab`; update index mapping |
| `crates/harvester_app/src/platform/ui/layout.rs` | Add "Since Checkpoint" to tab bar; fix dock/size closures to keep job panel visible for both job tabs |
| `crates/harvester_app/src/platform/ui/render.rs` | Handle new tab variant; filter jobs when tab is active |
| `crates/harvester_core/tests/update_jobs.rs` | Add `fetched_utc: None` to `Msg::JobDone` fixtures |
| `crates/harvester_core/tests/triage_orchestration.rs` | Add `fetched_utc: None` to `Msg::JobDone` fixtures |
| `crates/harvester_core/tests/persistence.rs` | Add `fetched_utc: None` to `CompletedJobSnapshot` fixtures |
| `crates/harvester_io/src/persistence_worker.rs` | Add `fetched_utc: None` to `CompletedJobSnapshot` fixtures |
| `crates/harvester_app/src/platform/app.rs` | Add `fetched_utc: None` to `Msg::JobDone` fixtures |
| `crates/harvester_app/src/platform/ui/render.rs` | Add `is_since_checkpoint: false` (or test value) to `JobRowView` fixtures |
| `docs/EngineeringDiary.md` | Add draft diary entry |

---

## New Tests

The following targeted tests should be added as part of this change:

1. **Tab selection persists `SinceCheckpoint`** (in `update.rs` or `harvester_core` tests):
   - Send `Msg::LeftTabSelected { index: 1 }` (SinceCheckpoint)
   - Assert `state.left_tab == LeftTab::SinceCheckpoint` (not overridden to `JobList`)

2. **`passes_since_filter_dt` semantics** (in `since_filter.rs`):
   - `since=None` ΓåÆ always true
   - `fetched_utc=None, since=Some(t)` ΓåÆ true
   - `fetched_utc=Some(t), since=Some(s)` where `t >= s` ΓåÆ true
   - `fetched_utc=Some(t), since=Some(s)` where `t < s` ΓåÆ false

3. **Persistence roundtrip of `fetched_utc`** (in `persistence.rs` or `persistence_worker.rs` tests):
   - Create a `PersistedJob` with a known RFC3339 `fetched_utc` string
   - Serialize and deserialize
   - Assert the deserialized value parses back to the same `DateTime<Utc>`

4. **Layout: jobs panel visible for both tabs** (if UI layout is testable):
   - Assert `show_jobs` is true for both `LeftTab::JobList` and `LeftTab::SinceCheckpoint`

---

## Verification

1. `cargo build --all-targets` — clean build with no errors
2. `cargo test -p harvester_core` — all existing and new tests pass (including updated `Msg::JobDone` and `CompletedJobSnapshot` fixtures)
3. `cargo test -p harvester_app` — all app-level tests pass (if stable in CI)
4. `cargo clippy --all-targets -- -D warnings` — no new warnings; chrono imports resolve cleanly
5. **Manual — tab appears:** Launch the app; confirm three tabs: **Jobs**, **Since Checkpoint**, **Prompt Lab**
6. **Manual — tab selection stable:** Click "Since Checkpoint"; confirm it stays selected (not reverted to "Jobs")
7. **Manual — panel stays visible:** Switch to "Since Checkpoint" tab; confirm the job tree panel (and URL input bar) remain visible, not collapsed
8. **Manual — no checkpoint:** "Since Checkpoint" shows all jobs (same as Jobs tab)
9. **Manual — checkpoint set:** Set a checkpoint; fetch new articles; "Since Checkpoint" shows only newer articles; Jobs shows all
10. **Manual — restart persistence:** Set checkpoint, fetch articles, restart app; confirm "Since Checkpoint" shows only post-checkpoint jobs (not all historical jobs)
11. **Manual — in-flight jobs:** While a fetch is in progress, confirm in-flight jobs appear in "Since Checkpoint" (expected: yes, due to `None` fallback)
12. **Manual — consistency check:** Verify the Since Checkpoint subset matches what Archive export would include for the same checkpoint
13. **Diary finalized:** Engineering diary entry updated with Evidence and Lessons Learned
