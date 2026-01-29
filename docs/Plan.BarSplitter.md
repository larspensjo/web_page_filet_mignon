# Plan: Bar Splitter Between Treeview and Preview

## Overview
Add a draggable vertical splitter bar between the left panels (PANEL_INPUT + PANEL_JOBS) and the preview panel (PANEL_PREVIEW). Users can drag the splitter to resize the total left width. PANEL_INPUT and PANEL_JOBS resize proportionally; `left_panel_width` is the sum of both columns (default 600 = 320 + 280).

## Architecture Decision
Implement a **new control type in CommanDuctUI** (not pure layout behavior):
- Requires mouse capture, hit-testing, cursor changes, and robust cancellation handling
- Matches existing handler pattern (`button_handler.rs`, `panel_handler.rs`)
- Enables future horizontal splitters or multiple splitters

## Requirements
- R1: Dragging updates left/preview widths in real time.
- R2: Min widths enforced for left and preview; clamping updates on window resize.
- R3: Unidirectional data flow; only actions update state.
- R4: Splitter has correct cursor, robust capture lifecycle, and no keyboard focus.
- R5: Single source of truth for left width; no shadow widths.
- R6: Unit tests lock clamping and reducer behavior.
- R7: Action boundary logging (drag start/end).

---

## Phase 0: Geometry and Invariants

### Step 0.1: Define event contract (types.rs + splitter_handler.rs)
Define a clear contract: splitter events emit `desired_left_width_px` in **window client coordinates** (not raw mouse x), already accounting for margins and splitter thickness.

**Requirements update:** R1 defined, R2 defined, R3 unchanged.

### Step 0.2: Add pure clamp helper (harvester_core)
Add `calc_left_width(desired_left, window_width, min_left, min_preview, splitter_total)` with dynamic max:
```
max_left = window_width - min_preview - splitter_total
```

**Requirements update:** R2 defined, R6 pending.

### Step 0.3: Clarify proportional resizing (layout.rs)
Document and enforce proportional resizing: PANEL_INPUT and PANEL_JOBS widths are derived from `left_panel_width` each layout pass.

**Requirements update:** R5 defined.

---

## Phase 1: CommanDuctUI Splitter Control

### Step 1.1: Add types (types.rs)
Add to `AppEvent` enum:
```rust
SplitterDragging {
    window_id: WindowId,
    control_id: ControlId,
    desired_left_width_px: i32,
},
SplitterDragEnded {
    window_id: WindowId,
    control_id: ControlId,
    desired_left_width_px: i32,
},
```

Add to `PlatformCommand` enum:
```rust
CreateSplitter {
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
    orientation: SplitterOrientation,
},
```

Add new enum:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitterOrientation {
    Vertical,   // Divides left/right (user drags horizontally)
    Horizontal, // Divides top/bottom (future extension)
}
```

**Requirements update:** R1 partial, R3 partial.

### Step 1.2: Add ControlKind variant (window_common.rs)
Add `Splitter` to `ControlKind` enum.

**Requirements update:** R4 partial.

### Step 1.3: Create splitter_handler.rs (new file)
**Location:** `src/CommanDuctUI/src/controls/splitter_handler.rs`

Core implementation:
1. `handle_create_splitter_command()` - Creates a custom window class with its own WndProc
2. Custom WndProc handles:
   - `WM_SETCURSOR` - Return `IDC_SIZEWE` cursor for vertical splitter
   - `WM_LBUTTONDOWN` - Call `SetCapture()`, enter drag mode
   - `WM_MOUSEMOVE` - If captured, compute desired width and emit `SplitterDragging`
   - `WM_LBUTTONUP` - Call `ReleaseCapture()`, emit `SplitterDragEnded`
   - `WM_CAPTURECHANGED` / `WM_CANCELMODE` - Ensure drag is cancelled and state resets
   - `WM_PAINT` - Draw splitter bar (subtle vertical line or 3D effect)

Internal state tracking:
```rust
pub(crate) struct SplitterInternalState {
    orientation: SplitterOrientation,
    is_dragging: bool,
    drag_start_mouse_x: i32,
    drag_start_control_x: i32,
}
```

**Requirements update:** R4 implemented, R1 partial.

### Step 1.4: Register handler (controls.rs)
Add: `pub(crate) mod splitter_handler;`

**Requirements update:** R4 partial.

### Step 1.5: Wire command dispatch (app.rs)
Add match arm for `PlatformCommand::CreateSplitter` in command execution.

**Requirements update:** R4 partial.

### Step 1.6: Add splitter state to NativeWindowData (window_common.rs)
Add field: `splitter_states: HashMap<ControlId, SplitterInternalState>`

**Requirements update:** R4 partial.

---

## Phase 2: Application Layer Integration

### Step 2.1: Add control ID constant (constants.rs)
```rust
pub const SPLITTER_MAIN: ControlId = ControlId::new(6001);
```

**Requirements update:** R4 partial.

### Step 2.2: Add left panel width and window width to UiState (state.rs)
Add to `UiState` struct:
```rust
left_panel_width: i32,  // Default: 600 (320 + 280)
window_width: i32,
```

Add accessor methods and include in `Default` impl.

**Requirements update:** R2 partial, R3 partial, R5 partial.

### Step 2.3: Add Msg variants (msg.rs)
```rust
SplitterMoved { desired_left_width_px: i32 },
WindowResized { window_width: i32 },
```

**Requirements update:** R2 partial, R3 partial.

### Step 2.4: Handle messages in update.rs
Use `calc_left_width` and dynamic max (based on `window_width`) to clamp:
```rust
Msg::SplitterMoved { desired_left_width_px } => {
    let clamped = calc_left_width(
        desired_left_width_px,
        state.ui.window_width(),
        MIN_LEFT_WIDTH,
        MIN_PREVIEW_WIDTH,
        SPLITTER_TOTAL_WIDTH,
    );
    state.ui.set_left_panel_width(clamped);
    state.dirty = true;
    (state, Vec::new())
}
Msg::WindowResized { window_width } => {
    state.ui.set_window_width(window_width);
    let clamped = calc_left_width(
        state.ui.left_panel_width(),
        window_width,
        MIN_LEFT_WIDTH,
        MIN_PREVIEW_WIDTH,
        SPLITTER_TOTAL_WIDTH,
    );
    state.ui.set_left_panel_width(clamped);
    state.dirty = true;
    (state, Vec::new())
}
```

Constants: `MIN_LEFT_WIDTH = 200`, `MIN_PREVIEW_WIDTH = 200`

**Requirements update:** R2 implemented, R3 implemented.

### Step 2.5: Update AppViewModel (view_model.rs)
Add:
```rust
pub left_panel_width: i32,
pub window_width: i32,
```

Update `view()` in state.rs to populate these fields.

**Requirements update:** R5 partial.

### Step 2.6: Update layout.rs
**Create splitter control:**
```rust
commands.push(PlatformCommand::CreateSplitter {
    window_id,
    parent_control_id: None,
    control_id: SPLITTER_MAIN,
    orientation: SplitterOrientation::Vertical,
});
```

**Apply style after creation (separate command):**
```rust
commands.push(PlatformCommand::ApplyStyleToControl {
    window_id,
    control_id: SPLITTER_MAIN,
    style_id: StyleId::Splitter,
});
```

**Add layout rule for splitter (order 305, between PANEL_JOBS at 300 and PANEL_PREVIEW at 310):**
```rust
LayoutRule {
    control_id: SPLITTER_MAIN,
    parent_control_id: None,
    dock_style: DockStyle::Left,
    order: 305,
    fixed_size: Some(4),  // 4px wide splitter bar
    margin: (6, 0, 6, 0),
}
```

**Make PANEL_INPUT and PANEL_JOBS use dynamic widths:**
- Refactor `initial_commands()` to accept `left_panel_width` or fetch from view model
- Split proportionally: input ~53% (~320 at 600), jobs ~47% (~280 at 600)

**Requirements update:** R5 implemented.

### Step 2.7: Handle splitter events in app.rs (harvester_app)
In `handle_event()`:
```rust
AppEvent::SplitterDragging { desired_left_width_px, .. } |
AppEvent::SplitterDragEnded { desired_left_width_px, .. } => {
    let _ = self.msg_tx.send(Msg::SplitterMoved {
        desired_left_width_px
    });
}
```

**Requirements update:** R1 partial, R3 partial.

### Step 2.8: Handle window resize events
Ensure window resize event dispatches `Msg::WindowResized { window_width }`.

**Requirements update:** R2 partial.

### Step 2.9: Re-emit layout on width change (render.rs)
Track previous `left_panel_width` and `window_width`; emit `DefineLayout` when they change.

**Requirements update:** R1 implemented.

---

## Phase 3: Styling and Traceability

### Step 3.1: Add StyleId::Splitter variant (styling_primitives.rs)

**Requirements update:** R4 partial.

### Step 3.2: Define splitter style (layout.rs)
```rust
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::Splitter,
    style: ControlStyle {
        background_color: Some(Color { r: 0x40, g: 0x44, b: 0x4B }),
        ..Default::default()
    },
});
```

**Requirements update:** R4 partial.

### Step 3.3: Optional hover highlight
In `splitter_handler.rs`, track hover state and paint brighter background on hover.

**Requirements update:** R4 partial.

### Step 3.4: Add logging (harvester_app)
Log drag start and end with `engine_logging`; avoid logging every mouse move.

**Requirements update:** R7 implemented.

---

## Edge Cases and Robustness

1. **Minimum widths:** Clamp splitter position to keep left panels >= 200px and preview >= 200px
2. **Window resize:** Re-clamp based on new window width
3. **Capture loss:** Handle `WM_CAPTURECHANGED` and `WM_CANCELMODE` to avoid stuck dragging
4. **DPI scaling:** Consider scaling splitter width and margins
5. **Focus:** Splitter should not take keyboard focus

---

## Critical Files to Modify

| File | Changes |
|------|---------|
| `src/CommanDuctUI/src/types.rs` | Add SplitterOrientation, AppEvent variants, PlatformCommand::CreateSplitter |
| `src/CommanDuctUI/src/window_common.rs` | Add ControlKind::Splitter, splitter_states field |
| `src/CommanDuctUI/src/controls/splitter_handler.rs` | **New file:** Core splitter Win32 implementation |
| `src/CommanDuctUI/src/controls.rs` | Register splitter_handler module |
| `src/CommanDuctUI/src/app.rs` | Wire CreateSplitter command dispatch |
| `src/CommanDuctUI/src/styling_primitives.rs` | Add StyleId::Splitter |
| `crates/harvester_app/src/platform/ui/constants.rs` | Add SPLITTER_MAIN constant |
| `crates/harvester_app/src/platform/ui/layout.rs` | Create splitter, define style, apply style, dynamic layout rules |
| `crates/harvester_app/src/platform/app.rs` | Handle splitter events + window resize -> Msg |
| `crates/harvester_core/src/state.rs` | Add left_panel_width + window_width to UiState |
| `crates/harvester_core/src/msg.rs` | Add Msg::SplitterMoved + Msg::WindowResized |
| `crates/harvester_core/src/update.rs` | Clamp for SplitterMoved + WindowResized |
| `crates/harvester_core/src/view_model.rs` | Add left_panel_width + window_width fields |

---

## Testing Strategy

1. **Unit tests for clamp helper:**
   - Min/max behavior, including tiny windows
   - Splitter total width accounted for

2. **Reducer tests in harvester_core:**
   - `Msg::SplitterMoved` updates state and clamps correctly
   - `Msg::WindowResized` re-clamps widths
   - View model reflects `left_panel_width` and `window_width`

3. **Integration testing:**
   - Manual: Drag splitter, verify proportional resizing
   - Verify cursor changes to resize cursor on hover
   - Verify min/max constraints after window resize

---

## Verification

1. `cargo build` - Ensure no compilation errors
2. `cargo clippy --all-targets -- -D warnings` - No warnings (after full implementation)
3. Manual test: Run the application, drag the splitter left and right
4. Verify: Left panels resize proportionally, preview panel adjusts
5. Verify: Splitter respects min/max width constraints and re-clamps on window resize

---

## Future Extensions

1. **Persistence:** Save/restore splitter position across sessions
2. **Double-click reset:** Reset to default position on double-click
3. **Horizontal splitter:** For top/bottom splits (architecture ready)
4. **Multiple splitters:** The control-based approach supports this
5. **Keyboard accessibility:** Arrow keys to adjust when focused
