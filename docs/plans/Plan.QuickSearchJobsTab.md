# Quick Search for the Jobs Tab — Design & Implementation Plan

## Overview

Add a persistent search box at the top of the Jobs list (left pane, `LeftTab::Jobs` only) that filters the visible jobs by a case-insensitive substring match against the displayed title and the article URL. `Ctrl+F` switches the left pane to the Jobs tab and gives focus to the search box. The filter is purely a *visibility* projection — `view.jobs` stays unfiltered so the preview pane and other consumers are unaffected.

## Goals

1. As the user types, the visible job rows are restricted to those whose title or URL contains the query (substring, case-insensitive).
2. `Ctrl+F` from anywhere in the main window: switch to `LeftTab::Jobs` and focus the search box, selecting any existing text so the next keystroke replaces it.
3. Arrow Up/Down in the filtered list skips hidden rows.
4. `Enter` in the search box moves keyboard focus to the list; if no row is selected, or the selected row is not in the filtered set, select the first visible row.
5. `Esc` in the search box clears the query and restores the full list. The currently selected job is preserved.
6. The selected job and its right-pane preview remain accessible even when the row is filtered out of the list (selection state survives the filter).
7. The query string is preserved when the user switches to another left tab and returns.

## Non-Goals (YAGNI)

- No search on `LeftTab::TriageReview`, `LeftTab::TriageResults`, or `LeftTab::PromptLab`. The search box is collapsed on those tabs.
- No regex, fuzzy matching, prefix-only matching, or whole-word matching.
- No match-highlighting inside the row text.
- No "next match / previous match" buttons or shortcuts.
- No persistence of the query across app restarts.

## UX Specification

### Layout

A new single-line edit control is added directly above the jobs list, inside `PANEL_JOBS`. Order inside `PANEL_JOBS` (top-to-bottom):

1. `LABEL_JOBS_HEADER_TITLE` (existing, currently size 0).
2. `LABEL_JOBS_HEADER_META` (existing).
3. **`INPUT_JOBS_SEARCH`** (new) — fills horizontally, fixed height (≈ 24 px) when `left_tab == LeftTab::Jobs`. Collapsed to height 0 on other tabs.
4. `TREE_JOBS` (existing list, `DockStyle::Fill`).

### Behavior table

| Trigger | Effect |
|--|--|
| User types in `INPUT_JOBS_SEARCH` | Reducer updates `ui.jobs_search_query`. The view builder recomputes `LeftPaneView.visible_jobs_after_filter` (a `Vec<JobId>`). Header count_label, list rows, Enter-first-match, and "is selection still visible" all read this single derived set. |
| `Esc` while focus is in `INPUT_JOBS_SEARCH` | Clears `ui.jobs_search_query`. Render layer emits `SetInputText("")` for the control so the widget text matches state. Focus returns to `TREE_JOBS`. Selection unchanged. |
| `Enter` while focus is in `INPUT_JOBS_SEARCH` | Moves focus to `TREE_JOBS`. If `selected_jobs_visible_in_filter == false` (or no selection), dispatch `Msg::JobSelected { job_id }` for `LeftPaneView.first_visible_job_id` if any. |
| `Ctrl+F` (anywhere in the main window) | Reducer switches `left_tab` to `LeftTab::Jobs`. Render layer issues `SetFocus { control_id: INPUT_JOBS_SEARCH, select_all: true }`. |
| Arrow Up / Down on `TREE_JOBS` | Native list-box behavior over the populated rows. Because only visible jobs are pushed, hidden rows are naturally skipped. |
| Tab switch away and back | `ui.jobs_search_query` is unchanged; the search box re-appears with the prior text and filtering re-applies. |
| Selected job no longer matches the filter | Row hidden, but `view.selected_job_id` stays `Some(_)` so right pane keeps showing it. List-box selection clears (existing behavior at `render_list_box.rs:67-70`). |
| Empty query | `visible_jobs_after_filter` equals the scope-only filtered set; behavior identical to today. |

## Architecture

### Data flow

```
Win32 input event
  → CommanDuctUI translation
  → AppEvent (e.g. InputTextChanged, new InputKeyDown, MenuActionClicked)
  → main loop (crates/harvester_app/src/platform/app.rs) maps to Msg
  → reducer (crates/harvester_core/src/update/mod.rs) — pure
  → AppState (ui.jobs_search_query)
  → view_builder derives LeftPaneView.visible_jobs_after_filter + first_visible_job_id + selected_jobs_visible_in_filter
  → render layer consumes the derived set:
      • build_list_box_items filters by membership
      • LeftPaneHeaderView.count_label reflects the filtered length
      • Platform-side Enter handler reads first_visible_job_id
  → PlatformCommand stream (PopulateListBox, SetInputText, SetFocus, ApplyStyleToControl, ...)
```

**Key invariant:** there is *one* visibility predicate, evaluated *once* in `view_builder.rs`. Render layer and platform layer consume the derived data, they do not re-implement the predicate. This addresses the chief concern that count, first-match, and rendered list could drift out of sync.

### State

`UiState` (`crates/harvester_core/src/state/ui_state.rs`) gets one new field:

```rust
jobs_search_query: String,   // default: String::new()
```

Plus a setter and getter mirroring the existing `input_buffer` pattern:

```rust
pub(super) fn jobs_search_query(&self) -> &str { &self.jobs_search_query }
pub(super) fn set_jobs_search_query(&mut self, text: String) { self.jobs_search_query = text; }
pub(super) fn clear_jobs_search_query(&mut self) { self.jobs_search_query.clear(); }
```

`AppState` exposes pass-through accessors so the reducer and view builder can read the query (same pattern as `input_buffer()`).

### Messages

Add three new `Msg` variants in `crates/harvester_core/src/msg.rs`:

```rust
/// User typed in the Jobs search box.
JobsSearchQueryChanged(String),
/// User pressed Esc inside the Jobs search box.
JobsSearchCleared,
/// User pressed Ctrl+F — switch to Jobs tab; render layer takes focus.
FocusJobsSearchRequested,
```

`JobsSearchCleared` is distinct from `JobsSearchQueryChanged(String::new())` so the platform layer can fold a focus change into the same render pass (the reducer stays pure).

### Reducer changes

In `crates/harvester_core/src/update/mod.rs`, three new arms:

```rust
Msg::JobsSearchQueryChanged(text) => { state.set_jobs_search_query(text); Vec::new() }
Msg::JobsSearchCleared           => { state.clear_jobs_search_query(); Vec::new() }
Msg::FocusJobsSearchRequested    => { state.set_left_tab(LeftTab::Jobs); Vec::new() }
```

No new effects. `set_left_tab` already exists (verified at `state/mod.rs:2208`).

### View model — derived visibility is the source of truth

`LeftPaneView` (`crates/harvester_core/src/view_model.rs:222`) gains:

```rust
pub jobs_search_query: String,
/// Job IDs visible on LeftTab::Jobs after scope + search filter (in original order).
/// Empty Vec means no jobs in the filtered set. Only meaningful when left_tab == Jobs;
/// on other tabs this is empty and unused.
pub visible_jobs_after_filter: Vec<crate::JobId>,
/// First entry of visible_jobs_after_filter, hoisted for convenience.
pub first_visible_job_id: Option<crate::JobId>,
/// True iff selected_job_id is Some(id) and id is in visible_jobs_after_filter.
/// Lets the platform layer decide whether Enter needs to select a new row.
pub selected_jobs_visible_in_filter: bool,
```

`view_builder.rs` populates these fields when assembling `LeftPaneView`. The existing `build_left_pane_header_view` is extended:

- For `LeftTab::Jobs`, `count_label` is `"{visible} jobs"` when query is empty and equals scope length; `"{visible} of {scope_total} jobs"` when the query is non-empty (so the user can see the filter is doing something).
- Other tabs unchanged.

A small private helper in `view_builder.rs` computes the visible set once:

```rust
fn compute_visible_jobs_for_jobs_tab(
    jobs: &[JobRowView],
    job_list_scope: JobListScope,
    query: &str,
) -> Vec<JobId> { /* scope filter then case-insensitive substring on title || url */ }
```

This helper is the *only* place the predicate lives. It gets focused unit tests.

### Rendering — consume the derived set

In `crates/harvester_app/src/platform/ui/render_list_box.rs`, `build_list_box_items` for `LeftTab::Jobs` is changed to walk `view.left_pane.visible_jobs_after_filter` instead of computing its own scope/search filter. For other tabs the existing scope-only filter at lines 104-109 is unchanged.

```rust
// Sketch — for the Jobs tab only:
let visible: &[JobId] = &view.left_pane.visible_jobs_after_filter;
let jobs_by_id: HashMap<_, _> = view.jobs.iter().map(|j| (j.job_id, j)).collect();
let items: Vec<_> = visible.iter().filter_map(|id| jobs_by_id.get(id)).copied().map(...).collect();
```

(Or a simpler nested loop if HashMap construction is overkill; rendering already iterates `view.jobs`.)

### Platform layer — control creation, theming, text sync, key events

`crates/harvester_app/src/platform/ui/constants.rs` — add:

```rust
pub const INPUT_JOBS_SEARCH: ControlId = ControlId::new(1502);   // adjacent to TREE_JOBS (1501)
pub const MENU_ACTION_FIND_JOBS: MenuActionId = MenuActionId::new(2);
```

`crates/harvester_app/src/platform/ui/layout/init.rs`:

- Create `INPUT_JOBS_SEARCH` as a single-line `CreateInput` child of `PANEL_JOBS`.
- Add a hidden-or-visible "Find..." menu item under the File menu with accelerator `"\tCtrl+F"`, dispatching `MENU_ACTION_FIND_JOBS`. This piggy-backs the existing menu/accelerator mechanism (cf. `"Add URL...\tCtrl+L"` at `layout/init.rs:21`) instead of inventing a new accelerator surface in `CommanDuctUI`.

`crates/harvester_app/src/platform/ui/layout/theme.rs`:

- Apply `StyleId::DefaultInput` to `INPUT_JOBS_SEARCH` in the same call site as `INPUT_URLS` / `INPUT_PROMPT_LAB_URL` (`theme.rs:884-893`). Done at startup, not as polish.

`crates/harvester_app/src/platform/ui/layout/rules.rs`:

- Add a `LayoutRule` for `INPUT_JOBS_SEARCH` as `DockStyle::Top` child of `PANEL_JOBS`, with `fixed_size: Some(24)` when `left_tab == LeftTab::Jobs`, `Some(0)` otherwise. Order between `LABEL_JOBS_HEADER_META` (order 1) and `TREE_JOBS` (bump to order 3).

Win32 dispatcher (`win32_platform_handler.rs` / `app.rs`):

- Translate `AppEvent::InputTextChanged { control_id: INPUT_JOBS_SEARCH, text, .. }` → `Msg::JobsSearchQueryChanged(text)`.
- Translate `AppEvent::MenuActionClicked { action_id: MENU_ACTION_FIND_JOBS }` → `Msg::FocusJobsSearchRequested`. After render, emit `PlatformCommand::SetFocus { control_id: INPUT_JOBS_SEARCH, select_all: true }`.
- Translate the new `AppEvent::InputKeyDown { control_id: INPUT_JOBS_SEARCH, key_code, .. }`:
  - `VK_ESCAPE` → dispatch `Msg::JobsSearchCleared`; after render, emit `SetFocus { control_id: TREE_JOBS, select_all: false }`.
  - `VK_RETURN` → after render, if `view.left_pane.selected_jobs_visible_in_filter == false` and `view.left_pane.first_visible_job_id.is_some()`, dispatch `Msg::JobSelected { job_id: first_visible_job_id.unwrap() }`. Then emit `SetFocus { control_id: TREE_JOBS, select_all: false }`.

### Render-side text sync for `INPUT_JOBS_SEARCH`

Mirror the pattern at `render_prompt_lab.rs:188-197`:

- The render module that owns the Jobs panel keeps a `prev_jobs_search_query: String`.
- On each render, if `view.left_pane.jobs_search_query != prev_jobs_search_query`, emit `PlatformCommand::SetInputText { control_id: INPUT_JOBS_SEARCH, text: view.left_pane.jobs_search_query.clone() }` and update `prev_jobs_search_query`.
- This keeps the widget text aligned after `Msg::JobsSearchCleared` and any future programmatic mutation.

### `CommanDuctUI` additions (generic, no Harvester naming)

Verified missing today and added by this plan, in a single `CommanDuctUI` change with a version bump and a `CHANGELOG.md` entry:

1. `AppEvent::InputKeyDown { window_id, control_id, key_code, modifiers }` — emitted from `WM_KEYDOWN` for edit controls. Mirrors the existing `ListBoxItemKeyDown` (`types.rs:270`). Translation lives next to the existing `EN_CHANGE → InputTextChanged` path in `window_common.rs:2415-2502`.
2. `PlatformCommand::SetFocus { window_id, control_id, select_all: bool }` — moves focus to a child control. `select_all` controls whether single-line edit content is selected on focus (used so Ctrl+F replaces existing text on retype). Verified no focus variant exists today in `src/CommanDuctUI/src/types.rs:517`.

Each gets a host-independent unit test in `CommanDuctUI` per its `Agents.md` ("reducer seam before Win32 syscall"). Dark-theme support is unaffected.

Ctrl+F is *not* a new accelerator surface — it reuses the existing menu-action accelerator mechanism via `"\tCtrl+F"` on the new Find menu item. This keeps `CommanDuctUI`'s blast radius to the two items above.

## Edge Cases

| Case | Behavior |
|--|--|
| Query is whitespace-only (e.g. `"   "`) | Treated literally; not trimmed. Avoids surprise. |
| Mixed-case query / row | Case-insensitive via `to_lowercase()` on both sides. Unicode-aware via `String::to_lowercase`. |
| Job has no `summary_title` | Match falls back to URL only. The list row still shows `compact_url_label(url, 80)`, which is a substring of the URL, so the user sees what matched. |
| Selected job is filtered out | Row hidden; `view.selected_job_id` stays `Some(_)`; preview pane unchanged. `selected_jobs_visible_in_filter = false`. |
| Click a visible row while a query is active | Existing `Msg::JobSelected` flow. Query unchanged. |
| Jobs change underneath the query | View re-renders on tick; visible set is recomputed from current state. Independent of mutation events. |
| Esc when query is already empty | No-op for state; render layer still emits `SetInputText("")` (idempotent) and `SetFocus(TREE_JOBS)`. |
| Ctrl+F while focus is already in the search box | Re-selects all text; no state change. |

## Alternatives Considered

1. **Filter only at the render layer.** Rejected — header count is built in core (`view_builder.rs:449`), and Enter-first-match needs a deterministic "first visible row" answer that the platform layer can read. Two predicates risks drift.
2. **Filter only in the view builder (filter `view.jobs` directly).** Rejected — `view.jobs` is consumed by the right-pane preview, header counts on other tabs, and any future readers. Filtering centrally would silently rewrite all of those. The plan keeps `view.jobs` unfiltered and uses a derived `Vec<JobId>` instead.
3. **Type-ahead find (no visible widget).** Rejected per UX choice — user wants discoverable, persistent search.
4. **Modal `Ctrl+F` bar (browser-style).** Rejected per UX choice.
5. **Bespoke global accelerator surface in `CommanDuctUI`.** Rejected — the existing menu-accelerator mechanism already covers Ctrl+L; reuse it for Ctrl+F via a Find menu item. Smaller change to `CommanDuctUI`.
6. **Search across all four left tabs with shared state.** Out of scope for v1. The derived-visibility design leaves room: per-tab fields could be added later.

## Testing

Following the repo guideline ("Prefer tests of reducer behavior, emitted effects, and public contracts").

### Reducer unit tests (`crates/harvester_core/src/update/tests/ui_state_tests.rs`)

- `jobs_search_query_changed_updates_state` — `Msg::JobsSearchQueryChanged("kube")` ⇒ `state.jobs_search_query() == "kube"`, no effects.
- `jobs_search_cleared_resets_query`.
- `focus_jobs_search_switches_left_tab` — given `left_tab != Jobs`, `Msg::FocusJobsSearchRequested` ⇒ `Jobs`. No effects.
- `jobs_search_query_persists_across_tab_switch`.
- `jobs_search_query_persists_when_jobs_mutate` (apply `JobProgress` / `JobDone` after a query is set).

### View-builder contract tests (`crates/harvester_core/src/state/tests.rs`)

- `visible_jobs_match_scope_when_query_empty` — `visible_jobs_after_filter` equals the scope-only filtered set.
- `visible_jobs_substring_case_insensitive` — fixture jobs `Kubernetes Pods`, `Rust Async`, `K-quotes`; query `"KUBE"` returns just `Kubernetes Pods`.
- `visible_jobs_match_url_when_title_lacks_term` — title without `github`, URL containing `github.com/...`, query `"github"` matches.
- `first_visible_job_id_is_first_in_set`.
- `selected_jobs_visible_in_filter_true_when_selection_visible`.
- `selected_jobs_visible_in_filter_false_when_selection_filtered_out` — and `view.selected_job_id` is still `Some(_)`.
- `view_jobs_unchanged_by_search_query` — `view.jobs` content unchanged regardless of query.
- `left_pane_header_count_label_reflects_filter` — empty query: `"{N} jobs"`; non-empty: `"{M} of {N} jobs"`.

### Render-layer test (`crates/harvester_app/src/platform/ui/render_tests.rs`)

- `list_box_items_for_jobs_tab_follow_visible_jobs_after_filter` — fixture view with `visible_jobs_after_filter = [j2]` ⇒ exactly one row.
- `list_box_items_for_other_tabs_ignore_visible_jobs_after_filter`.
- `list_box_selected_id_omitted_when_selection_filtered_out` — asserts the existing behavior at `render_list_box.rs:67-70` is preserved.
- `jobs_search_input_text_resyncs_to_state` — when `view.left_pane.jobs_search_query` changes between two renders, the second render emits `SetInputText { control_id: INPUT_JOBS_SEARCH, text: <new> }`. Idempotent — no command when the value is unchanged.

### Layout/theme tests (`crates/harvester_app/src/platform/ui/layout/tests.rs`)

- `jobs_search_input_has_layout_rule_on_jobs_tab` — non-zero `fixed_size` only when `left_tab == LeftTab::Jobs`.
- `jobs_search_input_receives_default_input_style` — startup theme commands include `ApplyStyleToControl { control_id: INPUT_JOBS_SEARCH, style_id: StyleId::DefaultInput }`.

### CommanDuctUI tests (`src/CommanDuctUI/...`)

- Translation of edit-control `WM_KEYDOWN` into `AppEvent::InputKeyDown` (callable with `HWND::default()`).
- `PlatformCommand::SetFocus` round-trips correctly and exposes `select_all` to the executor.

### Manual smoke test plan

1. Type characters → list narrows live.
2. Backspace until empty → full list restored.
3. Click another row → preview updates; query unchanged.
4. Esc in search → query cleared, focus on list, selection unchanged, edit control visibly empty.
5. Enter in empty search → focus moves to list; first row gets selected if no selection / selection invisible.
6. Ctrl+F from list → focus jumps to search box with text selected.
7. Switch to TriageReview and back → query still applied.
8. Switch to PromptLab and back → query still applied.
9. Selected-but-filtered: select a job, type a query that excludes it → list hides the row, preview still shows it; Enter selects first visible row.

## Implementation Phases

Each phase is independently buildable and testable. Run `cargo clippy --all-targets -- -D warnings` and `cargo fmt` at the end of each.

### Phase 1 — Derived Jobs visibility contract in core

- Add `jobs_search_query` to `UiState` + accessors on `AppState`.
- Add `Msg::JobsSearchQueryChanged` / `JobsSearchCleared` / `FocusJobsSearchRequested` and reducer arms.
- Add the four new `LeftPaneView` fields. Implement `compute_visible_jobs_for_jobs_tab`. Wire into `view_builder.rs`. Update `build_left_pane_header_view` to use the derived count.
- Add the reducer unit tests and view-builder contract tests above.
- **Done when:** `cargo test -p harvester_core` passes; `cargo build` clean.

### Phase 2 — Render layer consumes the derived set

- `build_list_box_items` on Jobs tab walks `visible_jobs_after_filter`.
- Add `prev_jobs_search_query` to the render state and emit `PlatformCommand::SetInputText` when the view-model query changes (idempotent guard).
- Add render-layer tests.
- **Done when:** Render tests pass; manual run shows the listbox follows the derived set (search input not yet wired).

### Phase 3 — Search-box control, layout, theming, text-changed wiring

- Add `INPUT_JOBS_SEARCH` constant.
- `CreateInput` in `layout/init.rs`.
- `ApplyStyleToControl` for `StyleId::DefaultInput` in `theme.rs` at the same call site as the other inputs.
- `LayoutRule` in `layout/rules.rs`; bump `TREE_JOBS` order.
- Dispatch `AppEvent::InputTextChanged { control_id: INPUT_JOBS_SEARCH, .. }` → `Msg::JobsSearchQueryChanged`.
- Layout/theme tests.
- **Done when:** Typing in the box visibly narrows the list and is dark-themed correctly.

### Phase 4 — Keyboard contract (Esc, Enter, Ctrl+F)

This phase is one *atomic* change spanning `CommanDuctUI` and Harvester:

- `CommanDuctUI`:
  - Add `AppEvent::InputKeyDown` and emit it from `WM_KEYDOWN` for edit controls (`window_common.rs` neighborhood).
  - Add `PlatformCommand::SetFocus { window_id, control_id, select_all: bool }` and an executor that calls `SetFocus`/`Edit_SetSel` as appropriate.
  - Unit tests per `CommanDuctUI/Agents.md` ("reducer seam before Win32 syscall").
  - Bump version in `src/CommanDuctUI/Cargo.toml`.
  - Add a `CHANGELOG.md` entry summarising both additions.
- Harvester:
  - Add `MENU_ACTION_FIND_JOBS` and a "Find...\tCtrl+F" item in the File menu (`layout/init.rs`).
  - Map `AppEvent::MenuActionClicked { action_id: MENU_ACTION_FIND_JOBS }` → `Msg::FocusJobsSearchRequested`, then emit `SetFocus { control_id: INPUT_JOBS_SEARCH, select_all: true }` after the next render.
  - Map `AppEvent::InputKeyDown` on `INPUT_JOBS_SEARCH`:
    - `VK_ESCAPE` → `Msg::JobsSearchCleared`, then `SetFocus(TREE_JOBS, select_all=false)`.
    - `VK_RETURN` → consult freshly-rendered view: if `!selected_jobs_visible_in_filter && first_visible_job_id.is_some()`, dispatch `Msg::JobSelected { job_id: first_visible_job_id }`. Then `SetFocus(TREE_JOBS, select_all=false)`.
- **Done when:** Manual smoke tests above all pass.

### Phase 5 — Diary

- Add a brief entry to `docs/EngineeringDiary.md` summarising the feature and the derived-visibility contract.
- (Dark-theme verification was already covered in Phase 3.)

## Out-of-Repo Side Effects

- None.
- `scripts/Start-HarvesterBatch.ps1` unchanged (no new CLI flags).
- `CommanDuctUI` version + `CHANGELOG.md` bumped in Phase 4.
