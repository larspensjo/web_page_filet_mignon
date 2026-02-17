# Plan: Prompt Lab ComboBox Model Selector Hardening (Revised)

## Goal
Make Prompt Lab model selection reliably visible and usable across lifecycle transitions and Win32 layout/theming edge cases, with clear diagnostics and regression tests.

## Current State (Verified in Source)
1. Render now resets model-selector cache on Prompt Lab hidden -> visible, forcing `SetComboBoxItems` and `SetComboBoxSelection` replay (`crates/harvester_app/src/platform/ui/render.rs`).
2. ComboBox creation applies `CB_SETMINVISIBLE` and logs result (`src/CommanDuctUI/src/controls/combobox_handler.rs`).
3. Layout currently enforces a hard-coded native ComboBox minimum height (`COMBOBOX_DROPDOWN_NATIVE_MIN_HEIGHT_PX = 260`) (`src/CommanDuctUI/src/window_common.rs`).
4. `CBN_DROPDOWN` / `CBN_CLOSEUP` are logged, but no runtime invariant check/self-heal exists (`src/CommanDuctUI/src/window_common.rs`).
5. Existing tests cover first-render default selection and catalog emission, but do not yet lock in hide/show replay behavior for this bug class (`crates/harvester_app/src/platform/ui/render.rs` tests).

## Key Gaps
1. Geometry policy still relies on hard-coded values (`260`, `min_visible_items = 12`) instead of runtime metrics.
2. No explicit unit test for visibility transition replay of combo items/selection.
3. No invariant/self-heal path when dropdown geometry is still invalid at open-time.
4. Logging in platform ComboBox path uses `log::*`; application uses `engine_logging`. Keep this mixed boundary intentional and documented, since `CommanDuctUI` is a submodule.

## Architecture Guidance
1. Keep app UDF intact: state changes remain `Msg -> update -> state -> render -> PlatformCommand`.
2. Keep geometry enforcement in platform layer: it is a native control concern and should not mutate app state.
3. Avoid introducing an app-level epoch unless needed. Current control lifecycle appears create-once + hide/show; start with targeted replay tests and geometry hardening first.

## Hardening Plan
### Step 1 - Lock In Existing Replay Behavior
1. Add render unit test: Prompt Lab hidden -> visible with unchanged catalog/selection still emits `SetComboBoxItems` and `SetComboBoxSelection`.
2. Add render unit test: unchanged visible state remains idempotent (no extra combo commands after second render).

### Step 2 - Replace Hard-Coded Geometry Policy
1. Introduce a small geometry policy helper in `CommanDuctUI` that computes effective dropdown constraints from runtime data when available.
2. Keep conservative fallback values only when metrics are unavailable.
3. Route both creation-time (`CB_SETMINVISIBLE`) and layout-time minimum-height logic through the same helper to avoid drift.

### Step 3 - Add Runtime Invariant + One-Shot Self-Heal
1. On `CBN_DROPDOWN`, validate current dropdown usability invariants (item count, measurable geometry, minimum visible rows intent).
2. If invalid, reapply geometry policy once and log warning with context (`window_id`, `control_id`, measured values, item count).
3. Guard against repeated churn with one-shot-per-open-cycle behavior.

### Step 4 - Telemetry Contract Tightening
1. Keep `[prompt-lab-model]` render logs for source/count and selection index.
2. Add concise platform warning log category for geometry correction path.
3. Keep logs at `info/warn` boundaries only; no per-message spam.

### Step 5 - Validate End-to-End
1. `cargo build`
2. Run targeted tests for render/platform modules.
3. Final gate: `cargo clippy --all-targets -- -D warnings`

## Test Plan
### Render Unit Tests
1. Hidden -> visible replay emits `SetComboBoxItems`.
2. Hidden -> visible replay emits `SetComboBoxSelection`.
3. Visible unchanged second render emits no combo churn.

### Platform Unit Tests
1. Geometry helper returns runtime-based result when metrics exist.
2. Geometry helper falls back safely when metrics unavailable.
3. Dropdown invariant check triggers one-shot correction on invalid geometry.

### Integration/Manual
1. Remote catalog load: models visible and selectable.
2. Local fallback catalog: deduped entries visible.
3. Reopen Prompt Lab repeatedly: selector remains populated and usable.
4. DPI/theme spot-check (100%/150% at minimum): dropdown remains visible and scrollable.

## Acceptance Criteria
1. No regression: selector items/selection replay on Prompt Lab reopen is test-locked.
2. Geometry logic is centralized, runtime-aware, and fallback-safe.
3. Dropdown-open invariant checks can self-heal at least one known invalid geometry scenario.
4. Observability is sufficient to diagnose failures without ad-hoc instrumentation.

## Blockers / Decisions Needed
1. Define exact runtime metrics source for row-height policy (Win32 API choice and compatibility baseline).
2. Decide whether to keep any hard minimum constants as safety rails and where to document them.
3. Confirm whether geometry self-heal should remain platform-local only (recommended) or emit a diagnostic app event.

## Future Extensions
1. Add a lightweight platform capability probe and branch behavior for older Windows builds.
2. Add an automated UI smoke test that opens Prompt Lab, expands dropdown, and verifies non-zero visible rows.
3. Generalize ComboBox hardening helper so future ComboBoxes inherit the same resilience by default.
