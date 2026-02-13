# Plan: Summary Preview and Browser Opening

## Context

The application creates per-article summaries during the briefing process, stored as
`ArticleSummaryResult` (title, summary, key_points) inside `BriefingSession`. However,
**there is no way to view these summaries**. Clicking an article shows the markdown-converted
article content, which is hard to read and not the user's primary interest.

**User workflow:**
1. Generate and read the **Briefing** (one-page aggregated overview)
2. Browse individual **summaries** of high-priority articles (filtered by triage tags)
3. Conditionally read the **full article** by opening the original URL in a browser

**Problems solved:**
- Summaries exist but are never displayed
- Markdown article preview is the default; summaries should be primary
- No way to open the original article URL in a browser
- No visual distinction between articles with/without summaries

---

## Architecture Alignment

This design follows the Unidirectional Data Flow Architecture (see `Agents.md`):

```
TreeView click  ──►  Msg::JobSelected  ──►  Reducer  ──►  State'  ──►  Render
Button click    ──►  Msg::OpenInBrowserClicked  ──►  Reducer  ──►  Effect::OpenUrlInBrowser
                                                                   ──►  Effect handler (IO)
```

- **Reducer is pure.** `select_job()` reads summary from `BriefingSession`, sets preview
  mode, stores formatted text. No IO.
- **Effects are isolated.** Browser opening is an `Effect`, executed by the effect handler.
- **Single source of truth.** Summary data lives in `BriefingSession`. The `AppViewModel`
  receives a read-only projection.

---

## Blockers

**Blocker A — Preview precedence bug (must fix):**
`AppState::view()` at `state.rs:141` currently prioritizes aggregate briefing preview over
the selected job:
```rust
let preview_text = briefing_preview.clone()
    .or_else(|| self.ui.preview_content().map(ToOwned::to_owned));
```
After briefing completes, this always shows the briefing text, not the article summary —
regardless of what the user clicks. The plan must introduce an explicit `PreviewMode` to
control what is displayed rather than relying on the `or_else` precedence chain.

**Blocker B — Gray items are visual-only; clicks still fire:**
`TreeItemDescriptor.style_override` controls rendering only (via `NM_CUSTOMDRAW` in
`treeview_handler.rs:1000–1047`). The `AppEvent::TreeViewItemSelectionChanged` event still
fires on label clicks (`treeview_handler.rs:1325`). True "disable interaction" is not
supported by CommanDuctUI today. **Resolution:** handle gracefully in the reducer — when
a job without a summary is clicked, show placeholder text "No summary available — run
Briefing first." Gray styling makes the intent clear; the reducer handles the edge case.

**Blocker C — Shell-fragile browser launch:**
`cmd /C start "" <url>` breaks on URLs with `&`, `%`, or spaces when processed through
cmd's parser. **Resolution:** use `ShellExecuteW` directly (Windows) which passes the URL
to the shell verbatim without cmd interpretation.

---

## Implementation Steps

### Step 1: Introduce `PreviewMode` in Core State

**File:** `crates/harvester_core/src/state.rs`

Add an enum to replace the implicit `or_else` precedence:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum PreviewMode {
    #[default]
    Briefing,
    SelectedJobSummary,
}
```

Add `preview_mode: PreviewMode` to `UiState` (the struct that holds `preview` today).

Change `AppState::view()` (line 141) to drive `preview_text` from mode:

```rust
let preview_text = match self.ui.preview_mode() {
    PreviewMode::SelectedJobSummary => self.ui.preview_content().map(ToOwned::to_owned),
    PreviewMode::Briefing => self.briefing.format_preview()
        .or_else(|| self.ui.preview_content().map(ToOwned::to_owned)),
};
```

When user selects a job, mode transitions to `SelectedJobSummary`. When briefing
starts/completes, mode reverts to `Briefing`. This gives deterministic, auditable precedence
instead of implicit chaining.

**Tests:**
- `briefing_complete_then_job_selected_shows_summary_not_briefing`
- `job_selected_then_briefing_completes_shows_briefing`
- `no_selection_shows_briefing_when_complete`

---

### Step 2: Add `summary_for_url()` to `BriefingSession`

**File:** `crates/harvester_core/src/briefing.rs`

Follow the `TriageSession::result_for_url()` pattern (triage.rs:199):

```rust
/// Returns the completed summary result for an article URL, if available.
pub fn summary_for_url(&self, url: &str) -> Option<&ArticleSummaryResult> {
    self.articles.iter().find_map(|article| match &article.summary_state {
        ArticleSummaryState::Completed { result } if article.url == url => Some(result),
        _ => None,
    })
}
```

**Tests:**
- `summary_for_url_returns_none_when_no_articles`
- `summary_for_url_returns_none_when_pending`
- `summary_for_url_returns_none_when_failed`
- `summary_for_url_returns_result_when_completed`
- `summary_for_url_returns_none_for_wrong_url`

---

### Step 3: Modify `select_job()` to Use Summaries

**File:** `crates/harvester_core/src/state.rs` (line ~459)

```rust
pub(crate) fn select_job(&mut self, job_id: JobId) {
    let Some(job) = self.jobs.get(&job_id) else { return };

    let content = match self.briefing.summary_for_url(&job.url) {
        Some(summary) => format_summary_for_preview(summary),
        None => "No summary available — run Briefing first.".to_string(),
    };

    if self.ui.select_job_with_mode(job_id, content, PreviewMode::SelectedJobSummary) {
        self.dirty = true;
    }
}
```

Note: even when no summary exists, we set preview mode to `SelectedJobSummary` with a
placeholder. This ensures visual consistency — the preview area always reflects the
selection state. The placeholder is better UX than ignoring the click (see Blocker B).

Add private formatting helper:

```rust
fn format_summary_for_preview(summary: &ArticleSummaryResult) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# {}", summary.title);
    out.push('\n');
    let _ = writeln!(out, "{}", summary.summary);
    if !summary.key_points.is_empty() {
        out.push('\n');
        let _ = writeln!(out, "## Key Points");
        out.push('\n');
        for point in &summary.key_points {
            let _ = writeln!(out, "  - {}", point);
        }
    }
    out
}
```

Revert mode to `Briefing` when briefing completes (in the existing briefing completion
handler in `update.rs`).

**Tests:**
- `selecting_job_with_summary_shows_formatted_summary`
- `selecting_job_without_summary_shows_placeholder`
- `selecting_job_sets_preview_mode_to_selected_job_summary`
- `format_summary_includes_title_summary_and_key_points`
- `format_summary_omits_key_points_section_when_empty`

---

### Step 4: Add `has_summary` to `JobRowView` and `selected_url` to `AppViewModel`

These are derived views computed in `AppState::view()`. Keep them in **separate, independent
derivation blocks** — do not mix triage and summary logic in the same loop.

**File:** `crates/harvester_core/src/view_model.rs`

```rust
// In JobRowView:
pub has_summary: bool,

// In AppViewModel:
pub selected_url: Option<String>,  // Some only when selected job has a completed summary
```

**File:** `crates/harvester_core/src/state.rs` — in `view()`:

```rust
// Block 1: triage annotations (existing, unchanged)
for job_view in &mut jobs {
    if let Some(result) = self.triage.result_for_url(&job_view.url) {
        job_view.triage_annotation = Some(TriageAnnotationView { ... });
    }
}

// Block 2: summary availability (new, separate loop for clarity)
for job_view in &mut jobs {
    job_view.has_summary = self.briefing.summary_for_url(&job_view.url).is_some();
}

// Derive selected_url — only expose when summary is available
let selected_url = self.ui.selected_job_id()
    .and_then(|job_id| self.jobs.get(&job_id))
    .and_then(|job| {
        self.briefing.summary_for_url(&job.url)?; // guard: summary must exist
        Some(job.url.clone())
    });
```

`selected_url` being `None` when no summary exists makes button enablement
correct-by-construction: the renderer simply checks `view.selected_url.is_some()`.

**Tests:**
- `view_has_summary_true_for_completed_articles`
- `view_has_summary_false_before_briefing`
- `view_selected_url_populated_when_summarized_job_selected`
- `view_selected_url_none_when_unsummarized_job_selected`
- `view_selected_url_none_when_no_selection`

---

### Step 5: Gray Styling for Articles Without Summaries

**File:** `src/CommanDuctUI/src/styling_primitives.rs`

Add variant to `StyleId` enum (submodule change — bump version in Cargo.toml):
```rust
TreeItemDisabled,
```

**File:** `crates/harvester_app/src/platform/ui/layout.rs`

In `define_dark_theme_styles()`:
```rust
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::TreeItemDisabled,
    style: ControlStyle {
        text_color: Some(Color { r: 0x60, g: 0x65, b: 0x6B }), // Muted gray
        ..Default::default()
    },
});
```

**File:** `crates/harvester_app/src/platform/ui/render.rs` — in `build_job_tree()`:

```rust
TreeItemDescriptor {
    id: job_tree_item_id(job.job_id),
    text: format_job_row(job),
    is_folder: true,
    state: CheckState::Unchecked,
    children,
    style_override: if job.has_summary { None } else { Some(StyleId::TreeItemDisabled) },
}
```

`TreeItemDescriptor.style_override` already wires into `NM_CUSTOMDRAW` — no new
infrastructure needed.

**Tests (render.rs):**
- `job_without_summary_gets_tree_item_disabled_style_override`
- `job_with_summary_has_no_style_override`

---

### Step 6: Add `Msg::OpenInBrowserClicked` and `Effect::OpenUrlInBrowser`

**File:** `crates/harvester_core/src/msg.rs`
```rust
/// User requested to open the currently selected article URL in the default browser.
OpenInBrowserClicked,
```

**File:** `crates/harvester_core/src/effect.rs`
```rust
/// Open a URL in the user's default web browser.
OpenUrlInBrowser {
    url: String,
},
```

**File:** `crates/harvester_core/src/update.rs`

```rust
Msg::OpenInBrowserClicked => {
    match state.selected_article_url() {
        Some(url) => vec![Effect::OpenUrlInBrowser { url }],
        None => Vec::new(),
    }
}
```

Add accessor to `AppState` (`state.rs`):
```rust
/// URL of the currently selected and summarized article.
/// Returns None if no job is selected or if the selected job has no summary.
pub fn selected_article_url(&self) -> Option<String> {
    let job_id = self.ui.selected_job_id()?;
    let job = self.jobs.get(&job_id)?;
    self.briefing.summary_for_url(&job.url)?; // guard
    Some(job.url.clone())
}
```

**Tests:**
- `open_in_browser_with_summarized_job_selected_emits_effect`
- `open_in_browser_with_unsummarized_job_selected_emits_nothing`
- `open_in_browser_with_no_selection_emits_nothing`

---

### Step 7: Implement Browser Opening Effect via `ShellExecuteW`

**File:** `crates/harvester_app/src/platform/effects.rs`

Use `ShellExecuteW` directly — it passes the URL verbatim to the Windows shell handler
without cmd string-parsing, avoiding breakage with `&`, `%`, spaces, and Unicode:

```rust
Effect::OpenUrlInBrowser { url } => {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;

    engine_info!("[browser] Opening URL: {}", url);

    let operation: Vec<u16> = OsStr::new("open").encode_wide().chain(Some(0)).collect();
    let url_wide: Vec<u16> = OsStr::new(&url).encode_wide().chain(Some(0)).collect();

    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr()),
            PCWSTR(url_wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns value > 32 on success
    if result.0 as isize <= 32 {
        engine_error!("[browser] ShellExecuteW failed for URL '{}', error code: {}", url, result.0 as isize);
    }
}
```

**Robustness:** `ShellExecuteW` is the canonical Windows API for "open with default handler".
It handles URL encoding, shell protocol dispatch, and browser selection without cmd parsing.

---

### Step 8: Add "Open in Browser" Button

**File:** `crates/harvester_app/src/platform/ui/constants.rs`
```rust
pub const BUTTON_OPEN_BROWSER: ControlId = ControlId::new(1009);
```

**File:** `crates/harvester_app/src/platform/ui/layout.rs`

In `initial_commands()`, after the Poll Sources button:
```rust
commands.push(PlatformCommand::CreateButton {
    window_id,
    parent_control_id: Some(PANEL_BUTTONS),
    control_id: BUTTON_OPEN_BROWSER,
    text: "Open in Browser".to_string(),
});
```

In `build_layout_rules()`:
```rust
LayoutRule {
    control_id: BUTTON_OPEN_BROWSER,
    parent_control_id: Some(PANEL_BUTTONS),
    dock_style: DockStyle::Left,
    order: 6,
    fixed_size: Some(160),
    margin: (6, 6, 6, 0),
},
```

In `apply_dark_theme()`:
```rust
commands.push(PlatformCommand::ApplyStyleToControl {
    window_id,
    control_id: BUTTON_OPEN_BROWSER,
    style_id: StyleId::DefaultButton,
});
```

---

### Step 9: Wire Button Click in Platform Layer

**File:** `crates/harvester_app/src/platform/app.rs`

```rust
AppEvent::ButtonClicked { control_id, .. }
    if control_id == ui::constants::BUTTON_OPEN_BROWSER =>
{
    let _ = self.msg_tx.send(Msg::OpenInBrowserClicked);
}
```

---

### Step 10: Update Rendering for Button Enable/Disable

**File:** `crates/harvester_app/src/platform/ui/render.rs`

Add to `TreeRenderState`:
```rust
prev_open_browser_enabled: Option<bool>,
```

In `render()`:
```rust
let open_browser_enabled = view.selected_url.is_some();
if tree_state.prev_open_browser_enabled != Some(open_browser_enabled) {
    cmds.push(PlatformCommand::SetControlEnabled {
        window_id,
        control_id: BUTTON_OPEN_BROWSER,
        enabled: open_browser_enabled,
    });
    tree_state.prev_open_browser_enabled = Some(open_browser_enabled);
}
```

**Tests:**
- `render_enables_open_browser_when_selected_url_is_some`
- `render_disables_open_browser_when_selected_url_is_none`
- `render_is_idempotent_for_open_browser_state`

---

## Files Modified (Summary)

### Core Logic (`harvester_core`):
| File | Change |
|------|--------|
| `src/briefing.rs` | Add `summary_for_url()` |
| `src/state.rs` | Add `PreviewMode` enum; modify `select_job()`; add `selected_article_url()`, `format_summary_for_preview()`; add `has_summary` + `selected_url` derivation in `view()` |
| `src/update.rs` | Handle `Msg::OpenInBrowserClicked`; revert `PreviewMode` to `Briefing` on briefing complete |
| `src/msg.rs` | Add `OpenInBrowserClicked` |
| `src/effect.rs` | Add `OpenUrlInBrowser { url }` |
| `src/view_model.rs` | Add `has_summary: bool` to `JobRowView`; add `selected_url: Option<String>` to `AppViewModel` |

### UI/Platform (`harvester_app`):
| File | Change |
|------|--------|
| `src/platform/ui/constants.rs` | Add `BUTTON_OPEN_BROWSER` |
| `src/platform/ui/layout.rs` | Create button, add layout rule, add dark theme style definition, apply style |
| `src/platform/ui/render.rs` | Add button enable/disable tracking; apply `TreeItemDisabled` style based on `has_summary` |
| `src/platform/app.rs` | Handle `ButtonClicked` for `BUTTON_OPEN_BROWSER` |
| `src/platform/effects.rs` | Implement `Effect::OpenUrlInBrowser` via `ShellExecuteW` |

### Submodule (`CommanDuctUI`):
| File | Change |
|------|--------|
| `src/styling_primitives.rs` | Add `TreeItemDisabled` to `StyleId` enum |
| `Cargo.toml` | Bump version |

---

## Verification

### Manual Testing
1. `cargo build`
2. Download articles, run Briefing to generate summaries
3. **Before Briefing:** Click an article → preview shows "No summary available — run Briefing first." Tree items appear gray.
4. **After Briefing:** Click a summarized article → preview shows formatted summary (title, body, key points). "Open in Browser" button becomes enabled.
5. **Click "Open in Browser"** → original URL opens in default browser
6. **Click a non-summarized article** (if any remain) → placeholder message, button stays disabled
7. **Briefing state transitions:** running briefing reverts preview to briefing text; selecting a job again switches back to summary

### Automated Tests
`cargo clippy --all-targets -- -D warnings` at end of implementation.

Key test locations:
- `crates/harvester_core/tests/` or `src/briefing.rs`, `src/state.rs`, `src/update.rs`
- `crates/harvester_app/src/platform/ui/render.rs` (existing test pattern)

---

## Robustness Notes

- **URL matching** uses exact string equality, consistent with `TriageSession::result_for_url()`. If URL normalization is ever needed, both should be updated together.
- **Summary cache** is already persisted across sessions (`Effect::PersistSummaryCache`, `Msg::SummaryCacheHydrated`). However, the current lookup path is by composite key (content_hash + prompt metadata), not URL. The `BriefingSession` is the correct lookup source for the current session. Cross-session URL-indexed lookup is a future enhancement (see below).
- **Stale state after archive:** `BriefingSession` resets on archive (`state.rs:454`), clearing all summaries. Gray styling reappears. This is correct behavior.
- **Briefing in progress:** `summary_for_url()` returns `None` for articles in `Pending` or `InProgress` state. The placeholder message handles this gracefully.

---

## Future Ideas

### Keyboard Shortcuts (Deferred — requires CommanDuctUI extension)
Add `AppEvent::KeyPressed { window_id, virtual_key_code }` + `WM_KEYDOWN` handling to
CommanDuctUI. Then bind Enter or a function key to `Msg::OpenInBrowserClicked`. This is
a clean CommanDuctUI feature request, independent of this plan.

### Double-Click to Open Browser
`NM_DBLCLK` handling could emit `AppEvent::TreeViewItemDoubleClicked`. Lower effort than
full keyboard support. Could trigger `Msg::OpenInBrowserClicked` directly.

### URL-Indexed Summary Cache (Cross-Session Summaries)
Add a secondary URL → summary index into `SummaryCache` (or a separate `UrlSummaryIndex`),
populated when summaries are computed and hydrated at startup. Would make summaries
available without re-running Briefing after restart. Aligns with `FI-LLM-Caching-0001`,
`FI-Storage-PreviewLoading-0001`.

### True "Disabled" Tree Items
Extend CommanDuctUI to suppress `AppEvent::TreeViewItemSelectionChanged` for items with
a specific style override (or a dedicated `disabled: bool` flag on `TreeItemDescriptor`).
Would enable genuine non-clickable behavior without relying on the reducer as a fallback.
Aligns with `FI-UX-PreviewRich-0001`.

### Status Bar Feedback After Browser Open
Have the effect handler send a follow-up `Msg::BrowserOpenSucceeded { url }` /
`Msg::BrowserOpenFailed { url, reason }` to update the status bar. Improves traceability
(every action should be explainable in `Action → State → Render`).

### Preview Panel Mode Selector
Add explicit mode toggle UI: **Summary** | **Article** | **Briefing**. The `PreviewMode`
enum introduced in this plan is the foundation. Restores in-app markdown viewing for users
who want it. Aligns with `FI-UX-PreviewRich-0001`, `FI-UX-PreviewSearch-0001`.

### Context Menu on Tree Items
Right-click menu: "Open in Browser", "Copy URL", "View full article", "Re-summarize".
Requires CommanDuctUI context menu support.

### Batch Browser Opening
Open top-N priority articles in browser at once. Useful for "open my morning reading list"
workflow.

### Summary Quality Metadata in Preview Footer
Show token counts, model used, and summary age from `SummaryCacheEntry.created_at_utc`
below the summary text.
