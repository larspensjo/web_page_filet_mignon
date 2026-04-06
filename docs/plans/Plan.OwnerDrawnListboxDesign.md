# Owner-Drawn ListBox Control — Design Spec

Date: 2026-04-05
Revised: 2026-04-05 (incorporates review findings and design decisions)

## Overview

Replace the Win32 TreeView used for the left-pane job tabs (Jobs, Triage Review, Triage Results) with a new owner-drawn ListBox control in CommanDuctUI. The new control renders structured, multi-line rows with a fixed-width badge column, enabling proper visual hierarchy per the Visual Design Spec.

**Scope:** Jobs, Triage Review, and Triage Results tabs only. PromptLab is a separate left-pane mode (`PANEL_LEFT_PROMPT_LAB`) and is excluded from this change.

CommanDuctUI version bumps from 0.10.7 to 1.0.0 with this change.

## Goals

- Flat list with structured rows: badge column + title + metadata line.
- Pixel-level control over row layout, typography, and color.
- Flicker-free rendering via double-buffered painting.
- Fixed-width badge column so titles align across all rows.
- One control replaces the TreeView on the three job-oriented tabs.
- Automated indirect-link collection replaces manual per-link toggling.
- Keyboard-driven exclude/include replaces checkbox toggling for pre-triage review.

## Non-Goals

- Tree hierarchy / expand-collapse (removed).
- Checkboxes (removed from all tabs).
- Type-ahead search.
- Multi-selection.
- Horizontal scrolling.
- PromptLab integration (separate left-pane layout, unchanged).
- Scroll position restoration (deferred — no existing Harvester scroll-tracking flow to build on).

---

## 1. New CommanDuctUI Control: ListBox

Implemented in `CommanDuctUI` 1.0.x. The owner-drawn control, shared types, scrolling, selection events, and programmatic selection commands now exist in the toolkit.

---

## 2. Badge Column Layout

### Fixed-Width Column

The badge column occupies a fixed pixel width on the left side of every row. The width is set per-control instance (not per-item) via `PopulateListBox`, so all titles align to the same x-position.

### Badge Rendering

Each badge is a rounded rectangle with centered text:
- Background and text color from its `StyleId`.
- Height matches the text line height.
- Width from `GetTextExtentPoint32W` plus 6px horizontal padding per side.
- 3px border radius.
- 4px gap between adjacent badges.
- On disabled rows, badge colors shift to their muted variants.

### Per-Tab Badge Configurations

| Tab             | Badge 1                                    | Badge 2                        | Column Width |
|-----------------|--------------------------------------------|--------------------------------|-------------|
| Triage Results  | Priority (`P3`–`P6`, colored by severity)  | Category (`Business`, `Policy`) | ~130px      |
| Triage Review   | Review status (`Review`, `Included`, etc.) | —                              | ~130px      |
| Jobs            | Job status (`Done`, `Fetch`, `ERR`, etc.)  | —                              | ~130px      |

The column width is tuned to fit the widest expected badge combination.

---

## 3. Row Layout and Spacing

### Row Structure (top to bottom)

- 6px top padding
- Badge line + Title (same vertical line, badges vertically centered with title): 14px, weight 500, Text Primary
- 2px gap
- Metadata: 12px, weight 400, Text Tertiary
- 6px bottom padding

Total row height: ~44px. Computed once from font metrics, constant for all rows.

### Selection Highlight

Full-row background fill using the selected row style, plus a 3px Accent Primary bar on the left edge.

### Hover State

Subtle background tone shift (Surface Raised) on mouse-over. No border or ring.

### Disabled Row Appearance

When `enabled: false`, the row renders with dimmed text (Text Tertiary for title, near-invisible for metadata) and muted badge colors. The row remains selectable and shows hover/selection highlight normally — the dimming communicates "excluded" status, not interactivity.

### Row Separation

No explicit divider lines. Separation from spacing and selection highlight only.

---

## 4. Metadata Per Tab

### Triage Results

Badges: `[P5]` (colored) + `[Business]` (muted pill).
Title: Article summary title.
Metadata: `source · tag count` (e.g. `aibusiness.com · 11 tags`).

### Triage Review

Badge: `[Review]` / `[Included]` / `[Auto Excluded]` / `[Excluded]` etc. (colored by status).
Title: Article summary title.
Metadata: `category · source` (e.g. `Security · venturebeat.com`).
Rows with `[Excluded]` status render as disabled.
Indirect-link articles show an `[Indirect]` badge (see Section 5).

### Jobs

Badge: `[Done]` / `[Fetch]` / `[ERR]` etc. (colored by status).
Title: Article summary title (or URL slug if no summary).
Metadata: `source · tokens · bytes` (e.g. `aibusiness.com · 2.4K tok · 15.3 KB`).

---

## 5. Indirect Links — Automated Collection and Polling

### Overview

The manual per-link tree toggling (`LinkToggleRequested` → download/delete individual links) is replaced by an automated batch feature. During polling, all indirect links (hyperlinks extracted from fetched articles) are collected, pre-filtered, and deduplicated. A button in the bottom bar triggers fetching the filtered set as new jobs.

### Collection Phase (during polling)

When a job's fetch completes and extracted links are available, the reducer collects all `LinkKind::Hyperlink` links into a global indirect-link pool:

- **Deduplication:** By normalized URL. If the same URL appears in links from multiple parent jobs, keep one entry. Also skip URLs that already exist in the jobs list.
- **Pre-triage filter:** Deferred to a future iteration. For now, only dedup and already-in-jobs-list filtering apply.
- **State:** Each indirect link tracks: URL and source job ID.

### Status Display

The status bar (bottom bar) shows indirect-link pool state:
- During polling: `"42 indirect links collected"`.
- After polling: `"42 indirect links ready"` or `"No indirect links"`.

### "Poll Indirect Links" Button

A new button in the bottom bar, placed beside the existing "Poll Sources" button (`BUTTON_POLL_SOURCES`, order 3 in `PANEL_BUTTONS`). The new button gets order 4, shifting "Open Browser" to order 5.

**Button state:**
- **Disabled** when the indirect-link pool is empty (no links collected yet).
- **Enabled** when there are indirect links ready to fetch.
- **Disabled** while indirect-link fetching is in progress (same pattern as Poll Sources during a poll).

**Behavior:**
- Creates a new job for each non-filtered indirect link URL.
- Each new job is marked with an `indirect: true` origin flag in state.
- Jobs enter the normal fetch pipeline.
- After fetch + triage completes, indirect-origin articles appear in the Triage Review tab with an `[Indirect]` badge to distinguish them from directly-polled articles.
- By default, indirect articles are **not included** (their pre-triage default is `Exclude`). The user reviews them in Triage Review and explicitly includes any that are relevant.

### State Lifecycle and the Accumulation Problem

Polling indirect links creates new jobs. Those jobs will themselves produce extracted links when fetched, which would normally feed back into the indirect-link pool. Without care, this creates a loop: poll indirects → fetch → extract links → pool grows → poll indirects again → …

**Resolution — generation-based accumulation:**

- The pool tracks a `generation: u32` counter, incremented each time "Poll Sources" runs (direct polling).
- Only links extracted from **direct-origin** jobs (current generation) feed the pool. Links extracted from indirect-origin jobs are **not** added to the pool.
- When "Poll Sources" runs again, the pool is cleared and the generation increments. Links from the new direct poll accumulate fresh.
- Pressing "Poll Indirect Links" does not clear or regenerate the pool — it consumes the current pool by creating jobs, then the pool becomes empty (button disables).
- The pool is ephemeral — not persisted to disk. Cleared on app startup.

This means:
1. Poll Sources → articles fetched → indirect links accumulate → button enables.
2. Press "Poll Indirect Links" → jobs created → pool drains → button disables.
3. Indirect jobs fetch and triage, but their extracted links do **not** re-fill the pool.
4. Next "Poll Sources" → fresh cycle.

### Data Model Additions

```rust
/// In harvester_core state:
pub struct IndirectLinkPool {
    pub generation: u32,
    pub links: Vec<IndirectLink>,
}

pub struct IndirectLink {
    pub url: String,
    pub source_job_id: JobId,
}

/// On Job:
pub enum JobOrigin {
    Direct,
    Indirect { source_job_id: JobId },
}
```

### Removed

- `build_link_children()` — no per-job link tree rows.
- `links_folder_tree_item_id()`, `links_show_more_tree_item_id()`, `link_tree_item_id()` — tree item ID helpers for link rows.
- `LinkToggleRequested` message — individual link download/delete toggling from the tree UI. (The `LinkDownloadCompleted`, `LinkDownloadFailed`, `LinkDeleted` messages remain as they're used by the effect runner pipeline.)

---

## 6. Keyboard-Driven Exclude/Include for Triage Review

### Overview

Checkboxes are removed from all left-pane tabs. In the Triage Review tab, the user excludes or re-includes articles using a keyboard shortcut on the currently selected row.

### Interaction

- **Shortcut key:** `X` toggles the selected article between included and excluded. Mnemonic: "e**X**clude."
- **Visual feedback:** When excluded, the row's badge changes to `[Excluded]` (muted style) and the row renders as disabled (dimmed). When re-included, the badge reverts to its previous status and the row renders normally.
- **Effect:** Dispatches `Msg::PreTriageDecisionSet { key, decision }` with `ManualDecision::Include` or `ManualDecision::Exclude`, same as the current checkbox toggle.
- **Scope:** Only active during pre-triage review mode (when `is_pre_triage_reviewing` is true). Outside pre-triage review, the key does nothing.

### Removed

- `job_row_check_policy()` — checkbox visibility logic.
- `CheckState` usage in left-pane layout.
- `TreeViewItemToggledByUser` event handling for pre-triage decisions.
- `AppUiStateProvider` / `UiStateProvider` trait — visual state markers replaced by badge styles and the `enabled` flag.

---

## 7. CommanDuctUI API Surface

Implemented. Commands, events, styles, dispatcher wiring, toolkit version bump, and changelog entries landed in `CommanDuctUI`.

---

## 8. Harvester Integration

Status: partial. The left pane now populates and selects through the listbox path for Jobs, Triage Review, and Triage Results, but the migration is not complete.

### Data Flow (unchanged pattern)

`render()` receives `AppViewModel` → builds `Vec<ListBoxItemDescriptor>` → compares with previous snapshot → issues `PopulateListBox` if changed.

Snapshot comparison simplified: compare item IDs, badges, title, metadata, enabled. Any difference triggers full repopulate (flicker-free with double buffering).

### New Format Functions

Replace flat-string formatters with structured item builders:

- `build_triage_results_item(job: &JobRowView) -> ListBoxItemDescriptor`
- `build_triage_review_item(job: &JobRowView) -> ListBoxItemDescriptor`
- `build_jobs_item(job: &JobRowView) -> ListBoxItemDescriptor`

The Triage Review builder sets `enabled: false` for excluded articles and adds `[Indirect]` badge for indirect-origin articles.

Existing helper functions (`triage_result_primary_label()`, `job_source_label()`, `compact_triage_tag_count()`, `title_case_label()`, `domain_from_url()`, `truncate_with_ellipsis()`, `compact_url_label()`) remain unchanged.

Current state:
- Structured listbox item building is in use.
- Excluded rows already render as disabled in Triage Review.
- Full badge parity is not done yet. Triage Results still renders only one badge, and Triage Review does not yet add the `[Indirect]` badge.
- Legacy tree formatters and tree-building code still exist in the codebase and should be removed at final cutover.

### New: Indirect Link Pool

- `IndirectLinkPool` added to `AppState`, populated during `Msg::JobFetchCompleted` when extracted links are available.
- New `Msg::PollIndirectLinks` triggered by the button — creates jobs for each non-filtered link.
- New view model field: `indirect_link_summary: Option<IndirectLinkSummary>` for status bar display.
- New `JobOrigin` field on jobs to distinguish direct vs indirect.

Current state:
- Reducer/state support for the indirect-link pool, generation handling, and `PollIndirectLinks` exists.
- The UI cutover is incomplete. The bottom-bar button and status display described below are not fully surfaced yet.

### New: Keyboard Exclude Handler

- `app.rs` event loop: when `ListBoxItemSelectionChanged` is active on Triage Review and the exclude key is pressed, dispatch `Msg::PreTriageDecisionSet` toggling the selected job's include/exclude state.
- This replaces the `TreeViewItemToggledByUser` → `PreTriageDecisionSet` path.

Current state:
- Not complete yet. Listbox selection events are wired to `Msg::JobSelected`.
- Keyboard exclude/include on `X` is still pending.
- TreeView toggle handling for pre-triage decisions still exists and must be removed when the listbox keyboard path replaces it.

### Removed from Harvester

- `format_job_row_legacy()`, `format_job_row_triage_review()`, `format_job_row_triage_results()`.
- `JobRowPresentation` enum.
- `build_job_tree()` and `build_link_children()`.
- `AppUiStateProvider` (and `UiStateProvider` trait usage) — markers replaced by badge styles.
- `triage_marker_for_priority()`.
- Tree snapshot diffing code (`TreeSnapshot`, structure comparison, `UpdateTreeItemText`, `UpdateTreeItemVisualState`).
- Checkbox-related: `job_row_check_policy()`, `CheckState` usage in layout.
- `TreeViewItemSelectionChanged` event handling — replaced by `ListBoxItemSelectionChanged`.
- `TreeViewItemToggledByUser` event handling for left-pane job/link toggles.
- `LinkToggleRequested` message and its reducer arm.
- Per-job link tree item ID helpers.

### Kept in Harvester

- All text helper functions listed above.
- Selection tracking (adapted for ListBox events).
- `Msg::PreTriageDecisionSet` (triggered by keyboard shortcut instead of checkbox).
- `Msg::LinkDownloadCompleted`, `LinkDownloadFailed`, `LinkDeleted` (effect runner pipeline, used by indirect-link batch fetch).
- Reading pane, tabs, bottom bar — unchanged.

---

## 9. What Stays in CommanDuctUI

Implemented decision: the TreeView remains in `CommanDuctUI`. Harvester is migrating away from it for the left pane without removing the generic TreeView control from the toolkit.

---

## 10. Test Plan

Status: partial coverage exists now. The toolkit control and parts of the reducer flow are covered, but the full Harvester cutover described below is not yet covered end-to-end.

### Unit Tests (Harvester)

- **Row builders:** Each of `build_triage_results_item`, `build_triage_review_item`, `build_jobs_item` produces correct badge text/style, title, metadata, and `enabled` flag for representative inputs.
- **Disabled row rendering:** Excluded articles produce `enabled: false` with `[Excluded]` badge.
- **Indirect badge:** Indirect-origin articles produce an `[Indirect]` badge in Triage Review.
- **Selection events:** `ListBoxItemSelectionChanged` dispatches `Msg::JobSelected` with the correct job ID.
- **Keyboard exclude toggle:** Pressing `X` on a selected Triage Review row dispatches `Msg::PreTriageDecisionSet` with toggled decision. Does nothing outside pre-triage review mode.
- **Indirect link pool — accumulation:** `JobFetchCompleted` for direct-origin jobs populates the pool. Dedup by URL works. Already-in-jobs-list URLs are skipped.
- **Indirect link pool — no loop:** `JobFetchCompleted` for indirect-origin jobs does **not** add links to the pool.
- **Indirect link pool — generation reset:** `PollSourcesClicked` clears the pool and increments the generation.
- **Poll indirect links:** `Msg::PollIndirectLinks` creates jobs for pool links with `JobOrigin::Indirect`, then the pool drains to empty.
- **Button state:** "Poll Indirect Links" button is disabled when pool is empty, enabled when links exist, disabled during indirect fetch.
- **PromptLab isolation:** Switching to `LeftTab::PromptLab` shows `PANEL_LEFT_PROMPT_LAB` and does not interact with the ListBox control.

### Manual Verification

- Dark theme: all new styles render correctly in both light and dark themes.
- Scroll behavior: large lists scroll smoothly, selection remains visible after scroll.
- Tab switching: switching between Jobs, Triage Review, and Triage Results repopulates with correct badge configurations.
- Indirect link flow: poll → collect → button → fetch → triage → appears with [Indirect] badge in Triage Review, excluded by default.
- Exclude toggle: keyboard shortcut dims the row and changes badge; pressing again restores it.

---

## Open Questions

No blocking open questions remain. The following are deferred to future iterations:

- **Pre-triage filter for indirect links:** Currently only dedup and already-in-jobs-list checks apply. A relevance filter (reusing the pre-triage infrastructure) can be added later if the pool grows too large to be useful without it.
- **Pool persistence:** The pool is ephemeral for now. If users need it to survive restarts, it can be added to the state file later.
