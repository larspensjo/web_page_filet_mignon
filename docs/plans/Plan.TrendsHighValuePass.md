# Trends High-Value Pass Implementation Plan

**Goal:** Improve the readability and usefulness of the Trends tab with the smallest high-value implementation pass that still respects current architecture and keeps `CommanDuctUI` generic.

**Scope:** This plan covers the first substantive upgrade to the existing Trends chart:
- render x-axis week labels and y-axis values
- reduce visible series from 10 to 5 by default
- visually de-emphasize secondary lines
- replace the right legend with endpoint labels
- change ranking to favor current activity over cumulative history

**Non-goals for this pass:**
- configurable time windows
- weighted counting modes
- smoothing
- per-entity drill-down
- persistence of chart preferences
- annotations, export, or co-occurrence analysis

## Current State

The current Trends path is split cleanly:
- `harvester_core` computes trend buckets and top-N ranking in [`crates/harvester_core/src/trends.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/crates/harvester_core/src/trends.rs)
- `harvester_core` exposes `TrendsTabView` in [`crates/harvester_core/src/view_model.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/crates/harvester_core/src/view_model.rs)
- `harvester_app` converts that view model into `ChartDataPacket` in [`crates/harvester_app/src/platform/ui/render.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/crates/harvester_app/src/platform/ui/render.rs)
- `CommanDuctUI` renders a generic owner-drawn chart in [`src/CommanDuctUI/src/controls/chart_handler.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/src/CommanDuctUI/src/controls/chart_handler.rs)

Observed limitations in the current implementation:
- `ChartDataPacket.week_labels` exists but is not rendered.
- The chart has dashed horizontal guides but no y-axis values.
- A fixed 130 px legend column reduces plot width and forces color-to-label lookup.
- All lines compete equally for attention.
- Ranking is based on total count across the whole window, then latest-week count, which over-favors stale spikes.
- The reducer currently computes trends with `window_weeks=13` and `top_n=10`, and `build_chart_data` currently does `.take(10)`.
- Single-point series are currently dropped because `paint_chart` returns early when `n_points < 2`.
- The chart infrastructure assumes generic lines and labels, which is good and should be preserved.

## Architecture Constraints

From repo instructions and current code shape:
- Preserve unidirectional flow: input -> action -> reducer -> state -> render.
- Keep reducers pure and testable.
- Keep `CommanDuctUI` generic. No Harvester-specific names like “company”, “product”, “momentum”, or “briefing” should cross into the UI crate.
- If `CommanDuctUI` changes, update its version and changelog.

That implies:
- ranking policy belongs in `harvester_core`
- Harvester-specific descriptive text belongs in `harvester_app` or `harvester_core` view state
- chart drawing primitives and generic presentation metadata belong in `CommanDuctUI`

## Design Principles

1. Prefer explicit chart presentation metadata over hard-coded widget behavior.
2. Keep the first pass deterministic and static; no hover, no animation, no hidden interaction.
3. Solve readability by reducing cognitive load, not by adding more chrome.
4. Make the generic chart capable of supporting future ranking modes and time-window toggles without another structural rewrite.

## Proposed Shape

### 1. Add generic chart presentation metadata

Extend the generic chart types in `CommanDuctUI` instead of baking assumptions into paint code.

Recommended additions:
- `ChartLineEmphasis` or equivalent generic per-line presentation metadata
  - `Primary`
  - `Secondary`
- optional `end_label` string per line
- packet-level `show_end_labels`
- packet-level toggles for:
  - `show_x_axis_labels`
  - `show_y_axis_labels`
  - `show_gridlines`

Avoid adding Harvester terms or business rules to the generic packet.

Recommended generic shape:

```rust
pub enum ChartLineEmphasis {
    Primary,
    Secondary,
}

pub struct ChartLineData {
    pub label: String,
    pub weekly_counts: Vec<u32>,
    pub color: u32,
    pub end_label: Option<String>,
    pub emphasis: ChartLineEmphasis,
}

pub struct ChartDataPacket {
    pub lines: Vec<ChartLineData>,
    pub week_labels: Vec<String>,
    pub is_loading: bool,
    pub show_x_axis_labels: bool,
    pub show_y_axis_labels: bool,
    pub show_end_labels: bool,
}
```

### 2. Keep ranking and series selection in `harvester_core`

The current ranking is computed in `compute_category_trend`. Replace the current sort key with a deterministic “current relevance” score.

Recommended scoring for this pass:
- primary: latest week count
- secondary: sum of the last 3 weeks with descending recency weight
- tertiary: total count across the full window
- final tie-breaker: display label ascending

This is intentionally simpler and safer than a full “momentum mode”. It improves recency sensitivity without adding UI toggles or introducing unstable statistics.

Example deterministic score:
- `score = latest * 100 + prev1 * 40 + prev2 * 20 + total`

Use integer math only, but compute the score in `u64`, not `u32`, to avoid overflow risk on large archives.

Recommended pure helper:

```rust
fn compute_recency_score(counts: &[u32]) -> u64
```

### 3. Reduce default visible lines from 10 to 5

Make this a `harvester_app` rendering choice, not a core truncation change.

Current source state already computes `top_n=10` in the reducer via `state.set_entity_index(index, 13, 10)`. The clean first-pass change is:
- keep the reducer calls unchanged
- keep `harvester_core` computing the top 10
- change `build_chart_data` in `render.rs` from `.take(10)` to `.take(5)`

This preserves flexibility for future ranking modes and “show more” options while improving readability immediately.

### 4. Replace legend column with endpoint labels

Render the plot across the full available width and label only the last point of each visible line.

Rules:
- offset labels slightly right of the last point
- if labels collide vertically, resolve with a deterministic stack/spacing pass
- after vertical placement, clamp labels against the right edge
- if a label still cannot fit, prefer clipping or a small vertical shift over restoring the legend
- secondary lines should use muted text matching their muted line color

This remains generic because endpoint labels are a chart behavior, not a Harvester behavior.

Recommended helper shape:

```rust
fn place_end_labels(
    last_points: &[(i32, i32, String)],
    min_label_spacing: i32,
    right_edge: i32,
    label_width_fn: impl Fn(&str) -> i32,
) -> Vec<PlacedLabel>
```

When `show_end_labels` is enabled:
- legacy `legend_w` should become 0
- the right margin should reserve only the width needed for endpoint labels
- label width should be measured at paint time with `GetTextExtentPoint32W`

When `show_end_labels` is false:
- keep the current legend behavior unchanged for backward compatibility

### 5. De-emphasize secondary lines

For the 5 visible lines:
- top 2 or 3 lines: stronger color and 2 px stroke
- remaining lines: muted gray-tinted color and 1 px or 2 px thinner stroke if feasible

Do not rely on alpha blending; GDI support is awkward. Use explicit darker/lighter COLORREF values.

### 6. Render axis labels

Implement:
- x-axis week labels, likely only every Nth tick if space is tight
- y-axis numeric values at the same positions as horizontal guides

The chart should reserve layout margins based on actual text needs rather than a fixed legend width.

Concrete rendering notes:
- select `DEFAULT_GUI_FONT` before measuring or drawing axis text
- call `SetBkMode(TRANSPARENT)` before axis labels and endpoint labels
- derive left margin from measured y-axis label width rather than a hard-coded value
- use a deterministic stride helper for x-axis labels so narrow panes skip labels cleanly

## Proposed Slices

### Slice A: Generic chart layout and axes

**Goal:** make the current chart legible without changing ranking yet.

Changes:
- add left/bottom margins for axis labels
- render y-axis values aligned with horizontal guides
- render x-axis week labels using `week_labels`
- remove the fixed right legend column from layout calculations when endpoint labeling is enabled

Files likely affected:
- [`src/CommanDuctUI/src/types.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/src/CommanDuctUI/src/types.rs)
- [`src/CommanDuctUI/src/controls/chart_handler.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/src/CommanDuctUI/src/controls/chart_handler.rs)
- [`src/CommanDuctUI/CHANGELOG.md`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/src/CommanDuctUI/CHANGELOG.md)
- `src/CommanDuctUI/Cargo.toml`

Testing:
- unit tests for layout helpers that compute margins/tick positions
- manual visual verification for narrow and wide panes

Implementation details to pin down in this slice:
- `build_y_axis_ticks(max_val: u32, tick_count: usize) -> Vec<u32>` should return “nice” rounded values
- left margin should be computed from the widest formatted tick label
- `resolve_x_label_stride` should use a concrete width heuristic rather than ad hoc skipping

### Slice B: Endpoint labels and visual emphasis

**Goal:** remove legend lookup friction and make hierarchy visible.

Changes:
- add generic per-line presentation metadata
- add endpoint label rendering
- add deterministic overlap resolution for endpoint labels
- add muted styling for secondary lines
- render one-point series as dots instead of dropping them

Files likely affected:
- [`src/CommanDuctUI/src/types.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/src/CommanDuctUI/src/types.rs)
- [`src/CommanDuctUI/src/controls/chart_handler.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/src/CommanDuctUI/src/controls/chart_handler.rs)
- [`crates/harvester_app/src/platform/ui/render.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/crates/harvester_app/src/platform/ui/render.rs)

Testing:
- pure helper tests for label placement and collision resolution
- render-path tests asserting emitted `ChartDataPacket` line styles

### Slice C: Recency-weighted ranking

**Goal:** make the five visible series more relevant.

Changes:
- replace current total-first sorting in `compute_category_trend`
- keep tie-breaking deterministic
- update or extend core tests that currently lock in old top-N ordering

Files likely affected:
- [`crates/harvester_core/src/trends.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/crates/harvester_core/src/trends.rs)
- potentially [`crates/harvester_core/src/state.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/crates/harvester_core/src/state.rs) if the configured `top_n` changes

Testing:
- new unit tests covering:
  - recent mover outranks stale spike
  - identical latest week falls back to weighted recent history
  - ordering remains deterministic on ties

### Slice D: Harvester-specific framing text

**Goal:** explain the chart without violating the generic UI boundary.

Changes:
- add one short descriptive line above or near the chart, owned by app/view state, for example:
  - `Top 5 products by recent activity, last 13 weeks`
- keep it outside `CommanDuctUI`; it should be another label control in app layout if implemented

This slice is optional in the first coding pass. It is useful, but less critical than axes and ranking.

## Recommended Implementation Order

1. Slice A
2. Slice C
3. Slice B
4. Slice D

Why this order:
- axes immediately improve comprehension
- ranking improves content quality before polishing presentation
- endpoint labels work better once the plotted set is already better curated
- descriptive text is cheap and can be added last

## Data and API Recommendations

To keep the design flexible, prefer these API choices:

- Add generic chart metadata instead of special-casing the Trends tab in `chart_handler.rs`.
- Keep scoring helpers pure and local to `harvester_core::trends`.
- Keep color selection in `harvester_app::platform::ui::render`, where Harvester chooses which lines are emphasized.
- Do not let `CommanDuctUI` decide which lines are “important”; it should render what the packet requests.

Recommended pure helpers:
- `compute_recency_score(&[u32]) -> u64`
- `build_y_axis_ticks(max_val: u32, tick_count: usize) -> Vec<u32>`
- `resolve_x_label_stride(plot_width: i32, label_count: usize) -> usize`
- `place_end_labels(last_points: &[(i32, i32, String)], min_label_spacing: i32, right_edge: i32, label_width_fn: impl Fn(&str) -> i32) -> Vec<PlacedLabel>`

## Robustness Considerations

### Empty and degenerate states

Handle these explicitly:
- loading state
- no lines
- one-point series
- all-zero counts
- fewer weeks than label stride logic expects
- long labels such as `Amazon Bedrock`

The chart should never panic, divide by zero, or draw outside the client rect.

Specific expectations:
- one-point series should render a dot marker, not disappear
- last-value-zero endpoint labels should clamp inside the plot instead of falling below it
- all-zero charts may still show a flat baseline; that is acceptable if axis labels remain sane

### Layout pressure

Expect the right pane to become narrow.

Therefore:
- x-axis labels should support stride-skipping
- endpoint labels need a fallback when the last point sits near the right edge
- y-axis width should be driven by the largest rendered tick label
- endpoint labels should use deterministic vertical spacing before right-edge clamping

### Determinism

Any collision resolution or scoring logic must be stable:
- same data must produce the same line order
- same data must produce the same label positions
- tie-break on display label where needed

### Compatibility

If `ChartDataPacket` changes:
- keep loading and empty-state rendering behavior unchanged
- update all packet-construction tests in `harvester_app`
- update `CommanDuctUI` changelog and version

Versioning note:
- current `CommanDuctUI` version is `0.9.1`
- this plan should target `0.10.0` if `ChartDataPacket` / `ChartLineData` gain new required fields

## Testing Strategy

### `harvester_core`

Add or update unit tests in [`crates/harvester_core/src/trends.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/crates/harvester_core/src/trends.rs):
- recent activity outranks stale total volume
- identical recency scores fall back to label ordering
- short windows still rank deterministically when fewer than 3 weeks exist

Prefer synthetic datasets with explicit week placement.

### `harvester_app`

Add render tests in [`crates/harvester_app/src/platform/ui/render.rs`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/crates/harvester_app/src/platform/ui/render.rs):
- chart packet emits only 5 visible lines
- emphasized vs secondary line styling is assigned as expected
- week labels remain aligned with weekly counts
- empty/loading packets remain valid
- existing chart packet tests should be updated to account for `.take(5)` rather than `.take(10)`

### `CommanDuctUI`

Add pure-helper tests where possible:
- tick generation
- x-label stride selection
- endpoint label placement
- right-edge label clamping
- one-point series point placement

Manual verification should cover:
- narrow pane
- wide pane
- long labels
- high max value
- low max value
- all lines ending near the same y-position
- last point at zero
- single-week or single-point data

### Validation commands when implementation is complete

Per repo instructions:
- `cargo build`
- `cargo clippy --all-targets -- -D warnings`

If `CommanDuctUI` changes, verify changelog/version updates in the same change.

## Risks and Tradeoffs

### Risk: ranking change may hide historically important entities

Mitigation:
- use recency weighting, not latest-week-only ranking
- keep future room for an explicit `Absolute` mode

### Risk: endpoint labels may overlap badly

Mitigation:
- implement deterministic spacing helper
- only render labels for visible lines
- allow modest y-offset adjustment

### Risk: first pass overfits to Trends

Mitigation:
- keep all new chart fields generic
- avoid domain terms in shared types and comments

### Risk: reducing to 5 lines removes useful context

Mitigation:
- document this as a readability-first default
- keep future ability to add a “show more” or ranking-mode switch

## Future Extensions After This Pass

These remain good next steps, but should stay out of the initial change:
- configurable windows: 4w, 13w, 26w, 1y
- `Absolute` vs `Momentum` ranking modes
- weighted counts by triage priority
- alias canonicalization
- smoothing overlay
- CSV export
- “new entrant” markers
- annotations for spikes and dips

## Backlog Check Against `FutureIdeas.md`

I checked the current `TrendInsights` backlog items in [`docs/FutureIdeas.md`](/abs/path/c:/Users/larsp/src/web_page_filet_mignon/main/docs/FutureIdeas.md).

Conclusion:
- none of the `FI-UX-TrendInsights-*` items should be marked completed based on current source state
- some future items would become easier after this pass, but they are still not implemented today

Why:
- configurable windows are not present
- weighted counts are not present
- alias mapping is not present
- export is not present
- smoothing, annotations, new-entrant markers, and surprise ranking are not present

So this plan should be added without changing backlog completion status.

## Recommended Deliverable for the First Coding Pass

Ship one PR with:
- generic chart axis rendering
- recency-weighted ranking
- 5 visible lines by default via `build_chart_data().take(5)` while core still computes 10
- endpoint labels
- muted secondary line styling
- tests in `harvester_core`, `harvester_app`, and `CommanDuctUI`
- `CommanDuctUI` `0.10.0` changelog/version update if shared types or painting change
- short diary entry after implementation lands

This is the smallest pass that materially improves both chart legibility and chart relevance without locking the architecture into a dead-end.
