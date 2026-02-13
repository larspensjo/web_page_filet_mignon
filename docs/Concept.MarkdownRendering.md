# Concept: Native Markdown Preview via Rich Edit

**Status:** Proposal
**Target:** Richer visualization of content without web browser overhead.
**Architecture:** Unidirectional Data Flow (Render-time transformation).

## 1. Objective

Replace the existing plain-text Win32 Edit control (`VIEWER_PREVIEW`) with a **Rich Edit 4.1 (`RICHEDIT50W`)** control. Implement a transformation layer that converts raw Markdown into Rich Text Format (RTF) streams to support headers, emphasis, and lists.

This implementation directly addresses `[FI-UX-PreviewRich-0001]` from `FutureIdeas.md`.

## 2. Architectural Design

We adhere to the **Unidirectional Data Flow**:
1.  **State**: `harvester_core` continues to store and expose `preview_text` as a raw Markdown `String`. The core logic remains ignorant of UI presentation details (RTF).
2.  **Render**: `harvester_app`'s `render` module transforms this Markdown string into an RTF-formatted string using a transient parser.
3.  **Command**: A new `PlatformCommand` instructs `CommanDuctUI` to update the control content.
4.  **Platform**: `CommanDuctUI` wraps the Win32 `RichEdit` API to display the content.

### Data Flow Diagram

```mermaid
graph LR
    S[AppState (Markdown)] -->|View Model| R[harvester_app Render]
    R -->|Input String| P[pulldown-cmark Parser]
    P -->|Events| G[RTF Generator]
    G -->|RTF String| C[PlatformCommand::SetRichEditContent]
    C -->|Msg Loop| W[Win32 RichEdit Control]
```

## 3. Implementation Breakdown

### 3.1. External Dependencies

We require a robust, spec-compliant Markdown parser. Writing a parser from scratch is error-prone.
*   **Crate:** `pulldown-cmark`
*   **Location:** Add to `crates/harvester_app/Cargo.toml`.
*   **Justification:** It is the de-facto standard in Rust, extremely fast (zero-allocation parsing where possible), and event-based (perfect for streaming to an RTF buffer).

### 3.2. Crate: `CommanDuctUI` (Platform Layer)

We need to introduce the Rich Edit control capabilities.

**Tasks:**
1.  **Initialize Library**: In `Win32ApiInternalState::new`, call `LoadLibraryW(w!("Msftedit.dll"))` to register the `RICHEDIT50W` window class.
2.  **New Control Kind**: Add `ControlKind::RichEdit`.
3.  **New Command**: Add `PlatformCommand::CreateRichEdit` and `PlatformCommand::SetRichEditContent`.
4.  **Control Handler**: Create `src/controls/richedit_handler.rs`.
    *   Creation: Use `CreateWindowExW` with class `MSFTEDIT_CLASS` (defined in `windows` crate).
    *   Styling: Rich Edit does not use `WM_CTLCOLOR`. It requires `EM_SETBKGNDCOLOR` for background and `EM_SETCHARFORMAT` for default text color.
    *   Content Setting: While `WM_SETTEXT` works if the string starts with `{\rtf`, robust implementation often uses `EM_STREAMIN` to handle larger buffers without encoding issues. For the MVP (<64KB text), `WM_SETTEXT` is acceptable if prefixed correctly.

### 3.3. Crate: `harvester_app` (Transformation Layer)

We need a dedicated module to translate Markdown events to RTF codes.

**Tasks:**
1.  **New Module**: `src/platform/ui/markdown_to_rtf.rs`.
2.  **Logic**:
    *   Initialize an RTF Header: `{\rtf1\ansi\deff0 ...`.
    *   Define a Font Table: `{\fonttbl{\f0\fnil\fcharset0 Segoe UI;}{\f1\fnil\fcharset0 Consolas;}}`.
    *   Define a Color Table: `{\colortbl ;\red224\green229\blue236; ...}` (matching the Dark Theme palette).
    *   Iterate `pulldown_cmark::Parser::new(text)`.
    *   Map events:
        *   `Start(Heading(level))` -> `\pard\sa200\sb100\b\f0\fs{size} ` (Paragraph definition, bold, font size).
        *   `End(Heading)` -> `\par`.
        *   `Start(List)` / `Start(Item)` -> `\par\bullet\tab ` (Visual simulation is robust/easy) OR `\pntext` (Native RTF lists, harder to get right). *Recommendation: Visual simulation for MVP.*
        *   `Start(Emphasis)` -> `\i`.
        *   `Start(Strong)` -> `\b`.

**Example Logic (Conceptual):**

```rust
// In harvester_app/src/platform/ui/markdown_to_rtf.rs

use std::fmt::Write;
use pulldown_cmark::{Parser, Event, Tag};

pub fn convert_markdown_to_rtf(markdown: &str) -> String {
    let mut rtf = String::with_capacity(markdown.len() * 2);

    // Header: Standard RTF preamble + Dark Theme Colors (fg/bg)
    // \cf1 = Foreground (Off-White), \cb2 = Background (Dark Grey)
    rtf.push_str(r"{\rtf1\ansi\deff0{\fonttbl{\f0 Segoe UI;}}{\colortbl;\red224\green229\blue236;\red38\green42\blue46;}\cf1\cb2\fs20 ");

    let parser = Parser::new(markdown);
    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let size = match level {
                    // RTF font size is half-points (24 = 12pt)
                    pulldown_cmark::HeadingLevel::H1 => 32,
                    pulldown_cmark::HeadingLevel::H2 => 28,
                    _ => 24,
                };
                // \pard = reset para, \sa = space after, \b = bold
                write!(rtf, r"\pard\sa120\sb60\b\fs{} ", size).unwrap();
            }
            Event::End(Tag::Heading(..)) => {
                rtf.push_str(r"\par\pard\sa60\sb60\b0\fs20 "); // Reset to body text
            }
            Event::Start(Tag::Emphasis) => rtf.push_str(r"\i "),
            Event::End(Tag::Emphasis) => rtf.push_str(r"\i0 "),
            Event::Start(Tag::Strong) => rtf.push_str(r"\b "),
            Event::End(Tag::Strong) => rtf.push_str(r"\b0 "),
            Event::Text(t) => escape_rtf_text(&mut rtf, &t),
            Event::SoftBreak | Event::HardBreak => rtf.push_str(r"\par "),
            Event::Start(Tag::List(..)) => {}, // Logic to track indentation needed
            Event::Start(Tag::Item) => rtf.push_str(r"\par\bullet\tab "),
            _ => {} // Ignore complex tags for MVP
        }
    }

    rtf.push('}'); // Close RTF group
    rtf
}

fn escape_rtf_text(buf: &mut String, text: &str) {
    // RTF requires escaping \, {, } and handling Unicode
    for c in text.chars() {
        match c {
            '\\' => buf.push_str(r"\\"),
            '{' => buf.push_str(r"\{"),
            '}' => buf.push_str(r"\}"),
            '\n' => buf.push_str(r"\par "),
            c if c.is_ascii() => buf.push(c),
            c => write!(buf, "\\u{}?", c as u32).unwrap(),
        }
    }
}
```

### 3.4. Render Integration

In `harvester_app/src/platform/ui/render.rs`:

1.  Detect if the `VIEWER_PREVIEW` is targeted.
2.  Instead of passing the string directly, pass it through `convert_markdown_to_rtf`.
3.  Emit `PlatformCommand::SetRichEditContent` instead of `SetViewerContent` (or update `SetViewerContent` to handle the distinction).

## 4. Robustness & Security

*   **Recursion Limits:** `pulldown-cmark` is non-recursive (iterative), preventing stack overflows on malicious deeply nested input.
*   **Escape Handling:** The RTF generator must strictly escape `{`, `}`, and `\` to prevent RTF injection (which can cause crashes or OOM in the Windows control).
*   **Performance:** RTF generation is $O(N)$ and effectively a single pass.
*   **Unicode:** Windows Rich Edit natively handles `\uN` unicode escapes.

## 5. Testing Strategy

1.  **Golden Tests (Snapshot Testing):**
    *   Create a test in `harvester_app` that feeds a sample Markdown string.
    *   Assert the output RTF matches a saved "golden" RTF string.
    *   This ensures styling rules don't drift unintentionally.

2.  **Property Tests (Fuzzing):**
    *   Feed random strings into `convert_markdown_to_rtf`.
    *   Assert that `{` and `}` braces in the output remain balanced (basic RTF validity).

## 6. Future Extensions (Enabled by this plan)

*   **Clickable Links:** The Rich Edit control supports `EN_LINK` notifications. We can map these to `AppEvent::OpenUrlInBrowser` in the future.
*   **Search/Highlighting:** Rich Edit has native API (`EM_FINDTEXT`, `EM_SETCHARFORMAT`) to highlight text ranges.
*   **Copy/Paste:** Rich Edit handles copying as RTF automatically, allowing users to paste formatted text into Word/Outlook.

## 7. Migration Steps

1.  **Refactor `CommanDuctUI`**: Add `RichEdit` support. (Requires submodule update).
2.  **Add `pulldown-cmark`**: Update `harvester_app/Cargo.toml`.
3.  **Implement Converter**: Add the RTF logic.
4.  **Update Layout**: Change `VIEWER_PREVIEW` creation from `CreateInput` to `CreateRichEdit`.
5.  **Update Renderer**: Wire the converter.
