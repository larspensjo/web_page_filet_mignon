# Results List Hover Flicker Remediation Plan

**Status:** Revised 2026-07-17 after `docs/Review.ResultsListHoverFlicker.md`
**Scope:** `src/CommanDuctUI` owner-drawn list box plus Harvester's list-render command diffing  
**Goal:** Preserve the Results-row hover treatment while eliminating the transient blank/partially painted row and avoiding unnecessary list repaints.

## Problem statement

The Results panel uses CommanDuctUI's owner-drawn list box. A hover transition currently:

1. updates `ListBoxState::hover_index`;
2. invalidates the previous and current row rectangles;
3. handles `WM_PAINT` directly against the visible window DC;
4. fills the invalid region with the normal row background;
5. paints the final row background, badges, and text in separate GDI calls.

The row is not removed or recreated, and Win32 background erasure is already suppressed. The flicker is the intermediate state of a direct-to-screen repaint becoming visible before the final row is complete.

The paint path also does more work than the dirty region requires. It walks every visible row and recalculates badge-slot widths across every item once per painted row. Separately, Harvester emits `SetListBoxRowDensity` and `PopulateListBox` on every full application render, and both native handlers currently invalidate even when their inputs are unchanged.

## Design decisions

- Fix the rendering defect in CommanDuctUI, where the generic control owns native painting.
- Render each `WM_PAINT` into an off-screen GDI surface and copy the completed dirty rectangle to the window in one `BitBlt`. There must be no visible intermediate clear/background-only state.
- Paint only rows intersecting the dirty rectangle. Use pure geometry helpers for dirty-row calculation so the behavior is unit-testable without a live HWND.
- Cache badge-slot measurements in list-box state. Invalidate the cache when item or font inputs change; do not remeasure the whole list for every visible row.
- Make CommanDuctUI's density, population, and selection handlers idempotent. Repeated commands with identical payloads must not mutate state, reset scroll/selection, rebuild measurement caches, ensure an already-selected row visible, or invalidate the window.
- Also diff the Harvester list render model so unchanged native commands are not emitted. CommanDuctUI's idempotency remains the generic safety net for all hosts.
- Preserve the existing dark palette, hover color, selected-row precedence, selection accent, disabled-row behavior, scrolling, keyboard navigation, and public `PlatformCommand` API.
- Prefer a small reusable GDI buffering guard in `gdi_utils.rs` over ad hoc handle cleanup in the list-box handler. If buffer creation fails, fall back to the existing direct painter so the control remains functional under GDI resource pressure.
- Use an explicit compatible memory DC/bitmap rather than `BufferedPaintInit`/`BeginBufferedPaint`. The uxtheme API is a valid alternative, but the local RAII guard avoids adding process-level uxtheme initialization/lifetime coupling, keeps GDI resource ownership visible, and permits an explicit direct-paint fallback. Record this choice in the CommanDuctUI diary so it can be reconsidered deliberately later.

## Submodule and review workflow

`src/CommanDuctUI` is a separate Git repository recorded by the parent as a gitlink.

- During implementation, leave both repositories uncommitted. The parent workspace can build and test against the dirty submodule working tree.
- Do not attempt to record a new parent gitlink while the submodule changes are uncommitted; a gitlink can reference only a committed submodule object.
- Stop for review after implementation and verification.
- Only after explicit approval: commit CommanDuctUI in its own repository, then update the parent gitlink and parent `Cargo.lock` to that exact commit/version. Do not mix the toolkit commit with the parent integration commit.
- Preserve any unrelated dirty/staged work in either repository.

## Phase 1: Lock the behavioral and geometry contracts

**Files:**

- Modify: `src/CommanDuctUI/src/controls/listbox_handler.rs`
- Tests: inline `#[cfg(test)]` module in the same file

- [ ] Extract a pure `visible_row_range`/paint-plan helper taking item count, scroll row, row height, client height, and dirty `RECT`. It must return only item indices whose row rectangles intersect the dirty area.
- [ ] Cover a single hovered row, two separated dirty rows represented by their bounding rectangle, partially aligned dirty rectangles, scrolled lists, empty lists, and rectangles below the final item.
- [ ] Add an explicit partial-row test: when the client bottom cuts through the next row and the dirty rectangle intersects that strip, the paint plan includes the partially visible row.
- [ ] Keep scroll clamping, page navigation, and `SCROLLINFO` calculations on the existing floor-based `visible_rows` semantics. Only paint intersection changes to include a partial bottom row; do not silently change paging or scrollbar behavior.
- [ ] Extract pure update-decision seams for row-density, item/column-width, and requested-selection changes.
- [ ] Prove that identical density, item payload, and selected item are no-ops, while actual density/content/width/selection changes request only the necessary layout, scrollbar, visibility, and paint work.
- [ ] Keep the existing `row_rect` and navigation tests passing.

**Gate:** Run `cargo test` inside `src/CommanDuctUI`. The new tests should initially expose the missing dirty-range and idempotency behavior, then pass before native painting is changed.

## Phase 2: Add safe off-screen GDI painting

**Files:**

- Modify: `src/CommanDuctUI/src/controls/gdi_utils.rs`
- Modify: `src/CommanDuctUI/src/controls/listbox_handler.rs`

- [ ] Add a crate-private RAII paint-buffer guard that owns a compatible memory DC and bitmap, restores the previously selected bitmap, and releases all GDI handles on every exit path.
- [ ] Size the buffer to `PAINTSTRUCT::rcPaint`, not the whole control. Establish a viewport/origin mapping so existing list drawing remains expressed in client coordinates.
- [ ] Expose one `present` operation that restores the source origin as needed and performs exactly one `BitBlt` of the completed dirty rectangle.
- [ ] Treat an empty dirty rectangle as a no-op and handle failed DC/bitmap creation without leaking resources.
- [ ] Refactor `WM_PAINT` so `BeginPaint`/`EndPaint` still bracket every paint, the list is drawn into the buffer, and only the completed buffer is presented to the window DC.
- [ ] Retain `WM_ERASEBKGND => LRESULT(1)` and `InvalidateRect(..., false)`; double buffering complements rather than replaces those contracts.
- [ ] Keep a direct-paint fallback for buffer-allocation failure, with a contextual warning through the crate's existing logging path.

**Gate:** Run CommanDuctUI tests and `cargo build` in the submodule. Inspect the GDI ownership paths specifically for selected-object restoration, `DeleteObject`, `DeleteDC`, and early returns.

## Phase 3: Make list painting dirty-region aware and cache layout measurements

**Files:**

- Modify: `src/CommanDuctUI/src/controls/listbox_handler.rs`

- [ ] Change the painter to accept explicit client and dirty rectangles instead of rediscovering paint state through `HWND` while holding a mutable `ListBoxState` borrow.
- [ ] Fill the dirty buffer background once, then draw only rows returned by the Phase 1 paint plan. Include the empty tail below the last row in the background fill.
- [ ] Preserve visual precedence exactly: selected, hovered, disabled, normal; selection continues to win over hover.
- [ ] Add cached badge-slot widths to `ListBoxState` and split measurement from drawing.
- [ ] Measure all badge positions once after the item/font inputs change, then reuse the cached widths for every row and subsequent hover repaint.
- [ ] Invalidate the measurement cache on item replacement and on any future/current font replacement path. Density, hover, selection, and scrolling must not invalidate it.
- [ ] Remove the per-row call that scans and measures badges for the entire list.
- [ ] Keep the existing pure widest-badge-per-position tests, and add cache invalidation/reuse tests around the new state seam.

**Gate:** Run CommanDuctUI tests. Add temporary debug-only paint counters if useful during development, but remove them before review. A hover transition should measure zero badge text after the cache has been warmed.

## Phase 4: Make CommanDuctUI commands idempotent

**Files:**

- Modify: `src/CommanDuctUI/src/controls/listbox_handler.rs`

- [ ] In `handle_set_list_box_row_density_command`, return without scrollbar work or invalidation when the effective density is unchanged.
- [ ] In `handle_populate_list_box_command`, return without replacing items, rebuilding caches, updating scroll information, or invalidating when both descriptors and badge-column width are unchanged.
- [ ] In `handle_set_list_box_selection_command`, resolve the requested ID and return without `ensure_row_visible` or invalidation when that index is already selected.
- [ ] When population really changes, continue preserving selection by stable `ListBoxItemId` and clamping the current scroll row.
- [ ] Route the decision through the pure seams tested in Phase 1; keep Win32 calls only in the effectful branch.
- [ ] Confirm that style, selection, resize, and scroll changes still invalidate the required area.

**Gate:** Run CommanDuctUI tests and add regression assertions that repeated identical density, population, and selection commands produce a no-op decision.

## Phase 5: Suppress redundant commands in Harvester

**Files:**

- Modify: `crates/harvester_app/src/platform/ui/render.rs`
- Modify: `crates/harvester_app/src/platform/ui/render_list_box.rs`
- Modify: `crates/harvester_app/src/platform/ui/render_tests.rs`

- [ ] Add a dedicated `ListBoxRenderState` under `TreeRenderState`, keeping previous row density, item descriptors, and selected item ID as separate values.
- [ ] Emit `SetListBoxRowDensity` only when density changes.
- [ ] Emit `PopulateListBox` only when item descriptors change, and recompute `badge_column_width` from those items at emission time. Do not keep the derived width as an independent diff key/source of truth; if later profiling justifies memoizing it, store it only as a memo tied to the exact item value.
- [ ] Emit `SetListBoxSelection` only when the selected item changes to `Some`; preserve the current public selection contract rather than introducing an unrelated API change.
- [ ] Add render tests for initial emission, an identical second render, selection-only change to `Some`, row-content/badge-only change, and a tab transition that changes density.
- [ ] Add an explicit selection `Some -> None` test with unchanged items that expects no list-box command. There is currently no clear-selection command and native population preserves a stable selected ID, so this test deliberately locks the pre-existing retained-highlight contract instead of allowing the behavior to change accidentally during diffing.
- [ ] Preserve the current lifecycle invariant that list controls are created once with their window and `TreeRenderState` is created for that same lifetime. If a future/current path recreates the native control tree without replacing `TreeRenderState`, reset `ListBoxRenderState` alongside `layout_initialized` so the next render fully resynchronizes native state.
- [ ] Update existing render tests that currently assume every render contains `PopulateListBox`; assertions should distinguish initial/content-changing renders from unchanged renders.

**Gate:** Run `cargo test -p harvester_app`. An unchanged view must produce none of the three list-box commands, and selection-only changes must not repopulate the list.

## Phase 6: Release metadata and durable documentation

**CommanDuctUI files:**

- Modify: `src/CommanDuctUI/Cargo.toml`
- Modify: `src/CommanDuctUI/Cargo.lock`
- Modify: `src/CommanDuctUI/CHANGELOG.md`
- Modify: `src/CommanDuctUI/docs/EngineeringDiary.md`

**Parent files:**

- Modify: `Cargo.lock`
- Modify: `docs/EngineeringDiary.md`

- [ ] Bump CommanDuctUI from `2.3.2` to patch release `2.3.3`; the fix changes performance/painting behavior without changing the public API.
- [ ] Add a user-facing changelog entry describing atomic buffered list-row painting and redundant repaint suppression.
- [ ] Update the submodule lockfile and the parent workspace lockfile to `commanductui 2.3.3`.
- [ ] Add a concise CommanDuctUI diary entry covering the native paint fix, GDI ownership, dirty-region planning, and badge measurement cache.
- [ ] Add a concise parent diary entry covering the Results flicker symptom, host command diffing, and the submodule integration.
- [ ] Do not modify dark-theme tokens or introduce Harvester terminology into CommanDuctUI.

## Phase 7: Verification and visual acceptance

Run inside `src/CommanDuctUI`, in order:

1. `cargo build`
2. `cargo test`
3. `cargo clippy --all-targets -- -D warnings`
4. `cargo fmt`
5. Re-run `cargo test` if formatting changes code.

Run in the parent workspace, in order:

1. `cargo build`
2. `cargo test -p harvester_app`
3. `cargo test`
4. `cargo clippy --all-targets -- -D warnings`
5. `cargo fmt`
6. Re-run targeted tests if formatting changes code.

Manual Windows acceptance matrix:

- [ ] Move the pointer slowly and rapidly across compact Results rows: hover appears without blank text, background flash, or left-edge flash.
- [ ] Move between non-adjacent rows quickly: both old and new hover states settle cleanly.
- [ ] Repeat in expanded Jobs and Triage Review rows so the generic control is covered at both densities.
- [ ] Verify selected-row styling remains stable while the pointer enters/leaves the selected row.
- [ ] Scroll with wheel and scrollbar, then hover immediately; row hit testing and cached layout remain aligned.
- [ ] Resize the window and repeat; dirty buffering uses the new client dimensions.
- [ ] Let background job/result updates arrive while hovering; unchanged app renders do not repopulate the control, and actual content changes repaint atomically.
- [ ] Confirm dark background, hover fill, disabled rows, badges, ellipsizing, scrollbar theme, keyboard navigation, and click selection are unchanged.

## Review gate and submodule integration

- [ ] Stop with all implementation changes uncommitted in both repositories and report both `git status` outputs plus verification results for review.
- [ ] After explicit approval, commit the `2.3.3` change inside `src/CommanDuctUI` first.
- [ ] In the parent repository, verify `git submodule status src/CommanDuctUI` points at that reviewed commit, then record the gitlink and parent integration changes.
- [ ] Verify a clean checkout with `git submodule update --init --recursive` builds against the recorded commit rather than relying on an uncommitted submodule working tree.

## Acceptance criteria

- Hover transitions never expose a background-only or partially painted row under normal operation.
- One successful buffered presentation updates the dirty rectangle for each `WM_PAINT`.
- Hover painting is proportional to the dirty rows and does not remeasure all article badges.
- Identical density/population/selection inputs cause neither native invalidation nor Harvester list commands.
- Actual data, supported selection-to-`Some`, scroll, style, density, and size changes still render correctly; selection `Some -> None` retains the explicitly tested existing native-highlight contract until a clear-selection API is designed separately.
- CommanDuctUI ships the fix as `2.3.3` with changelog and diary updates.
- The parent repository records the exact reviewed CommanDuctUI commit through its gitlink only after the submodule review gate.
