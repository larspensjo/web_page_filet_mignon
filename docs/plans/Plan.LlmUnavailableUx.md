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

When AI is unavailable, append or replace status text with a warning such as:
- `AI features unavailable: OPENAI_API_KEY is not set`
- `AI features unavailable: no ArticleTriage model is available`

Severity should be `Warning`, not `Information`.

Rationale:
- the surface already exists
- it is always visible
- it supports severity styling already
- it avoids introducing new layout complexity for the first slice

### 3. Add configuration-aware empty-state copy

When the left tab is `Triage Results` and AI is unavailable, the header should not say `no triage results yet`.

Recommended header copy:
- `Triage Results (AI unavailable)`

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

Do not infer availability from scattered fields during render. Introduce an explicit app-level availability reason derived from startup/effect results.

Recommended shape in `harvester_core` state:

```rust
pub enum AiAvailability {
    Available,
    Unavailable { reason: AiUnavailableReason },
}

pub enum AiUnavailableReason {
    MissingApiKey,
    NoTriageModel,
    NoBriefingModel,
    MetadataNotLoaded,
}
```

For the first pass, `MissingApiKey` and `NoTriageModel` are enough. `NoBriefingModel` can be added immediately if the existing metadata pipeline already makes it cheap.

Why explicit state is better than ad hoc render checks:
- reducers can centralize the meaning of `AI unavailable`
- tests can assert exact reasons and action gating
- render code stays simple and declarative
- future provider or metadata failures can plug into the same contract

## Implementation Plan

---

## Step 1 - Model explicit AI availability in `harvester_core`

Files likely affected:
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/view_model.rs`
- possibly `crates/harvester_core/src/msg.rs` only if an additional message is needed

Add reducer-owned state representing whether AI-dependent workflows are currently available and why not.

Recommended state API:

```rust
pub fn ai_availability(&self) -> &AiAvailability
pub fn triage_ai_available(&self) -> bool
pub fn briefing_ai_available(&self) -> bool
pub fn ai_unavailable_status_text(&self) -> Option<String>
```

Recommended view-model additions:

```rust
pub ai_unavailable_message: Option<String>,
pub triage_blocked_reason: Option<String>,
pub briefing_blocked_reason: Option<String>,
```

Do not embed environment inspection logic in render. The view model should carry preformatted operator-facing strings.

Tests to add in core:
- startup state without metadata defaults to an expected initial value
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

Recommended approach:
- when startup detects the missing key, dispatch a reducer message that records `AiUnavailableReason::MissingApiKey`
- when `LlmMetadataLoaded` arrives with zero effective models for triage, let the reducer record `AiUnavailableReason::NoTriageModel` if no stronger reason is already known
- when metadata later becomes valid, clear the unavailable state back to `Available`

Important ordering rule:
- `MissingApiKey` should win over `NoTriageModel` because it is more specific and more actionable in the current architecture

Reducer rules to preserve:
- pure reducer, no environment reads in core
- reason changes should be explicit and testable
- briefing orchestration and triage orchestration should both consult the same availability source where appropriate

Tests to add:
- startup unavailable reason survives initial metadata load with zero models
- valid metadata clears temporary `MetadataNotLoaded` state
- `MissingApiKey` is not overwritten by weaker downstream metadata-derived reasons

---

## Step 3 - Gate main actions from view-model state

Files likely affected:
- `crates/harvester_core/src/state.rs`
- `crates/harvester_app/src/platform/ui/render.rs`

Update the view-model derivation so `triage_can_start` and `briefing_can_start` become false when the corresponding AI capability is unavailable, even if the rest of the workflow prerequisites are satisfied.

This should make the existing render code disable:
- `BUTTON_TRIAGE`
- `BUTTON_BRIEFING`

No render-layer special casing should be needed beyond the existing `SetControlEnabled` logic.

Tests to add:
- triage button disabled when pre-triage is ready but AI unavailable
- briefing button disabled when article corpus is otherwise briefing-ready but AI unavailable
- unrelated controls such as `Poll Sources` remain enabled

---

## Step 4 - Surface a persistent status-bar warning

Files likely affected:
- `crates/harvester_app/src/platform/ui/render.rs`

Update `render_status_section` so AI-unavailable text is rendered with `MessageSeverity::Warning`.

Recommended behavior:
- when `view.ai_unavailable_message` is present, severity becomes `Warning`
- the message is appended to the existing status line unless that makes the line too noisy; if so, prefer replacing the less-important trailing informational segment rather than truncating the warning

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

Recommended header behavior:

```text
Triage Results (AI unavailable)
```

when unavailable, otherwise keep the current count-based behavior.

If the current pane body already has a generic placeholder surface, populate it with a two-line message:

```text
Article triage is unavailable because OPENAI_API_KEY is not set.
Set OPENAI_API_KEY and restart the app to enable triage.
```

If no dedicated body surface exists without larger UI changes, updating the header plus the status-bar warning is still acceptable for the first slice.

Tests to add:
- triage-results header uses `AI unavailable` copy when blocked
- current `no triage results yet` copy remains for genuinely empty but available state

---

## Step 6 - Prevent misleading reducer transitions

Files likely affected:
- `crates/harvester_core/src/update.rs`

Today `Msg::TriageClicked` can switch to `LeftTab::TriageResults` before the metadata/availability problem is fully communicated. Tighten that path.

Recommended rule:
- if AI is explicitly unavailable, do not treat the click as a normal triage-start transition
- either keep the current tab unchanged, or allow the tab switch only if the target tab will now show the explicit unavailable state

Preferred first-pass behavior:
- keep the click a no-op for workflow state
- rely on disabled buttons so the click path is mostly unreachable in normal use

Why this still matters:
- keyboard shortcuts, stale UI state, or future event paths may still dispatch `Msg::TriageClicked`
- reducer logic should defend its invariants independently of the UI layer

Tests to add:
- `TriageClicked` emits no triage-start effects when AI unavailable
- `TriageClicked` does not consume pre-triage when AI unavailable
- `TriageClicked` does not incorrectly imply success by transitioning into a misleading state

---

## Step 7 - Apply the same pattern to Briefing

Files likely affected:
- `crates/harvester_core/src/update.rs`
- `crates/harvester_app/src/platform/ui/render.rs`

The original report surfaced in Triage, but Briefing depends on the same AI availability class and should not lag behind.

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
- `AI features unavailable: no briefing model is available`

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

3. If the reducer only checks startup-time missing key, future non-key failures may still look ambiguous.
Mitigation: design the enum for multiple reasons from the start.

4. Disabled buttons alone may not be enough for users who are already on the Triage Results tab.
Mitigation: also update header and placeholder copy.

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
