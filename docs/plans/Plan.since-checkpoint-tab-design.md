# Design: "Since Checkpoint" Left-Pane Tab

**Date:** 2026-03-01
**Status:** Revised after fourth review

---

## Context

The left pane currently has two tabs: **Jobs** (all fetched articles) and **Prompt Lab**. Users need a focused view of only the articles fetched since the last briefing checkpoint — the same subset that would appear in the next Archive export or Briefing. A dedicated **Since Checkpoint** tab provides a quick way to see what's new without scrolling through older jobs.

---

## Shared Algorithm

All three features (Archive, Briefing, Since Checkpoint tab) use the same core rule:

> Include a job/article if `fetched_utc >= briefing_since_utc`, or if `fetched_utc` is missing/malformed (include by default).

Archive and Briefing apply this by reading `fetched_utc` from article frontmatter on disk (in `harvester_engine/src/export.rs:passes_since_filter` and `harvester_engine/src/briefing.rs:scan_and_prepare_articles`). The new tab applies the same rule using `fetched_utc` stored on `JobState` in memory, populated at job-completion time from the same source value (`(config.fetched_utc)()` in `engine.rs:435`).

A shared helper is **required** to prevent semantic drift. The helpers are placed in **`harvester_engine/src/since_filter.rs`** because `harvester_core` already depends on `harvester_engine` — putting them in `harvester_core` would make `harvester_engine` unable to use them without creating a circular dependency.

```rust
/// Used by Archive, Briefing, and the UI tab (via harvester_core's dependency on harvester_engine).
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

/// Convenience wrapper for callers that hold a raw string (export.rs, briefing.rs).
/// Encapsulates parse-and-fallback so callers don't repeat the rfc3339 logic.
pub fn passes_since_filter_str(
    fetched_utc_str: Option<&str>,
    since: Option<DateTime<Utc>>,
) -> bool {
    let dt = fetched_utc_str
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    passes_since_filter_dt(dt, since)
}
```

Place both in **`crates/harvester_engine/src/since_filter.rs`** (new file). Declare `pub mod since_filter;` in `harvester_engine/src/lib.rs`. `harvester_core` accesses both helpers through its existing dependency on `harvester_engine`.

**Note — `chrono` dependency:** `chrono` is already a direct dependency of `harvester_core` (`Cargo.toml:9`). No change to `Cargo.toml` is required.

**Design note — `None` semantics (in-flight and failed jobs):** Jobs with `fetched_utc = None` are included in Since Checkpoint by the fallback rule. This covers: (a) in-flight jobs, (b) failed jobs, (c) legacy jobs from session files predating this field. This behavior intentionally differs from Archive/Briefing on one point: Archive and Briefing only see on-disk completed articles, so they never encounter in-flight or failed jobs at all. The Since Checkpoint tab is a live view and the choice to show unresolved work is deliberate — users benefit from seeing what is being worked on alongside what was just completed. This is documented in UI tests and the Engineering Diary entry.

**Design note — legacy persisted jobs:** On first startup after this change, all historical jobs restored from existing session files will have `fetched_utc = None` and will appear in Since Checkpoint. This matches the archive/briefing fallback. Correct timestamps propagate from the next fetch cycle onward. No backfill is needed. Malformed `fetched_utc` strings in persisted state are silently treated as `None` (include); logging for observability is a follow-up.

**Design note — selection behavior when filtered:** If the currently selected job is not in the Since Checkpoint view, the selection is preserved but no preview is shown (or first visible item is selected, depending on existing behavior). This is out of scope; see Follow-ups.

---

## Data Flow

```
engine.rs:(config.fetched_utc)()  →  JobOutcome.fetched_utc (String)
    → Msg::JobDone.fetched_utc (String)
    → update.rs: destructured in Msg::JobDone arm; passed to state.apply_done()
    → JobState.fetched_utc (Option<DateTime<Utc>>, parsed in apply_done())
    → PersistedJob.fetched_utc (persisted as Option<String>)
    → JobRowView.is_since_checkpoint (bool, computed in AppState::view())
    → render.rs: filtered job list when LeftTab::SinceCheckpoint is active
```

The same `fetched_utc` value the engine writes into the article frontmatter (read by briefing/archive) is now also returned in `JobOutcome`, so both paths always agree.

---

## Implementation Steps

### Step 1 — Thread `fetched_utc` from engine through to `JobState`

**`crates/harvester_engine/src/types.rs`**
- Add `fetched_utc: Option<String>` to `JobOutcome`

**`crates/harvester_engine/src/engine.rs`** (around line 435 / 457–466)
- The `(config.fetched_utc)()` call already produces the RFC3339 string written to frontmatter
- Capture it into a local variable and include it in `JobOutcome`

**`crates/harvester_core/src/msg.rs`** (lines 51–56, `Msg::JobDone`)
- Add `fetched_utc: Option<String>` to `Msg::JobDone`

**`crates/harvester_io/src/effect_runner.rs`** (lines 1118–1138, `EngineEvent::JobCompleted` handler)
- For the `Ok` branch: pass `outcome.fetched_utc` into `Msg::JobDone`
- For the `Err` branch (failed jobs): pass `fetched_utc: None` — consistent with the "include by default" fallback

**`crates/harvester_core/src/update.rs`** (around line 130, `Msg::JobDone` match arm)
- Destructure the new field: `Msg::JobDone { ..., fetched_utc } =>`
- Pass `fetched_utc` into `state.apply_done(...)`
- **Update all `Msg::JobDone` constructors repo-wide** (see fixture checklist below)

**`crates/harvester_core/src/state.rs`**
- Add `fetched_utc: Option<DateTime<Utc>>` to `JobState` (around lines 2181–2190)
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
- Add `fetched_utc: Option<String>` (stored as RFC3339 string for serialization simplicity)
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
- `crates/harvester_io/src/persistence.rs` (test section around line 564)
- `crates/harvester_io/src/persistence_worker.rs` (test section around line 225)
- Any other inline tests in `state.rs`

### Step 3 — Add shared since-filter helper in `harvester_engine`

**`crates/harvester_engine/src/since_filter.rs`** (new file)
- Define `passes_since_filter_dt` and `passes_since_filter_str` as shown in the Shared Algorithm section above
- Add unit tests covering:
  - `since=None` → always true (for both helpers)
  - `fetched_utc=None, since=Some(t)` → true
  - `fetched_utc=Some(t), since=Some(s)` where `t >= s` → true
  - `fetched_utc=Some(t), since=Some(s)` where `t < s` → false
  - `passes_since_filter_str` with a valid RFC3339 string → same result as `_dt`
  - `passes_since_filter_str` with a malformed string → treats as `None` (include)

**`crates/harvester_engine/src/lib.rs`**
- Declare `pub mod since_filter;`

`harvester_core` accesses `harvester_engine::since_filter::passes_since_filter_dt` via its existing dependency. No new crate dependency is introduced.

### Step 4 — Compute `is_since_checkpoint` in the view model

`JobState` does not have access to `self.briefing_since_utc()`, so the boolean must be computed at the `AppState` level where that method is available, and passed into the view row.

**`crates/harvester_core/src/view_model.rs`** (around line 937, `JobRowView`)
- Add `is_since_checkpoint: bool`

**`crates/harvester_core/src/state.rs`** — `JobState::to_view` (or equivalent row-building function)
- Extend the signature to accept `is_since_checkpoint: bool` as a parameter
- Assign it directly to `JobRowView.is_since_checkpoint`

**`crates/harvester_core/src/state.rs`** — `AppState::view()` function (around lines 537–687)
- In the `jobs.iter()` map loop, compute `is_since_checkpoint` before calling `to_view`:
  ```rust
  use harvester_engine::since_filter::passes_since_filter_dt;
  let since = self.briefing_since_utc();
  // inside the loop:
  let is_since = passes_since_filter_dt(job.fetched_utc, since);
  job.to_view(is_since, ...)
  ```

### Step 5 — Add `SinceCheckpoint` tab variant

**`crates/harvester_core/src/tabs.rs`**
- Add `SinceCheckpoint` to `LeftTab` between `JobList` and `PromptLab`
- Update `to_index()`:
  - `JobList` → 0
  - `SinceCheckpoint` → 1
  - `PromptLab` → 2
- Update `from_index()` and any exhaustive match arms

### Step 6 — Fix reducer tab-selection logic (P0) and split `close_prompt_lab()`

**`crates/harvester_core/src/state.rs`** — `close_prompt_lab()` (around line 1989)
- Currently this function resets the active tab to `LeftTab::JobList` alongside closing Prompt Lab internals. Split these two concerns:
  - Introduce `close_prompt_lab_internals()` (or rename the existing function) that resets only Prompt Lab internal state (panel state, etc.) without touching `left_tab`
  - Introduce `set_left_tab(tab: LeftTab)` that sets `left_tab` unconditionally

**`crates/harvester_core/src/update.rs`** (around line 1048, `Msg::LeftTabSelected`)
- Rewrite the handler to set the tab directly via `state.set_left_tab(msg_tab)`, regardless of which tab is selected
- When selecting `PromptLab`: additionally call `close_prompt_lab_internals()` in reverse (or whatever opens the lab) — keep the existing Prompt Lab open/close side-effect separate from tab selection

**`crates/harvester_core/src/update.rs`** (around line 1065, `Msg::PromptLabCloseRequested`)
- This handler should call `close_prompt_lab_internals()` to close lab state and then call `state.set_left_tab(LeftTab::JobList)`. This preserves existing behavior: Prompt Lab close always returns to `JobList`. Preserving the previously selected non-PromptLab tab on close is a follow-up (see Follow-ups).

**Invariant to verify:** After this refactor, sending `Msg::LeftTabSelected { tab: LeftTab::SinceCheckpoint }` must result in `state.left_tab == LeftTab::SinceCheckpoint` with no subsequent override.

### Step 7 — Register tab in the UI and fix layout

**`crates/harvester_app/src/platform/ui/layout.rs`** (lines 144–149)
- Add `"Since Checkpoint".to_string()` as index-1 item in the tab bar vec
- **Fix layout collapse:** The `left_tab_dock` and `left_tab_size` closures (around line 1109) currently only show `PANEL_LEFT_JOBS` for `LeftTab::JobList`. Update them so the panel remains visible for both `JobList` and `SinceCheckpoint`:
  ```rust
  let show_jobs = matches!(left_tab, LeftTab::JobList | LeftTab::SinceCheckpoint);
  let jobs_dock = if show_jobs { DockStyle::Fill } else { DockStyle::Top };
  let jobs_size = if show_jobs { None } else { Some(0) };
  ```
  This also preserves the URL input bar for both job-related tabs.

**`crates/harvester_app/src/platform/ui/render.rs`**
- Handle `LeftTab::SinceCheckpoint` in tab-selection render (index 1)
- In job list rendering (around line 1344, `build_job_tree`): filter the jobs iterator based on the active tab:
  ```rust
  let jobs_iter: Box<dyn Iterator<Item = &JobRowView>> =
      if view.left_pane.left_tab == LeftTab::SinceCheckpoint {
          Box::new(view.jobs.iter().filter(|j| j.is_since_checkpoint))
      } else {
          Box::new(view.jobs.iter())
      };
  ```
  Access the tab via `view.left_pane.left_tab` (not via a local parameter), since `build_job_tree` receives `view: &AppViewModel`.

**Assumption — tree item IDs:** `TreeItemDescriptor` IDs are derived from `job.job_id`. Filtering the iterator before building tree items naturally excludes nodes without causing ID collisions or breaking expansion state for visible items. Verify this assumption holds before landing.

### Step 8 — Engineering diary entry

Add a draft diary entry to `docs/EngineeringDiary.md` as part of this change:

**Draft entry:**

```
## Since Checkpoint Tab (2026-03-01)

**Context:** Users needed a way to see only articles fetched since the last briefing
checkpoint without scrolling through older jobs. Archive and Briefing already had
this filter; the UI had no equivalent.

**Change:** Added a new `SinceCheckpoint` left-pane tab. Threaded `fetched_utc`
from the engine through `JobOutcome` → `Msg::JobDone` → `JobState` → `JobRowView`.
Persisted via `PersistedJob` for restart correctness. Fixed reducer tab-selection
logic that was overriding any non-PromptLab tab to `JobList` by splitting
`close_prompt_lab()` into tab-selection and lab-internals concerns. Extracted
shared `passes_since_filter_dt` / `passes_since_filter_str` helpers into
`harvester_engine/src/since_filter.rs` to keep semantics consistent across Archive,
Briefing, and the new tab while respecting the crate dependency graph.

**Inclusion semantics:** Since Checkpoint intentionally includes in-flight and
failed jobs (fetched_utc = None → include by default), unlike Archive/Briefing
which only see completed on-disk articles. This gives users visibility into work
in progress.
```

Finalize with **Evidence** (manual verification results) and **Lessons Learned** once implementation is complete.

---

## Follow-ups (out of scope for this change)

- **Unify `export.rs` and `briefing.rs`:** Migrate both to use `passes_since_filter_str` from `harvester_engine::since_filter`. The helper is exported now to make this a mechanical change.
- **Selected-job visibility:** If a user selects an older job in the Jobs tab and switches to Since Checkpoint, the preview pane continues to show the hidden job. File a follow-up to either clear the selection or auto-select the first visible item when switching tabs with a filtered-out selection.
- **Preserve tab on Prompt Lab close:** Currently `PromptLabCloseRequested` returns to `JobList`. A follow-up could store the previously active non-PromptLab tab and restore it on close.
- **Observability for malformed timestamps:** Malformed `fetched_utc` strings in persisted state are silently treated as `None`. A follow-up can add a `log::warn!("[since-filter] ...")` for debugging.

---

## Files Modified

| File | Change |
|------|--------|
| `crates/harvester_engine/src/types.rs` | Add `fetched_utc: Option<String>` to `JobOutcome` |
| `crates/harvester_engine/src/engine.rs` | Populate `fetched_utc` in `JobOutcome` |
| `crates/harvester_engine/src/since_filter.rs` | **New file:** `passes_since_filter_dt` + `passes_since_filter_str` helpers + unit tests |
| `crates/harvester_engine/src/lib.rs` | Declare `pub mod since_filter` |
| `crates/harvester_core/src/msg.rs` | Add `fetched_utc: Option<String>` to `Msg::JobDone` |
| `crates/harvester_io/src/effect_runner.rs` | Thread `fetched_utc` into `Msg::JobDone`; pass `None` in the `Err` branch |
| `crates/harvester_core/src/update.rs` | Destructure `fetched_utc` in `Msg::JobDone` arm; rewrite `Msg::LeftTabSelected` to call `set_left_tab()` unconditionally; update `Msg::PromptLabCloseRequested` to use `close_prompt_lab_internals()` + `set_left_tab(JobList)`; update all test fixtures constructing `Msg::JobDone` |
| `crates/harvester_core/src/state.rs` | Add `fetched_utc` to `JobState`; parse in `apply_done()`; compute `is_since_checkpoint` in `view()` and pass into `to_view()`; update `restore_completed_jobs`; add `fetched_utc` to `CompletedJobSnapshot`; introduce `set_left_tab()` and `close_prompt_lab_internals()` replacing overloaded `close_prompt_lab()` |
| `crates/harvester_io/src/persistence.rs` | Add `fetched_utc: Option<String>` with `#[serde(default)]` to `PersistedJob`; update `CompletedJobSnapshot` fixtures (~line 564) |
| `crates/harvester_core/src/view_model.rs` | Add `is_since_checkpoint: bool` to `JobRowView` |
| `crates/harvester_core/src/tabs.rs` | Add `SinceCheckpoint` to `LeftTab`; update index mapping |
| `crates/harvester_app/src/platform/ui/layout.rs` | Add "Since Checkpoint" to tab bar; fix dock/size closures to keep job panel visible for both job tabs |
| `crates/harvester_app/src/platform/ui/render.rs` | Handle new tab variant; filter `view.jobs` via `view.left_pane.left_tab` in `build_job_tree`; update `JobRowView` fixtures with `is_since_checkpoint: false` |
| `crates/harvester_core/tests/update_jobs.rs` | Add `fetched_utc: None` to `Msg::JobDone` fixtures |
| `crates/harvester_core/tests/triage_orchestration.rs` | Add `fetched_utc: None` to `Msg::JobDone` fixtures |
| `crates/harvester_core/tests/persistence.rs` | Add `fetched_utc: None` to `CompletedJobSnapshot` fixtures (~line 39) |
| `crates/harvester_io/src/persistence_worker.rs` | Add `fetched_utc: None` to `CompletedJobSnapshot` fixtures (~line 225) |
| `crates/harvester_app/src/platform/app.rs` | Add `fetched_utc: None` to `Msg::JobDone` fixtures (~line 885) |
| `docs/EngineeringDiary.md` | Add draft diary entry |

---

## New Tests

The following targeted tests should be added as part of this change:

1. **Tab selection preserves `SinceCheckpoint`** (in `update.rs` or `harvester_core` tests):
   - Send `Msg::LeftTabSelected { tab: LeftTab::SinceCheckpoint }`
   - Assert `state.left_tab == LeftTab::SinceCheckpoint` (not overridden to `JobList`)

2. **`PromptLabCloseRequested` still returns to `JobList`** (regression guard):
   - Set `state.left_tab = LeftTab::SinceCheckpoint`; send `Msg::PromptLabCloseRequested`
   - Assert `state.left_tab == LeftTab::JobList`

3. **`passes_since_filter_dt` semantics** (in `since_filter.rs`):
   - `since=None` → always true
   - `fetched_utc=None, since=Some(t)` → true
   - `fetched_utc=Some(t), since=Some(s)` where `t >= s` → true
   - `fetched_utc=Some(t), since=Some(s)` where `t < s` → false

4. **`passes_since_filter_str` semantics** (in `since_filter.rs`):
   - Valid RFC3339 string → same result as `_dt` equivalent
   - Malformed string → treated as `None` (include by default)

5. **Persistence roundtrip of `fetched_utc`** (in `persistence.rs` or `persistence_worker.rs` tests):
   - Create a `PersistedJob` with a known RFC3339 `fetched_utc` string
   - Serialize and deserialize
   - Assert the deserialized value parses back to the same `DateTime<Utc>`

6. **Persistence backward compatibility** (in `persistence.rs` tests):
   - Deserialize a `PersistedJob` JSON blob that lacks the `fetched_utc` field entirely
   - Assert `fetched_utc` is `None` (not a parse error)

7. **Render-level filtering: `SinceCheckpoint` shows only matching jobs** (in `render.rs` or `harvester_app` tests):
   - Construct a view model with two jobs: one with `is_since_checkpoint: true`, one with `false`
   - Assert that `build_job_tree` with `left_tab = SinceCheckpoint` produces exactly one tree item
   - Assert that `build_job_tree` with `left_tab = JobList` produces two tree items

8. **Layout: jobs panel visible for both tabs** (if UI layout is testable):
   - Assert `show_jobs` is true for both `LeftTab::JobList` and `LeftTab::SinceCheckpoint`
   - Assert `show_jobs` is false for `LeftTab::PromptLab`

---

## Verification

1. `cargo build --all-targets` — clean build with no errors
2. `cargo test -p harvester_core` — all existing and new tests pass (including updated `Msg::JobDone` and `CompletedJobSnapshot` fixtures)
3. `cargo test -p harvester_engine` — new `since_filter` unit tests pass
4. `cargo test -p harvester_app` — all app-level tests pass (if stable in CI)
5. `cargo clippy --all-targets -- -D warnings` — no new warnings
6. **Manual — tab appears:** Launch the app; confirm three tabs: **Jobs**, **Since Checkpoint**, **Prompt Lab**
7. **Manual — tab selection stable:** Click "Since Checkpoint"; confirm it stays selected (not reverted to "Jobs")
8. **Manual — Prompt Lab close returns to Jobs:** Open Prompt Lab from Since Checkpoint; close it; confirm "Jobs" tab is active (not Since Checkpoint — expected per current behavior)
9. **Manual — panel stays visible:** Switch to "Since Checkpoint" tab; confirm the job tree panel (and URL input bar) remain visible, not collapsed
10. **Manual — no checkpoint:** "Since Checkpoint" shows all jobs (same as Jobs tab)
11. **Manual — checkpoint set:** Set a checkpoint; fetch new articles; "Since Checkpoint" shows only newer articles; Jobs shows all
12. **Manual — restart persistence:** Set checkpoint, fetch articles, restart app; confirm "Since Checkpoint" shows only post-checkpoint jobs (not all historical jobs)
13. **Manual — in-flight jobs:** While a fetch is in progress, confirm in-flight jobs appear in "Since Checkpoint" (expected: yes, due to `None` fallback)
14. **Manual — failed jobs:** Trigger a failed fetch; confirm failed jobs appear in "Since Checkpoint" (expected: yes, `fetched_utc: None` → include)
15. **Manual — consistency check (completed jobs only):** For completed jobs only, verify the Since Checkpoint subset matches what Archive export would include for the same checkpoint
16. **Diary finalized:** Engineering diary entry updated with Evidence and Lessons Learned