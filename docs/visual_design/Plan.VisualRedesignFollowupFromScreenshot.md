# Visual Redesign Follow-up Plan (Screenshot Review)

Date: 2026-04-03

Reference:
- `docs/visual_design/VisualDesignSpec.md`
- Screenshot review of the running `harvester_app` after the first warm-palette pass

## Goal

Close the remaining visible gaps between the implemented redesign and the target design system.

This pass is not about another global palette sweep. The warm palette is already mostly in place. The next work should focus on the gaps that remain obvious in the screenshot:

1. The jobs list is still a dense text dump rather than a scan surface.
2. The UI still relies too much on visible box framing and borders.
3. Secondary buttons are still too visually dominant.
4. The reading pane still feels like a utility panel more than an editorial surface.
5. Text hierarchy between primary, secondary, and tertiary content is still too weak.

## Repository Guardrails

- Keep all behavior inside the presentation layer unless a better view-facing data shape is genuinely needed.
- Preserve unidirectional data flow: input -> action -> reducer -> state -> render.
- Keep reducers pure; visual changes belong in render/layout/RTF formatting, not side-effect paths.
- If this pass adds new semantic styles to `CommanDuctUI`, update `src/CommanDuctUI/Cargo.toml`, append an entry to `src/CommanDuctUI/CHANGELOG.md`, and preserve dark-theme-safe defaults.
- When visual formatting changes alter user-facing text contracts, update the relevant unit tests rather than relying only on screenshot review.

## Screenshot Findings vs Spec

### 1. Jobs list violates the “scan surface” guidance

Observed:
- Each row reads as one long unbroken line of status, URL, tokens, and bytes.
- The most important information is not visually separated from metadata.
- The URL dominates the row instead of acting as supporting detail.

Spec conflict:
- `VisualDesignSpec.md` says list rows should be scannable before they are fully read.
- The recommended structure is title first, metadata second, sparse semantic emphasis, and aligned visual columns.

### 2. Pane separation still depends too much on framing

Observed:
- The tree area, token area, and reading pane still read as boxed regions.
- Borders and outlines are still doing too much of the structural work.

Spec conflict:
- The spec explicitly prefers spacing and tonal contrast over heavy framing.

### 3. Bottom action bar hierarchy is only partially implemented

Observed:
- `Generate Briefing` is correctly stronger than before.
- But `Triage Articles` and `Poll Sources` still read like near-peers instead of clearly demoted secondary actions.

Spec conflict:
- The spec calls for one dominant primary action per context and visual demotion of the rest.

### 4. Reading pane is improved, but not yet editorial

Observed:
- The raised surface helps, but the content still reads as a large, bright block.
- Long lines and strong contrast make the pane feel operational rather than comfortable.

Spec conflict:
- The reading pane should feel editorial, with controlled width, comfortable line height, and generous internal breathing room.

### 5. Text hierarchy is still too flat

Observed:
- Muted text is often too close to primary text.
- The jobs list, footer, and tab labels still compete too evenly.

Spec conflict:
- The spec expects hierarchy to come first from size, weight, spacing, and tone.

## Implementation Priorities

### Priority 1: Redesign the jobs rows

This is the highest-value change. It is the main thing preventing the UI from feeling like the target design.

#### Intent

Convert each row from:

- status + URL + bytes + tokens as one string

Into a two-level scan structure:

- primary line: short title / hostname / meaningful label
- secondary line: status, source, token count, byte count, and other metadata

#### Proposed behavior

- Prefer article title when available.
- Fall back to a shortened URL or host/path summary when title is unavailable.
- Move tokens/bytes out of the main title line.
- Keep status markers like `OK`, `ERR`, `REVIEW` compact and consistent.
- Use muted metadata styling for secondary information.
- Keep row text shorter overall so selected rows are easier to parse.

#### Likely files

- `crates/harvester_app/src/platform/ui/render.rs`

#### Current code note

- The primary target is `format_job_row_legacy()` in `render.rs`.
- `format_job_row_triage_review()` and `format_job_row_triage_results()` are already structurally cleaner and should be treated as minor-tuning paths, not full redesign targets.
- `JobRowView` already carries the fields needed for this pass, including `summary_title`, so no new view-model fields should be introduced unless a concrete need appears during implementation.

#### Constraints

- `TreeView` is still a Win32 tree control, so this pass likely remains text-based.
- Do not solve this with more punctuation or bracket syntax.
- Do not attempt multiline TreeView rows unless explicit owner-draw support is added to `CommanDuctUI`; standard Win32 TreeView items should be treated as single-line.
- Rely on compact prefixes, aggressive URL shortening, and moving tokens/bytes out of the primary scan path.

## Priority 2: Reduce visual chrome and rely more on tone/spacing

#### Intent

Make the app feel calmer and less boxed-in without requiring new CommanDuctUI capabilities.

#### Proposed changes

- Reduce or remove any remaining unnecessary edge outlines where surface contrast already separates regions.
- Make the toolbar token area read more like part of the top surface, less like three separately boxed widgets.
- Revisit tree and reading-pane surrounding margins so the major regions are defined by spacing first.
- Keep the splitter visible, but quieter.

#### Likely files

- `crates/harvester_app/src/platform/ui/layout.rs`

#### Current code note

- There is no evidence in current layout code of heavy explicit border drawing.
- Treat this priority primarily as a `layout.rs` spacing, margin, and surface-contrast pass.
- Do not plan treeview custom-draw changes here unless a concrete border artifact is traced to framework code.

#### Constraints

- CommanDuctUI still does not support radius, shadows, alpha, or true internal padding fields.
- This pass should stay inside tone, spacing, and existing control styles.

## Priority 3: Demote secondary buttons

#### Intent

Make only one constructive action feel primary in the footer.

#### Proposed changes

- Keep `Generate Briefing` as `PrimaryButton`.
- Keep `Stop / Finish` as `DestructiveButton`.
- Re-style `Triage Articles` and `Poll Sources` to feel lighter:
  - more muted text
  - quieter fill
  - optional border-like separation only if needed for affordance
- Keep disabled states visibly lower-contrast than today.

#### Likely files

- `crates/harvester_app/src/platform/ui/layout.rs`
- `src/CommanDuctUI/src/styling_primitives.rs` only if an additional semantic button style is needed

#### Constraints

- First try tuning `DefaultButton` itself before adding a new `SecondaryButton` style.
- If `DefaultButton` cannot cleanly represent a true secondary button, add a dedicated `SecondaryButton` style in CommanDuctUI in a small follow-up.
- If new semantic button styles are added to `CommanDuctUI`, the same change must update its version and changelog.

## Priority 4: Make the reading pane more editorial

#### Intent

Increase reading comfort without changing the underlying RichEdit architecture.

#### Proposed changes

- Further soften body-text contrast where possible while keeping headings strong.
- Constrain effective text width conservatively using fixed RTF indents if needed, not responsive max-width tricks.
- Increase breathing room above and below section breaks in the rendered RTF.
- Review list spacing in the reading pane so bullets and headings feel more intentional.
- Consider a slightly clearer distinction between H1, H2, and body in the RTF output.

#### Likely files

- `crates/harvester_app/src/platform/ui/markdown_to_rtf.rs`
- `crates/harvester_app/src/platform/ui/layout.rs`

#### Constraints

- RichEdit and the current RTF path limit layout sophistication.
- This pass should avoid fragile formatting tricks.
- Do not try to implement true responsive centering or dynamic max-width behavior by rewriting RTF on resize unless that becomes an explicit scoped feature.
- Prefer stable fixed left/right paragraph indents over resize-coupled layout logic.
- If body text is softened, keep the change conservative and verify readability against the current `ViewerReadable` background rather than making a large palette shift in the RTF layer.

## Priority 5: Strengthen text hierarchy

#### Intent

Make primary, secondary, tertiary, and disabled text easier to distinguish at a glance.

#### Proposed changes

- Audit where `DefaultText` is being used for metadata that should instead use tertiary or secondary styling.
- Reduce visual weight of inactive tab labels.
- Make footer text more clearly tertiary except for the most important live state.
- Revisit token-meter text so it only escalates when near the threshold, not by default.
- If the existing style vocabulary is insufficient, add semantic text styles in `CommanDuctUI` rather than hardcoding Harvester-specific colors into control handlers.

#### Current code note

- Do not assume `tab_bar_handler.rs` needs changes up front. Inactive tab contrast is already derived formulaically from the active palette and may be acceptable.
- Prefer auditing misuse of `DefaultText` and reusing existing muted styles such as `TreeItemDisabled` before adding new semantic tertiary-text styles.

#### Likely files

- `crates/harvester_app/src/platform/ui/layout.rs`
- `crates/harvester_app/src/platform/ui/render.rs`
- Potentially `src/CommanDuctUI/src/controls/tab_bar_handler.rs` if inactive-tab contrast needs framework-level tuning

## Suggested Execution Order

1. Jobs-row hierarchy and shortening in `render.rs`, primarily `format_job_row_legacy()`
2. Secondary/tertiary text audit across labels and footer, preferring existing muted styles before new framework styles
3. Footer button demotion, starting with `DefaultButton` tuning before introducing `SecondaryButton`
4. Pane chrome reduction and spacing pass in `layout.rs`
5. Reading-pane RTF refinement in `markdown_to_rtf.rs`
6. Update unit tests for row formatting and RTF output
7. Final screenshot review against the spec checklist

## Verification

For this pass, success should be evaluated visually first, then mechanically.

### Visual checks

- Active task is obvious within two seconds
- Jobs list can be scanned row-by-row without reading full URLs
- Primary action is obvious and secondary actions recede
- Reading pane feels calmer and easier to read than the current screenshot
- Borders are no longer carrying most of the structure

### Mechanical checks

- Record clean baselines before starting if this pass is executed in a fresh branch:
  - `cargo build`
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
- `cargo build`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- Update `crates/harvester_app/src/platform/ui/render.rs` unit tests to lock the new row-shortening and metadata-ordering behavior.
- Update `crates/harvester_app/src/platform/ui/markdown_to_rtf.rs` unit tests to lock any new spacing, indent, or color changes.
- If `CommanDuctUI` changes, verify its tests still pass and that the submodule version/changelog were updated in the same change.

## Confirmed Non-Issues

- No new view-model fields are required for the jobs-row pass based on the current code.
- TreeView selection accent handling is already implemented and is not itself a redesign problem.
- Tab-bar inactive contrast is already palette-derived; do not treat it as broken without fresh visual evidence.

## Non-goals for this follow-up

- Border radius
- Hover states
- Shadows
- Alpha effects
- Full custom row rendering beyond what current TreeView/CommanDuctUI support reasonably allows

Those remain future CommanDuctUI enhancement work unless this pass uncovers a blocker that cannot be solved with text, spacing, tone, and existing style hooks.
