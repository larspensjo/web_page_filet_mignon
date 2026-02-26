# Implementation Plan: Move Prompt Lab to Left Panel

**Date:** 2026-02-26
**Status:** Draft

---

## Draft Diary Entry

**Context:** The Prompt Lab occupies a tab on the right-pane alongside Triage, Summary, Briefing,
and Trends. This creates two UX problems: (1) Prompt Lab results have nowhere to render — the
`output_json` from completed runs is computed in `PromptLabView` but no `RichEdit` control displays
it; (2) even if a result viewer were added, the user cannot see Prompt Lab settings and the result
at the same time because they share the same right-pane area.

**Change:** Move Prompt Lab controls to the left panel behind a new left-panel tab bar. The left
side gains two tabs: "Job List" (tree view, default) and "Prompt Lab" (configuration controls). The
right panel drops the PromptLab tab and keeps 4 content tabs: Triage, Summary, Briefing, Trends.
When the left tab is "Prompt Lab", right-panel viewers show lab results instead of production
results. This gives simultaneous config+result visibility. Affected subsystems: `harvester_core`
(state, tabs, view model, update), `harvester_app` (layout, render, constants, event mapping).

---

## Goal

Give Prompt Lab its own left-panel tab so configuration controls live on the left and experiment
results render in the existing right-panel viewers (Triage/Summary/Briefing). The user can see
settings and results side-by-side. No functional change to the Prompt Lab domain logic, LLM
pipelines, or persistence.

---

## The Problem (detailed)

1. **No result viewer.** `PromptLabRunSummaryView.output_json` is populated in
   `crates/harvester_core/src/view_model.rs` (line ~232) but orphaned — no UI control reads it. The
   Prompt Lab tab panel contains only configuration controls (radio buttons, combo box, text inputs,
   labels, buttons). There is no `RichEdit` viewer inside the Prompt Lab tab.

2. **Spatial conflict.** The right pane shows either Prompt Lab settings **or** Triage/Summary/
   Briefing results, never both. The user's workflow is: configure → run → switch tab to see result
   → switch back to tweak → re-run → switch tab again. This destroys flow.

3. **Vertical space exhaustion.** Adding a result viewer inside the existing Prompt Lab tab would
   not solve the problem: in advanced mode, Prompt Lab controls consume ~858px of vertical space,
   leaving almost no room for result content.

---

## Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Left panel navigation | Tab bar with radio buttons (same mechanism as right panel) | Proven pattern, no new widget types needed |
| Right-panel tab count | 4 tabs (drop PromptLab) | Prompt Lab controls move left |
| Result routing | Context-sensitive: when left tab = Prompt Lab, right viewers show lab results | Single source of truth per tab, no duplicate viewers |
| Auto-switch on run complete | Yes, switch right panel to the tab matching the lab stage | Reduces clicks; user sees result immediately |
| Result indicator | Preview header shows "[Lab]" prefix when displaying lab output | Clear distinction from production content |
| INPUT panel when lab active | Hidden (Prompt Lab has its own URL input) | Maximizes left-panel space for lab controls |
| `open_prompt_lab()` / `close_prompt_lab()` | Rewired to set `left_tab` instead of `active_tab` | Preserves existing bridge messages |
| `PromptLabState.visible` | Derived from `left_tab` transitions; remove as independent field in follow-up | Single source of truth — `left_tab` is canonical |
| Lab result parsing | Reuse `validate_triage` / `validate_summary` from `harvester_engine` | Schema-drift resistance; consistent failure semantics with production |
| Right-pane source typing | `PreviewSource` enum (`Production` / `PromptLab { stage, run_id }`) not a bool | Richer labeling, easier debugging, future-proof |
| Which lab run to show | Latest run for the currently selected stage | Predictable; matches what the user just configured |
| Right tab on close (left→JobList) | Keep current right tab as-is | Least surprising; avoids forced context switch |
| Trends tab during lab mode | Always production-sourced | Trends has no lab equivalent; mixing would confuse |

---

## Architecture

### New State

```
tabs.rs:
  LeftTab { JobList (default), PromptLab }

state.rs (AppState):
  left_tab: LeftTab                   // new field, private
  select_left_tab(&mut self, tab)     // new setter
  left_tab(&self) -> LeftTab          // new accessor
```

### Modified State

```
tabs.rs:
  AppTab — remove PromptLab variant (only Triage, Summary, Briefing, Trends remain)

state.rs:
  open_prompt_lab()  → sets left_tab = PromptLab (instead of active_tab = PromptLab)
  close_prompt_lab() → sets left_tab = JobList; keeps current right tab as-is
                       (instead of forcing active_tab = Summary)
  PromptLabState.visible — during migration, open()/close() still call it for
                           backward compat; follow-up cleanup removes it entirely
                           and derives visibility from left_tab.
```

### View Model Changes

```
view_model.rs:
  RightPaneView:
    - remove prompt_lab field (moves to LeftPaneView)
    + preview_source: PreviewSource   // Production or PromptLab { stage, run_id }
    triage_markdown   → sourced from lab output_json when PromptLab && stage == Triage
    summary_markdown  → sourced from lab output_json when PromptLab && stage == Summary
    briefing_markdown → sourced from lab output_json when PromptLab && stage == Briefing
    trends            → always production-sourced (unaffected by lab mode)

  PreviewSource (new enum):
    Production
    PromptLab { stage: PromptLabStage, run_id: PromptLabRunId }

  + LeftPaneView:
    + left_tab: LeftTab
    + prompt_lab: PromptLabView   // moved here from RightPaneView

  AppViewModel:
    - remove root-level prompt_lab field (single home is now LeftPaneView)
    + left_pane: LeftPaneView     // new field
```

### Message Changes

```
msg.rs:
  + Msg::LeftTabSelected { tab: LeftTab }
  Msg::TabSelected      — restricted to AppTab (no PromptLab variant)
```

### Reducer Changes (update.rs)

```
Msg::LeftTabSelected { tab } →
    engine_info!("[left-tab] switching to {:?}", tab)
    state.select_left_tab(tab)
    if tab == PromptLab: state.prompt_lab.open()
    if tab == JobList:   state.prompt_lab.close()
    // Note: does NOT change active_tab (right panel keeps its current tab)

Msg::TabSelected { tab } →
    unchanged, but PromptLab variant removed from AppTab

Prompt Lab run completion (Success + Failure branches) →
    after state.complete_prompt_lab_run() / state.fail_prompt_lab_run():
    + engine_info!("[prompt-lab-auto-tab] switching right tab to {:?}", stage)
    + auto-switch right-panel tab to match lab stage
      (Triage → AppTab::Triage, Summary → AppTab::Summary, Briefing → AppTab::Briefing)
    + only when left_tab == PromptLab
```

### Layout Changes (layout.rs)

**New controls:**

| Control | ID | Type | Parent |
|---|---|---|---|
| `PANEL_LEFT` | new | Panel | root (replaces PANEL_INPUT/PANEL_JOBS as dock-left structural parent) |
| `PANEL_LEFT_TAB_BAR` | new | Panel | PANEL_LEFT, DockStyle::Top, height 28 |
| `BUTTON_LEFT_TAB_JOBS` | new | RadioButton | PANEL_LEFT_TAB_BAR, group_start: true |
| `BUTTON_LEFT_TAB_PROMPT_LAB` | new | RadioButton | PANEL_LEFT_TAB_BAR, group_start: false |
| `PANEL_LEFT_JOBS` | new | Panel | PANEL_LEFT, contains PANEL_INPUT + PANEL_JOBS |
| `PANEL_LEFT_PROMPT_LAB` | new | Panel | PANEL_LEFT, contains all Prompt Lab controls |

**Top-level and internal docking rules:**

`PANEL_LEFT` becomes the single dock-left root panel:
- `PANEL_LEFT`: `DockStyle::Left`, `fixed_size: Some(left_panel_width)`, replaces both
  `PANEL_INPUT` and `PANEL_JOBS` at the root level. The splitter controls this width.

Inside `PANEL_LEFT_JOBS` (when active):
- `PANEL_INPUT`: `DockStyle::Left`, `fixed_size: Some(INPUT_PANEL_FIXED_WIDTH)` (500 or 0 if
  toggled hidden)
- `PANEL_JOBS`: `DockStyle::Fill`

This simplifies the top-level layout — the root has only `PANEL_LEFT`, `SPLITTER_MAIN`, and
`PANEL_PREVIEW`, rather than two separate left panels.

**Reparenting:**

- `PANEL_INPUT`, `PANEL_JOBS` (and header, tree view) → children of `PANEL_LEFT_JOBS`
- All `PANEL_PROMPT_LAB` children → children of `PANEL_LEFT_PROMPT_LAB`
- `PANEL_PROMPT_LAB` removed from right-panel hierarchy

**Layout rule switching:**

```
When left_tab == JobList:
    PANEL_LEFT_JOBS:       DockStyle::Fill
    PANEL_LEFT_PROMPT_LAB: DockStyle::Top, fixed_size: 0

When left_tab == PromptLab:
    PANEL_LEFT_JOBS:       DockStyle::Top, fixed_size: 0
    PANEL_LEFT_PROMPT_LAB: DockStyle::Fill
    PANEL_INPUT:           (hidden — its parent is hidden)
```

**Right panel:**

- Remove `PANEL_TAB_PROMPT_LAB` and `BUTTON_TAB_PROMPT_LAB`
- Tab bar now has 4 radio buttons instead of 5
- Tab content switching logic unchanged (Fill/collapse pattern)

**Splitter:**

- `SPLITTER_MAIN` position and behavior unchanged — it still controls the width between
  `PANEL_LEFT` and `PANEL_PREVIEW`

**`Msg::ToggleInputPanel` interaction:**

When `left_tab == PromptLab`, the INPUT panel is invisible because its parent (`PANEL_LEFT_JOBS`)
is collapsed to zero height. If `ToggleInputPanel` fires during this state, the reducer continues
to toggle `state.input_panel_visible` normally. The change takes visible effect when the user
switches back to the Job List tab. No special-case logic is needed; verify this round-trip in
Slice 3 QA.

### Render Changes (render.rs)

- `render_tab_bar_section()`: remove `BUTTON_TAB_PROMPT_LAB` sync; add left tab bar sync
  (`BUTTON_LEFT_TAB_JOBS`, `BUTTON_LEFT_TAB_PROMPT_LAB`)
- `render_prompt_lab_section()`: target controls now live under `PANEL_LEFT_PROMPT_LAB`;
  control IDs stay the same (no change to render logic itself)
- Preview header rendering: prepend `"[Lab] "` to header text when
  `right_pane.preview_source` is `PreviewSource::PromptLab { .. }`

### Event Mapping Changes (app.rs)

- `RadioButtonSelected` for `BUTTON_LEFT_TAB_JOBS` → `Msg::LeftTabSelected { tab: JobList }`
- `RadioButtonSelected` for `BUTTON_LEFT_TAB_PROMPT_LAB` → `Msg::LeftTabSelected { tab: PromptLab }`
- Remove mapping for `BUTTON_TAB_PROMPT_LAB`

### Result Routing (view_model.rs, state.rs)

**Content policy per right-panel tab when `left_tab == PromptLab`:**

| Right tab | Source | Notes |
|---|---|---|
| Triage | Lab run (if `selected_stage == Triage`) | Validated via `validate_triage` |
| Summary | Lab run (if `selected_stage == Summary`) | Validated via `validate_summary` |
| Briefing | Lab run (if `selected_stage == Briefing`) | Pass-through formatting |
| Trends | **Always production** | No lab equivalent |

If the right tab does not match the lab's `selected_stage` and no prior lab run exists for that
stage, show an explicit placeholder: `"### No Lab Result\n\nRun a {stage} experiment to see
results here."` — never show stale production text when `preview_source` is `PromptLab`.

**Run status handling:**

- **Completed runs:** Validate `output_json` using existing `harvester_engine` validators
  (`validate_triage`, `validate_summary`). Format the validated DTO into markdown using the same
  patterns as production (`build_right_pane_view`). If validation fails, render
  `"### Parse Error\n\nCould not parse lab result."`
- **Failed runs:** Format `failure_reason` as markdown: `"### Run Failed\n\n{reason}"`. The user
  sees the error in the right-panel viewer rather than a blank screen.
- **In-flight runs:** Show `"### Running…"` placeholder.
- `engine_info!("[prompt-lab-route] source={:?} stage={:?}", preview_source, stage)` at routing
  decision point for traceability.

**When `left_tab == JobList`:**
- `preview_source = PreviewSource::Production`
- Existing production-sourced content, unchanged
- Trends always production regardless of left tab

---

## Slices

### Slice 1 — Left Tab State and Messages

**Goal:** `LeftTab` enum, state field, accessor, setter, `Msg::LeftTabSelected`, reducer handler.
No UI changes yet.

**Files:**
- `crates/harvester_core/src/tabs.rs` — add `LeftTab` enum
- `crates/harvester_core/src/state.rs` — add `left_tab` field, `select_left_tab()`, `left_tab()`
- `crates/harvester_core/src/msg.rs` — add `Msg::LeftTabSelected { tab: LeftTab }`
- `crates/harvester_core/src/update.rs` — handle `Msg::LeftTabSelected`

**Tests:**
- `select_left_tab` sets field and marks dirty
- `select_left_tab` is idempotent: selecting the already-active tab does not flip dirty
- `Msg::LeftTabSelected` dispatches correctly, toggles prompt lab open/close
- `Msg::PromptLabOpenRequested` sets left tab to PromptLab and opens lab state
- `Msg::PromptLabCloseRequested` sets left tab to JobList and closes lab state

**Verification:** `cargo build`, existing tests pass

---

### Slice 2 — Remove PromptLab from Right-Panel Tabs

**Goal:** `AppTab` loses `PromptLab` variant. Right panel renders 4 tabs. Prompt Lab controls
become invisible (orphaned from layout) until Slice 3 reconnects them.

**Files:**
- `crates/harvester_core/src/tabs.rs` — remove `AppTab::PromptLab`
- `crates/harvester_core/src/state.rs` — update `open_prompt_lab()` / `close_prompt_lab()` to use
  `left_tab` instead of `active_tab`
- `crates/harvester_core/src/update.rs` — remove `AppTab::PromptLab` from `Msg::TabSelected` match
- `crates/harvester_app/src/platform/ui/constants.rs` — remove `BUTTON_TAB_PROMPT_LAB`,
  `PANEL_TAB_PROMPT_LAB`
- `crates/harvester_app/src/platform/ui/layout.rs` — remove Prompt Lab tab creation and layout rules
- `crates/harvester_app/src/platform/ui/render.rs` — remove Prompt Lab radio button from tab bar sync
- `crates/harvester_app/src/platform/app.rs` — remove `BUTTON_TAB_PROMPT_LAB` event mapping

**Tests:**
- Verify `AppTab` exhaustive matches compile
- `open_prompt_lab` sets `left_tab = PromptLab`
- `close_prompt_lab` sets `left_tab = JobList`

**Verification:** `cargo build`, app runs with 4 right-panel tabs

---

### Slice 3 — Left Panel Tab Bar and Reparenting

**Goal:** Left panel gains tab bar. Prompt Lab controls appear in the left panel when the
Prompt Lab left tab is selected.

**Files:**
- `crates/harvester_app/src/platform/ui/constants.rs` — add `PANEL_LEFT`, `PANEL_LEFT_TAB_BAR`,
  `BUTTON_LEFT_TAB_JOBS`, `BUTTON_LEFT_TAB_PROMPT_LAB`, `PANEL_LEFT_JOBS`,
  `PANEL_LEFT_PROMPT_LAB`
- `crates/harvester_app/src/platform/ui/layout.rs`:
  - `initial_commands()`: create new structural panels and left tab radio buttons; reparent
    INPUT/JOBS under `PANEL_LEFT_JOBS`; reparent Prompt Lab controls under `PANEL_LEFT_PROMPT_LAB`
  - `build_layout_rules()`: add left-tab switching rules (Fill/collapse); reparent existing
    Prompt Lab layout rules to new parent
- `crates/harvester_app/src/platform/ui/render.rs`:
  - Add `render_left_tab_bar_section()` to sync left radio buttons
  - Include `left_tab` in layout-change detection hash
- `crates/harvester_app/src/platform/app.rs`:
  - Map `BUTTON_LEFT_TAB_JOBS` → `Msg::LeftTabSelected { tab: JobList }`
  - Map `BUTTON_LEFT_TAB_PROMPT_LAB` → `Msg::LeftTabSelected { tab: PromptLab }`

**Tests:**
- Left tab switching produces correct layout rules (Jobs panel Fill / PromptLab zero-height and vice versa)
- Exactly-one-Fill per parent in both left and right panel hierarchies
- `ToggleInputPanel` round-trip: toggle while on PromptLab tab, switch to JobList, verify
  INPUT panel reflects the toggled state
- Left tab buttons are in a separate radio group from right tab buttons (group_start isolation)

**Verification:** `cargo build`, app runs, clicking left tabs switches between Job List and
Prompt Lab. Prompt Lab controls render in the left panel. Right panel always shows 4 content tabs.

---

### Slice 4 — View Model: LeftPaneView and Lab-Sourced Content

**Goal:** View model carries left-pane state. Right-pane content is sourced from lab results when
the Prompt Lab left tab is active.

**Files:**
- `crates/harvester_core/src/view_model.rs`:
  - Add `LeftPaneView { left_tab, prompt_lab }`
  - Move `prompt_lab` from `RightPaneView` to `LeftPaneView`
  - Add `preview_source: PreviewSource` to `RightPaneView`
  - Add lab-result formatting functions that reuse existing validators:
    - `format_lab_triage_markdown(output_json) -> Option<String>` — call `validate_triage` from
      `harvester_engine`, format the validated `TriageResult` DTO as markdown
    - `format_lab_summary_markdown(output_json) -> Option<String>` — call `validate_summary`,
      format the validated `SummaryResult` DTO as markdown
    - `format_lab_briefing_markdown(output_json) -> Option<String>` — pass through
- `crates/harvester_core/src/state.rs`:
  - `build_view_model()`: build `LeftPaneView`
  - `build_right_pane_view()`: when `left_tab == PromptLab`, route lab `output_json` through the
    formatting functions into `triage_markdown` / `summary_markdown` / `briefing_markdown`
- `crates/harvester_app/src/platform/ui/render.rs`:
  - Read `prompt_lab` from `left_pane` instead of `right_pane`
  - Prefix preview header with `"[Lab] "` when `preview_source` is `PromptLab`

**Tests:**
- `build_right_pane_view` returns lab triage markdown when lab is active with a completed triage run
- `build_right_pane_view` returns production content when left tab is JobList
- `format_lab_triage_markdown` correctly parses valid JSON (via `validate_triage`)
- `format_lab_summary_markdown` correctly parses valid JSON (via `validate_summary`)
- Malformed `output_json` yields parse-error markdown (not `None`)
- Failed run renders `"### Run Failed\n\n{reason}"` markdown
- In-flight run renders `"### Running…"` placeholder
- When left tab is PromptLab and right tab is Trends, trends data is still production-sourced
- When left tab is PromptLab and no lab run exists for the viewed stage, explicit placeholder shown
- Left tab buttons reflect reducer state correctly in render output
- Right tab bar emits exactly 4 tabs (no PromptLab); Prompt Lab controls render under left parent

**Verification:** `cargo build`, tests pass. Prompt Lab results display in right-panel viewers.

---

### Slice 5 — Auto-Switch Right Tab on Lab Run Completion

**Goal:** When a Prompt Lab run completes, the right panel automatically switches to the tab
matching the lab stage.

**Files:**
- `crates/harvester_core/src/update.rs`:
  - In **both** the `LlmResultKind::Success` and failure branches for Prompt Lab runs, after
    `state.complete_prompt_lab_run(...)` or `state.fail_prompt_lab_run(...)`:
    - Map `PromptLabStage::Triage` → `state.select_tab(AppTab::Triage)`
    - Map `PromptLabStage::Summary` → `state.select_tab(AppTab::Summary)`
    - Map `PromptLabStage::Briefing` → `state.select_tab(AppTab::Briefing)`
  - Only auto-switch when `left_tab == PromptLab` (so production completions are unaffected)
  - On failure, the auto-switch ensures the user sees the error markdown (from Slice 4) in the
    right-panel viewer, rather than stale content from a different tab

**Tests:**
- Prompt Lab triage run completion auto-selects `AppTab::Triage`
- Prompt Lab triage run **failure** also auto-selects `AppTab::Triage`
- Auto-switch only fires when `left_tab == PromptLab`

**Verification:** `cargo build`, run a Prompt Lab experiment, right panel switches to result tab
automatically.

---

### Slice 6 — Cleanup and Polish

**Goal:** Remove dead code, run clippy, ensure all tests pass.

**Tasks:**
- Remove any remaining references to `AppTab::PromptLab` in comments, UI text, or dead code paths
- Verify `Msg::PromptLabOpenRequested` / `Msg::PromptLabCloseRequested` work correctly with
  left-tab switching
- Ensure splitter drag works correctly with the new `PANEL_LEFT` wrapper
- Grep for removed constants/variants (`AppTab::PromptLab`, `BUTTON_TAB_PROMPT_LAB`,
  `PANEL_TAB_PROMPT_LAB`) to catch stragglers in comments or dead code
- Follow-up: remove `PromptLabState.visible` field entirely, deriving visibility from `left_tab`
  (or mark as TODO if deferred)
- Run `cargo clippy --all-targets -- -D warnings`
- Run `cargo nextest run`
- Manual QA: full end-to-end walkthrough (load articles → triage → switch to Prompt Lab →
  configure → run → see result in right panel → switch back to Job List → verify production
  content restored)

---

## Risk & Mitigation

| Risk | Impact | Mitigation |
|---|---|---|
| Layout engine doesn't support nested Fill-within-Fill smoothly | Controls misaligned or invisible | Test `PANEL_LEFT` as sole dock-left parent early in Slice 3; fallback: flatten hierarchy |
| Radio button group isolation (left vs right) | Left tab clicks affect right tab state | `group_start: true` on first button of each group; both groups in separate panels |
| Prompt Lab output_json is raw JSON, not markdown | Right-panel viewers show garbage | Validate via existing `validate_triage`/`validate_summary` then format (Slice 4); fail-safe to "Parse error" message |
| Dual visibility state (`left_tab` vs `PromptLabState.visible`) | State drift / contradictions | `left_tab` is canonical; bridge methods still call `open()`/`close()` during migration; follow-up removes `visible` field |
| Existing tests hardcode `AppTab::PromptLab` | Compile failures | Exhaustive search-and-update in Slice 2 |
| INPUT panel reappears when switching back to Job List | Visual glitch | Test round-trip: PromptLab → JobList → verify INPUT panel visibility matches prior state |

---

## Not in Scope

- New Prompt Lab features (only relocates existing controls)
- Changes to Prompt Lab domain logic (`PromptLabState`, run records, compare batches)
- Changes to LLM pipelines, persistence, or caching
- Changes to CommanDuctUI submodule (layout engine, widget types)
- Keyboard shortcuts for left-tab switching (deferred)
