# Plan: CheckBox Control for Prompt Lab Section Toggles

## Draft Diary Entry
Type: Implementation
Context: Prompt Lab section expand/collapse controls (`Compare`, `Context`, `Templates`, `Run details`) are currently implemented as radio buttons even though they are independent booleans. This creates semantic mismatch and has repeatedly caused dark-theme regressions when adding button-like controls.
Change: `commanductui` (new CheckBox control + dark-mode classic-render helper + paint routing + event), `harvester_app` (Prompt Lab section control creation/render/event wiring + tests).

---

## Review Summary (Against Current Code)

### Findings
1. **Semantic mismatch in current implementation**  
   `crates/harvester_app/src/platform/ui/layout.rs` creates Prompt Lab section toggles with `CreateRadioButton { group_start: true }`, but each section is independently open/closed.

2. **Dark-mode styling is still split and easy to miss**  
   `src/CommanDuctUI/src/controls/radiobutton_handler.rs` enables dark mode on create, while `src/CommanDuctUI/src/app.rs` separately forces classic rendering (`SetWindowTheme("", "")`) during style application. This split is fragile.

3. **Routing is currently radio-button specific**  
   `src/CommanDuctUI/src/controls/paint_router.rs` routes `ControlKind::RadioButton` for both `WM_CTLCOLORBTN` and `WM_CTLCOLORSTATIC`, but there is no CheckBox kind yet.

4. **Existing tests that must be updated are broader than layout/render only**  
   `crates/harvester_app/src/platform/app.rs` contains event-handler tests asserting `AppEvent::RadioButtonSelected` for section toggles, so migration requires test rewiring there too.

### Blockers
- No functional blocker found.  
- Main risk is regression from partial migration (new CheckBox path added but old radio-path references left in tests/handlers/style application).

---

## Architecture/Robustness Decisions

1. Preserve UDF: UI click -> `AppEvent::CheckBoxToggled` -> app handler dispatches reducer `Msg::*SectionToggled`.
2. Keep reducers unchanged for now (toggle semantics already correct).
3. Introduce and use `apply_button_dark_mode_classic_render(...)` in the base implementation now (not deferred), so button-like controls have one canonical dark-mode setup path.
4. Add explicit CheckBox-specific tests in `commanductui` and migration tests in `harvester_app`.

---

## Implementation Plan

### Step 1: Extend platform API types
**File:** `src/CommanDuctUI/src/types.rs`

- Add `PlatformCommand` variants:
  - `CreateCheckBox { window_id, parent_control_id, control_id, text }`
  - `SetCheckBoxChecked { window_id, control_id, checked }`
- Add `AppEvent` variant:
  - `CheckBoxToggled { window_id, control_id, checked }`

Why: keeps checkbox event payload self-contained and avoids implicit state assumptions.

---

### Step 2: Add CheckBox control kind + style id
**Files:**
- `src/CommanDuctUI/src/window_common.rs`
- `src/CommanDuctUI/src/styling_primitives.rs`

- Add `ControlKind::CheckBox` in `window_common.rs` (actual enum location).
- Add `StyleId::CheckBox` in `styling_primitives.rs`.

Note: keep `StyleId::RadioButton` for existing mode/stage radios.

---

### Step 3: Add dark-mode helper and use it in base plan
**Files:**
- `src/CommanDuctUI/src/window_common.rs`
- `src/CommanDuctUI/src/app.rs`
- `src/CommanDuctUI/src/controls/radiobutton_handler.rs`
- `src/CommanDuctUI/src/controls/checkbox_handler.rs` (new)

Implement helper in `window_common.rs`:
- `apply_button_dark_mode_classic_render(hwnd: HWND)`
- Behavior:
  - call `try_enable_dark_mode(hwnd)`
  - call `SetWindowTheme(hwnd, "", "")` (classic render for CTLCOLOR)

Use helper:
- In `radiobutton_handler` after creation (replace direct `try_enable_dark_mode` call).
- In new `checkbox_handler` after creation.
- In `app.rs` style application branch for button-like controls (`RadioButton`, `CheckBox`, and optionally `Button` if already relying on same behavior) to keep behavior idempotent and centralized.

Rationale: this directly addresses the known regression pattern and fulfills the “use now” requirement.

---

### Step 4: Implement CheckBox handler
**Files:**
- `src/CommanDuctUI/src/controls/checkbox_handler.rs` (new)
- `src/CommanDuctUI/src/controls.rs`

Create handler modeled on `radiobutton_handler.rs`:
- Create style: `BS_AUTOCHECKBOX | WS_CHILD | WS_VISIBLE | WS_TABSTOP`.
- Register kind: `window_data.register_control_kind(control_id, ControlKind::CheckBox)`.
- Set checked: send `BM_SETCHECK` with `1/0` mapping (`BST_CHECKED/BST_UNCHECKED` semantics).

Add unit tests:
- checkbox style includes `WS_TABSTOP`
- checkbox style includes `BS_AUTOCHECKBOX`
- checked-state mapping true/false

---

### Step 5: Command dispatch wiring in `commanductui`
**File:** `src/CommanDuctUI/src/app.rs`

- Import `checkbox_handler`.
- Handle:
  - `PlatformCommand::CreateCheckBox`
  - `PlatformCommand::SetCheckBoxChecked`
- Ensure style application recognizes `ControlKind::CheckBox` and applies `StyleId::CheckBox` with same palette behavior as radios.

---

### Step 6: Paint routing for CheckBox
**File:** `src/CommanDuctUI/src/controls/paint_router.rs`

Add routes:
- `(ControlKind::CheckBox, WM_CTLCOLORBTN) => PaintRoute::Button`
- `(ControlKind::CheckBox, WM_CTLCOLORSTATIC) => PaintRoute::Button`

Add tests:
- checkbox routes BTN -> Button
- checkbox routes STATIC -> Button

---

### Step 7: Emit `CheckBoxToggled` from Win32 event path
**File:** `src/CommanDuctUI/src/window_common.rs`

In `BN_CLICKED` handling:
- For `ControlKind::CheckBox`, read state via `BM_GETCHECK` and emit:
  - `AppEvent::CheckBoxToggled { window_id, control_id, checked }`

Keep existing `RadioButtonSelected` behavior unchanged for true radio groups.

---

### Step 8: Harvester UI constants and creation/render migration
**Files:**
- `crates/harvester_app/src/platform/ui/constants.rs`
- `crates/harvester_app/src/platform/ui/layout.rs`
- `crates/harvester_app/src/platform/ui/render.rs`

Constants:
- Rename only section toggle IDs (same numeric values):
  - `BTN_PROMPT_LAB_SECTION_*` -> `CHK_PROMPT_LAB_SECTION_*`

Layout:
- Replace section-toggle `CreateRadioButton` with `CreateCheckBox`.
- Keep Prompt Lab mode/stage controls as radio buttons.

Render:
- Replace section-toggle `SetRadioButtonChecked` with `SetCheckBoxChecked`.
- Update tests that currently assert `SetRadioButtonChecked` for section toggles.

Style application in layout:
- Apply `StyleId::CheckBox` to section toggles.
- Keep `StyleId::RadioButton` for mode/stage controls.

---

### Step 9: Harvester app event rewiring
**File:** `crates/harvester_app/src/platform/app.rs`

- Replace section-toggle handlers from `AppEvent::RadioButtonSelected` to `AppEvent::CheckBoxToggled`.
- Keep emitted reducer messages unchanged:
  - `Msg::PromptLabCompareSectionToggled`, etc.

Tests to update/add:
- Existing event-handler tests for mode/stage remain radio-based.
- Section toggle tests should send `CheckBoxToggled` and assert same `Msg::*SectionToggled` outputs.

---

### Step 10: Verification

1. `cargo build`
2. `cargo test -p commanductui`
3. `cargo test -p harvester_app`
4. `cargo clippy --all-targets -- -D warnings`

(Per workspace guidance: clippy at end of complete implementation.)

---

## Test Matrix (Lock-in)

### `commanductui`
- Unit: `checkbox_handler` style and check-state mapping.
- Unit: `paint_router` checkbox routes for both CTLCOLOR messages.
- Unit/integration: `window_common` emits `CheckBoxToggled` with correct checked value.
- Regression: radio controls still emit `RadioButtonSelected`.

### `harvester_app`
- Render test: section open states emit `SetCheckBoxChecked` for `CHK_*` ids.
- Layout test: section controls are created as `CreateCheckBox`.
- App handler test: `CheckBoxToggled` maps to existing reducer toggle messages.

---

## Future Ideas Backlog Impact

- **FI-Architecture-UiFramework-0008**: not fully closed, but materially advanced by introducing shared helper `apply_button_dark_mode_classic_render(...)` and CheckBox routing parity.
- No backlog item should be marked closed yet unless strategy-table refactor is completed.

---

## Engineering Diary Finalization Notes

When implementation is complete, finalize entry in `docs/EngineeringDiary.md` with:
- Evidence: `cargo build`, targeted tests (`commanductui`, `harvester_app`), final `cargo clippy --all-targets -- -D warnings`.
- Refs: `commanductui` CheckBox handler + paint routing + dark-mode helper, `harvester_app` Prompt Lab section toggle migration.
