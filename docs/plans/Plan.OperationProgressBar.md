# Plan: General-Purpose Operation Progress Bar

## Goal

Add a compact progress bar to the footer that shows progress for the currently active operation (polling, triage, or summarizing). Auto-switch to the Poll Stats tab when polling completes.

## Problem

After clicking "Poll resources", the only feedback is the button greying out. There is no indication of what is happening, how far along it is, or when it finishes. The user must manually discover results on the Poll Stats tab.

## Design Decisions

- **Single shared progress bar** — only one operation runs at a time (poll → triage → summary).
- **Compact inline bar** (~80px) in the footer row — footer height unchanged at 32px.
- **Text label** next to the bar: "Polling: 3/7", "Triaging: 5/12", "Summarizing: 2/8".
- **Collapsed when idle** — controls shrink to zero width via layout collapse (not just cleared text), since CommanDuctUI does not expose a generic control-visibility toggle.
- **Auto-switch** to Poll Stats tab when polling completes.
- **New message** `PollStarted { total }` from effect runner, since source count is only known after loading sources.toml.

---

## Implementation Steps

### Step 1: Add `OperationProgress` to the view model

**Files:** `crates/harvester_core/src/view_model.rs`

Add a struct and field to `AppViewModel`:

```rust
/// Progress for the single active operation shown in the footer bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationProgress {
    pub label: String,      // "Polling", "Triaging", "Summarizing"
    pub completed: u32,
    pub total: u32,
}
```

Add to `AppViewModel`:
```rust
pub operation_progress: Option<OperationProgress>,
```

Default to `None`.

### Step 2: Add poll progress tracking to `SourceStateIndex`

**Files:** `crates/harvester_core/src/source_state.rs`

Add fields:
```rust
poll_total: Option<usize>,
poll_completed: usize,
```

Add/modify methods:
- `start_poll()` — also clear `poll_total = None` and `poll_completed = 0`.
- `set_poll_total(total: usize)` — sets `poll_total`.
- `record_poll_stat()` — also increment `poll_completed += 1`.
- `record_source_error()` — also increment `poll_completed += 1`, but **only when `poll_in_progress` is true** (this method can also be called outside of polling).
- `end_poll()` — also clear `poll_total = None`.
- `poll_progress() -> Option<(usize, usize)>` — returns `Some((poll_completed.min(total), total))` when `poll_total` is `Some(total)`, `total > 0`, and `poll_in_progress` is true. The `total > 0` guard avoids a transient `0/0` UI state when no sources are enabled.

Add tests:
- `poll_progress_none_when_idle` — returns `None` by default.
- `poll_progress_available_after_set_total` — returns `Some((0, N))` after `set_poll_total`.
- `poll_progress_increments_on_stat` — increments on `record_poll_stat`.
- `poll_progress_increments_on_error` — increments on `record_source_error` during poll.
- `poll_progress_not_incremented_on_error_outside_poll` — `record_source_error` when not polling does not increment.
- `poll_progress_none_when_total_is_zero` — returns `None` when `set_poll_total(0)`.
- `poll_progress_none_after_end_poll` — cleared after `end_poll`.

### Step 3: Add `Msg::PollStarted` and wire the reducer

**Files:**
- `crates/harvester_core/src/msg.rs`
- `crates/harvester_core/src/update.rs`

Add variant to `Msg`:
```rust
/// Effect runner reports the total number of enabled sources to poll.
PollStarted {
    total: usize,
},
```

Add reducer handler in `update.rs` (near `PollSourcesClicked`):
```rust
Msg::PollStarted { total } => {
    state.source_states.set_poll_total(total);
    Vec::new()
}
```

Also update `SourcePollCompleted` and `SourcePollFailed` handlers — `poll_completed` increment happens inside `record_poll_stat()` and `record_source_error()` (Step 2), so no reducer changes needed for those.

Add to `AllSourcesPollEnded` handler:
```rust
Msg::AllSourcesPollEnded => {
    state.end_poll();
    state.pre_triage_coordinator.note_poll_sources_ended();
    state.select_tab(AppTab::PollStats);  // <-- NEW: auto-switch
    Vec::new()
}
```

Add tests:
- `poll_started_sets_total` — `PollStarted { total: 5 }` makes `poll_progress()` return `Some((0, 5))`.
- `poll_complete_increments_progress` — after `PollStarted` + `SourcePollCompleted`, progress is `(1, N)`.
- `poll_failed_increments_progress` — after `PollStarted` + `SourcePollFailed`, progress is `(1, N)`.
- `poll_ended_auto_switches_to_poll_stats_tab` — after `AllSourcesPollEnded`, active tab is `PollStats`.

### Step 4: Send `PollStarted` from the effect runner

**Files:** `crates/harvester_io/src/effect_runner.rs`

In `execute_poll_all_sources()`, after loading the registry and before the loop over enabled sources, count enabled sources and send:

```rust
let registry = load_sources(&sources_path);
// ... existing config_dir, allowed_dirs setup ...

let enabled: Vec<_> = registry.sources.into_iter().filter(|s| s.enabled).collect();
let _ = msg_tx.send(Msg::PollStarted { total: enabled.len() });

for config in enabled {
    // ... existing per-source poll logic ...
}
```

This replaces the current `registry.sources.into_iter().filter(|s| s.enabled)` iteration with a collected vec so we can count first.

Also update `summarize_batch_msg` in `crates/harvester_batch/src/runner.rs` to handle the new variant:
```rust
Msg::PollStarted { total } => format!("PollStarted(total={total})"),
```

### Step 5: Compute `operation_progress` in the view model build

**Files:** `crates/harvester_core/src/state.rs`

In the `view_model()` method (around line 720), compute:

```rust
let operation_progress = if let Some((completed, total)) = self.source_states.poll_progress() {
    Some(OperationProgress {
        label: "Polling".into(),
        completed: completed as u32,
        total: total as u32,
    })
} else if matches!(self.triage.phase(), TriagePhase::Triaging) {
    let completed = self.triage.completed_count() + self.triage.failed_count();
    Some(OperationProgress {
        label: "Triaging".into(),
        completed: completed as u32,
        total: self.triage.total() as u32,
    })
} else if matches!(self.briefing.phase(), BriefingPhase::Summarizing) {
    let completed = self.briefing.completed_summary_count() + self.briefing.failed_summary_count();
    Some(OperationProgress {
        label: "Summarizing".into(),
        completed: completed as u32,
        total: self.briefing.total() as u32,
    })
} else {
    None
};
```

Wire into `AppViewModel { operation_progress, ... }`.

Note: Use `matches!(self.triage.phase(), TriagePhase::Triaging)` — there is no `is_in_progress()` method on `TriageSession`. The `LoadingArticles` phase doesn't have a meaningful completed/total. This mirrors what `progress_text()` does — it returns counts only during `TriagePhase::Triaging`.

### Step 6: Add layout controls for the operation progress bar

**Files:**
- `crates/harvester_app/src/platform/ui/constants.rs`
- `crates/harvester_app/src/platform/ui/layout.rs`
- `crates/harvester_core/src/view_model.rs`

Add constants:
```rust
pub const LABEL_OPERATION_PROGRESS: ControlId = ControlId::new(NEXT_FREE_ID);
pub const PROGRESS_OPERATION: ControlId = ControlId::new(NEXT_FREE_ID + 1);
```

In layout creation (where `LABEL_STATUS` is created in `PANEL_BOTTOM`), add:
```rust
commands.push(PlatformCommand::CreateLabel {
    window_id,
    parent_control_id: Some(PANEL_BOTTOM),
    control_id: LABEL_OPERATION_PROGRESS,
    initial_text: String::new(),
    class: LabelClass::Default,
});

commands.push(PlatformCommand::CreateProgressBar {
    window_id,
    parent_control_id: Some(PANEL_BOTTOM),
    control_id: PROGRESS_OPERATION,
});
```

Add layout rules — dock right within `PANEL_BOTTOM`, before `LABEL_STATUS` (which fills remaining space):

```rust
LayoutRule {
    control_id: PROGRESS_OPERATION,
    parent_control_id: Some(PANEL_BOTTOM),
    dock_style: DockStyle::Right,
    order: 10,
    fixed_size: Some(80),  // ~2cm when visible; 0 when idle
    margin: (6, 6, 6, 0),
},
LayoutRule {
    control_id: LABEL_OPERATION_PROGRESS,
    parent_control_id: Some(PANEL_BOTTOM),
    dock_style: DockStyle::Right,
    order: 20,
    fixed_size: Some(120),  // enough for "Summarizing: 12/24"; 0 when idle
    margin: (6, 6, 6, 6),
},
```

Note: Right-docked controls are processed in order, so `PROGRESS_OPERATION` (order 10) docks first (rightmost), then `LABEL_OPERATION_PROGRESS` (order 20) next to it. `LABEL_STATUS` with `DockStyle::Fill` takes remaining space on the left. Adjust order values and sizes as needed during implementation — test visually.

Apply dark theme styling to the new controls (in `apply_dark_theme`).

#### Idle collapse via layout

CommanDuctUI does not expose a generic control-visibility toggle. Use layout collapse instead: when `operation_progress` is `None`, set `fixed_size: Some(0)` for both controls so they consume no space.

Add a view-model flag to drive this:
```rust
pub operation_progress_visible: bool,  // true when operation_progress.is_some()
```

Thread this through `LayoutConfig` (or equivalent layout-view struct) and into the layout rule builder, so `build_layout_command` produces the correct `fixed_size` based on this flag. This follows the same pattern used by `input_panel_visible`.

Add layout tests:
- `operation_controls_have_width_when_visible` — non-zero fixed_size when flag is true.
- `operation_controls_collapse_when_hidden` — `fixed_size: Some(0)` when flag is false.

### Step 7: Render the operation progress bar

**Files:** `crates/harvester_app/src/platform/ui/render.rs`

Add to `TreeRenderState`:
```rust
prev_operation_progress_text: Option<String>,
prev_operation_progress_range: Option<(u32, u32)>,
prev_operation_progress_pos: Option<u32>,
```

Add `render_operation_progress_section()` (modeled on `render_token_progress_section()`):

```rust
fn render_operation_progress_section(
    window_id: WindowId,
    view: &AppViewModel,
    tree_state: &mut TreeRenderState,
    cmds: &mut Vec<PlatformCommand>,
) {
    let (text, range, pos) = match &view.operation_progress {
        Some(op) => (
            format!("{}: {}/{}", op.label, op.completed, op.total),
            (0u32, op.total),
            op.completed,
        ),
        None => (String::new(), (0u32, 0u32), 0u32),
    };

    emit_if_changed(&mut tree_state.prev_operation_progress_text, text, cmds, |text| {
        PlatformCommand::SetControlText {
            window_id,
            control_id: LABEL_OPERATION_PROGRESS,
            text,
        }
    });
    emit_if_changed(&mut tree_state.prev_operation_progress_range, range, cmds, |(min, max)| {
        PlatformCommand::SetProgressBarRange {
            window_id,
            control_id: PROGRESS_OPERATION,
            min,
            max,
        }
    });
    emit_if_changed(&mut tree_state.prev_operation_progress_pos, pos, cmds, |position| {
        PlatformCommand::SetProgressBarPosition {
            window_id,
            control_id: PROGRESS_OPERATION,
            position,
        }
    });
}
```

Call it from the main render function, after `render_status_section`.

### Step 8: Keep existing text progress in status line

**Decision:** Keep the existing `briefing_progress` / `triage_progress` text in the status line. They cover phases that don't have counts (loading, generating briefing). The progress bar coexists — it shows during the counting phases, the text covers the other phases. This avoids removing existing behavior.

### Step 9: View-model tests

**Files:** `crates/harvester_core/src/state.rs` (or appropriate test module)

Add tests verifying `operation_progress` derivation:
- `operation_progress_from_poll` — poll progress maps to `OperationProgress { label: "Polling", .. }`.
- `operation_progress_from_triage` — triage in `TriagePhase::Triaging` maps to `OperationProgress { label: "Triaging", .. }`.
- `operation_progress_from_briefing` — briefing in `BriefingPhase::Summarizing` maps to `OperationProgress { label: "Summarizing", .. }`.
- `operation_progress_poll_takes_precedence` — poll progress present overrides triage/briefing.
- `operation_progress_none_when_idle` — no active operation returns `None`.
- `operation_progress_none_during_triage_loading` — `TriagePhase::LoadingArticles` does not produce progress (no meaningful counts yet).

### Step 10: Clippy + build verification

Run `cargo clippy --all-targets -- -D warnings` and fix any issues.

---

## Files Changed Summary

| File | Change |
|------|--------|
| `crates/harvester_core/src/view_model.rs` | Add `OperationProgress` struct + field |
| `crates/harvester_core/src/source_state.rs` | Add `poll_total`, `poll_completed`, new methods + tests |
| `crates/harvester_core/src/msg.rs` | Add `Msg::PollStarted { total }` |
| `crates/harvester_core/src/update.rs` | Handle `PollStarted`, auto-switch tab in `AllSourcesPollEnded` + tests |
| `crates/harvester_core/src/state.rs` | Compute `operation_progress` in `view_model()` |
| `crates/harvester_io/src/effect_runner.rs` | Send `PollStarted` in `execute_poll_all_sources()` |
| `crates/harvester_batch/src/runner.rs` | Handle new `Msg` variant in `summarize_batch_msg` |
| `crates/harvester_app/src/platform/ui/constants.rs` | Add `LABEL_OPERATION_PROGRESS`, `PROGRESS_OPERATION` |
| `crates/harvester_app/src/platform/ui/layout.rs` | Create controls + layout rules in `PANEL_BOTTOM` |
| `crates/harvester_app/src/platform/ui/render.rs` | Add `render_operation_progress_section()` + `TreeRenderState` fields |

## Risks

- **Layout tuning** — the 80px bar + 120px label widths are estimates. May need adjustment after visual testing on the actual app.
- **Exhaustive match** — adding `Msg::PollStarted` will break exhaustive matches in `summarize_batch_msg` and any other match on `Msg`. The compiler will catch these.

## Engineering Diary Entry

When implemented, add to `docs/EngineeringDiary.md`:

> **Operation Progress Bar** — Polling, triage, and summary runs previously exposed only partial progress cues (disabled buttons, status text). Added reducer-owned operation progress in `AppViewModel`, introduced `Msg::PollStarted { total }` so poll totals are known before per-source completions arrive, and rendered a footer operation-progress section that reuses the existing diff-based UI render pattern. Poll completion now auto-selects `PollStats`. Lesson: for shared progress widgets, derive progress from explicit reducer state rather than parsing existing status text. If a UI surface must fully hide when idle, confirm the platform/layout layer supports collapse semantics before finalizing the plan.
