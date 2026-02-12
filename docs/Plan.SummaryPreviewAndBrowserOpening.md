# Plan: Summary Preview and Browser Opening

## Context

The application creates per-article summaries during the briefing process, stored as
`ArticleSummaryResult` (title, summary, key_points) inside `BriefingSession`. However,
**there is no way to view these summaries**. Clicking an article in the treeview shows the
markdown-converted article content, which is hard to read and not the user's primary interest.

**User workflow:**
1. Generate and read the **Briefing** (one-page aggregated overview)
2. Browse individual **summaries** of high-priority articles (filtered by triage tags)
3. Conditionally read the **full article** by opening the original URL in a browser

**Problems solved:**
- Summaries exist but are never displayed
- Markdown article preview is the default, but summaries should be primary
- No way to open the original article URL in a browser
- No visual distinction between articles with/without summaries

**Outcome:** Replace article preview with summary preview, add "Open in Browser" button,
and style articles without summaries as disabled (gray, non-clickable).

---

## Architecture Alignment

This design follows the Unidirectional Data Flow Architecture (see `Agents.md`):

```
TreeView click  ──►  Msg::JobSelected  ──►  Reducer  ──►  State'  ──►  Render
Button click    ──►  Msg::OpenInBrowserClicked  ──►  Reducer  ──►  Effect::OpenUrlInBrowser
                                                                   ──►  Effect handler (IO)
```

- **Reducer is pure:** `select_job()` reads summary from `BriefingSession`, formats text, stores
  in `PreviewState`. No IO.
- **Effects are isolated:** Browser opening is an `Effect` dispatched by the reducer, executed
  by the effect handler.
- **Single source of truth:** Summary data lives in `BriefingSession.articles`. The `AppViewModel`
  receives a read-only projection.

---

## Implementation Steps

### Step 1: Add `summary_for_url()` to `BriefingSession`

**File:** `crates/harvester_core/src/briefing.rs`

Follow the existing `TriageSession::result_for_url()` pattern (triage.rs:199-206):

```rust
/// Look up a completed summary by article URL.
/// Returns None if the article has no completed summary.
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
- `summary_for_url_returns_result_when_completed`
- `summary_for_url_returns_none_for_wrong_url`

---

### Step 2: Modify `select_job()` to Use Summaries

**File:** `crates/harvester_core/src/state.rs` (line ~459)

Change the job selection logic:

```rust
pub(crate) fn select_job(&mut self, job_id: JobId) {
    let job = match self.jobs.get(&job_id) {
        Some(job) => job,
        None => return,
    };

    // Only allow selection if a summary exists for this article
    let summary = match self.briefing.summary_for_url(&job.url) {
        Some(s) => s,
        None => return, // No summary → do nothing
    };

    let formatted = format_summary_for_preview(summary);
    if self.ui.select_job(job_id, Some(&formatted)) {
        self.dirty = true;
    }
}
```

Add formatting helper (private function in `state.rs`):

```rust
fn format_summary_for_preview(summary: &ArticleSummaryResult) -> String {
    use std::fmt::Write;
    let mut output = String::new();
    let _ = writeln!(output, "# {}", summary.title);
    output.push('\n');
    let _ = writeln!(output, "{}", summary.summary);
    if !summary.key_points.is_empty() {
        output.push('\n');
        let _ = writeln!(output, "## Key Points");
        output.push('\n');
        for point in &summary.key_points {
            let _ = writeln!(output, "  - {}", point);
        }
    }
    output
}
```

**Tests:**
- `selecting_job_with_summary_shows_formatted_summary`
- `selecting_job_without_summary_does_nothing`
- `selecting_job_without_summary_keeps_previous_preview`
- `format_summary_for_preview_includes_title_and_key_points`
- `format_summary_for_preview_omits_key_points_section_when_empty`

---

### Step 3: Add `selected_url` to `AppViewModel`

**File:** `crates/harvester_core/src/view_model.rs`

Add field to `AppViewModel`:
```rust
pub selected_url: Option<String>,
```

Default to `None`.

**File:** `crates/harvester_core/src/state.rs` — in `view()` (line ~140)

Populate from the selected job:
```rust
let selected_url = self.ui.selected_job_id()
    .and_then(|job_id| self.jobs.get(&job_id))
    .and_then(|job| {
        // Only expose URL when summary is available (i.e., button should be active)
        self.briefing.summary_for_url(&job.url)?;
        Some(job.url.clone())
    });
```

This ensures the "Open in Browser" button is only enabled when a summarized article is
selected — correctness by construction.

---

### Step 4: Add `has_summary` Flag to `JobRowView`

**File:** `crates/harvester_core/src/view_model.rs`

Add to `JobRowView`:
```rust
pub has_summary: bool,
```

**File:** `crates/harvester_core/src/state.rs` — in `view()` (around line 117-130)

After building job views, annotate summary availability:
```rust
for job_view in &mut jobs {
    if let Some(result) = self.triage.result_for_url(&job_view.url) {
        job_view.triage_annotation = Some(TriageAnnotationView { ... });
    }
    job_view.has_summary = self.briefing.summary_for_url(&job_view.url).is_some();
}
```

This follows the exact pattern used for triage annotations (line 119).

---

### Step 5: Gray Styling for Articles Without Summaries

**File:** `src/CommanDuctUI/src/styling_primitives.rs`

Add new variant to `StyleId` enum:
```rust
pub enum StyleId {
    // ...existing variants...
    TreeItemDisabled,
}
```

**File:** `crates/harvester_app/src/platform/ui/layout.rs`

Define the style in `define_dark_theme_styles()`:
```rust
commands.push(PlatformCommand::DefineStyle {
    style_id: StyleId::TreeItemDisabled,
    style: ControlStyle {
        text_color: Some(Color { r: 0x60, g: 0x65, b: 0x6B }), // Muted gray
        ..Default::default()
    },
});
```

**File:** `crates/harvester_app/src/platform/ui/render.rs` — in `build_job_tree()`

Apply style override when no summary:
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

**Note:** `TreeItemDescriptor.style_override` already exists and is wired through
`NM_CUSTOMDRAW` in `treeview_handler.rs:1000-1047`. No new infrastructure needed.

---

### Step 6: Add Message and Effect for Browser Opening

**File:** `crates/harvester_core/src/msg.rs`

```rust
/// User requested to open the currently selected article in the default browser.
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

Add handler in the `match msg` block:
```rust
Msg::OpenInBrowserClicked => {
    if let Some(url) = state.selected_article_url() {
        vec![Effect::OpenUrlInBrowser { url }]
    } else {
        Vec::new()
    }
}
```

**File:** `crates/harvester_core/src/state.rs`

Add accessor:
```rust
/// Returns the URL of the currently selected article, if one is selected and has a summary.
pub fn selected_article_url(&self) -> Option<String> {
    let job_id = self.ui.selected_job_id()?;
    let job = self.jobs.get(&job_id)?;
    self.briefing.summary_for_url(&job.url)?; // Guard: only if summarized
    Some(job.url.clone())
}
```

**Tests:**
- `open_in_browser_with_selected_job_emits_effect`
- `open_in_browser_without_selection_emits_nothing`
- `open_in_browser_without_summary_emits_nothing`

---

### Step 7: Implement Browser Opening Effect

**File:** `crates/harvester_app/src/platform/effects.rs`

```rust
Effect::OpenUrlInBrowser { url } => {
    engine_info!("[browser] Opening URL: {}", url);
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if let Err(e) = std::process::Command::new("cmd")
        .args(["/C", "start", "", &url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        engine_error!("[browser] Failed to open URL {}: {}", url, e);
    }
}
```

**Robustness notes:**
- `CREATE_NO_WINDOW` prevents a flash of a console window.
- The empty `""` argument before `&url` is required by `start` to handle URLs
  containing `&` or spaces correctly (it treats the first quoted arg as window title).
- Errors are logged but don't produce a follow-up Msg — opening a browser is
  fire-and-forget; the user sees the result directly.

---

### Step 8: Add "Open in Browser" Button

**File:** `crates/harvester_app/src/platform/ui/constants.rs`

```rust
pub const BUTTON_OPEN_BROWSER: ControlId = ControlId::new(1009);
```

**File:** `crates/harvester_app/src/platform/ui/layout.rs`

In `initial_commands()`, add after the Poll Sources button:
```rust
commands.push(PlatformCommand::CreateButton {
    window_id,
    parent_control_id: Some(PANEL_BUTTONS),
    control_id: BUTTON_OPEN_BROWSER,
    text: "Open in Browser".to_string(),
});
```

In `build_layout_rules()`, add layout rule:
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

In `apply_dark_theme()`, add style:
```rust
commands.push(PlatformCommand::ApplyStyleToControl {
    window_id,
    control_id: BUTTON_OPEN_BROWSER,
    style_id: StyleId::DefaultButton,
});
```

---

### Step 9: Wire Up Button Click Event

**File:** `crates/harvester_app/src/platform/app.rs`

In `handle_event()`, add:
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

In `render()`, add button state tracking:
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

---

## Blockers

### Keyboard Shortcut Support Does Not Exist

The CommanDuctUI submodule has **no `AppEvent` variant for keyboard events** (no `KeyPressed`,
no accelerator table support). All current interactions are button clicks, menu actions, tree
selection, or input text changes.

Adding keyboard shortcut support (e.g., Enter to open browser) requires:
1. Adding `AppEvent::KeyPressed { window_id, virtual_key_code }` to `src/CommanDuctUI/src/types.rs`
2. Handling `WM_KEYDOWN` or `WM_CHAR` in the window procedure
3. Routing it through the event dispatch system

**Decision:** Defer keyboard shortcuts to Phase 2 (see Future Ideas). The button alone provides
full functionality. Keyboard support is a separate concern and should be planned independently.

---

## Files Modified (Summary)

### Core Logic (`harvester_core`):
| File | Changes |
|------|---------|
| `src/briefing.rs` | Add `summary_for_url()` method |
| `src/state.rs` | Modify `select_job()`, add `selected_article_url()`, add `format_summary_for_preview()`, populate `has_summary` and `selected_url` in `view()` |
| `src/update.rs` | Handle `Msg::OpenInBrowserClicked` |
| `src/msg.rs` | Add `OpenInBrowserClicked` variant |
| `src/effect.rs` | Add `OpenUrlInBrowser { url }` variant |
| `src/view_model.rs` | Add `selected_url: Option<String>` to `AppViewModel`, add `has_summary: bool` to `JobRowView` |

### UI/Platform (`harvester_app`):
| File | Changes |
|------|---------|
| `src/platform/ui/constants.rs` | Add `BUTTON_OPEN_BROWSER` |
| `src/platform/ui/layout.rs` | Create button, add layout rule, apply style |
| `src/platform/ui/render.rs` | Add button enable/disable tracking, apply gray style to unsummarized jobs |
| `src/platform/app.rs` | Handle `ButtonClicked` for `BUTTON_OPEN_BROWSER` |
| `src/platform/effects.rs` | Implement `Effect::OpenUrlInBrowser` |

### Submodule (`CommanDuctUI`):
| File | Changes |
|------|---------|
| `src/styling_primitives.rs` | Add `TreeItemDisabled` variant to `StyleId` |

---

## Verification

### Manual Testing
1. **Build:** `cargo build`
2. **Download articles** and **run Briefing** to generate summaries
3. **Before Briefing:** Click on an article — should do nothing. Articles appear gray.
4. **After Briefing:** Click on an article with summary — preview shows formatted summary
   (title, body, key points)
5. **Click "Open in Browser"** — original URL opens in default browser
6. **Button state:** "Open in Browser" is disabled when no summarized article is selected
7. **Triage ordering:** High-priority articles still sort first, gray/non-gray styling
   visually separates summarized from unsummarized

### Automated Tests
Run `cargo clippy --all-targets -- -D warnings` at end of implementation.

Key test scenarios (in `crates/harvester_core/`):
- `briefing.rs`: `summary_for_url` correctness (match, miss, pending, failed)
- `state.rs`: `select_job` with/without summary, `selected_article_url` accessor
- `update.rs`: `Msg::OpenInBrowserClicked` → `Effect::OpenUrlInBrowser` mapping
- `render.rs`: `has_summary` drives `style_override` on tree items

---

## Robustness Considerations

- **URL matching:** `BriefingSession.summary_for_url()` uses exact string match (`article.url == url`).
  This is consistent with `TriageSession.result_for_url()`. If URL normalization becomes an issue
  (e.g., trailing slashes, scheme differences), both should be updated together.
- **Stale summaries:** After archive/reset, `BriefingSession` is reset to default
  (`state.rs:454`), which clears all articles. Gray styling will reappear. This is correct.
- **Race condition:** If briefing completes while the user already has a job selected,
  the next `Tick` → `render()` cycle will update the gray styling. The user would need to
  re-click to see the summary. This is acceptable UX.
- **Browser command failure:** Logged via `engine_error!` but no user-visible error. A follow-up
  could add status bar feedback.

---

## Future Ideas

### Keyboard Shortcuts (Phase 2)
Add keyboard navigation to CommanDuctUI:
- `Enter` on selected tree item → open in browser
- `Space` → toggle checkbox (already works natively for TreeView)
- Arrow keys already work (native TreeView behavior)
- Requires `AppEvent::KeyPressed` + `WM_KEYDOWN` handling in CommanDuctUI

### Double-Click to Open Browser
An alternative to keyboard shortcuts: double-clicking a tree item opens the URL directly.
CommanDuctUI already handles `NM_DBLCLK` partially — could be extended to emit a
`TreeViewItemDoubleClicked` event. Lower effort than full keyboard support.

### Summary Cache Lookup as Fallback
Currently, summaries are only available during the current session (from `BriefingSession`).
The `SummaryCache` persists across sessions but requires a composite key (content_hash +
prompt metadata), not just URL. A future enhancement could:
- Index `SummaryCache` by URL as a secondary index
- Provide summaries for articles from previous sessions without re-running Briefing
- This would make the "gray" state temporary — articles would eventually all become clickable

### Markdown-to-HTML Preview
Instead of opening the original web page, render the saved markdown as HTML locally
and open it in the browser. Useful for offline reading or when the original page
has changed/disappeared. Could use a lightweight template with CSS for readability.

### Preview Panel Modes
Add a mode selector to the preview panel header:
- **Summary** (default) — LLM-generated summary
- **Article** — markdown content (current behavior)
- **Briefing** — aggregated briefing view
This would restore in-app article viewing for users who want it.

### Context Menu on Tree Items
Right-click context menu with options:
- "Open in Browser" (same as button)
- "Copy URL to clipboard"
- "View full article" (in-app)
- "Re-summarize" (force regeneration)

### Summary Quality Indicators
Show metadata alongside the summary:
- Token counts (input/output)
- Model used
- Summary age (from `SummaryCacheEntry.created_at_utc`)
- Confidence indicators from the LLM response

### Batch Browser Opening
Select multiple articles (via checkboxes) and open all of them in the browser at once.
Useful for "open the top 5 priority articles in tabs" workflow.

### Status Bar Feedback for Browser Actions
Show "Opened https://... in browser" in the status bar after the effect executes.
Requires the effect handler to send a follow-up `Msg` back to the reducer.
