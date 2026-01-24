# Dark Theme Implementation Plan

## Overview

This document provides a step-by-step implementation plan for adding optional dark theme support to CommanDuctUI and the Harvester application. The approach follows the existing command-event pattern and maintains backward compatibility.

### Design Principles

1. **Opt-in behavior**: If no styles are defined/applied, all controls render with system defaults (current behavior)
2. **Command-driven**: Uses existing `DefineStyle` + `ApplyStyleToControl` commands (CDU-Styling-DefineV1/V2)
3. **Unidirectional data flow**: Style definitions flow from app → CommanDuctUI → Win32 rendering
4. **No breaking changes**: Existing applications continue to work without modification
5. **Testable at each phase**: Application remains runnable and testable after each implementation phase

### Architecture Overview

```
Harvester App (layout.rs)
    ↓ PlatformCommand::DefineStyle
CommanDuctUI (app.rs)
    ↓ Parse into Win32 resources (HFONT, HBRUSH, COLORREF)
    ↓ Store in defined_styles: HashMap<StyleId, ParsedControlStyle>
    ↓ PlatformCommand::ApplyStyleToControl
    ↓ Store in window_data.applied_styles: HashMap<ControlId, StyleId>
    ↓ WM_PAINT / WM_CTLCOLOR* / WM_DRAWITEM / NM_CUSTOMDRAW
Win32 Rendering
```

---

## Cross-Cutting Concerns

These apply across multiple phases and should be kept in mind throughout implementation.

### A) ControlKind Tracking (avoid runtime class-name introspection)

Instead of calling `GetClassNameW()` to determine control type in `execute_apply_style_to_control()`, store a `ControlKind` enum in `NativeWindowData` at creation time:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlKind {
    Button,
    ProgressBar,
    TreeView,
    Static,   // Labels and panels
    Edit,     // Input fields
}
```

Each `CreateButton`, `CreateProgressBar`, etc. handler records the kind in the window data. Then `ApplyStyleToControl` dispatches on the stored kind rather than querying Win32. This is more robust and unit-testable without a HWND.

### B) GDI Resource Lifetime & Cleanup

- Styles own their created `HFONT`/`HBRUSH` (already the case via `ParsedControlStyle::Drop`).
- On teardown (window destroy / app shutdown), resources are deleted exactly once.
- Owner-drawn button handler creates a temporary brush per paint. This is acceptable for MVP but can be optimized later by caching the "pressed" brush in the parsed style.

### C) Fallback Drawing for Owner-Drawn Controls

Once `BS_OWNERDRAW` is set, Windows won't draw the button. The `handle_wm_drawitem` handler **must always render something**:

- If style lookup fails: use `GetSysColor(COLOR_BTNFACE)` for background, `GetSysColor(COLOR_BTNTEXT)` for text.
- Handle `ODS_DISABLED`: use `GetSysColor(COLOR_GRAYTEXT)` for text.
- Handle `ODS_SELECTED` and `ODS_FOCUS` at minimum.

### D) Flicker Prevention

- Return `LRESULT(1)` (TRUE) from `WM_ERASEBKGND` when handled — prevents DefWindowProc from erasing with the class brush.
- In `WM_PAINT`, fill only `ps.rcPaint` (the dirty region), not the entire client rect.
- Avoid double-painting the same region in both handlers.
- `WS_EX_COMPOSITED` is a last resort only if visual tearing persists.

---

## MVP Implementation

The MVP delivers a flat dark theme with proper colors for all control types. No neumorphic shadows yet.

---

## Phase 1: Foundation - StyleId Variants + ControlKind Tracking

**Goal**: Add the new style identifiers and the `ControlKind` tracking infrastructure needed by later phases.

### Step 1a: Add StyleId Variants

**`src/CommanDuctUI/src/styling_primitives.rs`**

Add two new variants to the `StyleId` enum (after line 75):

```rust
pub enum StyleId {
    // ... existing variants ...
    SummaryFolderMissingFile,
    // New variants for dark theme:
    HeaderLabel,        // Amber/orange header text
    ProgressBar,        // Dark progress bar with colored fill
}
```

### Step 1b: Add ControlKind Enum and Tracking

**`src/CommanDuctUI/src/window_common.rs`**

Add a `ControlKind` enum and tracking in `NativeWindowData`:

```rust
/// Identifies the type of a control for style dispatch.
/// Stored at creation time, eliminating runtime Win32 class-name queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlKind {
    Button,
    ProgressBar,
    TreeView,
    Static,   // Labels and panels
    Edit,     // Input/edit controls
}
```

Add to `NativeWindowData`:
```rust
pub(crate) struct NativeWindowData {
    // ... existing fields ...
    control_kinds: HashMap<ControlId, ControlKind>,
}

impl NativeWindowData {
    pub(crate) fn register_control_kind(&mut self, control_id: ControlId, kind: ControlKind) {
        self.control_kinds.insert(control_id, kind);
    }

    pub(crate) fn get_control_kind(&self, control_id: ControlId) -> Option<ControlKind> {
        self.control_kinds.get(&control_id).copied()
    }
}
```

**`src/CommanDuctUI/src/controls/button_handler.rs`**, **`progress_handler.rs`**, **`label_handler.rs`**, **`input_handler.rs`**, **`treeview_handler.rs`**

In each control's creation function, after `CreateWindowExW()` succeeds, call:
```rust
window_data.register_control_kind(control_id, ControlKind::Button); // or appropriate kind
```

### Implementation Notes

- Pure additions, no existing behavior changes
- All existing `StyleId` values remain unchanged
- The new variants and `ControlKind` will be unused until later phases
- `ControlKind` eliminates the need for `GetClassNameW()` in Phases 6/7 (see Cross-Cutting Concern A)
- **Serialization caution**: If `StyleId` is ever persisted (config/state), enum variant additions can break stable IDs. If persisted anywhere in the future, use string-based keys at the persistence boundary or explicitly version the schema.

### Build & Test

```bash
cargo build --workspace
```

**Expected Result**: Clean build, no warnings.

**QA Testing**: Run the application. It should look identical to before (all system defaults).

**Unit Tests**: None required (enum extension is non-functional).

---

## Phase 2: Main Window Background Support

**Goal**: Enable `MainWindowBackground` style to control the window's base color, preventing white flicker on startup.

### Files to Modify

#### **`src/CommanDuctUI/src/window_common.rs`**

**Change 1: Add WM_ERASEBKGND handler** (add new function around line 1220):

```rust
fn handle_wm_erasebkgnd(
    self: &Arc<Self>,
    hwnd: HWND,
    wparam: WPARAM,
    lparam: LPARAM,
    window_id: WindowId,
) -> LRESULT {
    unsafe {
        // Check if MainWindowBackground style is defined
        if let Some(style) = self.get_parsed_style(StyleId::MainWindowBackground)
            && let Some(brush) = style.background_brush
        {
            let hdc = HDC(wparam.0 as *mut c_void);
            let mut rect = RECT::default();
            if GetClientRect(hwnd, &mut rect).is_ok() {
                FillRect(hdc, &rect, brush);
                return LRESULT(1); // TRUE - we handled it
            }
        }
        // No style defined - use default behavior (pass original params for correctness)
        DefWindowProcW(hwnd, WM_ERASEBKGND, Some(wparam), Some(lparam))
    }
}
```

**Change 2: Add WM_ERASEBKGND dispatch** in `handle_window_message()` (add case around line 890):

```rust
match msg {
    // ... existing cases ...
    WM_ERASEBKGND => {
        lresult_override = Some(self.handle_wm_erasebkgnd(hwnd, wparam, lparam, window_id));
    }
    WM_PAINT => {
        lresult_override = Some(self.handle_wm_paint(hwnd, wparam, lparam, window_id));
    }
    // ... rest of cases ...
}
```

**Change 3: Modify WM_PAINT handler** (modify existing function around line 1199):

```rust
fn handle_wm_paint(
    self: &Arc<Self>,
    hwnd: HWND,
    _wparam: WPARAM,
    _lparam: LPARAM,
    _window_id: WindowId,
) -> LRESULT {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        if !hdc.is_invalid() {
            // Use MainWindowBackground style if defined, otherwise system default
            let brush = if let Some(style) = self.get_parsed_style(StyleId::MainWindowBackground)
                && let Some(bg_brush) = style.background_brush
            {
                bg_brush
            } else {
                HBRUSH((COLOR_WINDOW.0 + 1) as *mut c_void)
            };

            FillRect(hdc, &ps.rcPaint, brush);
            _ = EndPaint(hwnd, &ps);
        }
    }
    SUCCESS_CODE
}
```

### Implementation Notes

- `WM_ERASEBKGND` eliminates the white flash when the window opens
- `WM_PAINT` ensures repaints use the correct background
- Both functions gracefully fall back to `COLOR_WINDOW` if no style is defined
- Uses existing `get_parsed_style()` infrastructure
- **Child control flash**: Some child controls may still flash white if they paint their own background before their style is applied. Mitigation: define `MainWindowBackground` before `ShowWindow`, and apply all child styles before showing the window (the ordering in Phase 8 already does this).

### Build & Test

```bash
cargo build --workspace
cargo clippy --workspace
```

**Expected Result**: Clean build, no clippy warnings.

**QA Testing**:
1. Run the application
2. Should look identical (no styles defined yet)
3. No white flicker on startup (handled by WM_ERASEBKGND)

**Manual Style Test** (optional, for developers):
Add this to harvester_app's `layout.rs` in `initial_commands()` before control creation:

```rust
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::MainWindowBackground,
    style: ControlStyle {
        background_color: Some(Color { r: 0x2E, g: 0x32, b: 0x39 }),
        ..Default::default()
    },
});
```

**Expected**: Window background turns dark gray (#2E3239). Remove this test code after verification.

---

## Phase 3: Panel Background Support

**Goal**: Enable styled backgrounds for panel controls (PANEL_JOBS, PANEL_INPUT, etc.).

### Files to Modify

#### **`src/CommanDuctUI/src/controls/panel_handler.rs`**

**Change: Enhance forwarding_panel_proc** (modify existing function around line 28):

```rust
unsafe extern "system" fn forwarding_panel_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        // Forward important messages to parent
        if (msg == WM_COMMAND
            || msg == WM_CTLCOLOREDIT
            || msg == WM_CTLCOLORSTATIC
            || msg == WM_NOTIFY
            || msg == WM_ERASEBKGND)  // <-- Add this line
            && let Ok(parent) = GetParent(hwnd)
            && !parent.is_invalid()
        {
            return SendMessageW(parent, msg, Some(wparam), Some(lparam));
        }

        DefWindowProcW(hwnd, msg, Some(wparam), Some(lparam))
    }
}
```

### Implementation Notes

- Panels are STATIC controls with a custom window procedure (`forwarding_panel_proc`)
- **Primary mechanism**: The parent's `WM_CTLCOLORSTATIC` handler already recognizes panels by control_id and returns their background brush. This handles the panel's own surface painting for STATIC controls.
- Forwarding `WM_ERASEBKGND` is added as a supplementary measure for cases where the STATIC control doesn't fully repaint via CTLCOLORSTATIC (e.g., exposed regions after resize).
- The existing `WM_CTLCOLORSTATIC` forwarding already handles child control coloring inside panels.
- **Correctness-by-construction**: Panel window styles are constructed through `PanelWindowStyle::base()` which always sets `WS_CLIPCHILDREN | WS_CHILD | WS_VISIBLE`. This makes it impossible to create a panel without clipping its children, preventing background erases from overpainting the URL drop box or tree view.

### Approach Priority (try in order)

1. **First**: Simply apply the style and verify `WM_CTLCOLORSTATIC` handles the panel's background via the existing `label_handler.rs` lookup.
2. **Second** (if gaps remain): Forward `WM_ERASEBKGND` to parent as shown above.
3. **Third** (if neither works): Handle `WM_PAINT` directly in `forwarding_panel_proc` — use `FillRect` with the applied style's brush. This requires access to `internal_state`/`window_id` (e.g., via `GWLP_USERDATA` on the panel HWND or a global lookup).

### Build & Test

```bash
cargo build --workspace
cargo clippy --workspace
```

**Expected Result**: Clean build.

**QA Testing**:
1. Run the application
2. Should look identical (no panel styles defined yet)
3. Verify URL drop box and tree view remain visible when resizing (WS_CLIPCHILDREN enforced by `PanelWindowStyle`)

**Manual Style Test** (optional):
Add to `layout.rs`:

```rust
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::PanelBackground,
    style: ControlStyle {
        background_color: Some(Color { r: 0x26, g: 0x2A, b: 0x2E }),
        text_color: Some(Color { r: 0xE0, g: 0xE5, b: 0xEC }),
        ..Default::default()
    },
});
// After PANEL_JOBS creation:
commands.push(PlatformCommand::ApplyStyleToControl {
    window_id,
    control_id: PANEL_JOBS,
    style_id: StyleId::PanelBackground,
});
```

**Expected**: PANEL_JOBS turns dark gray (#262A2E). Remove test code after verification.

---

## Phase 4: Labels & Inputs

**Goal**: Verify existing label/input styling works with dark colors.

### Files to Check

- `src/CommanDuctUI/src/controls/label_handler.rs` (WM_CTLCOLORSTATIC handler)
- `src/CommanDuctUI/src/controls/input_handler.rs` (WM_CTLCOLOREDIT handler)

### Implementation Notes

Update the layout commands to define/apply `DefaultText` and `DefaultInput` styles so the existing label/input handlers can render dark colors. The custom draw handlers already:
 - Look up applied styles via `window_data.get_style_for_control(control_id)`
 - Apply `text_color` via `SetTextColor(hdc, color)`
 - Apply `background_color` via `SetBkColor(hdc, color)` and return brush

### Build & Test

**QA Testing**:
1. Confirm `layout.rs` defines `DefaultText` and `DefaultInput` styles (r/g/b: 0x2E/0x32/0x39 and 0x1A/0x1D/0x22 with light text) and applies them to `LABEL_STATUS` and `INPUT_URLS`.
2. Run the application

**Expected Result**:
- Labels render with styled text color on styled background
- Input fields render with styled text and background colors
- Text remains readable against dark backgrounds

**Example Test Code** (add to `layout.rs`):

```rust
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::DefaultText,
    style: ControlStyle {
        background_color: Some(Color { r: 0x2E, g: 0x32, b: 0x39 }),
        text_color: Some(Color { r: 0xE0, g: 0xE5, b: 0xEC }),
        ..Default::default()
    },
});
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::DefaultInput,
    style: ControlStyle {
        background_color: Some(Color { r: 0x1A, g: 0x1D, b: 0x22 }),
        text_color: Some(Color { r: 0xE0, g: 0xE5, b: 0xEC }),
        ..Default::default()
    },
});
// After control creation:
commands.push(PlatformCommand::ApplyStyleToControl {
    window_id, control_id: LABEL_STATUS, style_id: StyleId::DefaultText,
});
commands.push(PlatformCommand::ApplyStyleToControl {
    window_id, control_id: INPUT_URLS, style_id: StyleId::DefaultInput,
});
```

---

## Phase 5: TreeView Styling (Already Implemented)

**Goal**: Verify TreeView dark theme support.

### Files to Check

- `src/CommanDuctUI/src/controls/treeview_handler.rs` (NM_CUSTOMDRAW handler)

### Implementation Notes

**No code changes needed.** The existing `handle_notify_treeview_customdraw()` already:
- Responds to `CDDS_PREPAINT` and `CDDS_ITEMPREPAINT` draw stages
- Looks up applied `TreeView` style via `window_data.get_style_for_control()`
- Applies `text_color` and `background_color` to tree items
- Supports per-item style overrides via `TreeItemDescriptor::style_override`

### Build & Test

**QA Testing**:
1. Add `TreeView` style definition in `layout.rs`
2. Apply to `TREE_JOBS`
3. Run the application, add some jobs

**Expected Result**:
- Tree view background is dark
- Tree item text is light-colored
- Selection highlight remains visible (both when window is focused and unfocused)
- Checkbox states remain clear

**Watch for**: TreeView selection colors in custom draw. Windows uses different highlight colors for focused vs unfocused tree views. With dark backgrounds, the unfocused selection color (usually light gray) may have poor contrast. If so, explicitly set selection bg/fg in `CDDS_ITEMPREPAINT` when the item state includes `CDIS_SELECTED`.

**Example Test Code**:

```rust
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::TreeView,
    style: ControlStyle {
        background_color: Some(Color { r: 0x26, g: 0x2A, b: 0x2E }),
        text_color: Some(Color { r: 0xE0, g: 0xE5, b: 0xEC }),
        ..Default::default()
    },
});
commands.push(PlatformCommand::ApplyStyleToControl {
    window_id, control_id: TREE_JOBS, style_id: StyleId::TreeView,
});
```

---

## Phase 6: Owner-Drawn Buttons

**Goal**: Enable custom drawing for buttons to support dark backgrounds and light text.

### Files to Modify

#### **`src/CommanDuctUI/src/app.rs`**

**Change: Extend execute_apply_style_to_control()** (modify existing function around line 802):

Add button-specific logic when a style is applied to a button control:

```rust
pub(crate) fn execute_apply_style_to_control(
    self: &Arc<Self>,
    window_id: WindowId,
    control_id: ControlId,
    style_id: StyleId,
) -> PlatformResult<()> {
    // ... existing code to store style and get HWND ...

    let parsed_style = self.get_parsed_style(style_id);

    // NEW: Check if this is a button and enable owner-draw
    // (Uses ControlKind stored at creation time — see Cross-Cutting Concern A)
    if window_data.get_control_kind(control_id) == Some(ControlKind::Button) {
        // Only convert to owner-draw if the style provides enough info to render
        if let Some(ref style) = parsed_style {
            if style.background_color.is_some() && style.text_color.is_some() {
                unsafe {
                    let current_style = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
                    let new_style = (current_style & !WINDOW_STYLE(BS_TYPEMASK as u32))
                        | WINDOW_STYLE(BS_OWNERDRAW);
                    SetWindowLongW(hwnd, GWL_STYLE, new_style.0 as i32);
                    // SWP_FRAMECHANGED forces the window to recalculate its frame
                    SetWindowPos(hwnd, None, 0, 0, 0, 0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED)?;
                    InvalidateRect(hwnd, None, TRUE)?;
                }
            }
        }
    }

    // ... existing code for font, TreeView colors, etc. ...
}
```

#### **`src/CommanDuctUI/src/controls/button_handler.rs`**

**Change: Add WM_DRAWITEM handler** (add new public function at end of file):

```rust
use crate::controls::styling_handler::color_to_colorref;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_SELECTED, ODS_FOCUS};

pub(crate) fn handle_wm_drawitem(
    internal_state: &Arc<Win32ApiInternalState>,
    window_id: WindowId,
    draw_item_struct: *const DRAWITEMSTRUCT,
) -> Option<LRESULT> {
    unsafe {
        if draw_item_struct.is_null() {
            return None;
        }
        let dis = &*draw_item_struct;
        let control_id = ControlId::from_raw(dis.CtlID as i32);

        // Get applied style (with fallback to system colors — see Cross-Cutting C)
        let window_data = internal_state.get_window_data(window_id);
        let style_id = window_data.and_then(|wd| wd.get_style_for_control(control_id));
        let style = style_id.and_then(|sid| internal_state.get_parsed_style(sid));

        // Resolve colors: style values or system defaults as fallback
        let base_bg = style.as_ref()
            .and_then(|s| s.background_color.clone())
            .unwrap_or_else(|| colorref_to_color(GetSysColor(COLOR_BTNFACE)));
        let base_fg = style.as_ref()
            .and_then(|s| s.text_color.clone())
            .unwrap_or_else(|| colorref_to_color(GetSysColor(COLOR_BTNTEXT)));

        // Determine final colors based on button state
        let is_disabled = (dis.itemState & ODS_DISABLED).0 != 0;
        let is_pressed = (dis.itemState & ODS_SELECTED).0 != 0;

        let (bg_color, text_color) = if is_disabled {
            // Disabled: use system gray text, keep background
            (base_bg, colorref_to_color(GetSysColor(COLOR_GRAYTEXT)))
        } else if is_pressed {
            // Pressed: darken background by 20%
            let pressed_bg = Color {
                r: (base_bg.r as u32 * 80 / 100) as u8,
                g: (base_bg.g as u32 * 80 / 100) as u8,
                b: (base_bg.b as u32 * 80 / 100) as u8,
            };
            (pressed_bg, base_fg)
        } else {
            (base_bg, base_fg)
        };

        // Fill background
        let brush = CreateSolidBrush(color_to_colorref(&bg_color));
        FillRect(dis.hDC, &dis.rcItem, brush);
        let _ = DeleteObject(brush);

        // Get button text (dynamic length, no hardcoded buffer)
        let text_len = GetWindowTextLengthW(dis.hwndItem);
        let mut text_buf = vec![0u16; (text_len + 1) as usize];
        GetWindowTextW(dis.hwndItem, &mut text_buf);

        // Draw text
        SetTextColor(dis.hDC, color_to_colorref(&text_color));
        SetBkMode(dis.hDC, TRANSPARENT);

        // Apply font if available, saving old font for restoration
        let old_font = style.as_ref()
            .and_then(|s| s.font_handle)
            .map(|font| SelectObject(dis.hDC, font));

        let mut rect = dis.rcItem;
        DrawTextW(
            dis.hDC,
            &text_buf[..text_len as usize],
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );

        // Restore original font to avoid leaking GDI selection state
        if let Some(prev_font) = old_font {
            SelectObject(dis.hDC, prev_font);
        }

        // Draw focus rectangle (inset by 3px; scale by DPI in future)
        if (dis.itemState & ODS_FOCUS).0 != 0 {
            let mut focus_rect = dis.rcItem;
            InflateRect(&mut focus_rect, -3, -3);
            DrawFocusRect(dis.hDC, &focus_rect);
        }

        Some(LRESULT(1)) // TRUE - we handled it
    }
}
```

**Don't forget to add required imports** at the top of `button_handler.rs`:

```rust
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, FillRect, SetTextColor, SetBkMode,
    SelectObject, DrawTextW, DrawFocusRect, InflateRect, GetSysColor,
    TRANSPARENT, COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT,
};
use windows::Win32::UI::WindowsAndMessaging::{DT_CENTER, DT_VCENTER, DT_SINGLELINE};
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_SELECTED, ODS_FOCUS, ODS_DISABLED};
```

**Also add a helper** in `styling_handler.rs` (or `button_handler.rs`):

```rust
/// Convert a Win32 COLORREF (BGR) back to platform-agnostic Color (RGB)
pub(crate) fn colorref_to_color(cr: u32) -> Color {
    Color {
        r: (cr & 0xFF) as u8,
        g: ((cr >> 8) & 0xFF) as u8,
        b: ((cr >> 16) & 0xFF) as u8,
    }
}
```

#### **`src/CommanDuctUI/src/controls/mod.rs`**

**Change: Export button_handler publicly** (modify existing file):

```rust
pub(crate) mod button_handler;  // Make this line public within crate
```

#### **`src/CommanDuctUI/src/window_common.rs`**

**Change: Add WM_DRAWITEM dispatch** in `handle_window_message()` (add case around line 900):

```rust
match msg {
    // ... existing cases ...
    WM_DRAWITEM => {
        let draw_item_struct = lparam.0 as *const DRAWITEMSTRUCT;
        lresult_override = button_handler::handle_wm_drawitem(
            self,
            window_id,
            draw_item_struct,
        );
    }
    WM_CTLCOLORSTATIC => {
        // ... existing code ...
    }
    // ... rest of cases ...
}
```

**Don't forget to add required imports** at the top of `window_common.rs`:

```rust
use windows::Win32::UI::Controls::DRAWITEMSTRUCT;
use crate::controls::button_handler;
```

### Implementation Notes

- Buttons transition to `BS_OWNERDRAW` only when a style provides both background AND text colors
- Owner-drawn buttons still generate `BN_CLICKED` events normally
- Text is dynamically sized via `GetWindowTextLengthW()` (no hardcoded buffers, per Agents.md)
- **ODS_DISABLED**: Renders disabled buttons with system gray text — ensures disabled state is always visible
- **ODS_SELECTED** (pressed): Darkens background by 20% for visual feedback
- **ODS_FOCUS**: Draws inset focus rectangle for keyboard navigation
- **Font restoration**: Old font is saved before `SelectObject` and restored after drawing to avoid leaking GDI selection state
- **Fallback rendering**: If style lookup fails, uses `GetSysColor()` system defaults — button never renders blank
- **SWP_FRAMECHANGED**: Called after changing window style bits to force recalculation
- Focus rect inset (3px) should be DPI-scaled in future enhancement

### Build & Test

```bash
cargo build --workspace
cargo clippy --workspace
```

**Expected Result**: Clean build.

**QA Testing**:
1. Add `DefaultButton` style definition in `layout.rs`
2. Apply to `BUTTON_ARCHIVE` and `BUTTON_STOP`
3. Run the application

**Test Cases**:
- [ ] Buttons render with dark background and light text
- [ ] Clicking a button darkens it briefly (pressed state)
- [ ] Button click events still fire (Archive/Stop functionality works)
- [ ] Keyboard navigation shows focus rectangle
- [ ] Button text updates correctly (if text is changed dynamically)
- [ ] Disabled button (if applicable) renders with gray text, not invisible
- [ ] Button renders correctly at 125% and 150% DPI scaling

**Example Test Code**:

```rust
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::DefaultButton,
    style: ControlStyle {
        background_color: Some(Color { r: 0x2E, g: 0x32, b: 0x39 }),
        text_color: Some(Color { r: 0xE0, g: 0xE5, b: 0xEC }),
        ..Default::default()
    },
});
commands.push(PlatformCommand::ApplyStyleToControl {
    window_id, control_id: BUTTON_STOP, style_id: StyleId::DefaultButton,
});
commands.push(PlatformCommand::ApplyStyleToControl {
    window_id, control_id: BUTTON_ARCHIVE, style_id: StyleId::DefaultButton,
});
```

---

## Phase 7: Progress Bar Theming

**Goal**: Enable custom colors for progress bars (dark track, colored fill).

### Files to Modify

#### **`src/CommanDuctUI/src/app.rs`**

**Change: Extend execute_apply_style_to_control()** (modify existing function):

Add progress bar-specific logic:

```rust
pub(crate) fn execute_apply_style_to_control(
    self: &Arc<Self>,
    window_id: WindowId,
    control_id: ControlId,
    style_id: StyleId,
) -> PlatformResult<()> {
    // ... existing code ...

    // NEW: Check if this is a progress bar
    // (Uses ControlKind stored at creation time — see Cross-Cutting Concern A)
    if window_data.get_control_kind(control_id) == Some(ControlKind::ProgressBar) {
        unsafe {
            // Disable visual styles for this control to enable custom colors
            // IMPORTANT: Use empty strings, not null pointers — null can behave
            // differently across Windows versions.
            let empty = HSTRING::new();
            SetWindowTheme(hwnd, &empty, &empty)?;

            // Apply colors
            if let Some(bg_color) = parsed_style.as_ref().and_then(|s| s.background_color.as_ref()) {
                let colorref = color_to_colorref(bg_color);
                SendMessageW(hwnd, PBM_SETBKCOLOR, None, Some(LPARAM(colorref.0 as isize)));
            }
            if let Some(bar_color) = parsed_style.as_ref().and_then(|s| s.text_color.as_ref()) {
                // Repurpose text_color as bar fill color
                let colorref = color_to_colorref(bar_color);
                SendMessageW(hwnd, PBM_SETBARCOLOR, None, Some(LPARAM(colorref.0 as isize)));
            }
            InvalidateRect(hwnd, None, TRUE)?;
        }
    }

    // ... rest of existing code ...
}
```

**Don't forget to add required imports** at the top of `app.rs`:

```rust
use windows::Win32::UI::Controls::{PBM_SETBKCOLOR, PBM_SETBARCOLOR};
use windows::Win32::UI::Controls::Themes::SetWindowTheme;
use crate::controls::styling_handler::color_to_colorref;
```

### Implementation Notes

- `SetWindowTheme(hwnd, "", "")` with empty strings (not null) disables visual styles, enabling `PBM_SETBKCOLOR`/`PBM_SETBARCOLOR`
- `PBM_SETBKCOLOR` sets the track (background) color
- `PBM_SETBARCOLOR` sets the bar fill color
- **MVP semantic compromise**: We repurpose `text_color` as the bar fill color (progress bar doesn't have visible text). This works for MVP but is semantically awkward.
- **Better long-term**: Add `accent_color: Option<Color>` to `ControlStyle` so progress bar uses `background_color` = track, `accent_color` = fill. This avoids the `text_color` repurposing and gives a unified accent concept for buttons, progress bars, and focus indicators.
- This approach gives solid colors, not gradients (neumorphic glow is a future enhancement)
- Behavior may differ slightly across Windows versions; test on Windows 10 and 11

### Build & Test

```bash
cargo build --workspace
cargo clippy --workspace
```

**Expected Result**: Clean build.

**QA Testing**:
1. Add `ProgressBar` style definition in `layout.rs`
2. Apply to `PROGRESS_TOKENS`
3. Run the application, paste URLs to see progress

**Test Cases**:
- [ ] Progress bar track is dark (#1A1D22)
- [ ] Progress bar fill is cyan (#00C9FF)
- [ ] Progress updates smoothly as jobs complete
- [ ] Progress bar matches the dark theme aesthetic

**Example Test Code**:

```rust
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::ProgressBar,
    style: ControlStyle {
        background_color: Some(Color { r: 0x1A, g: 0x1D, b: 0x22 }), // Track
        text_color: Some(Color { r: 0x00, g: 0xC9, b: 0xFF }),       // Bar fill
        ..Default::default()
    },
});
commands.push(PlatformCommand::ApplyStyleToControl {
    window_id, control_id: PROGRESS_TOKENS, style_id: StyleId::ProgressBar,
});
```

---

## Phase 8: Harvester App Integration (Full Dark Theme)

**Goal**: Define and apply every dark-theme style in the Harvester layout so that the CommanDuctUI controls render with the new palette.

### Files to Modify

#### **`crates/harvester_app/src/platform/ui/layout.rs`**

**Change: Encapsulate theme definitions and application**  
- Add `define_dark_theme_styles()` and `apply_dark_theme()` helpers: the first fires before control creation and registers every `StyleId` (window, panel, labels, inputs, buttons, tree view, viewer, progress bar); the second runs immediately after control creation so every HWND is styled before the layout equations run.  
- `define_dark_theme_styles()` now requires the `FontDescription`/`FontWeight` imports because the viewer preview uses Cascadia Code with cyan text.  
- `initial_commands()` simply calls the helpers, then proceeds with the existing `Create*` commands, and finally defines the layout + shows the window.  
- `define_dark_theme_styles()` uses the color values from the plan: `MainWindowBackground` (#2E3239), `PanelBackground` (#262A2E with light text), `StatusBarBackground` (#2E3239 with muted text), `HeaderLabel` (#FFB347), `DefaultInput` (#1A1D22), `DefaultButton`, `TreeView`, `ViewerMonospace` (with the font), and `ProgressBar` (#1A1D22 track + #00C9FF fill).
- `apply_dark_theme()` plugs each control into the correct style: panels use `PanelBackground`, the bottom panel and status label use `StatusBarBackground`, headers use `HeaderLabel`, inputs/viewer/button/tree/progress use their respective styles.

### Implementation Notes

- Styles are always defined before any `Create*` commands, keeping the definition order predictable even if more variants are added later.
- The layout dance remains untouched: all controls are created, the dark theme is applied, and then the `DefineLayout`/`SignalMainWindowUISetupComplete`/`ShowWindow` sequence runs.
- Helpers keep `initial_commands()` readable and make future theme swaps trivial (just change the helpers instead of editing the command waterfall).

### Build & Test

```bash
cargo build --workspace
cargo clippy --workspace
cargo fmt --workspace
```

**Expected Result**: Clean build, no warnings.

**QA Testing - Full Regression Test**:

1. **Window Appearance**
   - [ ] Window background is dark gray (#2E3239)
   - [ ] No white flicker on startup
   - [ ] All panels are slightly darker gray (#262A2E)

2. **Labels**
    - [ ] Header labels (Preview, Job List, Input hint) are amber (#FFB347)
   - [ ] Status labels are light gray (#E0E5EC)
   - [ ] All text is readable against dark backgrounds
   - [ ] Token progress label updates correctly

3. **Input Fields**
   - [ ] URL input field is very dark (#1A1D22) with light text
   - [ ] Preview viewer is very dark with cyan text (#00C9FF)
   - [ ] Preview viewer uses Cascadia Code font (monospace)
   - [ ] Text input works normally (paste, type, select)

4. **Buttons**
   - [ ] Archive and Stop buttons are dark with light text
   - [ ] Buttons darken when clicked (pressed state)
   - [ ] Button click events fire correctly
   - [ ] Archive button opens file dialog
   - [ ] Stop button halts processing

5. **TreeView**
   - [ ] Job list background is dark (#262A2E)
   - [ ] Job items are light text (#E0E5EC)
   - [ ] Selection highlight is visible
   - [ ] Checkbox states are clear
   - [ ] Clicking items selects them
   - [ ] Toggling checkboxes works

6. **Progress Bar**
   - [ ] Track is dark (#1A1D22)
   - [ ] Fill bar is cyan (#00C9FF)
   - [ ] Progress updates smoothly
   - [ ] Percentage matches visual fill

7. **Functional Testing**
   - [ ] Paste URLs → Jobs appear in tree
   - [ ] Select job → Preview updates
   - [ ] Click Archive → Saves to file
   - [ ] Click Stop → Processing halts
   - [ ] Resume from saved state works

8. **Disabled State Testing**
   - [ ] Disabled button (if any) renders with visible gray text
   - [ ] Disabled controls don't disappear against dark background

9. **DPI / Scaling Testing**
   - [ ] Test at 100%, 125%, 150% DPI scaling
   - [ ] Text remains readable and centered at all scales
   - [ ] Focus rectangles and button padding look proportional

10. **Windows Theme Variations**
    - [ ] Test with Windows dark mode enabled in system settings
    - [ ] Test with Windows light mode — ensure custom theme overrides system
    - [ ] Verify app doesn't depend on any specific system theme state

### Performance Testing

- [ ] Startup time is not significantly slower
- [ ] UI remains responsive during heavy processing
- [ ] Memory usage is not significantly higher (styles are Arc-shared)

---

## Testing Strategy

### Unit Tests (cheap, high ROI)

These can be implemented without UI automation:

1. **Color conversion**: `color_to_colorref()` and `colorref_to_color()` round-trip correctly
2. **Pressed-darken logic**: Pure function test that 20% darkening produces correct RGB values
3. **Style parsing**: Verify `define_style()` produces correct `ParsedControlStyle` fields (font present when specified, brush present when color specified)
4. **Mapping correctness**: Verify `apply_style_to_control()` updates `window_data.applied_styles` map correctly

```rust
#[test]
fn test_darken_by_20_percent() {
    let color = Color { r: 100, g: 200, b: 50 };
    let darkened = darken_color(&color, 80); // 80% of original
    assert_eq!(darkened.r, 80);
    assert_eq!(darkened.g, 160);
    assert_eq!(darkened.b, 40);
}

#[test]
fn test_color_to_colorref_roundtrip() {
    let original = Color { r: 0x2E, g: 0x32, b: 0x39 };
    let cr = color_to_colorref(&original);
    let back = colorref_to_color(cr.0);
    assert_eq!(original, back);
}
```

### Integration Tests (without full UI automation)

Add an internal "diagnostic dump" (behind `#[cfg(test)]` or a feature flag) that returns, for each control: applied StyleId, resolved colors, and whether owner-draw is enabled. This can be verified in a headless test by driving the command pipeline without creating real windows.

---

## Future Enhancements

### 1. Neumorphic Soft Shadows

**Goal**: Add the soft shadow effect from DarkTheme.md for tactile depth.

**Approach**: Extend `ControlStyle` with shadow properties:

```rust
pub struct ControlStyle {
    pub font: Option<FontDescription>,
    pub text_color: Option<Color>,
    pub background_color: Option<Color>,
    // New fields:
    pub highlight_color: Option<Color>,   // Top-left soft shadow (light)
    pub shadow_color: Option<Color>,      // Bottom-right soft shadow (dark)
    pub shadow_blur: Option<i32>,         // Blur radius in pixels
    pub border_radius: Option<i32>,       // Corner rounding
}
```

**Implementation**:
- Use GDI+ `Graphics::FillRoundedRectangle()` with gradient brushes
- Draw two offset shadows (highlight + shadow) before drawing the main fill
- Apply to buttons, panels, and progress bars

**Challenges**:
- GDI+ has steeper learning curve than GDI
- Blurred shadows require alpha blending
- Performance impact on older hardware

### 2. Progress Bar Glow Effect

**Goal**: Add a soft glow to the progress bar fill to simulate "neon liquid."

**Approach**: Use GDI+ `PathGradientBrush` with transparency:

```rust
// Create gradient from center (opaque cyan) to edges (transparent)
let path = GraphicsPath::new();
path.AddRectangle(bar_rect);
let brush = PathGradientBrush::new(path);
brush.SetCenterColor(Color::Cyan);
brush.SetSurroundColors(&[Color::Transparent]);
```

**Benefit**: Makes the progress bar more visually striking.

### 3. Dynamic Theme Switching

**Goal**: Allow users to toggle between light/dark themes at runtime.

**Approach**:
- Add `SetTheme` command that applies a predefined set of styles
- Store theme preference in persistence layer
- Emit `DefineStyle` + `ApplyStyleToControl` commands on theme change
- Call `InvalidateRect` on all controls to trigger repaint

**Example**:

```rust
pub enum ThemePreset {
    Light,
    Dark,
}

commands.push(PlatformCommand::SetTheme {
    window_id,
    theme: ThemePreset::Dark,
});
```

Suggestion: When a MainWindowBackground style is redefined at runtime we could explicitly InvalidateRect/RedrawWindow the main window and call WM_ERASEBKGND to ensure the new brush is used before WM_PAINT runs—this avoids waiting for the next native repaint and keeps the transition smooth.

### 4. Custom TreeView Checkboxes

**Goal**: Replace system checkboxes with styled dark theme checkboxes.

**Approach**:
- Implement custom drawing in `NM_CUSTOMDRAW` handler
- Draw checkbox rect with dark border and background
- Draw checkmark using `DrawText` with Wingdings font or custom path

**Benefit**: More cohesive visual style.

### 5. Hover States for Buttons

**Goal**: Highlight buttons on mouse hover.

**Approach**:
- Handle `WM_MOUSEMOVE` and `WM_MOUSELEAVE` messages
- Track hovered control in window state
- Lighten button background by 10% during hover
- Call `InvalidateRect` to repaint

**Benefit**: Improved user feedback.

### 6. Animated Transitions

**Goal**: Smooth color transitions (e.g., button press, theme switch).

**Approach**:
- Use `WM_TIMER` to interpolate colors over time
- Store animation state in window data
- Repaint control at each timer tick with interpolated color

**Benefit**: More polished, modern feel.

### 7. High DPI Support

**Goal**: Ensure shadow sizes, border radii, and fonts scale with DPI.

**Approach**:
- Call `GetDpiForWindow()` on startup
- Scale all pixel values by DPI factor (e.g., 150% = 144 DPI)
- Store DPI in window data, query on every style application

**Benefit**: Crisp rendering on 4K displays.

### 8. Configuration File for Themes

**Goal**: Allow users to define custom themes without recompiling.

**Approach**:
- Create `themes.toml` or `themes.json` config file
- Parse theme definitions on startup
- Convert to `DefineStyle` commands dynamically

**Example `themes.toml`**:

```toml
[themes.dark]
MainWindowBackground = { bg = "#2E3239" }
PanelBackground = { bg = "#262A2E", fg = "#E0E5EC" }
DefaultButton = { bg = "#2E3239", fg = "#E0E5EC" }
```

**Benefit**: Non-developers can customize colors.

### 9. Accessibility: High Contrast Mode

**Goal**: Support Windows high contrast mode for visually impaired users.

**Approach**:
- Detect `SPI_GETHIGHCONTRAST` system parameter
- If enabled, ignore custom styles and use system colors
- Ensure all UI remains functional

**Benefit**: Compliance with accessibility standards.

### 10. Unified Accent Color

**Goal**: Single accent color used by progress bar fill, selected tree item highlight, focus rectangle, and button hover.

**Approach**:
- Add `accent_color: Option<Color>` to `ControlStyle`
- Progress bar uses it for bar fill (replacing `text_color` repurposing)
- Button focus rect uses it for the border color
- TreeView selected item uses it for background highlight
- Defined once in theme, applied consistently everywhere

**Benefit**: Visual consistency and simpler theme authoring.

### 11. Style Inheritance

**Goal**: Reduce repetition in style definitions by allowing styles to inherit from a base.

**Approach**:
- Add `inherits_from: Option<StyleId>` field to `ControlStyle` (or resolve at definition time)
- e.g., `HeaderLabel` inherits from `DefaultText` but overrides `text_color`
- Resolution happens in `define_style()` — merge parent fields with child overrides

**Benefit**: DRY theme definitions, easier maintenance.

### 12. Per-State Styling

**Goal**: Separate colors for hover, pressed, disabled, and focused states.

**Approach**:
- Add optional per-state color fields to `ControlStyle`:
  ```rust
  pub hover_background: Option<Color>,
  pub pressed_background: Option<Color>,
  pub disabled_text_color: Option<Color>,
  ```
- Keep defaults derived from base colors (e.g., hover = lighten 10%, pressed = darken 20%)

**Benefit**: Fine-grained control without hardcoded percentage adjustments.

### 13. Accessibility Contrast Checker

**Goal**: Warn at development time if foreground/background contrast ratio is too low.

**Approach**:
- In `define_style()`, calculate WCAG contrast ratio between `text_color` and `background_color`
- Log a warning via `engine_logging` if ratio < 4.5:1 (AA standard)
- Only active in debug builds (`#[cfg(debug_assertions)]`)

**Benefit**: Prevents accessibility regressions, catches hard-to-read color combinations early.

### 14. Theme Presets as Data

**Goal**: Ship a default `themes.toml` but still allow hard-coded fallback.

**Approach**: The existing `themes.toml` future idea (item 8), combined with a compiled-in default that's used when the file isn't present.

**Benefit**: Users can customize without code, but the app always has a working theme.

### 15. Unit Tests for Styling

**Goal**: Lock in styling behavior with unit tests.

**Approach**:
- Test `define_style()` creates correct HFONT and HBRUSH
- Test `execute_apply_style_to_control()` stores correct mappings
- Test `get_parsed_style()` retrieves correct style
- Mock `CreateFontW`, `CreateSolidBrush` to verify correct calls

**Example Test**:

```rust
#[test]
fn test_define_style_creates_font() {
    let state = Win32ApiInternalState::new();
    let style = ControlStyle {
        font: Some(FontDescription {
            name: Some("Arial".to_string()),
            size: Some(12),
            weight: Some(FontWeight::Bold),
        }),
        ..Default::default()
    };
    state.define_style(StyleId::DefaultText, style).unwrap();
    let parsed = state.get_parsed_style(StyleId::DefaultText).unwrap();
    assert!(parsed.font_handle.is_some());
}
```

**Benefit**: Prevents regressions during refactoring.

---

## Rollback Plan

If critical bugs are discovered after deployment:

1. **Immediate Rollback**: Comment out all `DefineStyle` and `ApplyStyleToControl` commands in `layout.rs`. Application reverts to system default appearance.

2. **Per-Phase Rollback**: If a specific phase (e.g., owner-drawn buttons) causes issues, comment out the corresponding changes in CommanDuctUI. Styles for other controls remain active.

3. **Git Revert**: If the entire implementation must be reverted:
   ```bash
   git revert <commit-hash>
   cargo build --workspace
   ```

---

## Version Update & Changelog

Per Agents.md, since CommanDuctUI is a git submodule:

### **`src/CommanDuctUI/Cargo.toml`**

Update version number:

```toml
[package]
name = "commanductui"
version = "0.3.0"  # Increment minor version (non-breaking additions)
```

### **`src/CommanDuctUI/CHANGELOG.md`** (create if doesn't exist)

```markdown
# Changelog

## [0.3.0] - 2026-01-XX

### Added
- Dark theme support via optional styling commands
- `ControlKind` enum stored per control for deterministic style dispatch
- `WM_ERASEBKGND` handler for main window to enable custom backgrounds
- `WM_PAINT` handler respects `MainWindowBackground` style
- Owner-drawn button support via `WM_DRAWITEM` handler with ODS_DISABLED/ODS_SELECTED/ODS_FOCUS states
- Progress bar theming via `SetWindowTheme` and color messages
- New `StyleId` variants: `HeaderLabel`, `ProgressBar`
- Panel background styling via `WM_CTLCOLORSTATIC` + `WM_ERASEBKGND` forwarding
- `colorref_to_color()` helper for Win32 COLORREF to platform-agnostic Color conversion
- Fallback rendering for owner-drawn buttons (system colors when no style defined)

### Changed
- Buttons transition to `BS_OWNERDRAW` only when a style provides both background AND text colors (non-breaking)
- Progress bars disable visual styles when a style is applied (non-breaking)

### Fixed
- None

### Breaking Changes
- None (all changes are opt-in via commands)
```

---

## Summary

This plan delivers a complete flat dark theme for CommanDuctUI and the Harvester application. Each phase is independently testable and leaves the application in a runnable state. Future enhancements (neumorphic shadows, animations, etc.) can be layered on incrementally without disrupting the core implementation.
