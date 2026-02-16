# Plan: Prompt Lab ComboBox Model Selector Hardening

## Goal
Prevent recurrence of two classes of failures:

1. Render cache suppresses required ComboBox state re-send after visibility/control lifecycle transitions.
2. Native dropdown geometry becomes invalid for visibility/usability despite model data being present.

## Scope
- Prompt Lab model selector in Advanced mode.
- Render/update/effect/presentation pipeline and native Win32 ComboBox behavior.
- No redesign of Prompt Lab business logic.

## Non-Goals
- Full theming redesign of all Win32 controls.
- New data provider behaviors beyond current catalog loading.

---

## Lessons Learned (Root Causes)

1. Data pipeline correctness was necessary but insufficient. Logs proved models were loaded and inserted, but UI remained unusable due to native geometry.
2. Cached render state can hide lifecycle bugs. Recreated/re-shown controls may need explicit state replay even if model state did not change.
3. Win32 ComboBox requires explicit geometry/theming guardrails; default behavior is sensitive to style/layout interactions.

---

## Hardening Design

### A) Render Cache Robustness via UI Epoch

Introduce a Prompt Lab UI epoch (monotonic integer) that invalidates control-specific cache snapshots.

- Increment epoch on:
  - Prompt Lab visibility transitions.
  - Prompt Lab layout mode transitions (Basic/Advanced) when control tree differs.
  - Any control recreation event for model selector.
- In render cache, pair model selector cache with epoch:
  - `prev_prompt_lab_model_catalog: Option<(u64, Vec<ModelId>)>`
  - `prev_prompt_lab_selected_model: Option<(u64, String)>`
- If epoch mismatch, force emission of:
  - `SetComboBoxItems`
  - `SetComboBoxSelection`

Expected property: cache never suppresses first required state push for a fresh/recreated control.

### B) Deterministic “Ensure Selector State” Command Path

Add explicit idempotent command flow after combo creation.

- New app-level intent: ensure model selector state is applied.
- Trigger on create/recreate and Prompt Lab open.
- Handler emits full state (items + selection) irrespective of prior cache values.
- Keep reducer pure: emit effect/request; platform executes side-effect.

Expected property: state replay does not rely on incidental render ordering.

### C) Native Geometry Policy (Dynamic, Not Hard-Coded)

Replace fixed minimum assumptions with runtime-derived sizing.

- Compute target dropdown height from runtime metrics:
  - item height (from control/font metrics)
  - desired visible rows
  - borders/padding
- Apply via supported Win32 message/style path (`CB_SETMINVISIBLE` plus layout constraints).
- Keep a conservative lower bound as fallback only.

Expected property: dropdown remains visible across DPI/theme/font variations.

### D) Runtime Invariants + Self-Heal

On `CBN_DROPDOWN` and selected layout transitions:

- Verify invariants:
  - native combo height >= required minimum
  - item count visible path consistent with expected min rows
- If violated:
  - reapply geometry once
  - log warning with context (`window_id`, `control_id`, measured metrics)

Expected property: transient native misconfiguration is corrected automatically.

### E) Telemetry Contract (Action → Reducer → State → Render → Native)

Keep structured logs at boundaries for model selector only:

- catalog load source/count sample
- reducer catalog set
- render command emission (items + selection + epoch)
- native item count confirmation
- dropdown open with measured geometry

Expected property: quick diagnosis without ad-hoc instrumentation.

---

## Implementation Steps

### Step 1 — Epoch-based cache invalidation
- Add epoch field and transition updates.
- Update render cache keys to include epoch.
- Add tests for hide/show and mode transitions.

### Step 2 — Ensure-state command path
- Add explicit ensure command/intent and invoke on combo create/open.
- Keep idempotent behavior in platform layer.
- Add tests for recreate/reopen replay.

### Step 3 — Dynamic geometry
- Implement runtime metric-based dropdown sizing.
- Preserve fallback lower bound.
- Add unit tests for computed sizing policy.

### Step 4 — Invariant checks and self-heal
- Add `CBN_DROPDOWN` checks and one-shot correction.
- Add warning telemetry with measurement values.
- Add tests/mocks for correction trigger conditions.

### Step 5 — Verification and cleanup
- Remove temporary debug-only logs if redundant.
- Run workspace build/tests.
- Run strict lint as final gate.

---

## Test Plan

### Reducer/Render Unit Tests
1. Prompt Lab hidden→visible re-emits model selector items/selection.
2. Advanced mode transition with control recreation re-emits selector state.
3. Epoch unchanged remains idempotent (no command churn).

### Platform/Native Unit Tests
1. Geometry calculator produces valid height from runtime metrics.
2. Fallback lower bound applies when metrics unavailable.
3. Invariant checker flags invalid geometry and schedules one-shot correction.

### Integration/Manual Validation
1. Remote catalog success path: dropdown shows all models.
2. Local fallback path: dropdown shows deduped configured models.
3. Reopen Prompt Lab repeatedly: selector remains populated and selectable.
4. DPI/theme variation spot-check: dropdown remains visible and usable.

---

## Acceptance Criteria

1. Model selector never appears empty when catalog has entries.
2. Reopen/recreate transitions reliably preserve selector usability.
3. Dropdown geometry is consistently visible across tested DPI/theme settings.
4. No paint/event churn regressions from hardening changes.
5. Tests covering lifecycle replay and geometry invariants are present and passing.

---

## Risks and Mitigations

1. Risk: over-emission of commands causing UI churn.
   - Mitigation: epoch-scoped invalidation + idempotent command handlers.
2. Risk: geometry logic differs across Windows versions.
   - Mitigation: runtime measurement + fallback policy + invariant self-heal.
3. Risk: additional logs increase noise.
   - Mitigation: keep at info/warn boundaries only; avoid hot-path spam.

---

## Deliverables

1. Code changes for epoch, ensure-state path, dynamic geometry, and invariant checks.
2. New/updated tests in render and platform layers.
3. This plan document and brief post-implementation notes linked from architecture docs.
