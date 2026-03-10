Now I have enough context to evaluate all four review suggestions against the actual code. Here is my analysis:

1. **Clear `CDIS_SELECTED`/`CDIS_FOCUS` (Suggestion 1):** Valid. The NMCUSTOMDRAW docs do allow modifying `uItemState`. This avoids the tree-lines/buttons erasure risk. The plan's claim that this is "undocumented" is wrong — accept.

2. **Full-row background via `text_rect.right → client_rect.right` (Suggestion 2):** Valid complement to Suggestion 1. Windows fills the text area via `clrTextBk`; we extend rightward with `FillRect`. Accept.

3. **`CDRF_NEWFONT` must fire on color changes (Suggestion 3):** Confirmed by code at line 1068–1074 — `CDRF_NEWFONT` is gated on a font being set. Colors alone will be silently ignored. Accept.

4. **Anchor accent bar to client x=0 (Suggestion 4):** Valid. Using `rect.left` from `TVM_GETITEMRECT WPARAM(0)` would stair-step inward with indentation. Accept.

---

# Plan: Custom TreeView Selection Styling

## Context

The TreeView's selected row currently uses the Windows native blue highlight, which clashes with the dark theme (`#262A2E` background). The goal is to paint selected rows with a theme-matching background color and add a thin left accent bar, creating a polished, modern selection indicator.

## Diary Draft

**Context:** Native Win32 TreeView selection highlight clashes with the app's dark theme.
**Change:** CommanDuctUI treeview custom draw extended with opt-in selection styling (background + accent bar). harvester_app defines the dark-theme selection colors.

## Approach: Two New `StyleId` Variants

Follow the existing `TabBar` / `TabBarAccent` pattern — add `TreeViewSelectedRow` and `TreeViewSelectionAccent` to `StyleId`.

- `TreeViewSelectedRow`: `background_color` = selected row bg, `text_color` = selected text color
- `TreeViewSelectionAccent`: `background_color` = accent bar color

No new types, commands, or parsing logic needed.

**Opt-in semantics:** If `TreeViewSelectedRow` is defined, selected items get a visually distinct background + accent bar. If not defined, selected items look identical to unselected items (current behavior — no regression).

## Files to Modify

| File | Change |
|------|--------|
| `src/CommanDuctUI/src/styling_primitives.rs` | Add two `StyleId` variants |
| `src/CommanDuctUI/src/controls/treeview_handler.rs` | Custom draw: selection bg + accent bar + testable helper |
| `src/CommanDuctUI/CHANGELOG.md` | Version 0.8.0 entry (new StyleId = breaking) |
| `src/CommanDuctUI/Cargo.toml` | Bump to 0.8.0 |
| `crates/harvester_app/src/platform/ui/layout.rs` | Define the two selection styles |

## Implementation Steps

### Step 1: `styling_primitives.rs` — Add StyleId variants

Add after `TreeItemDisabled`:

```rust
TreeViewSelectedRow,
TreeViewSelectionAccent,
```

### Step 2: `treeview_handler.rs` — Custom draw changes

**Imports to add:**
- `FillRect` from `Graphics::Gdi`
- `GetClientRect` from `UI::WindowsAndMessaging`
- `CDIS_SELECTED`, `CDIS_FOCUS` from `UI::Controls`

**New constant:**
```rust
const SELECTION_ACCENT_WIDTH: i32 = 3;
```

#### Testable color resolution helper

Extract a pure function for color precedence logic (no Win32 dependency):

```rust
/// Resolves text and background colors for a TreeView item, accounting for
/// base style, per-item override, and selection state.
///
/// Returns (text_color, background_color) as optional Color values.
fn resolve_item_colors(
    base_text: Option<&Color>,
    base_bg: Option<&Color>,
    override_text: Option<&Color>,
    override_bg: Option<&Color>,
    is_selected: bool,
    selection_text: Option<&Color>,
    selection_bg: Option<&Color>,
) -> (Option<Color>, Option<Color>) {
    // 1. Start with base style
    let mut text = base_text.cloned();
    let mut bg = base_bg.cloned();

    // 2. Per-item override wins over base
    let has_item_text_override = override_text.is_some();
    if let Some(c) = override_text { text = Some(c.clone()); }
    if let Some(c) = override_bg { bg = Some(c.clone()); }

    // 3. Selection: bg always wins; text wins unless per-item override set custom text
    if is_selected {
        if let Some(c) = selection_bg { bg = Some(c.clone()); }
        if !has_item_text_override {
            if let Some(c) = selection_text { text = Some(c.clone()); }
        }
    }

    (text, bg)
}
```

This is unit-testable without Win32.

#### CDDS_ITEMPREPAINT changes

Restructure the color application block (lines 1004–1047) to use the helper:

1. Look up base style, per-item override, and selection style.
2. Detect selection: `let is_selected = (nmtvcd.nmcd.uItemState.0 & CDIS_SELECTED.0) != 0;`
3. Call `resolve_item_colors(...)` to get final text/bg colors.
4. Apply resolved colors to `nmtvcd.clrText` / `nmtvcd.clrTextBk`.
5. Track whether any color was modified (for `CDRF_NEWFONT` gating below).
6. If selected AND `TreeViewSelectedRow` style exists:
   - **Suppress native highlight and focus rect:** clear `CDIS_SELECTED` and `CDIS_FOCUS` from `uItemState`:
     ```rust
     nmtvcd.nmcd.uItemState.0 &= !CDIS_SELECTED.0;
     nmtvcd.nmcd.uItemState.0 &= !CDIS_FOCUS.0;
     ```
     This is documented `NMCUSTOMDRAW` behavior — Windows allows modifying `uItemState` before returning from the notification. Windows then draws text using our `clrText`/`clrTextBk` without a native blue highlight or dotted focus rectangle, and without touching tree lines or expand/collapse buttons.
   - `nmtvcd.clrTextBk` is already set to the selection background color (step 4). Windows paints the text area using this color automatically.
   - For full-row fill (extending background rightward past the text): get the text rect via `TVM_GETITEMRECT` with `WPARAM(1)`, then get the control's client rect via `GetClientRect`. `FillRect` the strip from `text_rect.right` to `client_rect.right`. This leaves the left side (tree lines, buttons, indentation area) untouched.
   - Request `CDRF_NOTIFYPOSTPAINT` (for accent bar drawing in postpaint).
7. **`CDRF_NEWFONT` fix:** Return `CDRF_NEWFONT` whenever *either* the font was selected *or* any color (`clrText`/`clrTextBk`) was modified from the default. Without this flag, Windows ignores the modified color values. The current code only sets `CDRF_NEWFONT` when a font handle is present — this must be extended.

**Why clear `CDIS_SELECTED` rather than FillRect-over-full-row:** Painting a solid fill over the full row bounding rect (`TVM_GETITEMRECT WPARAM(0)`) during prepaint erases tree lines and expand/collapse buttons rendered by `TVS_HASLINES`/`TVS_HASBUTTONS`. The documented approach — clearing `CDIS_SELECTED` in `uItemState` and setting `clrTextBk` — tells Windows to draw the item normally with our chosen background, preserving all native chrome. For the small rightward strip beyond the text, a targeted `FillRect` is safe because that region contains no native elements.

#### CDDS_ITEMPOSTPAINT changes (after marker drawing, ~line 1137)

1. Query selected item: `TVM_GETNEXTITEM` with `TVGN_CARET`, compare with current `h_item_native`.
2. If selected AND `TreeViewSelectionAccent` style exists:
   - Get the control's client rect via `GetClientRect`.
   - `FillRect` a thin rect anchored to the left edge of the client area: `{ left: 0, top: item_rect.top, right: SELECTION_ACCENT_WIDTH, bottom: item_rect.bottom }`.
   - `item_rect` can come from `TVM_GETITEMRECT WPARAM(0)` for the top/bottom bounds; only `left` and `right` are overridden.
   - Use `CreateSolidBrush` / `DeleteObject` (same pattern as `draw_tree_item_marker`).
   
   **Why anchor to x=0:** Using `rect.left` from `TVM_GETITEMRECT WPARAM(0)` would stair-step inward with item indentation level, placing the accent over tree lines for nested items. A fixed `x=0` keeps the accent pinned to the left edge of the panel regardless of depth, matching standard sidebar UI patterns.

### Step 3: `layout.rs` — Define selection styles

In `define_dark_theme_styles`:

```rust
// Selected row: subtle lighter background
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::TreeViewSelectedRow,
    style: ControlStyle {
        background_color: Some(Color { r: 0x37, g: 0x3E, b: 0x47 }), // #373E47
        text_color: Some(Color { r: 0xE0, g: 0xE5, b: 0xEC }),       // same as base
        ..Default::default()
    },
});

// Accent bar: blue accent matching TabBar accent
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::TreeViewSelectionAccent,
    style: ControlStyle {
        background_color: Some(Color { r: 0x00, g: 0x80, b: 0xFF }), // #0080FF
        ..Default::default()
    },
});
```

### Step 4: Version bump + CHANGELOG

- `Cargo.toml`: bump to `0.8.0`
- `CHANGELOG.md`: add entry noting new `StyleId` variants (BREAKING) and custom draw selection rendering

## Testing & Verification

### Unit tests for `resolve_item_colors`

Add tests in `treeview_handler.rs` (or a dedicated test module):

1. **No styles defined** → returns `(None, None)`
2. **Base style only** → returns base colors regardless of selection
3. **Base + selected, no selection style** → returns base colors (no visual distinction)
4. **Base + selected, selection style defined** → returns selection bg, selection text
5. **Base + per-item override (disabled) + selected, selection style** → returns selection bg, but **per-item text** (muted text preserved)
6. **Base + per-item override (disabled) + not selected** → returns per-item override colors

### Visual verification

Run app, select items, verify:
- Custom dark background replaces native blue highlight on selected row
- Background extends full width to the right edge of the control
- Blue accent bar on the left edge of the panel (3px, fixed at x=0, does not indent)
- No tree lines or expand/collapse buttons erased
- Text readable, markers still render on selected rows
- Bold "new" font still works on selected rows
- Disabled/muted items retain muted text color when selected (bg changes, text stays muted)
- No dotted focus rectangle on selected item
- Keyboard navigation (arrow keys) updates selection styling
- Scroll with selection active — accent bar tracks correctly

### Regression verification

- Comment out the two `DefineStyle` calls → verify selected items look the same as unselected (current behavior, no crash)
- Build: `cargo clippy --all-targets -- -D warnings`

## Notes

**On the original `FillRect`-over-full-row approach:** The original plan proposed calling `FillRect` over the full row bounding box during `CDDS_ITEMPREPAINT` to cover the native highlight. This was revised because `TVM_GETITEMRECT WPARAM(0)` includes the indentation area where `TVS_HASLINES`/`TVS_HASBUTTONS` renders tree lines and expand/collapse buttons, and painting over it would erase them. The documented approach of clearing `CDIS_SELECTED` in `uItemState` is both safer and simpler.

**On `CDRF_SKIPDEFAULT`:** The original plan mentioned `CDRF_SKIPDEFAULT` as a potential fix for the focus rectangle. This is unnecessary and harmful: `CDRF_SKIPDEFAULT` suppresses all Windows rendering for the item (text, icons, checkboxes). Clearing `CDIS_FOCUS` from `uItemState` cleanly removes the dotted focus rect without side effects.

**On `CDRF_NEWFONT`:** The existing code only returns `CDRF_NEWFONT` when a font handle is present. Per MSDN, Windows ignores changes to `clrText`/`clrTextBk` unless `CDRF_NEWFONT` is in the return value. The fix must extend the `CDRF_NEWFONT` condition to cover any color modification, not just font changes.

## Future Ideas

- **Hover highlight**: Similar approach — track `WM_MOUSEMOVE` / hot-tracking, add `TreeViewHoverRow` StyleId, paint subtle hover background in prepaint
- **Focus vs selection distinction**: Different bg for focused-selected vs unfocused-selected (when another control has focus)
- **Animated transitions**: Smooth fade between normal/selected states (would need timer-based invalidation)
- **Configurable accent width**: Expose via `ControlStyle` or a dedicated config command
- **Selection accent on other controls**: Reuse pattern for list views or other selectable controls