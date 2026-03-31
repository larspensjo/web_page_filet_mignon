# Plan: Poll Stats Tab

Add a "Poll Stats" tab to the right pane that displays per-source statistics from the last completed poll cycle, using the same grouped format already produced by the batch runner.

## Scope

Eight files change across three layers: `harvester_core` (data + formatter module + view model), `harvester_batch` (call shared formatter), `harvester_app/platform/ui` (constants + layout + render). No changes to `CommanDuctUI`.

---

## Step 1 — Data layer: `crates/harvester_core/src/source_state.rs`

Add a second stats field to `SourceStateIndex`:

```rust
/// Snapshot of stats from the last *completed* poll cycle. Persists across poll starts.
last_completed_poll_stats: Vec<SourcePollStat>,
```

Change `end_poll()` to copy the current accumulator into the snapshot before clearing nothing:

```rust
pub fn end_poll(&mut self) {
    self.last_completed_poll_stats = self.poll_stats.clone();
    self.poll_in_progress = false;
}
```

`start_poll()` continues to clear only `poll_stats` (the live accumulator) — `last_completed_poll_stats` is untouched, so the previous poll's data remains visible while a new poll is in flight.

Add a public accessor:

```rust
pub fn last_completed_poll_stats(&self) -> &[SourcePollStat] {
    &self.last_completed_poll_stats
}
```

Also update `BatchObservation.source_poll_stats` in `state.rs` to use the completed snapshot instead of the live accumulator:

```rust
source_poll_stats: self.source_states.last_completed_poll_stats().to_vec(),
```

This removes the timing dependency (previously correct only because the batch runner happened to call `batch_observation()` between `end_poll()` and the next `start_poll()`). The field now always means "stats from the last finished poll", which is the correct semantics.

**Tests to add** in the existing `#[cfg(test)]` block:
- `last_completed_poll_stats_empty_before_any_poll` — accessor returns empty slice on a fresh index.
- `last_completed_poll_stats_set_after_end_poll` — start poll, record a stat, end poll; accessor returns that stat.
- `last_completed_poll_stats_preserved_during_next_poll` — after first poll ends, start a second poll (live accumulator clears); accessor still returns the first poll's stats.
- `last_completed_poll_stats_replaced_after_second_end_poll` — complete the second poll; accessor now returns the new stats.

---

## Step 2 — Shared formatter: new `crates/harvester_core/src/poll_stats_fmt.rs`

Create a new module as the single source of truth for poll-stats formatting. This eliminates the duplication that would otherwise exist between the UI tab and the batch runner.

```rust
pub fn format_poll_stats(stats: &[SourcePollStat]) -> String
```

Groups stats by `SourceKind` in canonical order (RSS → Brave → File → Curated → Script). Source kinds with no stats are omitted. Returns `"No poll data yet."` when `stats` is empty.

Per-source line format (matches today's batch output):
- Normal: `"  <id>: <parsed> parsed → <dedup_filtered> dedup-filtered → <emitted> emitted"`
- Zero-parsed: `"  <id>: 0 parsed"`

Group header format:
- `"<Label> (<n> source[s]): <total_emitted> emitted, <total_filtered> dedup-filtered"`

Expose from `harvester_core/src/lib.rs` as `pub use poll_stats_fmt::format_poll_stats;`.

**Update the batch runner** (`crates/harvester_batch/src/runner.rs`): replace the body of `print_poll_stats()` with a call to `harvester_core::format_poll_stats(stats)`, then print the result surrounded by the existing `--- Poll summary ---` / `--------------------` banner. The banner is batch-specific presentation and stays in `runner.rs`.

---

## Step 3 — View model: `crates/harvester_core/src/view_model.rs`

Add one field to `RightPaneView`:

```rust
/// Formatted text for the Poll Stats tab. None until the first poll completes.
pub poll_stats_markdown: Option<String>,
```

Update `RightPaneView::default()` — `poll_stats_markdown: None`.

---

## Step 4 — State → ViewModel: `crates/harvester_core/src/state.rs`

**4a. Populate `poll_stats_markdown`** in `build_right_pane_view`:

```rust
let poll_stats_markdown = {
    let stats = self.source_states.last_completed_poll_stats();
    if stats.is_empty() {
        None
    } else {
        Some(crate::poll_stats_fmt::format_poll_stats(stats))
    }
};

RightPaneView {
    active_tab: self.active_tab,
    triage_markdown: effective_triage_markdown,
    summary_markdown: effective_summary_markdown,
    briefing_markdown: effective_briefing_markdown,
    trends,
    poll_stats_markdown,
}
```

**4b. Header override** — extend the existing `preview_header_text` logic (currently at line ~683):

```rust
let preview_header_text = if self.active_tab() == AppTab::Briefing {
    Some(self.format_briefing_preview_header())
} else if self.active_tab() == AppTab::PollStats {
    Some("Poll Stats | last poll".to_string())
} else {
    None
};
```

This suppresses the selected-article metadata in the header when the Poll Stats tab is active, consistent with how Briefing handles it.

**Tests to add:**
- `poll_stats_header_override_when_tab_active` — with `AppTab::PollStats` active, `view.preview_header_text` equals `"Poll Stats | last poll"`.
- `poll_stats_header_not_overridden_on_other_tabs` — with any other tab active, `preview_header_text` is `None` (or the Briefing override value when Briefing is active).

---

## Step 5 — Tab enum: `crates/harvester_core/src/tabs.rs`

Add the new variant to `AppTab`:

```rust
pub enum AppTab {
    Triage,    // 0
    Summary,   // 1
    Briefing,  // 2
    Trends,    // 3
    PollStats, // 4
}
```

Update `to_index` and `from_index` to handle index 4. Update the round-trip test to include `AppTab::PollStats`. Update `from_index_out_of_range` to use index 5 instead of 4.

---

## Step 6 — Constants: `crates/harvester_app/src/platform/ui/constants.rs`

```rust
pub const PANEL_TAB_POLL_STATS: ControlId = ControlId::new(2214);
pub const VIEWER_POLL_STATS: ControlId = ControlId::new(5004);
```

(5001 = VIEWER_PREVIEW, 5002 = VIEWER_TRIAGE, 5003 = VIEWER_BRIEFING — 5004 is next.)

---

## Step 7 — Layout: `crates/harvester_app/src/platform/ui/layout.rs`

**7a. Tab bar label** — add `"Poll Stats"` to the `TAB_BAR_RIGHT` items list (5th entry, index 4).

**7b. Create controls** — after the Trends panel block, add:

```rust
commands.push(PlatformCommand::CreatePanel {
    window_id,
    parent_control_id: Some(PANEL_PREVIEW),
    control_id: PANEL_TAB_POLL_STATS,
});
commands.push(PlatformCommand::CreateRichEdit {
    window_id,
    parent_control_id: Some(PANEL_TAB_POLL_STATS),
    control_id: VIEWER_POLL_STATS,
});
```

**7c. Apply `ViewerReadable` style** — add `VIEWER_POLL_STATS` to the existing style loop at line ~2261:

```rust
for control_id in [VIEWER_PREVIEW, VIEWER_TRIAGE, VIEWER_BRIEFING, VIEWER_POLL_STATS] {
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id,
        style_id: StyleId::ViewerReadable,
    });
}
```

Without this the new tab renders with the default unstyled RichEdit appearance.

**7d. Layout rules** — in `build_layout_command`, add two new rules after the Trends rules:

```rust
LayoutRule {
    control_id: PANEL_TAB_POLL_STATS,
    parent_control_id: Some(PANEL_PREVIEW),
    dock_style: tab_dock(AppTab::PollStats),
    order: 6,
    fixed_size: tab_size(AppTab::PollStats),
    margin: (0, 0, 0, 0),
},
LayoutRule {
    control_id: VIEWER_POLL_STATS,
    parent_control_id: Some(PANEL_TAB_POLL_STATS),
    dock_style: DockStyle::Fill,
    order: 0,
    fixed_size: None,
    margin: (0, 0, 0, 0),
},
```

**7e. Tests** — update the layout tests that list all `PANEL_TAB_*` IDs (two arrays in layout.rs) to include `PANEL_TAB_POLL_STATS`. Update the "only active tab fills, others collapse" test to include `AppTab::PollStats` in the inactive set when testing other tabs as active. Add a test: every right-pane RichEdit viewer (`VIEWER_PREVIEW`, `VIEWER_TRIAGE`, `VIEWER_BRIEFING`, `VIEWER_POLL_STATS`) receives `ApplyStyleToControl` with `StyleId::ViewerReadable` in `initial_commands`.

---

## Step 8 — Render: `crates/harvester_app/src/platform/ui/render.rs`

**8a. Imports** — add `PANEL_TAB_POLL_STATS, VIEWER_POLL_STATS` to the constants import.

**8b. `TreeRenderState`** — add a prev-text field for dirty-check:

```rust
prev_poll_stats_text: Option<String>,
```

Initialize to `None` in `TreeRenderState::new()`.

**8c. Render the viewer** — after the briefing tab block, add:

```rust
// Poll Stats tab viewer.
let poll_stats_text = view
    .right_pane
    .poll_stats_markdown
    .as_deref()
    .unwrap_or("No poll data yet.");
if tree_state.prev_poll_stats_text.as_deref() != Some(poll_stats_text) {
    cmds.push(PlatformCommand::SetRichEditContent {
        window_id,
        control_id: VIEWER_POLL_STATS,
        rtf_text: convert_markdown_to_rtf(poll_stats_text),
    });
    tree_state.prev_poll_stats_text = Some(poll_stats_text.to_string());
}
```

No truncation — poll stats text is short and fixed-size by nature.

---

## Step 9 — Verify & lint

```
cargo build
cargo clippy --all-targets -- -D warnings
cargo nextest run
```

Fix any issues before considering the work complete.

---

## Files changed summary

| File | Change |
|------|--------|
| `crates/harvester_core/src/source_state.rs` | Add `last_completed_poll_stats` field + accessor; update `end_poll`; add tests |
| `crates/harvester_core/src/poll_stats_fmt.rs` | New module: `format_poll_stats` — single source of truth for poll-stats formatting |
| `crates/harvester_core/src/lib.rs` | Re-export `format_poll_stats` |
| `crates/harvester_core/src/view_model.rs` | Add `poll_stats_markdown` to `RightPaneView` |
| `crates/harvester_core/src/state.rs` | Update `BatchObservation` to use `last_completed_poll_stats`; populate `poll_stats_markdown` and header override in `build_right_pane_view` |
| `crates/harvester_core/src/tabs.rs` | Add `AppTab::PollStats`; update index mapping and tests |
| `crates/harvester_batch/src/runner.rs` | `print_poll_stats` delegates to `harvester_core::format_poll_stats` |
| `crates/harvester_app/src/platform/ui/constants.rs` | Add `PANEL_TAB_POLL_STATS`, `VIEWER_POLL_STATS` |
| `crates/harvester_app/src/platform/ui/layout.rs` | Add tab label; create controls; apply `ViewerReadable` style; add layout rules; update tests |
| `crates/harvester_app/src/platform/ui/render.rs` | Add `prev_poll_stats_text`; render Poll Stats viewer |
