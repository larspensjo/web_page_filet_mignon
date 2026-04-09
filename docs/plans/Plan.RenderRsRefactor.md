# Plan: Reduce render.rs Size

Reduce the size and responsibility footprint of `crates/harvester_app/src/platform/ui/render.rs` without changing the current render behavior, the existing unidirectional data flow, or the `CommanDuctUI` boundary.

## Scope

This plan covers only the `harvester_app/platform/ui` rendering layer. The goal is to make the render code easier to navigate, test, and evolve by splitting cohesive responsibilities into sibling modules under `crates/harvester_app/src/platform/ui/` while keeping `mod.rs` thin.

The live constraints are:
- Preserve the current architecture: input -> action -> reducer -> state -> render.
- Keep reducers pure and side effects isolated.
- Keep `crates/harvester_app/src/platform/ui/mod.rs` as a thin wrapper only.
- Do not move Harvester-specific behavior into `CommanDuctUI`.
- Preserve current UI behavior, especially render idempotency and the existing list-box based jobs pane.
- Do not sell this as a compile-time optimization; the payoff is maintainability, reviewability, and lower merge-friction rather than materially faster crate builds.

This plan does not change feature behavior. It is a structural refactor only.

---

## Current Analysis

`crates/harvester_app/src/platform/ui/render.rs` is currently 4,783 lines.

The file contains five distinct responsibility clusters:

1. Render orchestration and state caching
   - `TreeRenderState`
   - `render()`
   - `render_layout_only()`
   - `emit_if_changed()`

2. Control-section rendering
   - layout, status bar, progress bars, buttons, toggle state, tab selection

3. Prompt Lab rendering
   - one large section function plus Prompt Lab-specific status/metadata formatting

4. Preview and jobs-pane rendering helpers
   - preview RichEdit updates
   - trends chart data
   - list-box item construction and row/badge formatting

5. Tests and legacy code
   - a large `#[cfg(test)]` module
   - an older tree-view rendering path and legacy text-shaping helpers marked `dead_code`

The most important production hotspots are:
- `render_prompt_lab_section()` at roughly 500 lines
- `render_preview_section()` at roughly 225 lines
- the list-box row construction and formatting block
- the very large `TreeRenderState` definition, where Prompt Lab cache fields account for a large share of the struct

The most important structural observation is that the top-level `render()` function is already small and coherent. The file is large because too many sibling concerns live beside it.

---

## Refactor Goal

Turn `render.rs` into a coordinator module that owns:
- the shared render-state cache
- the shared `emit_if_changed()` helper
- top-level orchestration order
- any ordering-sensitive state transitions that must remain centralized

Move feature-local rendering and pure formatting logic into sibling modules so that each file has one clear reason to change.

Target end state:
- `render.rs` becomes an orchestration-focused module rather than a kitchen-sink implementation file.
- Prompt Lab, preview rendering, and list-box row formatting each live in their own focused module.
- legacy tree-view code is either deleted or explicitly isolated as legacy.
- tests no longer dominate the production file.

---

## Proposed Target Structure

Keep `crates/harvester_app/src/platform/ui/mod.rs` thin and add sibling files such as:

- `render.rs`
  - shared render state
  - `emit_if_changed()`
  - `render()`
  - `render_layout_only()`
  - module wiring and sequencing only

- `render_controls.rs`
  - status, progress, button, toggle, and tab-bar rendering

- `render_prompt_lab.rs`
  - Prompt Lab rendering and Prompt Lab-specific status/metadata text

- `render_preview.rs`
  - preview RichEdit rendering
  - trends chart-data building

- `render_list_box.rs`
  - list-box item building and emission
  - badge and row-label policy
  - list sorting/filtering logic

- `render_text.rs`
  - compact token/byte/url/title helpers and related pure string-formatting functions

- `render_tests.rs`
  - moved integration-heavy tests from `render.rs`

Optional temporary module during migration:
- `render_legacy.rs`
  - old tree-view path if the team wants a short-lived quarantine before deletion

Visibility policy:
- Prefer `pub(super)` for helpers that are only used by sibling render modules.
- Preserve `pub(crate)` only where there is an actual external caller outside the render split.

Test placement policy:
- Move tests into `render_tests.rs` first as a low-risk consolidation step.
- After the production split stabilizes, selectively move obviously local pure-helper tests into per-module `#[cfg(test)]` blocks only if that improves clarity.
- Keep cross-section idempotency and orchestration tests centralized.

---

## Step 1 — Move Tests Out First

Create `crates/harvester_app/src/platform/ui/render_tests.rs` and move the `#[cfg(test)]` block out of `render.rs`.

Why this goes first:
- It is the largest immediate file-size reduction.
- It does not change runtime behavior.
- It makes the production refactor easier to review.

Expected outcome:
- `render.rs` drops by roughly 2,200 lines immediately.
- Production code becomes much easier to scan before any logic moves.

Implementation notes:
- Keep tests near the render module rather than scattering them across feature files immediately.
- Use a test-only module declaration from `render.rs`.
- Preserve existing helper constructors and test organization during the first move. Do not rewrite tests yet.

---

## Step 2 — Remove or Isolate Legacy Tree-View Code

Evaluate the block beginning with `append_tree_commands()` and the older tree-item helpers.

Candidates:
- `append_tree_commands()`
- `JobRowPresentation`
- `job_row_presentation()`
- `job_row_check_policy()`
- `job_row_style_policy()`
- `build_job_tree()`
- `build_link_children()`
- `format_job_row_legacy()`
- `format_job_row_triage_review()`
- `format_job_row_triage_results()`
- any helper only retained to support the legacy tree path

Current evidence suggests that the live path uses `PopulateListBox`, not `PopulateTreeView`, and that the tree path is retained mainly for old tests and historical context.

Recommended approach:
- If the team is comfortable removing it, delete it after confirming no non-test references remain.
- If the team wants a lower-risk transition, move it to `render_legacy.rs` first, then delete it in a follow-up.

Expected outcome:
- another substantial reduction in `render.rs`
- a cleaner separation between live rendering and historical implementation remnants

If this step uses `render_legacy.rs` as a temporary landing zone, move the tree-row formatters with the tree path so live list-box code is not left depending on legacy-only helpers.

---

## Step 3 — Extract Pure Text and Label Formatting

Create `crates/harvester_app/src/platform/ui/render_text.rs` for pure formatting helpers that have no platform side effects.

Good initial candidates:
- `format_compact_tokens()`
- `format_compact_bytes()`
- `compact_url_label()`
- `url_slug_label()`
- `title_case_label()`
- `humanize_slug_with_limit()`
- `domain_from_url()`
- `truncate_with_ellipsis()`
- `compact_triage_tag_count()`

Special case:
- the dead viewer-shaping helpers (`normalize_windows_newlines()`, `shape_for_viewer()`, `add_spacing_before_headings()`, `normalize_bullets()`, `strip_bold_markers()`, `cap_blank_line_runs()`, `truncate_for_viewer()`) should not be extracted into a new live module unless they regain a live caller; treat them as cleanup candidates in Step 9 or move them only with tests if temporarily needed.

Why this is a good early extraction:
- These functions are pure and stable.
- They are easy to unit test.
- They do not depend on `WindowId`, `PlatformCommand`, or mutable render state.

Expected outcome:
- less noise in `render.rs`
- easier reuse from list-item and preview modules
- smaller compile-time cognitive surface for future rendering work

---

## Step 4 — Extract List-Box Rendering

Create `crates/harvester_app/src/platform/ui/render_list_box.rs` and move the jobs-pane list construction and emission there.

Candidates:
- `append_list_box_commands()`
- `compute_list_box_badge_column_width()`
- `build_list_box_items()`
- `build_list_box_item()`
- badge-style helpers
- row label helpers used by list construction, especially:
  - `job_display_label()`
  - `job_primary_label()`
  - `job_primary_label_with_limit()`
  - `job_source_label()`
  - `triage_result_primary_label()`
  - `filter_status_label()`
  - `job_status_label()`
  - `stage_label()`

Important logic to preserve exactly:
- `SinceCheckpoint` scope filtering
- `TriageResults` priority sorting only when triage is not in flight
- badge composition differences across left tabs
- the disabled-state rule for excluded Triage Review rows

Expected outcome:
- a single build-and-emit boundary for the left-pane list-box
- a clear separation between “what rows should exist” and the rest of the render orchestration
- easier testing of sorting, badges, and metadata formatting without reading the whole render file

---

## Step 5 — Extract Generic Control Rendering

Create `crates/harvester_app/src/platform/ui/render_controls.rs` for the smaller, cohesive control sections.

Candidates:
- `render_tab_bar_section()`
- `render_left_tab_bar_section()`
- `render_status_section()`
- `render_operation_progress_section()`
- `render_token_progress_section()`
- `render_main_controls_section()`
- `format_left_pane_header_meta()`
- `format_llm_usage_status()`

These functions are good extraction targets because they:
- are feature-light compared to Prompt Lab
- follow the same `emit_if_changed()` pattern
- are structurally independent of preview and jobs-pane formatting concerns

Implementation note:
- Keep `emit_if_changed()` in `render.rs` and call it from submodules, rather than cloning the pattern in multiple files.

---

## Step 6 — Split Render State Into Focused Sub-Structs

Before moving Prompt Lab, reshape `TreeRenderState` so it is grouped by concern.

Current problem:
- `TreeRenderState` is very large.
- Prompt Lab cache fields dominate both the struct definition and its default initialization.
- The state shape currently mirrors file sprawl instead of domain boundaries.

Recommended target shape:

```rust
pub struct TreeRenderState {
    layout: LayoutRenderState,
    controls: ControlsRenderState,
    prompt_lab: PromptLabRenderState,
    preview: PreviewRenderState,
    legacy_tree: LegacyTreeRenderState,
}
```

The exact names can change, but the principle should hold: state belongs with the feature whose idempotency it supports.

Implementation detail:
- this is the mechanically noisy step, because many call sites will change from `&mut tree_state.prev_foo` to `&mut tree_state.controls.prev_foo`, `&mut tree_state.preview.prev_bar`, and so on.
- prefer direct `pub(super)` field access inside sibling render modules over accessor methods unless a grouping genuinely needs invariants; this keeps the `emit_if_changed()` call pattern simple.

Recommended first-pass grouping:
- `LayoutRenderState`
  - layout invalidation cache and visibility-transition fields
  - `prev_left_panel_width`
  - `prev_input_panel_visible`
  - `prev_operation_progress_visible`
  - `prev_active_tab`
  - `prev_left_tab`
  - Prompt Lab layout-open/closed flags that currently participate in layout invalidation
- `ControlsRenderState`
  - status/progress/button/toggle previous values
- `PromptLabRenderState`
  - Prompt Lab control previous values and model-selector cache
- `PreviewRenderState`
  - preview, briefing, triage, poll-stats, and preview-header text cache
- `LegacyTreeRenderState`
  - old tree structure snapshots, only if the legacy tree path survives Step 2

Boundary note:
- `layout_view_from_app_view()` should be reviewed during this step because it straddles layout and Prompt Lab visibility concerns. It can remain in `render.rs` initially, but by the end of the split it should live with whichever module owns the layout boundary.

Why this step matters:
- Without it, moving Prompt Lab only relocates logic while leaving a giant central cache struct behind.
- With it, module APIs become cleaner and safer.

Important constraint:
- Do not introduce independent render-owned state machines.
- These sub-structs remain cache snapshots only; the app state still comes from `AppViewModel`.

---

## Step 7 — Extract Prompt Lab Rendering

Create `crates/harvester_app/src/platform/ui/render_prompt_lab.rs` and move the Prompt Lab rendering logic there after the state split.

Candidates:
- `render_prompt_lab_section()`
- `prompt_lab_status_text()`
- `prompt_lab_metadata_text()`
- `model_to_combo_index()`
- `combo_index_to_model()`

Why this should happen after the state split:
- Prompt Lab has the biggest concentration of cached render fields.
- The layout/render interaction around Prompt Lab visibility is ordering-sensitive.
- A clean extraction is easier once Prompt Lab state is explicit.

Important ordering rule to preserve:
- When the Prompt Lab tab becomes visible, model catalog selection cache is reset. That transition currently happens in layout rendering and must remain correct.

Visibility note:
- `model_to_combo_index()` and `combo_index_to_model()` are currently `pub(crate)`. If callers outside the render split still need them, either keep them in `render.rs` temporarily or move them to a small Prompt Lab support module while preserving `pub(crate)` visibility.

Expected outcome:
- the biggest live production hotspot leaves `render.rs`
- future Prompt Lab work stops inflating the general render coordinator

---

## Step 8 — Extract Preview and Trends Rendering

Create `crates/harvester_app/src/platform/ui/render_preview.rs`.

Candidates:
- `render_preview_section()`
- `build_chart_data()`
- `strip_leading_h1()`
- `truncate_markdown_for_preview()`

Note:
- `build_chart_data()` currently lives near the top of `render.rs`, well before the preview block, but it belongs with preview/trends rendering once extracted.

Optional:
- if `format_preview_context()` remains useful only for tests or legacy code, either move it here as a test helper or delete it if no live path needs it.

Expected outcome:
- preview rendering becomes isolated from jobs-pane and Prompt Lab concerns
- markdown-to-RTF orchestration lives in one place

---

## Step 9 — Final Cleanup of Dead Helpers

After the main moves are complete, remove any helpers that are still marked `#[allow(dead_code)]` and remain unused.

This likely includes:
- dead viewer-shaping helpers such as `normalize_windows_newlines()`, `shape_for_viewer()`, `add_spacing_before_headings()`, `normalize_bullets()`, `strip_bold_markers()`, `cap_blank_line_runs()`, and `truncate_for_viewer()`
- legacy viewer-shaping helpers no longer used by the RichEdit preview path
- obsolete constants kept only for dead code
- test-only helpers that can move into test modules

The point of this step is to avoid preserving accidental compatibility for code paths the app no longer uses.

---

## Risks and Mitigations

### Risk 1 — Breaking render idempotency

The most important behavioral property in the module is that repeated renders emit only the commands needed for changed UI state.

Mitigation:
- keep `emit_if_changed()` centralized
- preserve previous-value caches during extraction
- keep integration tests that render twice and assert no duplicate command emission

### Risk 2 — Breaking Prompt Lab cache reset behavior

Prompt Lab contains ordering-sensitive cache resets when the tab becomes visible.

Mitigation:
- keep visibility transition handling centralized until Prompt Lab state has been explicitly factored
- add focused tests around reopen/reselect behavior before and after extraction

### Risk 3 — Regressing Triage Results ordering

The current list-box logic intentionally avoids live re-sorting while triage is in flight to prevent flicker and structural rebuild churn.

Mitigation:
- keep this rule explicit in the extracted list-box module
- preserve tests around triage sorting and in-flight behavior

### Risk 4 — Moving code without improving structure

If functions are moved without splitting render state, the refactor will produce more files but not less complexity.

Mitigation:
- treat state factoring as a required middle step, not optional cleanup

### Risk 5 — Turning `mod.rs` into a second coordinator

The repo instructions explicitly require thin `mod.rs` files.

Mitigation:
- add sibling modules from `ui/mod.rs`
- keep orchestration in `render.rs`, not in `ui/mod.rs`

---

## Validation Strategy

Each extraction step should be validated before moving on.

Validation for every stage:
- existing render tests pass
- no new warnings from moved code
- repeated-render idempotency tests still pass
- Prompt Lab render behavior is unchanged where applicable

Expectation setting:
- treat successful validation as behavioral preservation and structural cleanup, not as evidence of faster compilation; compile-time wins are not a goal of this refactor.

Repository workflow checks once implementation work is done:
- `cargo build --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all`

Documentation follow-up once the refactor lands:
- add a short entry to `docs/EngineeringDiary.md` describing the module split and any reusable lesson about render-state factoring

---

## Recommended Execution Order

The safest practical order is:

1. Move tests into `render_tests.rs`
2. Remove or quarantine legacy tree-view code
3. Extract pure text helpers into `render_text.rs`
4. Extract list-box rendering into `render_list_box.rs`
5. Extract generic control rendering into `render_controls.rs`
6. Factor `TreeRenderState` into feature-local sub-structs
7. Extract Prompt Lab into `render_prompt_lab.rs`
8. Extract preview/trends into `render_preview.rs`
9. Remove remaining dead helpers and polish naming

This sequence keeps risk low and makes each review understandable.

---

## Expected Result

After the full refactor:
- `render.rs` should be primarily orchestration and shared render-cache wiring
- feature-specific rendering should live in focused sibling modules
- Prompt Lab no longer dominates the general render file
- tests no longer obscure production code
- future UI changes should have a clearer home and lower merge conflict risk

The first two steps alone should produce a noticeably smaller and easier-to-read `render.rs` before any high-risk movement begins.
