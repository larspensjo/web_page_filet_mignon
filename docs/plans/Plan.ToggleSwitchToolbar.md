# Plan: Global "Since Checkpoint" Toggle Switch in Top Toolbar

## Draft Diary Entry
Type: Implementation
Context: The "Since checkpoint only" checkbox lived inside the Jobs left pane, but its effect is global (filters all job lists, triage scopes, etc.). This was confusing UX — placing a global control inside a local pane implied it was pane-specific. Moving it to a new top toolbar panel communicates the global nature clearly.
Change: harvester_app + CommanDuctUI — new ToggleSwitch control type, new PANEL_TOOLBAR, status bar scope indicator.

---

## Context

`CHK_JOBS_SCOPE_SINCE_CHECKPOINT` is parented to `PANEL_JOBS` (the left pane) but affects global state (`AppState.job_list_scope`). A global control should live in a globally-visible location. The fix: create a dedicated top toolbar panel, add a modern sliding toggle switch (pill+knob, dark-themed), and reflect scope state in the status bar.

---

## Window Layout After Change

```
+- PANEL_TOOLBAR (Top, 40px) ──────────────────────────────────+
|  [○●]  Since checkpoint                                       |
+- PANEL_PROGRESS (Top, 64px) ──────────────────────────────────+
|  Tokens: 0 / N (0%)  [===========]                           |
+- Main area (Fill) ────────────────────────────────────────────+
|  Jobs (left pane)  |  Article detail (right pane)            |
+- PANEL_BUTTONS (Bottom, 44px) ────────────────────────────────+
|  [Stop/Finish]  [Triage Articles]  [Poll Sources]  [Browser]  |
+- PANEL_BOTTOM / status bar (Bottom, 32px) ────────────────────+
|  Session: Idle | Jobs: 42 | Since checkpoint | LLM: ...       |
+───────────────────────────────────────────────────────────────+
```

---

## Critical Files

### harvester_app
- `crates/harvester_app/src/platform/ui/constants.rs` — control ID constants
- `crates/harvester_app/src/platform/ui/layout.rs` — control creation + layout rules
- `crates/harvester_app/src/platform/ui/render.rs` — state sync + status bar text
- `crates/harvester_app/src/platform/app.rs` — event handler for toggle

### CommanDuctUI submodule
- `src/CommanDuctUI/src/types.rs` — PlatformCommand + AppEvent enums
- `src/CommanDuctUI/src/window_common.rs` — ControlKind enum, WM_APP constants, WndProc routing, `handle_wm_app_toggle_switch_clicked`
- `src/CommanDuctUI/src/controls.rs` — module declaration for new handler
- `src/CommanDuctUI/src/app.rs` — `execute_platform_command` match arms
- `src/CommanDuctUI/src/controls/` — new `toggle_switch_handler.rs`
- `src/CommanDuctUI/Cargo.toml` — version bump

### Existing patterns to follow exactly
- `src/CommanDuctUI/src/controls/chart_handler.rs` — custom WndProc class registration, GWLP_USERDATA state, WM_PAINT pattern
- `src/CommanDuctUI/src/controls/splitter_handler.rs` — hover state, WM_LBUTTONUP interaction
- `src/CommanDuctUI/src/controls/tab_bar_handler.rs` — use `GetAncestor(hwnd, GA_ROOT)` (not `GetParent`) for parent notifications (lesson from commit 155031f); `TabBarPalette` + `SetTabBarStyle` color-passing pattern; `WM_APP_TAB_SELECTED` + `handle_wm_app_tab_selected` in `window_common.rs` for event delivery
- `src/CommanDuctUI/src/window_common.rs::try_enable_dark_mode` — call after control creation

---

## Step 1: CommanDuctUI — New ToggleSwitch Control

### 1a. Update `src/types.rs`

Add to `PlatformCommand` enum:
```rust
CreateToggleSwitch {
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
    label: String,
    checked: bool,
},
SetToggleSwitchState {
    window_id: WindowId,
    control_id: ControlId,
    checked: bool,
},
SetToggleSwitchStyle {
    window_id: WindowId,
    control_id: ControlId,
    background: Color,
    pill_off: Color,
    pill_on: Color,
    knob: Color,
    text: Color,
},
```

Add to `AppEvent` enum:
```rust
ToggleSwitchToggled {
    window_id: WindowId,
    control_id: ControlId,
    checked: bool,
},
```

### 1b. Update `src/window_common.rs`

Add to `ControlKind` enum:
```rust
ToggleSwitch,
```

Add to the WM_APP constants block (after `WM_APP_TAB_SELECTED = WM_APP + 0x104`):
```rust
// Custom application message sent by ToggleSwitch WndProc to root on click/key-toggle.
pub(crate) const WM_APP_TOGGLE_SWITCH_CLICKED: u32 = WM_APP + 0x105;
```

In the root window WndProc match in `window_common.rs`, add a branch for the new message:
```rust
WM_APP_TOGGLE_SWITCH_CLICKED => {
    event_to_send = self.handle_wm_app_toggle_switch_clicked(hwnd, wparam, lparam, window_id);
}
```

Add the handler method (alongside `handle_wm_app_tab_selected`):
```rust
fn handle_wm_app_toggle_switch_clicked(
    self: &Arc<Self>,
    _hwnd_parent: HWND,
    wparam: WPARAM,
    lparam: LPARAM,
    window_id: WindowId,
) -> Option<AppEvent> {
    let hwnd_toggle = HWND(wparam.0 as *mut std::ffi::c_void);
    let control_id_raw = unsafe { GetDlgCtrlID(hwnd_toggle) };
    if control_id_raw == 0 { return None; }
    let control_id = ControlId::new(control_id_raw);
    let checked = lparam.0 != 0;
    Some(AppEvent::ToggleSwitchToggled { window_id, control_id, checked })
}
```

Also review any `ControlKind`-based invalidation paths in `window_common.rs` to confirm that `ToggleSwitch` does not need special handling there (it owns its own painting via WM_PAINT, so it likely does not).

### 1c. New file `src/controls/toggle_switch_handler.rs`

**Registration (called once via OnceLock):**
```rust
fn register_toggle_switch_class(hinstance: HINSTANCE) {
    // RegisterClassExW with class name "HarvesterToggleSwitchClass"
    // lpfnWndProc = toggle_switch_wnd_proc
    // hCursor = LoadCursorW(None, IDC_HAND)
    // hbrBackground = None (we paint everything ourselves)
}
```

**Per-control palette (analogous to `TabBarPalette`):**
```rust
struct ToggleSwitchPalette {
    background: Color,
    pill_off: Color,
    pill_on: Color,
    knob: Color,
    text: Color,
}

impl Default for ToggleSwitchPalette {
    fn default() -> Self {
        Self {
            background: Color { r: 0x2B, g: 0x2B, b: 0x2B }, // match panel
            pill_off:   Color { r: 0x4B, g: 0x4F, b: 0x57 }, // dark gray
            pill_on:    Color { r: 0x00, g: 0x80, b: 0xFF }, // blue accent — matches tab bar
            knob:       Color { r: 0xF0, g: 0xF0, b: 0xF0 }, // near-white
            text:       Color { r: 0xCC, g: 0xCC, b: 0xCC }, // light gray
        }
    }
}
```

**Per-control state (stored in GWLP_USERDATA):**
```rust
struct ToggleSwitchState {
    checked: bool,
    label: String,
    palette: ToggleSwitchPalette,
    // No event_tx needed — events are delivered via WM_APP_TOGGLE_SWITCH_CLICKED
    // to the root window WndProc, consistent with tab bar and splitter patterns.
}
```

**WndProc messages:**
- `WM_PAINT`: `BeginPaint` → fill entire client rect using `palette.background` →
  `RoundRect` pill (28×16px, centered vertically) using `palette.pill_off` or `palette.pill_on` →
  `Ellipse` knob (14px, left=OFF / right=ON) using `palette.knob` →
  `DrawTextW` label to the right of pill using `palette.text` → draw focus rect when focused →
  `EndPaint`
- `WM_ERASEBKGND`: return `LRESULT(1)` — suppress default erase, prevents flicker
- `WM_LBUTTONUP`: toggle `checked`, `InvalidateRect`, send notification:
  ```rust
  let root = GetAncestor(hwnd, GET_ANCESTOR_FLAGS(2)); // GA_ROOT
  SendMessageW(root, WM_APP_TOGGLE_SWITCH_CLICKED,
      Some(WPARAM(hwnd.0 as usize)),
      Some(LPARAM(state.checked as isize)));
  ```
  Use `WM_LBUTTONUP` (not `WM_LBUTTONDOWN`) so the user can cancel by moving the cursor
  away before releasing — standard Windows control behavior.
- `WM_KEYDOWN`: on `VK_SPACE` or `VK_RETURN`, toggle `checked`, `InvalidateRect`, send same
  `WM_APP_TOGGLE_SWITCH_CLICKED` notification. This satisfies keyboard accessibility.
- `WM_SETFOCUS` / `WM_KILLFOCUS`: `InvalidateRect` to repaint focus indicator.
- `WM_DESTROY`: `Box::from_raw` to free heap state
- Everything else: `DefWindowProcW`

**CreateWindowExW flags — keyboard accessibility:**
Include `WS_TABSTOP` in the window style so the control participates in tab-order focus
traversal:
```rust
CreateWindowExW(
    ...,
    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
    ...
)
```

**Dark mode — after `CreateWindowExW`:**
```rust
window_common::try_enable_dark_mode(hwnd);
// Do NOT call apply_button_dark_mode_classic_render — that is only for Win32 BS_* button
// controls that need WM_CTLCOLORBTN. A fully custom WndProc owns its own painting.
```

**Dark theme regression note (diary 2026-02-25):**
ToggleSwitch is fully owner-drawn and never goes through `WM_CTLCOLORBTN` / `StyleId` /
`ApplyStyleToControl`. The `every_button_like_control_has_a_style_applied` test only guards
Win32 button controls (Button/RadioButton/CheckBox) — it will not flag ToggleSwitch.
No `StyleId::ToggleSwitch` is needed; colors flow in via `SetToggleSwitchStyle` instead.

**Exported functions:**
```rust
pub(crate) fn handle_create_toggle_switch_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    parent_control_id: Option<ControlId>,
    control_id: ControlId,
    label: String,
    checked: bool,
)

pub(crate) fn handle_set_toggle_switch_state_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    checked: bool,
)
// Retrieves HWND via internal_state, updates state.checked, InvalidateRect to repaint

pub(crate) fn handle_set_toggle_switch_style_command(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    control_id: ControlId,
    background: Color,
    pill_off: Color,
    pill_on: Color,
    knob: Color,
    text: Color,
)
// Retrieves HWND, updates state.palette, InvalidateRect
```

### 1d. Update `src/controls.rs`

Add the module declaration alongside the other control handlers:
```rust
pub(crate) mod toggle_switch_handler;
```

### 1e. Update `src/app.rs` — `execute_platform_command`

The `execute_platform_command` match in `Win32ApiInternalState` (line ~380 of `app.rs`) is the
correct routing point — not `command_executor.rs`. Add three new arms:

```rust
PlatformCommand::CreateToggleSwitch {
    window_id, parent_control_id, control_id, label, checked
} => controls::toggle_switch_handler::handle_create_toggle_switch_command(
    self, window_id, parent_control_id, control_id, label, checked,
),
PlatformCommand::SetToggleSwitchState { window_id, control_id, checked } => {
    controls::toggle_switch_handler::handle_set_toggle_switch_state_command(
        self, window_id, control_id, checked,
    )
}
PlatformCommand::SetToggleSwitchStyle {
    window_id, control_id, background, pill_off, pill_on, knob, text
} => controls::toggle_switch_handler::handle_set_toggle_switch_style_command(
    self, window_id, control_id, background, pill_off, pill_on, knob, text,
),
```

### 1f. Version bump + CHANGELOG

Bump version in `src/CommanDuctUI/Cargo.toml`.
Update CHANGELOG if present.

---

## Step 2: harvester_app — Constants

**`crates/harvester_app/src/platform/ui/constants.rs`**

Add:
```rust
pub const PANEL_TOOLBAR: ControlId = ControlId::new(2015); // verify no conflict
pub const TS_JOBS_SCOPE: ControlId = ControlId::new(3020); // verify no conflict
```

Remove:
```rust
pub const CHK_JOBS_SCOPE_SINCE_CHECKPOINT: ControlId = ControlId::new(3014);
```

---

## Step 3: harvester_app — Layout

**`crates/harvester_app/src/platform/ui/layout.rs`**

### 3a. Control creation

Add panel creation (near other panel creations ~line 132):
```rust
commands.push(PlatformCommand::CreatePanel {
    window_id,
    parent_control_id: None,
    control_id: PANEL_TOOLBAR,
});
```

Add toggle creation:
```rust
commands.push(PlatformCommand::CreateToggleSwitch {
    window_id,
    parent_control_id: Some(PANEL_TOOLBAR),
    control_id: TS_JOBS_SCOPE,
    label: "Since checkpoint".to_string(),
    checked: false, // initial state; synced by render on first tick
});
```

Remove `CreateCheckBox` for `CHK_JOBS_SCOPE_SINCE_CHECKPOINT` (~line 340).

### 3b. Layout rules

The layout engine sorts rules ascending by `order` (`sorted.sort_by_key(|r| r.order)`). For
`DockStyle::Top`, lower order = processed first = topmost position. `PANEL_PROGRESS` is currently
at `order: 0` (line ~1154). To place `PANEL_TOOLBAR` above `PANEL_PROGRESS`:

- Change `PANEL_PROGRESS` `order: 0` → `order: 1` (line ~1154 of layout.rs).
- Add `PANEL_TOOLBAR` at `order: 0`.

Note: `LayoutRule.order` is `u32`; negative values will not compile.

Add layout rules:
```rust
// PANEL_TOOLBAR: docked Top, order 0 — topmost panel (above PANEL_PROGRESS at order 1)
LayoutRule {
    control_id: PANEL_TOOLBAR,
    parent_control_id: None,
    dock_style: DockStyle::Top,
    order: 0,
    fixed_size: Some(40),
    margin: (0, 0, 0, 0),
}

// TS_JOBS_SCOPE: docked Left inside toolbar, fixed width 200px
LayoutRule {
    control_id: TS_JOBS_SCOPE,
    parent_control_id: Some(PANEL_TOOLBAR),
    dock_style: DockStyle::Left,
    order: 10,
    fixed_size: Some(200),
    margin: (8, 8, 8, 8),
}
```

Also update existing `PANEL_PROGRESS` rule:
```rust
// Before:
order: 0,
// After:
order: 1,
```

Remove layout rule for `CHK_JOBS_SCOPE_SINCE_CHECKPOINT`.

### 3c. apply_dark_theme — panel background

Add `PANEL_TOOLBAR` to the panel list in `apply_dark_theme` that receives `StyleId::PanelBackground`
(currently lines 2056–2086). Without this the panel renders with the default Windows light gray
background instead of the dark theme.

```rust
// In the for control_id in [...] loop:
PANEL_TOOLBAR,
// alongside PANEL_PROGRESS, PANEL_BUTTONS, etc.
```

### 3d. apply_dark_theme — toggle switch style

After the `PanelBackground` loop, add the style command for the toolbar toggle switch:
```rust
commands.push(PlatformCommand::SetToggleSwitchStyle {
    window_id,
    control_id: TS_JOBS_SCOPE,
    background: Color { r: 0x2B, g: 0x2B, b: 0x2B },
    pill_off:   Color { r: 0x4B, g: 0x4F, b: 0x57 },
    pill_on:    Color { r: 0x00, g: 0x80, b: 0xFF }, // blue accent — same as tab bar
    knob:       Color { r: 0xF0, g: 0xF0, b: 0xF0 },
    text:       Color { r: 0xCC, g: 0xCC, b: 0xCC },
});
```

### 3e. apply_dark_theme — remove dangling CheckBox reference

Remove `CHK_JOBS_SCOPE_SINCE_CHECKPOINT` from the `StyleId::CheckBox` loop (currently line 2228).
This is a required compile-error fix — failing to do so will cause a "use of undeclared constant"
error when the constant is removed in Step 2.

```rust
// Before:
for control_id in [
    CHK_JOBS_SCOPE_SINCE_CHECKPOINT,   // ← REMOVE this line
    CHK_PROMPT_LAB_SECTION_COMPARE,
    ...
]

// After:
for control_id in [
    CHK_PROMPT_LAB_SECTION_COMPARE,
    ...
]
```

---

## Step 4: harvester_app — Render

**`crates/harvester_app/src/platform/ui/render.rs`**

State tracking field (~line 110): keep same field name/semantics, update the emitted command:
```rust
// Before:
emit_if_changed(&mut self.prev_jobs_scope_since_checkpoint_checked,
    view.left_pane.job_list_scope == JobListScope::SinceCheckpoint,
    || PlatformCommand::SetCheckBoxChecked {
        window_id, control_id: CHK_JOBS_SCOPE_SINCE_CHECKPOINT, checked: ... });

// After:
emit_if_changed(&mut self.prev_jobs_scope_since_checkpoint_checked,
    view.left_pane.job_list_scope == JobListScope::SinceCheckpoint,
    || PlatformCommand::SetToggleSwitchState {
        window_id, control_id: TS_JOBS_SCOPE,
        checked: view.left_pane.job_list_scope == JobListScope::SinceCheckpoint });
```

Status bar — in `render_status_section()` (~line 513), append scope indicator:
```rust
if view.left_pane.job_list_scope == JobListScope::SinceCheckpoint {
    parts.push("Since checkpoint".to_string());
}
```
(Same pattern as briefing/triage progress parts, separated by " | ".)

---

## Step 5: harvester_app — Event Handler

**`crates/harvester_app/src/platform/app.rs`** (~line 559)

Replace:
```rust
AppEvent::CheckBoxToggled { control_id, checked, .. }
    if control_id == ui::constants::CHK_JOBS_SCOPE_SINCE_CHECKPOINT => {
        let scope = if checked { JobListScope::SinceCheckpoint } else { JobListScope::All };
        let _ = self.msg_tx.send(Msg::JobListScopeSet { scope });
    }
```

With:
```rust
AppEvent::ToggleSwitchToggled { control_id, checked, .. }
    if control_id == ui::constants::TS_JOBS_SCOPE => {
        let scope = if checked { JobListScope::SinceCheckpoint } else { JobListScope::All };
        let _ = self.msg_tx.send(Msg::JobListScopeSet { scope });
    }
```

---

## Step 6: Update Broken Unit Tests

Three tests reference the removed constant and old event/command types. All three must be updated
to compile and correctly guard the new behavior.

### 6a. `layout.rs::new_controls_created_in_initial_commands` (~line 2367)

Replace the assertion that checks for `CreateCheckBox` with `CHK_JOBS_SCOPE_SINCE_CHECKPOINT`:
```rust
// Before:
assert!(
    commands.iter().any(|cmd| matches!(
        cmd,
        PlatformCommand::CreateCheckBox { control_id, .. }
            if *control_id == CHK_JOBS_SCOPE_SINCE_CHECKPOINT
    )),
    "jobs scope checkbox should be created"
);

// After:
assert!(
    commands.iter().any(|cmd| matches!(
        cmd,
        PlatformCommand::CreateToggleSwitch { control_id, .. }
            if *control_id == TS_JOBS_SCOPE
    )),
    "jobs scope toggle switch should be created"
);
```

### 6b. `render.rs::jobs_scope_checkbox_reflects_scope_state` (~line 2806)

Replace assertions checking `SetCheckBoxChecked` with `CHK_JOBS_SCOPE_SINCE_CHECKPOINT`:
```rust
// Before (SinceCheckpoint case):
PlatformCommand::SetCheckBoxChecked { control_id, checked: true, .. }
    if *control_id == CHK_JOBS_SCOPE_SINCE_CHECKPOINT

// After:
PlatformCommand::SetToggleSwitchState { control_id, checked: true, .. }
    if *control_id == TS_JOBS_SCOPE

// Before (All case):
PlatformCommand::SetCheckBoxChecked { control_id, checked: false, .. }
    if *control_id == CHK_JOBS_SCOPE_SINCE_CHECKPOINT

// After:
PlatformCommand::SetToggleSwitchState { control_id, checked: false, .. }
    if *control_id == TS_JOBS_SCOPE
```

Consider renaming the test to `jobs_scope_toggle_reflects_scope_state` for clarity.

### 6c. `app.rs::jobs_scope_checkbox_emits_typed_scope_message` (~line 1267)

Replace the event sent to the handler:
```rust
// Before:
handler.handle_event(AppEvent::CheckBoxToggled {
    window_id: WindowId::new(1),
    control_id: ui::constants::CHK_JOBS_SCOPE_SINCE_CHECKPOINT,
    checked: true,
});

// After:
handler.handle_event(AppEvent::ToggleSwitchToggled {
    window_id: WindowId::new(1),
    control_id: ui::constants::TS_JOBS_SCOPE,
    checked: true,
});
```

Apply the same substitution to the second `handle_event` call (`checked: false`).
Consider renaming the test to `jobs_scope_toggle_emits_typed_scope_message`.

---

## Sequencing — Stay Buildable

To keep the tree compilable through each step:

1. Add new enum variants (`PlatformCommand`, `AppEvent`, `ControlKind`) — additive, no breakage.
2. Add new constants (`PANEL_TOOLBAR`, `TS_JOBS_SCOPE`) in `constants.rs`.
3. Implement `toggle_switch_handler.rs` + wire into `controls.rs` and `app.rs`.
4. Add new layout rules and `PANEL_PROGRESS` order bump.
5. Update `render.rs` and `app.rs` event handler to new control.
6. Update the three broken tests.
7. Remove `CHK_JOBS_SCOPE_SINCE_CHECKPOINT` constant last (only after all references are gone).

---

## Verification

1. `cargo build` — both CommanDuctUI and harvester_app compile cleanly
2. Run app manually:
   - Top toolbar panel visible at the very top of the window, above PANEL_PROGRESS
   - Toggle renders dark pill + knob on a dark background (no white/light bleed)
   - Click and release toggle → job list scope changes → status bar shows/hides `| Since checkpoint`
   - Tab key reaches the toggle; space/enter toggles it from keyboard
   - Left pane no longer has the old checkbox
   - Toggle state is correct after `Msg::JobListScopeSet` from any source
3. `cargo clippy --all-targets -- -D warnings` — clean

---

## Notes / Risks

- **Accent color**: `pill_on` is `Color { r: 0, g: 0x80, b: 0xFF }` — the same blue accent as the
  tab bar. The original plan incorrectly used `0x00C8_7028` with the comment "accent orange";
  in Win32 BGR COLORREF that value is B=200, G=112, R=40 (blue-green, not orange). Corrected here.
- **No StyleId for ToggleSwitch**: Colors are passed via `SetToggleSwitchStyle` (matching the
  `SetTabBarStyle` pattern from `tab_bar_handler.rs`). A `StyleId::ToggleSwitch` is not needed
  because the control is fully owner-drawn and `every_button_like_control_has_a_style_applied`
  only guards Win32 button controls (CreateButton/RadioButton/CheckBox).
- **Event emission via WM_APP pattern**: The plan originally proposed storing a `Sender<AppEvent>`
  directly in `ToggleSwitchState`. This was corrected to use `WM_APP_TOGGLE_SWITCH_CLICKED` +
  `SendMessageW(GetAncestor(hwnd, GA_ROOT), ...)`, consistent with how `tab_bar_handler` and
  `splitter_handler` deliver events. `ToggleSwitchState` has no `event_tx` field.
- **Command routing in `app.rs`**: The plan originally named `command_executor.rs` as the file
  to update for new command variants. Corrected: `execute_platform_command` is a method on
  `Win32ApiInternalState` in `app.rs` (line ~380). Helper functions live in the handler file;
  routing arms go in `app.rs`.
- **Layout order for PANEL_TOOLBAR**: `LayoutRule.order` is `u32` — negative values do not
  compile. The layout engine sorts ascending; lower order = topmost for `DockStyle::Top`.
  Fix: `PANEL_TOOLBAR` at `order: 0`; `PANEL_PROGRESS` bumped from `order: 0` to `order: 1`.
- **PANEL_TOOLBAR visibility when PANEL_PROGRESS is active**: PANEL_TOOLBAR is always visible.
  When PANEL_PROGRESS is also visible, PANEL_TOOLBAR appears above it. This is intentional —
  the toolbar is a permanent global fixture.
- **Control ID conflicts**: Verify `PANEL_TOOLBAR = 2015` and `TS_JOBS_SCOPE = 3020` don't
  conflict with existing IDs in `constants.rs` before assigning.
- **No reducer changes**: `Msg::JobListScopeSet`, `JobListScope`, and `AppState.job_list_scope`
  are untouched. All existing reducer tests pass without modification.
- **ControlKind invalidation paths**: Review `window_common.rs` for any `ControlKind`-based
  control invalidation logic to confirm `ToggleSwitch` does not require special handling there.
  Since it owns its own painting, it most likely does not.
- **CommanDuctUI-level unit tests**: The review notes a gap in unit coverage for the new handler
  (create/state/style/event emission, click-cancel semantics). This is noted as a P2 gap.
  Existing patterns (chart, tab_bar) do not have handler-level unit tests in this repo, so
  adding them is optional for this iteration but desirable for long-term regression safety.
- **Reusing `CheckBoxToggled`**: The review asked whether `CheckBoxToggled` could be reused to
  reduce API churn. Not applied — `ToggleSwitchToggled` is a distinct semantic type and
  preserves exhaustive match safety; the three test updates are a bounded, mechanical change.
