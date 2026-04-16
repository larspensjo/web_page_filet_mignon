# API Key Warning Visibility — Design & Implementation Plan

## Goal

Make a missing `OPENAI_API_KEY` hard to miss in `harvester_app` by elevating it from a footer-only warning to a persistent, high-visibility setup notice, while also making the disabled AI workflows explain themselves clearly where the user encounters them.

This plan covers the combined implementation of:

1. a persistent warning strip in the main workflow area
2. stronger AI-disabled affordances in the affected workflow surfaces

The intended result is that the app reads as "setup incomplete" rather than "quietly degraded."

## Constraints

- Preserve unidirectional flow: platform/input -> `Msg` -> reducer -> state -> render.
- Keep reducers pure and unit-testable.
- Keep `harvester_app` platform code thin; derive copy and visibility in `harvester_core` where practical.
- Do not push Harvester-specific behavior into `CommanDuctUI`.
- Keep the footer warning as a secondary/status surface; do not remove it.
- Prefer small, local UI additions over expanding the custom tab-bar widget.

## Current State

- Startup already detects whether `OPENAI_API_KEY` is present in `crates/harvester_app/src/platform/app.rs`.
- Core state already exposes AI availability through:
  - `ai_unavailable_message`
  - `triage_blocked_reason`
  - `briefing_blocked_reason`
- The footer appends `AI features unavailable: OPENAI_API_KEY is not set` and switches to warning severity.
- Triage and Briefing already show blocked placeholder copy, but only after the user navigates to those tabs.
- The right-pane tab bar is currently static; there is no obvious existing support for dynamic badges or warning chips.
- The preview pane layout currently docks top-level children in this order:
  - right tab bar
  - preview header
  - preview context row
  - active tab content

## Problem Statement

The app does technically explain the problem today, but it does so in the least prominent place: the footer. That is appropriate for transient status, not for a configuration prerequisite that disables a major part of the product.

Two UX failures follow from that:

1. users can miss the warning entirely during startup
2. users can reach disabled AI workflows without seeing immediate, actionable setup guidance

## Recommendation Summary

Implement this in two tracks within one change set:

1. Add a persistent warning strip near the top of the preview pane whenever AI is unavailable because the API key is missing.
2. Strengthen the AI-disabled workflow surfaces by making the Triage and Briefing panes show explicit setup-required content, and by keeping the copy actionable and restart-oriented.

Do not implement tab badges in this pass. The current tab-bar API appears selection-oriented rather than content-rich, and adding badge support would widen scope into infrastructure.

---

## Track 1 — Persistent Warning Strip

### Outcome

When `OPENAI_API_KEY` is missing, the main workflow area should show a persistent warning strip immediately below the right-pane tab bar.

Example copy:

- Title: `AI features are disabled`
- Body: `Set OPENAI_API_KEY in the launch environment and restart to enable AI features.`

This should remain visible regardless of the active right-pane tab so the setup issue is visible even if the user stays on `Summary`, `Trends`, or `Poll Stats`.

### Why this location

- It is far more prominent than the footer.
- It avoids modal interruption.
- It is close to the AI tabs that are affected.
- The preview pane already has dedicated header/context rows, so adding one more top-docked row is structurally straightforward.

### Design

Add a new reducer-derived view surface for a persistent inline warning banner. Do not ask the renderer to infer banner content by parsing `ai_unavailable_message`; give it explicit banner view data.

Recommended new view model type in `harvester_core`:

```rust
pub struct InlineWarningView {
    pub title: String,
    pub body: String,
}
```

Recommended new `AppViewModel` field:

```rust
pub ai_warning_banner: Option<InlineWarningView>
```

Recommended visibility rule:

- show the banner only for `AiUnavailableReason::MissingApiKey`
- do not show it for `NoTriageModel`

That keeps the banner focused on the specific startup/setup problem the user reported and avoids over-promoting other availability states that may be expected or temporary.

### Proposed UI structure

Add a dedicated preview-pane warning row directly below `TAB_BAR_RIGHT` and above the optional preview header/context rows.

Recommended parent ordering in `PANEL_PREVIEW`:

- `TAB_BAR_RIGHT` at order `0`
- `PANEL_AI_WARNING` at order `1`
- `LABEL_PREVIEW_HEADER` at order `2`
- `PANEL_PREVIEW_CONTEXT` at order `3`

Recommended controls:

- `PANEL_AI_WARNING = ControlId::new(2008)`
- `LABEL_AI_WARNING_TITLE = ControlId::new(3016)`
- `LABEL_AI_WARNING_BODY = ControlId::new(3017)`

Recommended behavior:

- hidden/collapsed when `ai_warning_banner` is `None`
- visible with warning styling when `Some`
- title bold or accent-colored
- body concise enough to fit on one line if the label control does not wrap

Because Win32 label wrapping behavior may be limited in this code path, keep the body to one sentence. If wrapping proves reliable, a two-line banner is acceptable, but the first implementation should target a compact single-row strip.

### Proposed code changes

#### `crates/harvester_core/src/view_model.rs`

Add `InlineWarningView` and a new `AppViewModel::ai_warning_banner` field.

Default it to `None`.

#### `crates/harvester_core/src/state/view_builder.rs`

Build `ai_warning_banner` from AI availability state.

Recommended copy source of truth:

- title: `AI features are disabled`
- body: `Set OPENAI_API_KEY in the launch environment and restart to enable AI features.`

This should be generated in one place rather than duplicated in platform rendering.

#### `crates/harvester_app/src/platform/ui/constants.rs`

Add control IDs for the new panel and labels.

Reserve these concrete IDs unless another in-flight change has already claimed them:

- `PANEL_AI_WARNING = ControlId::new(2008)`
- `LABEL_AI_WARNING_TITLE = ControlId::new(3016)`
- `LABEL_AI_WARNING_BODY = ControlId::new(3017)`

#### `crates/harvester_app/src/platform/ui/layout/init.rs`

Create the new panel and labels as children of `PANEL_PREVIEW` / `PANEL_AI_WARNING`.

#### `crates/harvester_app/src/platform/ui/layout/rules.rs`

Insert a new top-docked layout row for `PANEL_AI_WARNING` immediately below `TAB_BAR_RIGHT`.

Recommended explicit ordering in `rules.rs`:

- keep `TAB_BAR_RIGHT` at order `0`
- add `PANEL_AI_WARNING` at order `1`
- move `LABEL_PREVIEW_HEADER` to order `2`
- move `PANEL_PREVIEW_CONTEXT` to order `3`

Within the banner panel:

- dock title to top or left depending on chosen layout
- dock body to fill or below title
- collapse the panel to zero height when not visible

If the banner is implemented as a single compact row, the panel can be around `36-44` px high. Avoid letting this grow into a `64` px block unless readability testing proves it is necessary.

#### `crates/harvester_app/src/platform/ui/layout/theme.rs`

Apply distinct warning styling so the strip does not look like ordinary metadata.

Recommended visual direction:

- warm warning background tint
- stronger foreground for title
- standard readable foreground for body

Do not make it visually identical to the footer status label. The whole point is that this surface reads as a setup warning rather than routine status.

#### `crates/harvester_app/src/platform/ui/render_preview.rs`

Render the banner title/body text and clear them when the banner is hidden.

This file is already responsible for preview-pane textual surfaces, so it is the right place to update the warning strip.

#### `crates/harvester_core/src/state/tests.rs` and/or existing view-builder tests

Add coverage for:

- banner visible when AI is unavailable due to missing API key
- banner hidden when AI is available
- banner hidden when AI is unavailable for non-key reasons
- banner copy matches the chosen source-of-truth text

#### `crates/harvester_app/src/platform/ui/layout/tests.rs`

Add layout tests for:

- warning strip controls are created at startup
- warning strip sits immediately below `TAB_BAR_RIGHT`
- warning strip collapses cleanly when hidden

#### `crates/harvester_app/src/platform/ui/render_tests.rs`

Add rendering tests for:

- warning banner text is emitted when `ai_warning_banner` is present
- banner text is cleared when `ai_warning_banner` becomes `None`

### Risks

- If the banner is too tall, it will reduce visible content area in the preview pane.
- If the copy is too verbose, it may truncate on smaller windows.
- If the styling is too subtle, it will not solve the visibility problem.

### Mitigations

- Keep the body short and action-oriented.
- Prefer a fixed-height compact strip.
- Leave the footer warning intact as a redundant secondary signal.

---

## Track 2 — Stronger AI-Disabled Affordances

### Outcome

When the user reaches Triage or Briefing while the API key is missing, the content should clearly say that setup is required and exactly what to do next.

The disabled workflows should read as intentionally blocked, not mysteriously empty.

### Recommended scope for this pass

Implement the stronger affordances in surfaces that already exist:

1. Triage tab placeholder copy
2. Briefing tab placeholder copy
3. Existing blocked-reason strings that drive button-disabled reasoning and related UI text

Do not add tab badges in this pass.

### Design

The current placeholder copy is technically correct but understated:

- `Article triage is unavailable because OPENAI_API_KEY is not set.`
- `Briefing is unavailable because OPENAI_API_KEY is not set.`

Replace that with more explicit setup-oriented copy, but only for the `MissingApiKey` case.

Keep generic blocked-state phrasing for other reasons such as pre-triage loading or model-unavailable states. The setup-required framing should not leak into transient or non-user-fixable conditions.

Recommended Triage placeholder:

```markdown
AI setup required

Triage is disabled because `OPENAI_API_KEY` is not set.

Set `OPENAI_API_KEY` in the launch environment and restart the app to enable article triage.
```

Recommended Briefing placeholder:

```markdown
AI setup required

Briefing is disabled because `OPENAI_API_KEY` is not set.

Set `OPENAI_API_KEY` in the launch environment and restart the app to enable briefing generation.
```

This keeps the message:

- specific to the affected feature
- explicit about the missing prerequisite
- explicit about the required restart

Avoid markdown heading syntax in the first pass. A plain leading line is less likely to render awkwardly in the RichEdit placeholder styling, and richer heading treatment can be added later if rendering looks good.

### Why placeholders instead of button-row helper text

The button row is already dense and uses fixed widths for its primary actions. Adding explanatory text there would likely create layout pressure and compete with operational controls.

The preview panes already accept markdown placeholder content and are where the user goes to understand why an AI feature is not producing output. That makes them the lowest-risk place to strengthen affordances.

### Proposed code changes

#### `crates/harvester_core/src/state/mod.rs`

Review `ai_unavailable_reason_text()`, `triage_blocked_reason()`, and `briefing_blocked_reason()`.

Recommended adjustment:

- keep `ai_unavailable_reason_text()` compact for footer/meta usage
- make `triage_blocked_reason()` and `briefing_blocked_reason()` more user-facing and action-oriented if needed

One good split is:

- compact/system-oriented reason: `OPENAI_API_KEY is not set`
- user-facing blocked reason: `AI setup is incomplete because OPENAI_API_KEY is not set`

#### `crates/harvester_core/src/state/view_builder.rs`

Rewrite the Triage and Briefing placeholder markdown to use a setup-required framing for `MissingApiKey`, while preserving a generic unavailable/loading template for other blocked reasons.

This file already constructs those placeholders and is the right place to centralize the improved copy.

#### `crates/harvester_core/src/state/tests.rs`

Add or update tests for:

- Triage placeholder content when the API key is missing
- Briefing placeholder content when the API key is missing
- generic placeholder content when the blocked reason is not the API key
- blocked reasons remain absent when AI is available

#### `crates/harvester_app/src/platform/ui/render_tests.rs`

If there are existing preview-content render tests for empty states, add assertions that the updated markdown is emitted for Triage and Briefing when blocked.

### Optional enhancement within the same track

If the preview context row can support it cleanly, add a short attention label when the active tab is `Triage` or `Briefing` and AI is blocked. The control infrastructure already exists via `LABEL_PREVIEW_ATTENTION`, so this is a low-risk enhancement if it fits naturally.

Example:

- attention: `Setup required`

Only do this if it fits the existing preview-context model naturally. It is optional because the banner plus stronger placeholders already solve the primary problem.

---

## Implementation Order

### Phase 1 — Core view-model plumbing

1. Add `InlineWarningView` and `AppViewModel::ai_warning_banner`.
2. Build the banner in `view_builder.rs` for `MissingApiKey` only.
3. Keep existing footer warning behavior unchanged.

### Phase 2 — Preview-pane warning strip

1. Add new control IDs.
2. Create the new panel and labels.
3. Add layout rules and hidden/visible sizing behavior.
4. Apply warning styling.
5. Render title/body text.

### Phase 3 — Stronger blocked AI surfaces

1. Rewrite Triage placeholder copy.
2. Rewrite Briefing placeholder copy.
3. Adjust blocked-reason helpers if needed for clearer user-facing language.

### Phase 4 — Tests and polish

1. Add view-model tests.
2. Add layout tests.
3. Add render tests.
4. Manually verify small-window readability and banner prominence.
5. Add a short entry to `docs/EngineeringDiary.md` describing the warning strip and the decision to scope it to `MissingApiKey` only.

---

## Testing Plan

### Automated

- `cargo build`
- targeted Rust tests for state/view-builder changes
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all`

Recommended test additions:

- `ai_warning_banner_present_for_missing_api_key`
- `ai_warning_banner_absent_when_ai_available`
- `ai_warning_banner_absent_for_non_key_ai_unavailability`
- `triage_placeholder_uses_setup_required_copy_when_api_key_missing`
- `briefing_placeholder_uses_setup_required_copy_when_api_key_missing`
- `triage_placeholder_preserves_generic_copy_for_non_api_block_reason`
- layout test proving the banner row sits immediately below the right tab bar
- render test proving banner labels update and clear correctly

### Manual

Launch `harvester_app` without `OPENAI_API_KEY` and confirm:

1. the footer still shows the warning
2. the new warning strip is visible immediately on startup
3. the banner remains visible when switching tabs
4. the Triage tab shows setup-required copy
5. the Briefing tab shows setup-required copy
6. `Trends` and `Poll Stats` remain usable and visually subordinate to the setup warning

Launch again with `OPENAI_API_KEY` set and confirm the banner disappears entirely.

---

## Non-Goals

- Startup modal/dialog
- Launcher-script validation
- Dynamic tab badges/chips in the custom tab bar
- Any `CommanDuctUI` widget API expansion

These can be revisited later if the banner still proves too easy to miss, but they should not be part of the first implementation.

## Open Questions

1. Should the banner appear on every tab, or only on AI-related tabs and the default landing tab?

Recommendation: show it on every tab while the key is missing. The user problem is setup visibility, not only feature-local explanation.

2. Should the banner mention only Triage and Briefing, or all AI-backed capabilities?

Recommendation: mention `Triage and Briefing`, since those are the obvious user-facing features in the app UI today.

3. Should the copy mention exactly where to set the variable?

Recommendation: not in the first pass. Keep the banner concise. If needed later, add launcher-specific setup guidance elsewhere.

## Success Criteria

- A first-time user launching without `OPENAI_API_KEY` notices within a few seconds that AI setup is incomplete.
- The user can infer the required fix from the UI alone: set the key and restart.
- Triage and Briefing no longer feel broken or mysteriously unavailable.
- The implementation stays within `harvester_core` state/view-building and `harvester_app` rendering/layout, without widening scope into shared UI infrastructure.
