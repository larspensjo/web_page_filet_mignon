# Triage Priority Marker Plan

Date: 2026-04-03

Reference:
- `docs/visual_design/VisualDesignSpec.md`
- `docs/EngineeringDiary.md`
- Existing TreeView marker support in `src/CommanDuctUI`

## Goal

Add triage-priority color markers to the left-pane `Triage Results` rows so priority is faster to scan without recreating the old “dead pixel” problem.

This plan keeps the existing textual priority cue (`P5`, `P4`, `P3`) and adds a small colored marker as a secondary signal.

Dots are chosen here instead of full badges because the current TreeView path already has generic marker support, while badge or pill rendering would require significantly heavier custom row rendering. The design goal is to add sparse semantic emphasis, not to turn each row into a custom-drawn composite widget.

## Why This Needs Its Own Plan

The design spec explicitly allows sparse semantic color where it materially improves scan speed, and triage priority is one of the clearest cases for that.

However, the repo already has a relevant warning from the earlier marker pass:

- tiny tree markers looked like rendering defects instead of deliberate UI
- marker placement was hard to get right
- job rows already carry multiple built-in TreeView visuals: expand/collapse glyphs, checkbox state icons, selection accent, and text

So this is not just a matter of returning a marker enum. The geometry needs to be made reliable first.

## Target UX

In `Triage Results` only:

- each job row keeps its textual prefix, for example `P5 Business: ...`
- a small colored dot appears before the row text as a secondary cue
- the dot color maps to triage priority
- the dot is visibly intentional, not tiny or ambiguous
- the dot does not collide with the expand/collapse button, checkbox, selection accent, or text

In all other left-pane tabs:

- no triage priority dots
- existing link download markers continue to work as they do now

## Design Rules

### 1. Marker is secondary, not primary

The dot should improve scan speed, not replace the row text. Users must still be able to understand priority from the row alone.

That means:

- keep `P5/P4/P3` in text
- do not tint the whole row
- do not add category color at the same time

### 2. Marker appears only where it helps

The dot should be restricted to `Triage Results`.

Reason:

- `Jobs` is already a mixed operational list
- `Triage Review` has its own inclusion/exclusion semantics
- adding priority markers outside `Triage Results` would increase noise

### 3. Use warm palette-aligned colors

The visual design spec is clear that semantic color should stay restrained and warm.

Recommended mapping:

- `P6-P7`: strongest warm alert tone, derived from the terracotta accent family
- `P5`: second-tier warm alert tone, still clearly high priority
- `P4`: softer amber/clay tone
- `P3`: muted warm neutral
- `P1-P2` or missing priority: no marker by default

Avoid:

- saturated blue for priority
- purple for priority
- destructive warning red unless the resulting tone is softened into the warm system

### 4. Geometry must be stable before rollout

The marker must be positioned from a real visual anchor, not by accumulating magic offsets blindly.

This is the highest-risk part of the work.

## Current Technical State

### Harvester

`harvester_app` currently exposes tree markers through `UiStateProvider::tree_item_marker()` in:

- `crates/harvester_app/src/platform/app.rs`

Current behavior:

- job rows return `TreeItemMarkerKind::None`
- link rows use markers for download state

### CommanDuctUI

Marker support already exists in:

- `src/CommanDuctUI/src/types.rs`
- `src/CommanDuctUI/src/controls/treeview_handler.rs`

Current marker system:

- generic marker enum: `None`, `Blue`, `Green`, `Yellow`, `Red`, `Purple`, `Gray`
- markers are custom-drawn circles in the TreeView paint path
- current geometry is driven by fixed constants such as marker diameter and left offset

### Relevant diary constraint

`docs/EngineeringDiary.md` already records that tiny job-tree status dots were removed because they were too small and looked like dead pixels.

This plan must explicitly avoid repeating that failure mode.

## Main Risks

### Risk 1: Marker placement is visually wrong

Symptoms:

- marker overlaps the checkbox
- marker is too far left and feels detached
- marker is too far right and collides with row text
- marker shifts unexpectedly across row states

Mitigation:

- do geometry work first
- isolate marker placement in a small helper if possible
- verify against selected and unselected rows
- verify against rows with and without expand/collapse glyphs

### Risk 2: Marker remains too small to read

Symptoms:

- looks like a dead pixel
- reads as rendering noise instead of UI signal

Mitigation:

- increase marker diameter modestly, likely to `8px` or `9px`
- keep a subtle border so the dot remains legible on dark surfaces

Note:

- the current implementation draws a `1px` border around the marker
- so a `6px` marker only yields a `4px` visible inner dot
- even a move to `8px` only gives a `6px` inner dot
- `9px` total diameter likely gives the safest improvement because it yields a `7px` colored interior

### Risk 3: Marker color competes with existing accent usage

Symptoms:

- priority dots fight the tab accent, selection accent, or primary button
- too many vivid colors appear in one row

Mitigation:

- keep colors warm and restrained
- only one marker per row
- do not color the rest of the row

## Implementation Plan

### Phase 1: Define the exact marker contract

Before touching code, lock the intended behavior:

- marker only in `LeftTab::TriageResults`
- marker only on `TreeItemKind::Job`
- marker only when `triage_annotation` exists
- mapping:
  - `priority >= 6` -> strongest warm marker
  - `priority == 5` -> high warm marker
  - `priority == 4` -> mid-strength warm marker
  - `priority == 3` -> muted neutral marker
  - `priority <= 2` or missing priority -> no marker

Deliverable:

- explicit mapping note in code comments or test names

### Phase 2: Rework generic marker geometry in CommanDuctUI

Files:

- `src/CommanDuctUI/src/controls/treeview_handler.rs`
- `src/CommanDuctUI/src/types.rs` if API changes are needed

Tasks:

1. Audit current marker placement logic.
   Determine whether `TVM_GETITEMRECT` in the current path gives a useful text-aligned anchor or whether the handler is relying on an imprecise row rect.

2. Replace ad hoc placement with a clearer geometry model.
   Preferred options:

   - compute marker position from the start of the text lane
   - or reserve a small marker lane between state icon and text

   Special care:

   - there is only one shared TreeView, so the marker lane must work across the actual Harvester row structure rather than a single idealized row
   - verify the marker lane against expand/collapse glyphs, checkbox icons, and the selection accent before finalizing constants
   - do not accept a placement fix that depends only on one screenshot state

3. Increase marker size slightly.
   Likely from `6px` to `8px` or `9px`.

4. Preserve vertical centering in the row.

5. Ensure the marker remains compatible with:
   - selection accent painting
   - checkbox state icon rendering
   - row text painting

Expected result:

- marker sits consistently in one intentional lane
- marker no longer feels randomly offset

### Phase 3: Warm the generic marker palette

Files:

- `src/CommanDuctUI/src/controls/treeview_handler.rs`
- CommanDuctUI version/changelog files

Tasks:

1. Revisit the hardcoded marker colors.
2. Tune `Red`, `Yellow`, and `Gray` toward the warm dark theme instead of generic Material-style defaults.
3. Keep the names generic; do not introduce Harvester-specific color semantics into CommanDuctUI.

Important side effect:

- `Red` and `Yellow` are already used by link download markers
- warming those colors is acceptable and likely desirable for consistency
- but it is a deliberate shared change, not an isolated triage-only change
- visual review and tests should include link markers as well

Recommended direction:

- `Red` becomes a warm terracotta-alert tone
- `Yellow` becomes a muted amber/clay
- `Gray` becomes a warm tertiary neutral

Expected result:

- priority dots feel native to the redesign instead of bolted on

### Phase 4: Wire priority markers in Harvester

File:

- `crates/harvester_app/src/platform/app.rs`

Tasks:

1. Update `UiStateProvider::tree_item_marker()`.
2. Detect whether the current row belongs to the `Triage Results` job tree context by reading `state.left_tab()`.
3. Read triage priority directly from reducer state without rebuilding the full view model during paint.
   Concrete lookup path:
   `job_id` -> `state.jobs.get(&job_id)` -> `job.url` -> `state.triage.result_for_url(&url)` -> `result.priority`
4. Return marker kind according to priority mapping.
5. Keep existing link-row markers unchanged.

Important constraint:

- no marker on `Jobs`
- no marker on `Triage Review`
- no marker on rows without completed triage

### Phase 5: Add regression tests

#### Harvester tests

File:

- `crates/harvester_app/src/platform/app.rs`

Tests to add or update:

- job row in `Triage Results` with `P7` returns the strongest marker
- job row in `Triage Results` with `P5` returns the high marker
- job row in `Triage Results` with `P4` returns the mid marker
- job row in `Triage Results` with `P3` returns the muted marker
- job row in `Triage Results` with `P2` returns `None`
- job row in `Jobs` still returns `None`
- job row in `Triage Review` still returns `None`
- link markers still return download-state colors as before

#### CommanDuctUI tests

File:

- `src/CommanDuctUI/src/controls/treeview_handler.rs`

Tests to add or update:

- marker color helper returns expected warm colors
- placement helper, if extracted, returns expected geometry for representative row rectangles
- post-paint request logic still behaves correctly when markers are present

### Phase 6: Visual review

Launch with:

- `cargo run -p harvester_app`

Review checklist:

- dot is clearly visible but not loud
- dot aligns correctly on every visible row
- dot does not overlap the checkbox or expand/collapse glyph
- selected rows still read clearly
- `P5/P4/P3` text remains readable
- `P6/P7` still feel meaningfully more urgent than `P5`
- priority scan speed improves in `Triage Results`
- `Jobs` tab remains clean and free of priority markers

### Phase 7: Documentation and boundary hygiene

Because this crosses the CommanDuctUI boundary:

1. Update `src/CommanDuctUI/Cargo.toml` version
2. Append a changelog entry to `src/CommanDuctUI/CHANGELOG.md`
3. Add a short diary entry to `docs/EngineeringDiary.md`

The diary entry should note:

- marker geometry was adjusted to avoid the prior tiny-dot failure mode
- priority dots are restricted to triage results
- colors were tuned to the warm dark system

## Recommended Order of Work

1. Inspect and refactor marker placement in CommanDuctUI
2. Adjust marker size and palette in CommanDuctUI
3. Wire priority mapping in Harvester
4. Add regression tests
5. Run:
   - `cargo build`
   - `cargo test`
   - `cargo clippy --all-targets -- -D warnings`
6. Launch with `cargo run -p harvester_app`
7. Review placement visually before finalizing

## Acceptance Criteria

This plan is complete when:

- `Triage Results` rows show a clear priority marker without losing text readability
- markers are absent from tabs where they do not help
- marker placement is stable and visually intentional
- the colors fit the warm design system
- the old “dead pixel” problem does not return
- CommanDuctUI remains generic infrastructure rather than acquiring Harvester-specific concepts
