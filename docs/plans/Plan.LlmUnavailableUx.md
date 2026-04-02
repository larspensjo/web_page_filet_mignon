# Plan: Make AI-Unavailable State Obvious

Make missing AI configuration obvious in the app UI so users do not end up in a dead-end flow like `Triage Results (no triage results yet)` when triage never actually started.

## Scope

This plan covers operator-visible UX for unavailable LLM features in the desktop app.

In scope:
- disable AI-dependent actions when the app cannot run them
- surface a persistent warning in an existing global UI surface
- replace misleading empty-state copy with configuration-aware messaging
- preserve unidirectional data flow: startup/effects produce state, reducers stay pure, render derives the UI
- add regression tests around reducer state and rendered commands/text

Out of scope for this pass:
- adding environment-variable editors or settings dialogs
- retry/reconnect workflows for transient provider outages
- supporting multiple providers beyond the current OpenAI-backed path
- changing `CommanDuctUI` unless a missing generic capability is discovered during implementation

No `CommanDuctUI` changes are expected for the first pass because the app already has the necessary primitives: button enable/disable, label severity, and text updates.

## Problem Statement

Current behavior is technically correct in logs but misleading in the UI.

Observed failure mode:
- app starts without `OPENAI_API_KEY`
- startup logs `OPENAI_API_KEY not set; LLM features disabled`
- prompt metadata loads with zero effective models
- pre-triage still succeeds, so the workflow appears partly healthy
- user clicks `Triage Articles`
- reducer switches to `Triage Results`, but no triage request is dispatched
- the header says `no triage results yet`, which reads like an empty successful run instead of a blocked action

This is a UX bug because the UI does not distinguish between:
- `there are zero triage results because you have not run triage yet`
- `there are zero triage results because triage is currently impossible`

## UX Goals

1. Prevent the user from invoking an action that is guaranteed not to run.
2. Explain the blocker at the point of action, not only in logs.
3. Keep the warning persistent but low-friction; avoid modal dialogs.
4. Use exact cause text when known: missing API key vs no effective model.
5. Preserve future extensibility so other AI-unavailable reasons can reuse the same UI path.

## Proposed UX

### 1. Disable AI-dependent actions

When AI features are unavailable, disable:
- `Triage Articles`
- `Generate Briefing`

Rationale:
- this is the strongest signal and prevents the dead-end click path
- it matches current render architecture, which already derives button enabled state from the view model

Recommended text behavior for the first pass:
- keep the button labels unchanged
- use surrounding status/empty-state messaging to explain why

Do not rename the buttons to `AI unavailable` in this pass. Disabled buttons plus explicit nearby explanation is clearer and avoids layout churn.

### 2. Add a persistent global warning

Use the existing bottom status label as the persistent warning surface.

When AI is unavailable, append status text with a warning such as:
- `AI features unavailable: OPENAI_API_KEY is not set`
- `AI features unavailable: no ArticleTriage model is available`

Severity should be `Warning`, not `Information`.

Rationale:
- the surface already exists
- it is always visible
- it supports severity styling already
- it avoids introducing new layout complexity for the first slice

First-pass append strategy:
- keep the existing status text intact
- append the AI warning as the last segment with the existing separator convention
- do not replace unrelated status like session state, checkpoint state, or operation progress

### 3. Add configuration-aware empty-state copy

When the left tab is `Triage Results` and AI is unavailable, the header should not say `no triage results yet`.

Recommended header copy when there are no triage results:
- `Triage Results (AI unavailable)`

If triage results already exist from an earlier run, preserve the count and add the unavailable suffix instead of replacing the whole header. Example:
- `Triage Results (3 with triage | AI unavailable)`

Recommended body/placeholder copy in the corresponding viewer or preview area:
- `Article triage is unavailable because OPENAI_API_KEY is not set.`
- `Set OPENAI_API_KEY and restart the app to enable triage.`

If the app can distinguish a non-key configuration problem, substitute the exact cause.

This same pattern should apply to Briefing if the user navigates there while AI is unavailable.

### 4. Optional inline toolbar hint

If the current button row has space without creating layout regressions, add a small passive label near the main action buttons:
- `AI disabled: set OPENAI_API_KEY to enable Triage and Briefing.`

This is optional for the first implementation slice. The status bar plus disabled buttons may already be sufficient.

## State Model Recommendation

Do not infer availability from scattered fields during render. Introduce an explicit reducer-owned availability reason in `AppState`, and derive user-facing strings only when building the view model.

Recommended shape in `harvester_core` state:

```rust
pub enum AiAvailability {
    Available,
    Unavailable { reason: AiUnavailableReason },
}

pub enum AiUnavailableReason {
    MissingApiKey,
    NoTriageModel,
}
```

First-pass scope note:
- keep `AiAvailability` as one shared blocker for AI-dependent workflows
- support only shared blocker reasons in this pass: `MissingApiKey` and `NoTriageModel`
- do not try to encode asymmetric per-feature states like `triage available but briefing unavailable` in the first implementation
- if per-feature availability becomes necessary later, split the model into two fields or a richer structure in a follow-up change

Initial state recommendation:
- default `AppState.ai_availability` to `Available`
- do not show an `AI unavailable` warning during normal startup before evidence arrives
- transition to `Unavailable` only when a concrete signal is received from startup or metadata

Why explicit state is better than ad hoc render checks:
- reducers can centralize the meaning of `AI unavailable`
- tests can assert exact reasons and action gating
- render code stays simple and declarative
- future provider or metadata failures can plug into the same contract

Responsibility split:
- `AppState` stores `AiAvailability`
- `update.rs` mutates it in response to messages
- `view_model.rs` formats `AiAvailability` into operator-facing strings like `ai_unavailable_message`
- `render.rs` consumes only view-model fields and does not inspect raw availability enums

## Implementation Plan

---

## Step 1 - Model explicit AI availability in `harvester_core`

Files likely affected:
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/view_model.rs`
- `crates/harvester_core/src/msg.rs`

Add reducer-owned state to `AppState` representing whether AI-dependent workflows are currently available and why not.

Keep presentation formatting out of `AppState`. The state layer should store only the enum and helper predicates; `view_model.rs` should format it into user-visible strings.

Recommended state API:

```rust
pub fn ai_availability(&self) -> &AiAvailability
pub fn triage_ai_available(&self) -> bool
pub fn briefing_ai_available(&self) -> bool
```

Recommended view-model additions:

```rust
pub ai_unavailable_message: Option<String>,
pub triage_blocked_reason: Option<String>,
pub briefing_blocked_reason: Option<String>,
```

Do not embed environment inspection logic in render. The view model should carry preformatted operator-facing strings.

Tests to add in core:
- startup state defaults to `Available` before evidence arrives
- metadata with zero effective triage model yields unavailable state
- metadata with valid triage model yields available state
- missing API key path can be represented explicitly without relying on logs

---

## Step 2 - Feed the reason from startup/effects into reducer state

Files likely affected:
- `crates/harvester_app/src/platform/app.rs`
- `crates/harvester_io/src/effect_runner.rs` only if metadata/result payload must be extended
- `crates/harvester_core/src/update.rs`

Current startup knows whether `OPENAI_API_KEY` is missing before the effect runner is built. That is the strongest source of truth for the specific `MissingApiKey` reason.

Add an explicit reducer message for startup-detected blockers.

Recommended message:

```rust
Msg::AiAvailabilityDetected { availability: AiAvailability }
```

Recommended approach:
- when startup detects the missing key, dispatch `Msg::AiAvailabilityDetected { availability: AiAvailability::Unavailable { reason: AiUnavailableReason::MissingApiKey } }`
- when `LlmMetadataLoaded` arrives with zero effective models for triage, let the reducer record `AiUnavailableReason::NoTriageModel` if no stronger reason is already known
- when metadata later becomes valid, clear the unavailable state back to `Available`

Important ordering rule:
- `MissingApiKey` should win over `NoTriageModel` because it is more specific and more actionable in the current architecture

Reducer invariant:
- message arrival order must not matter
- if `LlmMetadataLoaded` sets `NoTriageModel` and `AiAvailabilityDetected(MissingApiKey)` arrives later, the reducer must overwrite to `MissingApiKey`
- if `MissingApiKey` is already set, later metadata loads with zero models must not downgrade it to `NoTriageModel`

Reducer rules to preserve:
- pure reducer, no environment reads in core
- reason changes should be explicit and testable
- briefing orchestration and triage orchestration should both consult the same availability source where appropriate

Tests to add:
- startup unavailable reason survives initial metadata load with zero models
- valid metadata clears the unavailable state back to `Available`
- `MissingApiKey` is not overwritten by weaker downstream metadata-derived reasons
- reducer behavior is correct regardless of whether startup or metadata message arrives first

---

## Step 3 - Gate main actions from view-model state

Files likely affected:
- `crates/harvester_core/src/state.rs`
- `crates/harvester_app/src/platform/ui/render.rs`

Update the view-model derivation so `triage_can_start` and `briefing_can_start` become false when AI is unavailable, even if the rest of the workflow prerequisites are satisfied.

Also update any default constructors or initial view-model values so `briefing_can_start` does not momentarily default to a misleading enabled state before the first full derivation pass.

This should make the existing render code disable:
- `BUTTON_TRIAGE`
- `BUTTON_BRIEFING`

No render-layer special casing should be needed beyond the existing `SetControlEnabled` logic.

Tests to add:
- triage button disabled when pre-triage is ready but AI unavailable
- briefing button disabled when article corpus is otherwise briefing-ready but AI unavailable
- unrelated controls such as `Poll Sources` remain enabled
- initial render does not briefly expose enabled AI actions when a missing-key startup message has already been applied

---

## Step 4 - Surface a persistent status-bar warning

Files likely affected:
- `crates/harvester_app/src/platform/ui/render.rs`

Update `render_status_section` so AI-unavailable text is rendered with `MessageSeverity::Warning`.

Recommended behavior:
- when `view.ai_unavailable_message` is present, severity becomes `Warning`
- the message is appended to the existing status line as the final segment; do not replace unrelated status text in the first pass

Recommended first-pass copy:
- `AI features unavailable: OPENAI_API_KEY is not set`
- `AI features unavailable: no triage model is available`

The wording should stay action-oriented and concrete. Avoid vague text like `AI error` or `Configuration problem`.

Tests to add in render:
- status label severity becomes warning when AI unavailable
- status text includes the exact unavailable message
- status severity remains information during normal operation

---

## Step 5 - Make Triage Results empty state configuration-aware

Files likely affected:
- `crates/harvester_app/src/platform/ui/render.rs`
- possibly `crates/harvester_core/src/state.rs` if the header/body strings are better derived there

Update the `LeftTab::TriageResults` header logic so it distinguishes:
- empty because nothing has been triaged yet
- empty because triage is unavailable
- results exist, but new triage runs are currently unavailable

Recommended header behavior:

```text
Triage Results (AI unavailable)
```

when unavailable and there are no existing results, otherwise keep the current count-based behavior and add an unavailable suffix when historical results exist.

If the current pane body already has a generic placeholder surface, populate it with a two-line message:

```text
Article triage is unavailable because OPENAI_API_KEY is not set.
Set OPENAI_API_KEY and restart the app to enable triage.
```

If no dedicated body surface exists without larger UI changes, updating the header plus the status-bar warning is still acceptable for the first slice.

Tests to add:
- triage-results header uses `AI unavailable` copy when blocked
- current `no triage results yet` copy remains for genuinely empty but available state
- result count is preserved in the header when historical triage results exist but AI is currently unavailable

---

## Step 6 - Prevent misleading reducer transitions

Files likely affected:
- `crates/harvester_core/src/update.rs`

Today `Msg::TriageClicked` can switch to `LeftTab::TriageResults` before the metadata/availability problem is fully communicated. Tighten that path.

Recommended rule:
- if AI is explicitly unavailable, do not treat the click as a normal triage-start transition
- either keep the current tab unchanged, or allow the tab switch only if the target tab will now show the explicit unavailable state

Preferred first-pass behavior:
- do not start triage or consume pre-triage state
- allow switching to `TriageResults` only if that surface now shows the explicit unavailable explanation
- otherwise leave the current tab unchanged
- rely on disabled buttons so the click path is mostly unreachable in normal use

Why this still matters:
- keyboard shortcuts, stale UI state, or future event paths may still dispatch `Msg::TriageClicked`
- reducer logic should defend its invariants independently of the UI layer

Tests to add:
- `TriageClicked` emits no triage-start effects when AI unavailable
- `TriageClicked` does not consume pre-triage when AI unavailable
- `TriageClicked` does not incorrectly imply success by transitioning into a misleading state
- blocked `TriageClicked` either preserves the current tab or lands on an explicit unavailable-state tab, never on the misleading generic empty-state copy

---

## Step 7 - Apply the same pattern to Briefing

Files likely affected:
- `crates/harvester_core/src/update.rs`
- `crates/harvester_app/src/platform/ui/render.rs`

The original report surfaced in Triage, but Briefing depends on the same shared AI availability class and should not lag behind.

Minimum parity for this pass:
- disable `Generate Briefing` when AI unavailable
- include the same status-bar warning
- ensure Briefing does not enter a misleading `ready but empty` presentation when the blocker is configuration

This avoids solving the same bug twice in adjacent workflows.

---

## Copy Recommendations

Use exact, operator-facing text.

Recommended unavailable messages:
- `AI features unavailable: OPENAI_API_KEY is not set`
- `AI features unavailable: no triage model is available`

Per-feature model-specific copy is intentionally deferred for this first pass because the state model stays shared, not per-feature.

Recommended triage placeholder:
- `Article triage is unavailable because OPENAI_API_KEY is not set.`
- `Set OPENAI_API_KEY and restart the app to enable triage.`

Recommended briefing placeholder:
- `Briefing is unavailable because OPENAI_API_KEY is not set.`
- `Set OPENAI_API_KEY and restart the app to enable briefing.`

Avoid:
- `Something went wrong`
- `No results yet` when the action never ran
- provider jargon like `effective model map empty`

## Risks and Tradeoffs

1. Overloading the status bar with too much text can reduce scanability.
Mitigation: keep the unavailable message short and explicit.

2. If availability is derived in too many places, triage and briefing can drift.
Mitigation: one reducer-owned availability contract, reused by both workflows.

3. The first-pass shared availability model cannot express asymmetric feature availability.
Mitigation: limit the first pass to shared blockers only and split to per-feature availability in a follow-up if needed.

4. Disabled buttons alone may not be enough for users who are already on the Triage Results tab.
Mitigation: also update header and placeholder copy.

5. Startup and metadata messages may arrive in either order.
Mitigation: enforce reducer precedence so `MissingApiKey` always wins over `NoTriageModel`, independent of message order.

## Validation Strategy

### Reducer tests

- missing-key reason blocks triage start
- missing-key reason blocks briefing start
- `TriageClicked` does not consume pre-triage when blocked
- valid metadata transitions back to available state

### Render tests

- triage button disabled when AI unavailable
- briefing button disabled when AI unavailable
- status label rendered with warning severity and expected text
- triage-results header uses `AI unavailable` wording when blocked
- existing empty-state wording still appears when AI is available but there are simply no triage results yet

### Manual verification

1. Launch app without `OPENAI_API_KEY`.
2. Confirm `Triage Articles` and `Generate Briefing` are disabled.
3. Confirm status bar shows warning text.
4. Navigate to Triage Results and confirm the UI explains the configuration blocker.
5. Relaunch with a valid `OPENAI_API_KEY`.
6. Confirm warning clears and actions become available again.

### Validation commands when implementation is complete

```powershell
cargo build
cargo test -p harvester_core
cargo test -p harvester_app
cargo clippy --workspace --all-targets -- -D warnings
```

## Diary Follow-up

When implementation lands, add a short entry to `docs/EngineeringDiary.md` covering:
- the misleading `no triage results yet` failure mode
- the explicit AI-availability state added to the reducer/view model
- the operator-visible safeguards added in the UI

Refs should include the core reducer/state files and the render file that owns the action/status/empty-state behavior.
