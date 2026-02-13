# Plan: Briefing Preview Presentation Upgrade

## Goal
Improve briefing readability and presentation in the preview pane while preserving the
unidirectional data flow architecture and keeping rendering robust on the existing Win32
text control.

## Scope
- Improve typography (proportional font for briefing/summary preview content).
- Improve output structure from the LLM flow so content is easier to scan.
- Improve preview rendering behavior for markdown-like content without introducing fragile UI coupling.
- Add tests that lock behavior and prevent regressions.

## Non-Goals
- No full rich-text/HTML control migration in this iteration.
- No changes to persistence model for existing summary cache keys unless explicitly required.
- No release build work.

## Current State (Observed)

### Source locations
| Concern                        | File                                                              |
|-------------------------------|-------------------------------------------------------------------|
| Briefing domain state          | `crates/harvester_core/src/briefing.rs`                          |
| View model                     | `crates/harvester_core/src/view_model.rs`                        |
| Reducer                        | `crates/harvester_core/src/update.rs`                            |
| Render path (app)              | `crates/harvester_app/src/platform/ui/render.rs`                 |
| UI layout and styles           | `crates/harvester_app/src/platform/ui/layout.rs`                 |
| Style IDs (submodule)          | `src/CommanDuctUI/src/styling_primitives.rs`                     |
| Briefing prompts               | `crates/harvester_engine/src/llm/prompts/briefing.rs`            |

### Key observations
- `VIEWER_PREVIEW` is a read-only multiline input styled with `StyleId::ViewerMonospace`
  (Cascadia Code 10pt, `#00C9FF` on `#1A1D22`). The bright cyan on dark is hard to read
  for long-form prose.
- `BriefingSession::format_preview()` produces a delimiter-heavy string
  (`=== Executive Briefing ===`, etc.) with no markdown structure.
- `AppViewModel` already has a `briefing_preview: Option<String>` field, but the render path
  in `render.rs` uses `view.preview_text` for the viewer. Whether and how
  `briefing_preview` is routed into the viewer is the integration point to clarify before
  implementation.
- `normalize_windows_newlines` in `render.rs` is the only pre-processing step in the
  render path today.
- `StyleId` is stored in a `HashMap` in CommanDuctUI (not exhaustively matched), so adding
  a new variant requires only: (1) the enum entry in the submodule, (2) one `DefineStyle`
  call, (3) one `ApplyStyleToControl` call. No cascading match arm updates.
- Briefing prompts (V1–V3) request plain prose; they do not instruct markdown structure
  inside JSON string fields.
- `BriefingResult::theme_summary()` produces numbered plain text lines.

## Proposed Architecture

1. Keep JSON schema and validator as source-of-truth contract throughout.
2. Introduce a dedicated preview formatting function in `harvester_core::briefing`:
   - `BriefingSession::format_preview()` → structured markdown-like text.
   - No new public types needed in this iteration; the output is still `String`.
3. Introduce a lightweight markdown shaper in `harvester_app::platform::ui::render`:
   - Input: raw preview string from view model.
   - Output: text shaped for the Win32 text control
     (heading spacing, bullet normalization, whitespace limits, Windows newlines).
   - Stateless pure function; testable in isolation.
4. Add `StyleId::ViewerReadable` to the CommanDuctUI submodule and wire it to
   `VIEWER_PREVIEW` only.
5. Keep all state mutation in reducers; renderer remains a stateless transform at the edge.

## Blocker Analysis

| # | Blocker | Mitigation |
|---|---------|------------|
| 1 | `VIEWER_PREVIEW` is a Win32 Edit control; inline styles (bold, heading sizes) are not possible without migrating to RichEdit or a custom renderer. | Accept limitation. Use visual spacing (blank lines, indent, ASCII decoration) to distinguish sections. True rich rendering is a future item (FI-UX-PreviewRich-0001). |
| 2 | Adding `StyleId::ViewerReadable` requires bumping the CommanDuctUI submodule version and updating its CHANGELOG. | Plan includes the submodule bump. Not a blocker — process is documented in Agents.md. |
| 3 | `briefing_preview` field exists in `AppViewModel` but may not be routed through to `VIEWER_PREVIEW`. Clarify before Phase 2. | Inspect `update.rs` / `state.rs` to confirm the route during Phase 1 and document the finding before writing Phase 2 code. |
| 4 | Prompt wording changes in Phase 3 can shift output quality across models. | Keep changes minimal; add a new prompt version (V4) rather than editing existing ones. Verify test assertion on version count. |

## Implementation Plan

### Phase 1: Typography — `ViewerReadable` style
**Crates touched:** `CommanDuctUI` (submodule), `harvester_app`.

1. Add `StyleId::ViewerReadable` to `src/CommanDuctUI/src/styling_primitives.rs`.
2. Bump `version` in `src/CommanDuctUI/Cargo.toml` and update its `CHANGELOG`.
3. In `crates/harvester_app/src/platform/ui/layout.rs`:
   - Define `StyleId::ViewerReadable`:
     - Font: `"Segoe UI"`, size 10 (proportional), weight Normal.
     - Background: same dark as `DefaultInput` (`#1A1D22`).
     - Text: calmer near-white (`#D8DEE9`) — readable for long-form prose.
   - Apply `StyleId::ViewerReadable` to `VIEWER_PREVIEW` instead of `ViewerMonospace`.
   - Leave `ViewerMonospace` applied to `INPUT_URLS` and anything else that expects it.
4. `cargo build` — validate no compile errors.

**Test:** No unit test needed for style selection (platform-level, not pure logic).
Visual inspection sufficient for this phase.

### Phase 2: Structured Briefing Output
**Crates touched:** `harvester_core`.

Rewrite `BriefingSession::format_preview()` in `crates/harvester_core/src/briefing.rs`:

```
# Executive Briefing

## Executive Summary

{executive_summary}

## Themes

1. **{theme.name}** — {theme.description}
2. ...

## Session Info

Articles: {total} total, {summarized} summarized, {failed} failed
```

Rules:
- Use `#` / `##` heading markers so the future markdown renderer or shaper can detect them.
- Keep `**bold**` markers around theme names as hints for future rich rendering.
- Use `---` horizontal rule as section separator only if the shaper (Phase 4) will convert
  it to a blank line; otherwise omit — YAGNI.
- Single blank line between sections; no trailing blank lines.
- Cap the total preview length at `MAX_BRIEFING_PREVIEW_CHARS` (e.g. 32 768) to prevent
  UI lag on large briefings. Truncate with an `[...truncated]` marker.

Confirm the routing of `briefing_preview` in `AppViewModel` → `view.preview_text` in
the reducer/view assembly before writing this phase. If `briefing_preview` is not yet
wired, add the minimal wiring as part of this phase (reducer change only — no new fields).

**Tests** (in `harvester_core`):
- `briefing_format_preview_contains_sections` — assert `# Executive Briefing`,
  `## Executive Summary`, `## Themes`, `## Session Info` headings present.
- `briefing_format_preview_theme_list_stable` — known themes render in numbered order.
- `briefing_format_preview_counts_correct` — article/summarized/failed counts match session.
- `briefing_format_preview_none_when_not_complete` — returns `None` in non-Complete phases.
- `briefing_format_preview_truncates_at_limit` — a synthetic result exceeding the char cap
  produces output ending with `[...truncated]`.

### Phase 3: Prompt Guidance (Without Schema Break)
**Crates touched:** `harvester_engine`.

Add `BRIEFING_PROMPT_V4` in `crates/harvester_engine/src/llm/prompts/briefing.rs`:

- Request markdown-friendly prose *inside JSON string fields*:
  - `executive_summary`: concise paragraphs, may use `**key term**` for emphasis.
  - Theme `description`: one or two sentences, plain prose.
- Preserve `expected_format` and schema fields unchanged.
- Do not add new JSON fields.

Update the prompt registry to mark V4 as the active version.

**Tests** (in `harvester_engine`):
- Prompt registry resolves `AggregateBriefing` to V4.
- Version count assertion updated to 4.

### Phase 4: Preview Renderer Robustness
**Crates touched:** `harvester_app`.

Add a pure function `shape_for_viewer(text: &str) -> String` in
`crates/harvester_app/src/platform/ui/render.rs`:

Transformations (applied in order, before `normalize_windows_newlines`):
1. **Heading spacing**: insert a blank line before any line beginning with `#` (unless
   already preceded by a blank line or is the first line).
2. **Bullet normalization**: normalize `- ` and `* ` list markers to `• ` for
   Win32 rendering (avoids markdown syntax noise in plain control).
3. **Bold marker stripping**: strip `**…**` markers (since the control cannot render bold).
4. **Blank-line capping**: collapse runs of 3+ consecutive blank lines to 2 blank lines.
5. **Length guard**: if the shaped text exceeds `MAX_VIEWER_CHARS` (64 KiB characters),
   truncate with `\r\n[display truncated]`.

Wire `shape_for_viewer` into the preview render path:
```rust
let preview_text = view
    .preview_text
    .as_deref()
    .map(shape_for_viewer)          // new
    .as_deref()
    .map(normalize_windows_newlines)
    .unwrap_or_default();
```

Keep `shape_for_viewer` side-effect free and pure.

**Tests** (in `harvester_app`):
- `shape_adds_blank_line_before_heading` — `"text\n# H"` → blank line inserted.
- `shape_heading_already_preceded_by_blank_not_doubled` — idempotent.
- `shape_bullet_normalized` — `"- item"` → `"• item"`.
- `shape_bold_markers_stripped` — `"**term**"` → `"term"`.
- `shape_blank_line_runs_capped` — 4 blank lines → 2.
- `shape_length_guard_truncates` — string over limit gets `[display truncated]` suffix.
- `render_preview_uses_shaper` — integration: a `view.preview_text` with `# Heading` produces
  `SetViewerContent` with the heading-spaced form.

### Phase 5: Test Gate
Run the full test suite and clippy:
```
cargo build
cargo test -p harvester_core -p harvester_app -p harvester_engine
cargo clippy --all-targets -- -D warnings
```

Expected: all new tests pass, no regressions, no new warnings.

## Robustness and Future-Proofing

- `shape_for_viewer` is intentionally dumb: it only applies visual spacing and strips
  markers that cannot render. No markdown AST, no parser dependency.
- The `MAX_BRIEFING_PREVIEW_CHARS` and `MAX_VIEWER_CHARS` constants prevent UI lag on
  pathological inputs.
- Plain-text fallback: if model output contains no markdown markers, the shaper is a no-op.
- The `ViewerReadable` style is applied only to `VIEWER_PREVIEW`. All other controls are
  unaffected.
- The submodule version bump follows the existing procedure (Agents.md: bump Cargo.toml
  version, update CHANGELOG, note if breaking).

## Validation Checklist (When Implementing)
1. `cargo build` after each phase.
2. `cargo test -p harvester_core -p harvester_app -p harvester_engine` after Phases 2–4.
3. `cargo clippy --all-targets -- -D warnings` as final gate.
4. Visual inspection of briefing preview text in the running app after Phase 1 (font change)
   and after Phase 2/4 (structure change).

## Optional Extensions (Next Iteration)
These are captured in FutureIdeas.md and are explicitly out of scope here:

1. **Raw vs. Formatted toggle** — let operator switch between shaped and raw preview.
   (Related: FI-UX-PreviewRich-0001)
2. **Copy-as-markdown** — clipboard action for briefing output.
3. **Outline navigation** — heading list for quick scroll. (FI-UX-PreviewOutline-0001)
4. **Per-section collapsing** — Executive Summary / Themes / Session Info collapse.
5. **Rich rendering** — migrate `VIEWER_PREVIEW` to RichEdit for actual bold/heading sizing.
   (FI-UX-PreviewRich-0001)
6. **Typed preview document structs** — `BriefingPreviewDocument` intermediate type to
   eliminate ad-hoc string assembly and enable structured export.
   (Related: FI-Storage-ExportArtifacts-0001)
7. **Preview quality indicators** in header: stub/paywall/duplicate signals.
   (FI-UX-PreviewIndicators-0001)
