# Plan: Markdown Rendering (MVP First)

## Purpose
Implement robust markdown rendering in the preview pane using native Win32 Rich Edit, while preserving unidirectional data flow and keeping reducers pure.

## Current Baseline (verified in source)
- `harvester_core` produces markdown briefing text (`#`, `##`, numbered items, `**bold**`) in `crates/harvester_core/src/briefing.rs`.
- `harvester_app` currently shapes markdown-ish text for a plain `CreateInput` viewer in `crates/harvester_app/src/platform/ui/render.rs`.
  - The `shape_for_viewer` pipeline **strips** markdown syntax (bold markers, headings get spacing but remain as `#` text, bullets become `•`). This must be **bypassed** entirely when the Rich Edit path is active — feeding already-mangled text into a markdown parser is wrong.
  - The `normalize_windows_newlines` call must also be **skipped** for RTF output; RTF uses `\par`, not CRLF.
  - The truncation guard (`MAX_VIEWER_CHARS = 64 KiB`) currently caps the shaped string. For RTF the guard must apply to the **markdown input** before conversion, because the RTF output is typically 2–4× larger.
  - Idempotence tracking (`prev_preview_text`) compares markdown strings, which remains correct since state is still markdown.
- `CommanDuctUI` currently supports `CreateInput` and `SetViewerContent`, but has no `CreateRichEdit` / `SetRichEditContent`.
- `CommanDuctUI` already includes `Win32_UI_Controls_RichEdit` in its feature flags (`Cargo.toml` version 0.2.6), so the Win32 API surface is ready.

## MVP Definition
MVP = Render a safe markdown subset as RTF in `VIEWER_PREVIEW` using Rich Edit:
- headings (`#`, `##`, `###`)
- emphasis/strong (`*`, `**`)
- unordered/ordered list items (visual simulation: `\bullet\tab`)
- paragraph breaks and line breaks
- strict RTF escaping and unicode safety
- graceful fallback for unsupported constructs (render as plain text via `pulldown-cmark`'s `Event::Text`)

No clickable links, tables, code-block syntax coloring, or nested-list perfection in MVP.

## Architectural Guardrails
- State remains a raw markdown string in core; no RTF in domain state.
- Conversion happens at the render edge in `harvester_app`.
- Reducers stay pure; no Win32/IO/parser side effects in update logic.
- Platform layer only executes commands (`PlatformCommand`) and sends events back.
- Traceability stays: `Action → Reducer → ViewModel → Render transform → PlatformCommand`.

## Blockers and Risks
1. **`shape_for_viewer` bypass (correctness blocker):** The existing pipeline that strips bold markers, normalizes bullets, and adds heading spacing must be fully removed from the Rich Edit code path. Feeding pre-mangled text into `pulldown-cmark` produces garbage.
2. **`\cb` background in Rich Edit:** `WM_CTLCOLOREDIT` does not fire for Rich Edit. Background color must be set via `EM_SETBKGNDCOLOR`. The RTF `\cbN` tag in the preamble is unreliable for setting the control background in RICHEDIT50W; rely on the Win32 message instead.
3. **Rich Edit class registration:** `Msftedit.dll` must be loaded before the first Rich Edit control is created. It must happen in `Win32ApiInternalState::new`.
4. **RTF group balancing:** Unbalanced `{}`/`\\` from escaping bugs silently crash or misrender the Rich Edit control. The converter must be thoroughly tested.
5. **`WM_SETTEXT` size limit:** Reliable for payloads under 64 KiB. For larger content, prefer `EM_STREAMIN`. The MVP truncates markdown input before conversion to stay within this budget.
6. **Submodule commit order:** The `CommanDuctUI` API additions must be committed and the top-level submodule pointer updated before `harvester_app` code can use the new commands. Mixing these in one commit causes build failures.
7. **Unicode surrogate pairs:** `\uN?` handles BMP codepoints (U+0000–U+FFFF). For codepoints above U+FFFF (e.g. emoji), the surrogate pair form `\uHIGH?\uLOW?` is required. The converter must handle this correctly or replace out-of-BMP characters with `?`.

---

## MVP Implementation Plan

### Step 1: Add Rich Edit control support in `CommanDuctUI` (submodule)
Scope:
- Add `ControlKind::RichEdit`.
- Add commands:
  - `PlatformCommand::CreateRichEdit { window_id, parent_control_id, control_id }`
  - `PlatformCommand::SetRichEditContent { window_id, control_id, rtf_text }`
- Add handler module `src/CommanDuctUI/src/controls/richedit_handler.rs`.
- In `Win32ApiInternalState::new`: call `LoadLibraryW(w!("Msftedit.dll"))` once.
  - Log a warning (do not panic) if the DLL fails to load; control creation will fail later with a clear error.
- Wire command execution in `src/CommanDuctUI/src/app.rs` + `src/CommanDuctUI/src/command_executor.rs`.
- Control creation notes:
  - Use `CreateWindowExW` with `MSFTEDIT_CLASS` as the class name.
  - After creation, send `EM_SETBKGNDCOLOR` for background color.
  - Send `EM_SETCHARFORMAT` (CFM_COLOR) to set default text color.
  - `WM_CTLCOLOREDIT` does **not** fire for Rich Edit — do not rely on it.
- Content setting:
  - Send `WM_SETTEXT` with the RTF string (must start with `{\rtf`).
  - For Step 6, upgrade to `EM_STREAMIN` for large or repeated payloads.
- Follow the exact guard pattern of `execute_create_input`: check for duplicate control ID, resolve parent HWND, register `ControlKind`, register HWND mapping.

Tests (in `src/CommanDuctUI`):
- Unit test: `CreateRichEdit` command routes to the richedit handler without error.
- Unit test: `SetRichEditContent` command returns an appropriate error when the target control does not exist.
- Unit test: `CreateRichEdit` returns an error when called a second time with the same control ID (idempotence guard mirrors `CreateInput` behavior).

Suggested commit message:
- `commanductui: add RichEdit control and rich-text platform commands`

### Step 2: Submodule release hygiene
Scope:
- Bump `src/CommanDuctUI/Cargo.toml` version to `0.2.7`.
- Add entry in `src/CommanDuctUI/CHANGELOG.md` documenting new `RichEdit` commands.
- Update the top-level workspace submodule pointer.

Tests:
- `cargo build` in submodule and top-level workspace succeeds after pointer update.

Suggested commit message:
- `commanductui: bump version and changelog for RichEdit support`

### Step 3: Add markdown→RTF converter in `harvester_app`
Scope:
- Add `pulldown-cmark` to `crates/harvester_app/Cargo.toml`.
- New module: `crates/harvester_app/src/platform/ui/markdown_to_rtf.rs`.
- Public API: `pub fn convert_markdown_to_rtf(markdown: &str) -> String`
- RTF preamble must include:
  - Font table: `{\f0 Segoe UI;}` (body), `{\f1 Consolas;}` (monospace, for future code blocks)
  - Color table matching the dark theme from `layout.rs`:
    - `\red224\green229\blue236` — foreground (matches `ViewerReadable` text color `0xE0, 0xE5, 0xEC`)
    - `\red26\green29\blue34` — background (matches `ViewerReadable` background `0x1A, 0x1D, 0x22`)
  - Default character formatting: `\cf1\f0\fs20`
- Event mapping for MVP subset:
  - `Start(Heading { level })` → `\pard\sa120\sb60\b\fs{size} ` (H1=36, H2=32, H3=28 half-points)
  - `End(TagEnd::Heading)` → `\b0\fs20\par\pard\sa60\sb0 `
  - `Start(Paragraph)` → `\pard\sa60\sb0 `
  - `End(TagEnd::Paragraph)` → `\par `
  - `Start(Strong)` → `\b `
  - `End(TagEnd::Strong)` → `\b0 `
  - `Start(Emphasis)` → `\i `
  - `End(TagEnd::Emphasis)` → `\i0 `
  - `Start(List(None))` → track unordered list depth
  - `Start(List(Some(n)))` → track ordered list start number
  - `Start(Item)` → `\par\pard\li360\fi-180\bullet\tab ` (unordered) or numbered variant
  - `End(TagEnd::Item)` → reset paragraph formatting
  - `Text(t)` → `escape_rtf_text(buf, &t)`
  - `SoftBreak` → single space ` `
  - `HardBreak` → `\line `
  - All other events → plain-text fallback via `Event::Text` (already handled)
- RTF text escaping (`escape_rtf_text`):
  - `\\` → `\\\\`
  - `{` → `\\{`
  - `}` → `\\}`
  - `\n` → `\\par ` (should rarely appear inside a text event, but handle defensively)
  - ASCII printable → as-is
  - BMP non-ASCII (U+0080–U+FFFF): `\uN?` where N is the signed decimal codepoint
  - Above-BMP (U+10000+): replace with `?` in MVP (add note about surrogate pairs for post-MVP)
- Truncation: apply `MAX_VIEWER_CHARS` cap to the **markdown input** before passing to the parser. Append a marker like `\par [display truncated]` at the end of the RTF output when truncated.
- Close RTF with single `}`.

Tests (in `crates/harvester_app`):
- Unit: `escape_rtf_text` correctly escapes `\`, `{`, `}`.
- Unit: `escape_rtf_text` produces `\uN?` for non-ASCII BMP characters.
- Unit: `escape_rtf_text` replaces above-BMP characters with `?`.
- Unit: heading levels H1/H2/H3 produce the expected font sizes in output.
- Unit: bold, italic, and nested bold-italic produce correct RTF tags.
- Unit: unordered list items include `\bullet\tab`.
- Unit: soft break produces a space; hard break produces `\line`.
- Golden/snapshot: a representative briefing markdown (with heading, bullet list, bold, paragraph) produces the expected RTF string exactly.
- Property (brace-balance): for any arbitrary string input, the `{` and `}` count in the output is balanced. Use a loop over a variety of inputs including empty, Unicode, injection attempts `{\\}`, and very long strings.
- Property (no panic): for any arbitrary string input, `convert_markdown_to_rtf` must not panic.
- Unit: input longer than `MAX_VIEWER_CHARS` produces output containing the truncation marker.

Suggested commit message:
- `harvester_app: add markdown-to-rtf renderer with safety guards`

### Step 4: Switch preview control creation to Rich Edit
Scope:
- In `crates/harvester_app/src/platform/ui/layout.rs`, replace `PlatformCommand::CreateInput` for `VIEWER_PREVIEW` with `PlatformCommand::CreateRichEdit`.
- Remove `read_only`, `multiline`, `vertical_scroll` fields from that command (Rich Edit is always multiline; read-only is set via `EM_SETREADONLY` in the handler).
- Set `ES_READONLY | ES_MULTILINE | WS_VSCROLL | ES_AUTOVSCROLL` style flags in the richedit handler.
- Keep `PlatformCommand::ApplyStyleToControl` with `StyleId::ViewerReadable` for font/color setup — but the Rich Edit handler must translate this to `EM_SETBKGNDCOLOR` + `EM_SETCHARFORMAT` rather than relying on `WM_CTLCOLOREDIT`.
- `INPUT_URLS` remains a normal Edit control (unaffected).

Tests:
- Existing layout tests must still pass.
- Add test: the command sequence emitted during layout initialization for `VIEWER_PREVIEW` is `CreateRichEdit`, not `CreateInput`.

Suggested commit message:
- `harvester_app: create preview pane as RichEdit control`

### Step 5: Wire render path to emit rich-text commands
Scope:
- In `crates/harvester_app/src/platform/ui/render.rs`:
  - Remove the `shape_for_viewer` call from the `VIEWER_PREVIEW` update path.
  - Remove the `normalize_windows_newlines` call from this path.
  - Apply markdown input truncation (guard to `MAX_VIEWER_CHARS`) before passing to the converter.
  - Call `convert_markdown_to_rtf(preview_markdown)`.
  - Emit `PlatformCommand::SetRichEditContent` instead of `SetViewerContent` for `VIEWER_PREVIEW`.
  - The idempotence comparison (`prev_preview_text`) continues to compare raw markdown strings — no change needed there.
- Keep `shape_for_viewer` and `normalize_windows_newlines` in the module (they may still be used for other plain-text controls or tests); just remove them from the preview path.

Tests:
- Unit: when preview text contains `## Heading`, the emitted command is `SetRichEditContent` (not `SetViewerContent`) and the `rtf_text` contains `\b`.
- Unit: when preview text contains `**bold**`, the `rtf_text` contains `\b` and `\b0`.
- Unit: when preview text is unchanged from previous render, no `SetRichEditContent` command is emitted (idempotence).
- Unit: when preview text exceeds `MAX_VIEWER_CHARS`, the `rtf_text` contains the truncation marker.
- Unit: `shape_for_viewer` tests in the module still pass (the function is preserved but not called from the preview path).

Suggested commit message:
- `harvester_app: render preview markdown as RichEdit RTF`

### Step 6: Harden large-content behavior (still MVP)
Scope:
- Upgrade `SetRichEditContent` implementation in `CommanDuctUI` to use `EM_STREAMIN` instead of `WM_SETTEXT`.
  - `EM_STREAMIN` avoids a 64 KiB `WM_SETTEXT` ceiling and handles encoding more robustly.
  - Implement an `EDITSTREAM` callback that serves the RTF bytes from a `&[u8]` slice.
- Retain the pre-conversion markdown truncation guard as a defense-in-depth layer even when `EM_STREAMIN` is used.
- Add logging (`engine_warn!`) when truncation fires, including character counts and the `[preview]` category tag.

Tests:
- Unit: input of 100 KiB markdown does not panic and produces stable (truncated) RTF output.
- Unit: the `EM_STREAMIN` callback implementation correctly streams all bytes of a test RTF string.

Suggested commit message:
- `commanductui/harvester_app: harden RichEdit content streaming and truncation`

### Step 7: Final validation gate
Scope:
- Run required project checks after full implementation:
  - `cargo build`
  - `cargo clippy --all-targets -- -D warnings`
- Fix any warnings surfaced.

Suggested commit message:
- `chore: finalize markdown rendering MVP validation`

---

## Commit / Integration Strategy (important for submodule)
1. Commit Steps 1–2 inside `src/CommanDuctUI`.
2. Update top-level repo submodule pointer to the new commit.
3. Commit Steps 3–7 in the top-level repo.
4. Keep submodule and top-level commits logically independent; avoid mixed atomic changes that depend on unpublished submodule API.

---

## Post-MVP Roadmap

### Step 8: Links and interaction
Scope:
- In `CommanDuctUI`: enable `AutoURLDetect` (`EM_AUTOURLDETECT`) and handle `EN_LINK` notifications by dispatching a platform event.
- In `harvester_app`: map the `EN_LINK` event to an `AppEvent::OpenUrlInBrowser` action via the existing effect path.
- Addresses `[FI-UX-PreviewRich-0001]` in FutureIdeas.md.

Suggested commit message:
- `commanductui: add RichEdit link notifications`

### Step 9: Improved markdown coverage
Scope:
- Code blocks (`Tag::CodeBlock`): switch to `\f1` (Consolas), add a background tint via a text highlight color.
- Blockquotes (`Tag::BlockQuote`): left-indent with `\li360`, italicize.
- Horizontal rules (`Tag::Rule`): emit a paragraph with a bottom border (`\brdrb\brdrs`).
- Nested list indentation: track list depth and multiply `\li` indent by depth.

Suggested commit message:
- `harvester_app: extend markdown-to-rtf coverage for code blocks and blockquotes`

### Step 10: Outline navigation
Scope:
- During `convert_markdown_to_rtf`, also extract headings into a `Vec<(u8, String)>` (level, text) alongside the RTF string.
- Return both from the converter as a struct `RtfResult { rtf: String, headings: Vec<OutlineEntry> }`.
- Surface headings in the view model and render them as a selectable list above or beside the preview.
- Clicking an outline entry navigates the Rich Edit control via `EM_FINDTEXT` / `EM_SCROLLCARET`.
- Addresses `[FI-UX-PreviewOutline-0001]` in FutureIdeas.md.

Suggested commit message:
- `harvester_app: add heading outline extraction for preview navigation`

### Step 11: Find-in-preview
Scope:
- Add a search input that drives `EM_FINDTEXTEX` and `EM_SETCHARFORMAT` (highlight) in the Rich Edit control.
- Addresses `[FI-UX-PreviewSearch-0001]` in FutureIdeas.md.

Suggested commit message:
- `harvester_app: add find-in-preview using EM_FINDTEXTEX`

### Step 12: Raw/rich toggle
Scope:
- Add a toggle button in the preview header to switch between rendered RTF and raw markdown.
- Raw mode re-uses the old `SetViewerContent` path with minimal shaping (no bold-stripping, no bullet normalization — just truncation and CRLF normalization).
- Addresses `[FI-UX-PreviewRich-0001]` (toggle mode) in FutureIdeas.md.

Suggested commit message:
- `harvester_app: add raw/rich markdown toggle for preview pane`

### Step 13: Observability and diagnostics
Scope:
- Structured logging around render conversions and truncation counts (category tag `[markdown-rtf]`).
- Optional debug export of last generated RTF to a temp file for troubleshooting.

Suggested commit message:
- `harvester_app: add markdown render telemetry and debug diagnostics`

---

## Additional Ideas and Extensions
- Export briefing as `.rtf` artifact alongside markdown outputs (see `[FI-Storage-ExportArtifacts-0001]`).
- Section jump list from heading parse events for long briefings.
- Future optional abstraction: `PreviewDocument` intermediate model for multi-renderer targets (plain text, RTF, HTML).
- Above-BMP unicode (emoji, supplementary characters): implement surrogate-pair RTF encoding post-MVP rather than replacing with `?`.
