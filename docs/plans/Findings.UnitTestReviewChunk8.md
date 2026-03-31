# Chunk 8 Unit Test Review Findings

Reviewed scope:
- `src/CommanDuctUI/src/app.rs`
- `src/CommanDuctUI/src/command_executor.rs`
- `src/CommanDuctUI/src/types.rs`
- `src/CommanDuctUI/src/window_common.rs`
- `src/CommanDuctUI/src/controls/button_handler.rs`
- `src/CommanDuctUI/src/controls/chart_handler.rs`
- `src/CommanDuctUI/src/controls/checkbox_handler.rs`
- `src/CommanDuctUI/src/controls/combobox_handler.rs`
- `src/CommanDuctUI/src/controls/dialog_handler.rs`
- `src/CommanDuctUI/src/controls/label_handler.rs`
- `src/CommanDuctUI/src/controls/menu_handler.rs`
- `src/CommanDuctUI/src/controls/paint_router.rs`
- `src/CommanDuctUI/src/controls/panel_handler.rs`
- `src/CommanDuctUI/src/controls/radiobutton_handler.rs`
- `src/CommanDuctUI/src/controls/tab_bar_handler.rs`
- `src/CommanDuctUI/src/controls/treeview_handler.rs`

Review standard:
- keep `CommanDuctUI` generic infrastructure
- avoid Harvester-specific assumptions
- prefer stable control, routing, and layout behavior over implementation-detail checks
- avoid exact literal assertions unless they defend a real framework contract

## Findings

### 1. `menu_handler.rs` tests use app-flavored menu fixtures instead of generic infrastructure fixtures

**Files:** `src/CommanDuctUI/src/controls/menu_handler.rs:197-238`, `src/CommanDuctUI/src/controls/menu_handler.rs:241-258`

The menu handler behavior under test is generic:
- recursive menu registration assigns native ids
- `WM_COMMAND` round-trips back to `AppEvent::MenuActionClicked`

But the tests express that behavior with product-flavored action ids and labels:
- `LOAD_PROFILE_ID`
- `SAVE_PROFILE_AS_ID`
- `REFRESH_FILE_LIST_ID`

That makes the suite read like app behavior even though the infrastructure contract is simply “arbitrary semantic menu action ids survive registration and dispatch”.

**Recommendation:** Replace those fixtures with neutral ids and labels. Keep the semantic round-trip assertions, but remove app-shaped terminology from the test inputs.

### 2. `window_common.rs` and `app.rs` pin exact diagnostic/error text instead of the underlying behavior

**Files:** `src/CommanDuctUI/src/window_common.rs:3179-3249`, `src/CommanDuctUI/src/app.rs:1628-1647`

Several tests are useful in intent but too coupled to the current wording of diagnostics:
- layout validation tests assert message fragments like `multiple DockStyle::Fill children`, `without fixed_size`, and `negative fixed_size`
- `describe_hwnd_resolves_registered_control` checks exact description fragments including the current `window_id` and formatting shape

The durable contract is:
- invalid layouts are rejected for the right reason category
- registered controls can be described as control targets with their logical metadata

Exact diagnostic wording is not usually the API contract for a generic UI framework. These tests will fail on harmless wording cleanup or debug-format changes.

**Recommendation:** Assert structured behavior when possible, or at least relax the checks to category-level semantics instead of exact phrase fragments and formatting details.

### 3. `tab_bar_handler.rs` freezes current default theme literals more than tab behavior

**Files:** `src/CommanDuctUI/src/controls/tab_bar_handler.rs:721-726`

`tab_bar_palette_default_uses_dark_theme_colors` asserts exact default channel literals:
- background red channel `0x2E`
- active text red channel `0xE0`
- accent blue channel `0xFF`

This is a low-stability test unless the exact palette is itself a versioned framework contract. For generic infrastructure, the stronger contract is usually:
- default palette remains dark-theme-friendly
- active/inactive text remain distinguishable
- accent and hover colors derive predictably

The neighboring derivation test is much better because it protects palette behavior rather than a particular shipped constant set.

**Recommendation:** Keep the derivation test. Replace the literal default-palette check with semantic assertions about contrast, dark background range, and preserved accent behavior unless exact colors are intentionally contractual.

## Keep As-Is

These suites and modules are mostly aligned with the preferred review standard:
- most of `src/CommanDuctUI/src/app.rs`
- `src/CommanDuctUI/src/command_executor.rs`
- `src/CommanDuctUI/src/types.rs`
- most of `src/CommanDuctUI/src/window_common.rs`
- `src/CommanDuctUI/src/controls/button_handler.rs`
- most of `src/CommanDuctUI/src/controls/chart_handler.rs`
- `src/CommanDuctUI/src/controls/checkbox_handler.rs`
- `src/CommanDuctUI/src/controls/combobox_handler.rs`
- `src/CommanDuctUI/src/controls/dialog_handler.rs`
- `src/CommanDuctUI/src/controls/label_handler.rs`
- `src/CommanDuctUI/src/controls/paint_router.rs`
- `src/CommanDuctUI/src/controls/panel_handler.rs`
- `src/CommanDuctUI/src/controls/radiobutton_handler.rs`
- most of `src/CommanDuctUI/src/controls/tab_bar_handler.rs`
- `src/CommanDuctUI/src/controls/treeview_handler.rs`

Why:
- they primarily test generic control behavior, hit testing, geometry, mapping, routing, state preservation, or error handling
- most assertions are framework-level and do not depend on Harvester domain concepts
- the stronger tests in this chunk protect stable UI infrastructure contracts rather than app internals

## Follow-Up Actions For This Chunk

- Replace app-flavored menu fixtures in `menu_handler.rs` with neutral semantic ids and labels.
- Relax `window_common.rs` and `app.rs` diagnostic-string tests to category-level behavior.
- Replace the exact default tab-bar palette literal test with a semantic dark-theme contract test.
