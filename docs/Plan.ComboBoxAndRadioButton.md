# Plan: ComboBox & RadioButton for CommanDuctUI + Prompt Lab Model Selector Migration

## Source-validated status (2026-02-16)

This rewrite is checked against current code and paths in workspace.

### Key corrections from previous draft

- OpenAI provider path is currently:
    `crates/harvester_engine/src/llm/providers/openai.rs`
    (not `.../llm/openai_provider.rs`).
- Current Prompt Lab model selector is button-based and limited to 8 slots:
    - `BTN_PROMPT_LAB_MODEL_DEFAULT`
    - `BTN_PROMPT_LAB_MODEL_SLOT_0`
    - `PROMPT_LAB_MODEL_SLOT_COUNT`
- `WM_CTLCOLORLISTBOX` is already forwarded through panel parents (`panel_handler`).
- `lib.rs` already re-exports `AppEvent`/`PlatformCommand`; no export plumbing change needed.

---

## Goals

1. Add native `ComboBox` and `RadioButton` controls to CommanDuctUI.
2. Replace Prompt Lab model button-strip with one combo selector.
3. Improve model filtering so Prompt Lab only presents practical chat models.
4. Keep architecture command/event-driven and reducer-friendly.

---

## Design choices

| Decision | Choice | Why |
|---|---|---|
| Combo style | `CBS_DROPDOWNLIST` | Prevent invalid free-typed model IDs |
| Radio style | `BS_AUTORADIOBUTTON` | Native mutual exclusion by group |
| Version | `commanductui` `0.3.0` | New public enum variants are breaking |
| Dark theme | `try_enable_dark_mode` + `WM_CTLCOLORLISTBOX` | Match existing dark UI |
| Robustness | No fixed buffer lengths | Follow existing safety guideline |

---

## Architecture (unchanged pattern)

Input -> `PlatformCommand` -> Win32 handler -> `AppEvent` -> reducer `Msg`

- `CreateComboBox` / `SetComboBoxItems` / `SetComboBoxSelection`
- `CreateRadioButton`
- `ComboBoxSelectionChanged` / `RadioButtonSelected`

This stays consistent with existing command-event architecture (`[CDU-CmdEventPatternV1]`).

---

## Step 0 — Tighten OpenAI model filtering

### Files

- `crates/harvester_engine/src/llm/providers/openai.rs`
- `crates/harvester_engine/tests/llm_openai.rs`

### Change

In `OpenAiProvider::list_models()`:

1. Keep allow-list by family (`gpt-`, `o1-`, `o3-`, `o4-`).
2. Expand deny-list with: `audio`, `realtime`, `transcribe`, `search`, `instruct`, and dated snapshots.
3. Add helper:
     - `is_dated_snapshot(id: &str) -> bool`
     - Detects forms like `gpt-4-0613`, `...-2024-08-06`.

### Note on normalization ownership

`effects.rs` currently sorts+dedups model names after provider call. Keep that as cross-provider normalization. Provider-level filtering remains provider-specific responsibility.

### Tests

- Add integration test in `tests/llm_openai.rs` using wiremock for `/v1/models`:
    - validates filtered output includes only chat-relevant IDs.
- Add unit test for dated snapshot helper (either in-module test block or via black-box list filter cases).

---

## Step 1 — Extend CommanDuctUI public vocabulary

### Files

- `src/CommanDuctUI/src/types.rs`
- `src/CommanDuctUI/src/window_common.rs`
- `src/CommanDuctUI/src/styling_primitives.rs`

### Additions

- `PlatformCommand`:
    - `CreateComboBox`
    - `SetComboBoxItems`
    - `SetComboBoxSelection`
    - `CreateRadioButton`
- `AppEvent`:
    - `ComboBoxSelectionChanged`
    - `RadioButtonSelected`
- `ControlKind`:
    - `ComboBox`
    - `RadioButton`
- `StyleId`:
    - `ComboBox`
    - `RadioButton`

---

## Step 2 — Implement combo handler module

### Files

- New: `src/CommanDuctUI/src/controls/combobox_handler.rs`
- Update: `src/CommanDuctUI/src/controls.rs`

### Commands

- `handle_create_combobox_command()`
    - follow read/write/no-lock/write phase pattern (same as button robustness pattern).
    - style: `WS_CHILD | WS_VISIBLE | CBS_DROPDOWNLIST | CBS_HASSTRINGS | WS_VSCROLL`.
    - register `ControlKind::ComboBox`.
    - call `try_enable_dark_mode` when dark theme active.

- `handle_set_combobox_items()`
    - `CB_RESETCONTENT` then `CB_ADDSTRING` per item.
    - no implicit selection.

- `handle_set_combobox_selection()`
    - `CB_SETCURSEL` with `None -> -1`.
    - log warning on `CB_ERR` for out-of-range index.

### Event mapping

- `handle_cbn_selchange(window_id, control_id, hwnd_combo) -> AppEvent`
    - read `CB_GETCURSEL`.
    - map negative to `None`.

### Testability improvement

Extract pure helper for index mapping (e.g., `selection_from_raw_index`) and unit-test it directly.

---

## Step 3 — Implement radio handler module

### Files

- New: `src/CommanDuctUI/src/controls/radiobutton_handler.rs`
- Update: `src/CommanDuctUI/src/controls.rs`

### Command

- `handle_create_radiobutton_command()`
    - style baseline: `WS_CHILD | WS_VISIBLE | BS_AUTORADIOBUTTON`.
    - if `group_start`: add `WS_GROUP | WS_TABSTOP`.
    - register `ControlKind::RadioButton`.
    - enable dark mode best effort.

### Tests

- Pure helper for style composition with tests for group/non-group flags.

---

## Step 4 — Wire command dispatch in CommanDuctUI app

### File

- `src/CommanDuctUI/src/app.rs`

### Change

Add `execute_platform_command()` match arms for new combo/radio commands and import new handlers.

---

## Step 5 — Refine `WM_COMMAND` event dispatch

### File

- `src/CommanDuctUI/src/window_common.rs`

### Change

In `handle_wm_command()`:

- route `CBN_SELCHANGE` + `ControlKind::ComboBox` -> `ComboBoxSelectionChanged`
- route `BN_CLICKED` + `ControlKind::RadioButton` -> `RadioButtonSelected`
- keep fallback `BN_CLICKED` -> existing button handler
- keep existing `EN_CHANGE` debounce and `EN_VSCROLL` scroll logic

### Robustness add-on

Update leaf invalidation list to include `ControlKind::ComboBox` and `ControlKind::RadioButton` so relayout repaint behavior remains consistent.

---

## Step 6 — Dark-theme dropdown list handling

### Files

- `src/CommanDuctUI/src/window_common.rs`
- `src/CommanDuctUI/src/controls/paint_router.rs`
- `src/CommanDuctUI/src/controls/combobox_handler.rs`

### Critical nuance

For `WM_CTLCOLORLISTBOX`, `lParam` is listbox HWND (dropdown list), not always the combo HWND. Do not rely only on `ControlId -> ControlKind` lookup for this route.

### Implementation

1. Handle `WM_CTLCOLORLISTBOX` explicitly in message switch.
2. Route to `combobox_handler::handle_wm_ctlcolorlistbox(...)`.
3. Resolve style by `StyleId::ComboBox` then fallback `StyleId::DefaultInput`.

Optional enhancement (future-proof): maintain a reverse HWND map for auxiliary child HWNDs, but not required for initial rollout.

### Tests

- Extend `paint_router` tests with listbox routing case.

---

## Step 7 — Register modules and keep exports stable

### Files

- `src/CommanDuctUI/src/controls.rs`

### Change

Add module declarations:

- `combobox_handler`
- `radiobutton_handler`

No `lib.rs` export changes needed.

---

## Step 8 — Version and changelog

### Files

- `src/CommanDuctUI/Cargo.toml`
- `src/CommanDuctUI/CHANGELOG.md`

### Change

- Bump `0.2.8 -> 0.3.0`.
- Add changelog entry noting breaking enum variant additions and new controls.

---

## Step 9 — Replace Prompt Lab model buttons with one combo

### Files

- `crates/harvester_app/src/platform/ui/constants.rs`
- `crates/harvester_app/src/platform/ui/layout.rs`
- `crates/harvester_app/src/platform/ui/render.rs`
- `crates/harvester_app/src/platform/app.rs`

### Change details

1. Constants:
     - remove button-slot model constants.
     - add `COMBO_PROMPT_LAB_MODEL_SELECTOR = ControlId::new(3113)`.

2. Initial UI creation:
     - replace model button creation with one `CreateComboBox` in `PANEL_PROMPT_LAB_MODEL_ROW`.

3. Layout:
     - replace per-slot model rules with one fill rule for the combo.
     - remove `model_catalog` from `PromptLabLayoutConfig` (no longer needed for per-slot visibility).

4. Render:
     - if catalog changed -> issue one `SetComboBoxItems` with `"Default" + models`.
     - if selected model changed -> issue `SetComboBoxSelection` (`None/default -> 0`, model -> `index+1`).

5. Event handling:
     - remove model button click arms.
     - add `ComboBoxSelectionChanged` arm that maps selected index back to optional model.

### Robustness recommendation

Introduce tiny helper functions in app/render for index mapping both directions to avoid duplicated offset logic and off-by-one mistakes.

---

## Step 10 — Styling integration in harvester_app

### File

- `crates/harvester_app/src/platform/ui/layout.rs`

### Change

In `apply_dark_theme()`:

- apply `StyleId::ComboBox` to `COMBO_PROMPT_LAB_MODEL_SELECTOR`
    (or `StyleId::DefaultInput` fallback policy if desired).

This ensures popup list colors and closed control colors remain coherent.

---

## Step 11 — Validation

1. `cargo build --workspace`
2. `cargo nextest run`
3. `cargo clippy --workspace --all-targets -- -D warnings`

Manual checks:

1. Open Prompt Lab advanced mode.
2. Combo appears with `Default` selected.
3. Dropdown shows filtered model names only.
4. Select model -> override set.
5. Select `Default` -> override cleared.
6. Dropdown list is dark-themed.

---

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| `WM_CTLCOLORLISTBOX` control lookup mismatch | Handle listbox message explicitly, do not depend solely on `ControlId` lookup |
| Out-of-range selection index | Validate result of `CB_SETCURSEL`; log warning on `CB_ERR` |
| `BN_CLICKED` collision with push buttons | Disambiguate by `ControlKind`, fallback to push-button behavior |
| Empty model catalog | Keep `Default` as first/only item; no crash, no stale selection |
| API break for downstream users | SemVer bump to `0.3.0` + changelog with migration notes |

---

## Implementation order

1. Step 0 (model filtering)
2. Steps 1-3 (new control vocabulary + handlers)
3. Steps 4-6 (dispatch + dark theme)
4. Steps 7-8 (registration + versioning)
5. Steps 9-10 (Prompt Lab migration + styling)
6. Step 11 (full validation)

---

## Future ideas

1. **Typed selection controls API**
     - Add reusable helper utilities for `index <-> domain value` mapping to reduce duplicated offset logic.

2. **Searchable model picker (optional)**
     - If model count keeps growing, add a filtered picker dialog or type-ahead combo mode behind a flag.

3. **Provider capability metadata**
     - Extend model discovery with capability tags (`chat`, `vision`, `audio`, `realtime`) so filtering is data-driven, not string-pattern-driven.

4. **Reusable `WM_CTLCOLOR*` strategy map**
     - Centralize paint routing with message+kind strategy table to make future controls easier and safer to add.

5. **UI telemetry for model selection**
     - Lightweight event counters (`catalog_loaded`, `selection_changed`, `selection_cleared`) for diagnosing UX issues.
