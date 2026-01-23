# Plan: Markdown Preview Pane

## Scope and goals

### GoalPreviewAsContentInspectorV2
Add a **preview pane** that displays a job's extracted Markdown so the user can:
1. **Assess extraction quality** — judge whether the download pipeline effectively captured relevant text.
2. **Decide keep/skip** — quickly determine if a page's content is interesting or should be discarded.
3. **Monitor in-progress jobs** — see partial content during slow downloads to decide whether to wait for more.

### GoalStrongEncapsulationV1
Keep encapsulation strong:
- No exporting of internal struct state beyond what is needed for messages, effects, and view models.
- Prefer private fields with behavior-oriented methods (avoid "bag-of-getters").
- Keep `mod.rs` / `lib.rs` thin wrappers.

### GoalTestableIncrementalMvpV1
Work in small steps where the project remains **buildable and testable after each step**, and add unit tests whenever behavior is introduced or changed.

### NonGoalsV2
Not in MVP:
- Full-fidelity Markdown rendering (images, video, embedded HTML).
- Clickable links / HTML WebView.
- Full discard workflow that deletes files (later phase).
- Rich formatting or syntax highlighting in the preview control.

---

## Architectural approach

### ArchUnidirectionalDataFlowV1
Follow the existing unidirectional flow:
1. **UI event** → `Msg`
2. `harvester_core::update(state, msg)` → `(state, effects)`
3. App layer executes **effects** (I/O, engine, file reads) and emits **new messages** back.

### ArchContentThroughEventsV2
**Key insight:** The markdown content already exists in memory during the engine pipeline (after the Convert stage). Instead of writing to disk and re-reading for preview, **thread the content through the existing event pipeline**.

Hot path (in-session, no I/O):
```
Engine Convert stage → markdown exists in memory
  → Include in EngineEvent::JobCompleted
  → Map to Msg::JobDone { content_preview }
  → Store in JobState
  → Display when job selected
```

Cold path (after app restart, deferred to later phase):
```
JobSelected + no content in memory
  → Effect::LoadPreview { output_filename }
  → Read from disk
  → Msg::PreviewLoaded
```

### ArchPreviewStateAsEnumV2
Model preview as an explicit state machine to make impossible states unrepresentable:

```rust
enum PreviewState {
    Empty,                                         // No job selected
    Available { job_id: JobId, content: String },  // Content ready to display
    InProgress { job_id: JobId, content: String }, // Partial content, job still running
    Unavailable { job_id: JobId },                 // Job selected but no content (restored from persistence)
}
```

### ArchMemoryBudgetV2
Define `MAX_PREVIEW_CONTENT: usize = 40_960` (40 KB). Truncate markdown before storing in `JobState` with a `\n…[truncated]` marker. This bounds total memory at `job_count * 40KB` which is acceptable for typical sessions (50 jobs = 2MB).

### ArchIOInEffectsOnlyV1
All I/O stays in the app/effects layer:
- Reading Markdown file from disk (cold path only).
- Any future deletion/archival.

---

## Blockers to acknowledge early

### BlockerTreeViewSelectionEventV2
The UI event handler currently processes `InputTextChanged` and `ButtonClicked`. Preview requires a **TreeView selection-changed event** to trigger `Msg::JobSelected`.

**Action:** Verify CommanDuctUI emits selection-changed events for TreeView items. If not, this is a hard blocker — either extend CommanDuctUI or use an alternative trigger (e.g., double-click, dedicated "Preview" button).

### BlockerLayoutSplitV2
The current layout uses `DockStyle::Fill` for `TREE_JOBS`, occupying all remaining horizontal space. Adding a preview pane requires **splitting this region** (TreeView left, preview right).

**Action:** Verify CommanDuctUI supports nested panels with horizontal splitting within a Fill region. If not, use fixed-width TreeView (`DockStyle::Left`) and give preview `DockStyle::Fill`.

### BlockerUiControlForViewerV1
The UI currently creates an input box and a TreeView, but no dedicated read-only "viewer" control is wired.

**MVP decision:** create a read-only multiline control (via `CreateInput { read_only: true }`) and update it using `SetViewerContent`.

---

## Phase 0 — UI scaffolding

### Step0.1_AddPreviewControlIdsV2
**Change**
- Add new UI control IDs:
  - `VIEWER_PREVIEW` (read-only multiline)
  - `LABEL_PREVIEW_HEADER` (single-line: domain, tokens, stage)
  - `PANEL_PREVIEW` (container panel for header + viewer)

**Files**
- `crates/harvester_app/src/platform/ui/constants.rs`

**Acceptance**
- Builds and runs.
- No functional changes yet.

### Step0.2_CreatePreviewPaneLayoutV2
**Change**
- Commit to a three-column layout:

```
┌──────────────────────────────────────────────────┐
│ PANEL_PROGRESS (Top, height=64)                  │
├──────────────────────────────────────────────────┤
│ PANEL_INPUT    │ TREE_JOBS      │ PANEL_PREVIEW  │
│ (Left, 320px) │ (Left, 280px)  │ (Fill)         │
│                │                │  LABEL_HEADER  │
│                │                │  VIEWER_PREVIEW│
├──────────────────────────────────────────────────┤
│ PANEL_BUTTONS (Bottom, height=44)                │
├──────────────────────────────────────────────────┤
│ PANEL_BOTTOM (Bottom, height=32)                 │
└──────────────────────────────────────────────────┘
```

- Create the preview viewer control using `PlatformCommand::CreateInput` with:
  - `read_only: true`, `multiline: true`, `vertical_scroll: true`
- Create the header label above it.
- Preview pane gets `DockStyle::Fill` (largest area, primary decision surface).
- TreeView changes from Fill to `DockStyle::Left` with fixed width.

**Files**
- `crates/harvester_app/src/platform/ui/layout.rs`
- `crates/harvester_app/src/platform/ui/setup.rs` (control creation)

**Acceptance**
- App shows an empty preview box that does not accept user typing.
- TreeView and input pane still function as before.

---

## Phase 1 — Thread content through the event pipeline

### Step1.1_ExtendJobOutcomeWithContentV2
**Change**
- Extend `harvester_engine::JobOutcome` with:
  - `content_preview: Option<String>` — the markdown content, truncated to `MAX_PREVIEW_CONTENT`.
- After the Convert stage (markdown is in memory), capture a truncated copy.
- Store it in `JobOutcome` returned via `EngineEvent::JobCompleted`.

**Rationale:** The markdown string exists in memory during the pipeline. Capturing it here avoids any file-read I/O for preview during the active session.

**Truncation:** Apply `MAX_PREVIEW_CONTENT` limit at this point. Append `\n…[truncated]` if content exceeds the limit.

**Files**
- `crates/harvester_engine/src/types.rs` (add field to `JobOutcome`)
- `crates/harvester_engine/src/engine.rs` (capture content after Convert, before or during Write)

**Tests**
- Unit test verifying:
  - `JobOutcome` includes `content_preview` with expected content.
  - Content exceeding `MAX_PREVIEW_CONTENT` is truncated with marker.
  - Short content is stored as-is.

**Acceptance**
- `cargo test` passes.
- Job completion event carries content.

### Step1.2_ExtendMsgJobDoneWithContentV2
**Change**
- Extend `Msg::JobDone` to carry `content_preview: Option<String>`.
- In the effect runner's event mapping (`EngineEvent::JobCompleted` → `Msg::JobDone`), forward the content.

**Files**
- `crates/harvester_core/src/msg.rs` (extend `Msg::JobDone`)
- `crates/harvester_app/src/platform/effects.rs` (map content through)

**Acceptance**
- Build passes.

### Step1.3_StoreContentInJobStateV2
**Change**
- Add `content_preview: Option<String>` to internal `JobState` in core.
- In `update()` when handling `Msg::JobDone { Success }`:
  - Store `content_preview` in the job's state.
- Add accessor method: `fn content_preview(&self) -> Option<&str>` on `JobState`.

**Encapsulation**
- `content_preview` is a private field. Accessed only via method within the crate.

**Files**
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/update.rs`

**Tests**
- Core unit test:
  - `Msg::JobDone` with content stores it in state.
  - `Msg::JobDone` without content (failed job) stores `None`.

**Acceptance**
- Build + tests pass.

### Step1.4_StripFrontmatterBeforeStoringV2
**Change**
- Before storing `content_preview`, strip the YAML frontmatter block (delimited by `---` lines at the start).
- The frontmatter metadata (URL, title, tokens) is displayed separately in the preview header label — duplicating it in the body wastes space and creates noise.

**Implementation**
- Simple: if content starts with `---\n`, find the next `---\n` and skip past it (plus any trailing blank line).
- This can be a utility function in `harvester_engine` (where the content is captured) or in `harvester_core`.

**Tests**
- Content with frontmatter: frontmatter stripped, body preserved.
- Content without frontmatter: unchanged.
- Malformed frontmatter (no closing `---`): content preserved as-is (don't strip).

**Acceptance**
- Preview content shows only the article body, not metadata.

---

## Phase 2 — Selection triggers preview display

### Step2.1_AddPreviewStateAndSelectionV2
**Change**
- Add `PreviewState` enum to core (as described in ArchPreviewStateAsEnumV2).
- Add to `UiState`:
  - `preview: PreviewState` (private field)
- Add new message:
  - `Msg::JobSelected { job_id: JobId }`
- In `update()` when handling `Msg::JobSelected`:
  - If job has `content_preview` → set `PreviewState::Available { job_id, content }`
  - If job is still running and has no content yet → set `PreviewState::Empty` (or `Unavailable`)
  - If job has no content (restored from persistence) → set `PreviewState::Unavailable { job_id }`
  - Mark dirty.

**Stale-load protection:** If `Msg::JobSelected` arrives while a previous selection's cold-path load is pending, the new selection overwrites. When `PreviewLoaded` arrives for a stale `job_id`, discard it by comparing against current `preview.job_id()`.

**Encapsulation**
- Add methods on `UiState`:
  - `fn select_job(&mut self, job_id, content: Option<&str>)`
  - `fn preview_content(&self) -> Option<&str>` (for view model projection)
  - `fn selected_job_id(&self) -> Option<JobId>`

**Files**
- `crates/harvester_core/src/state.rs` (PreviewState enum, UiState fields + methods)
- `crates/harvester_core/src/msg.rs` (add `JobSelected`)
- `crates/harvester_core/src/update.rs` (handle `JobSelected`)

**Tests**
- Selecting a job with content → `PreviewState::Available`.
- Selecting a job without content → `PreviewState::Unavailable`.
- Selecting a different job replaces previous preview state.
- Selecting the already-selected job is a no-op (no redundant dirty flag).

**Acceptance**
- Build + tests pass.

### Step2.2_ProjectPreviewIntoViewModelV2
**Change**
- Extend `AppViewModel` with:
  - `preview_text: Option<String>` — the content to display (or `None` if empty/unavailable)
  - `preview_header: Option<PreviewHeaderView>` — metadata for header label
- Define:
  ```rust
  pub struct PreviewHeaderView {
      pub domain: String,
      pub tokens: Option<u32>,
      pub bytes: Option<u64>,
      pub stage: Stage,
      pub outcome: Option<JobResultKind>,
  }
  ```
- In `state.view()`: project from `PreviewState` and selected job's metadata.

**Files**
- `crates/harvester_core/src/view_model.rs`
- `crates/harvester_core/src/state.rs` (view() method)

**Tests**
- View model reflects preview content when job selected.
- View model reflects `None` when no selection.
- Header view populated from job metadata.

**Acceptance**
- Build + tests pass.

### Step2.3_WireTreeSelectionEventV2
**Change**
- Handle TreeView selection-changed event in `AppEventHandler` and map to `Msg::JobSelected`.
- Map TreeView item → `job_id`:
  - The TreeView items already use `TreeItemId(job.job_id)` — extract `job_id` from the selection event's item ID.
  - No separate mapping table needed if item IDs are stable job IDs.

**Files**
- `crates/harvester_app/src/platform/app.rs` (event handler)

**Acceptance**
- Selecting a completed job row shows its content in the preview pane.
- Selecting an in-progress or failed job shows appropriate empty/error state.

### Step2.4_RenderPreviewToControlV2
**Change**
- In the render function, when view model has `preview_text`:
  - Set `VIEWER_PREVIEW` content via `PlatformCommand::SetText` (or equivalent).
  - Set `LABEL_PREVIEW_HEADER` with formatted header (e.g., `"example.com | 1,234 tokens | 5.2 KB | Done"`).
- When `preview_text` is `None`:
  - Clear the viewer control.
  - Set header to empty or "(no selection)".

**Files**
- `crates/harvester_app/src/platform/ui/render.rs`

**Acceptance**
- Selecting a job updates the preview pane with content.
- Switching between jobs updates the preview.
- Deselecting clears the preview.

---

## Phase 3 — In-progress preview (live content during pipeline)

### Step3.1_SendContentAtConvertCompleteV2
**Change**
- After the Convert stage completes in the engine pipeline, emit a progress event carrying the markdown content:
  - Option A: Add `content_preview: Option<String>` to `EngineEvent::Progress` / `JobProgress`.
  - Option B: Emit a separate event: `EngineEvent::ContentAvailable { job_id, content }`.
- Truncate to `MAX_PREVIEW_CONTENT` before sending.

**Rationale:** This lets the user see extracted content *before* tokenizing and writing complete — useful for slow pipelines or assessing extraction quality early.

**Preferred:** Option A (extend `JobProgress`) to keep the event model simple.

**Files**
- `crates/harvester_engine/src/types.rs` (extend `JobProgress`)
- `crates/harvester_engine/src/engine.rs` (emit content after Convert)

**Tests**
- Progress event after Convert includes content.
- Progress events for other stages have `content_preview: None`.

**Acceptance**
- Build + tests pass.

### Step3.2_StorePartialContentOnProgressV2
**Change**
- In `Msg::JobProgress` handling in core:
  - If `content_preview` is `Some`, store in `JobState`.
  - Update `PreviewState` to `InProgress { job_id, content }` if this job is currently selected.

**Files**
- `crates/harvester_core/src/msg.rs` (extend `Msg::JobProgress` with `content_preview`)
- `crates/harvester_core/src/update.rs`
- `crates/harvester_app/src/platform/effects.rs` (map content through from engine event)

**Tests**
- Progress with content updates job state.
- If job is currently selected, preview updates live.
- If job is not selected, content stored silently for later.

**Acceptance**
- Selecting an in-progress job (post-Convert) shows partial content.
- Preview updates when Convert completes for the selected job.

---

## Phase 4 — UX polish

### Step4.1_PreviewHeaderWithQualitySignalsV2
**Change**
- Compute cheap quality signals during content storage and include in header:
  - **Token count** (already available from job metrics).
  - **Heading count** — count lines starting with `#` (cheap O(n) scan).
  - **Link density** — ratio of markdown links to total words. High ratio suggests navigation-heavy page.
- Display in header: `"example.com | 1,234 tokens | 8 headings | Done"`
- If link density > 0.3, append `"[nav-heavy]"` indicator.

**Files**
- `crates/harvester_core/src/state.rs` (compute signals on content store)
- `crates/harvester_core/src/view_model.rs` (include in `PreviewHeaderView`)
- `crates/harvester_app/src/platform/ui/render.rs` (format header string)

**Acceptance**
- Header shows meaningful at-a-glance metadata.
- Quality signals help decide keep/skip without reading full content.

### Step4.2_AddKeyboardShortcutsV1
**Change**
- Add shortcuts (MVP):
  - `Enter` / `K` keep (no-op, advance to next)
  - `D` discard (future: mark discarded)
  - `↑`/`↓` navigate jobs with preview updating

**Blocker**
- If CommanDuctUI does not expose key events in a usable way, this waits.

---

## Quality gates

### QualityMemoryBoundedV2
- All stored content respects `MAX_PREVIEW_CONTENT` (40 KB).
- Total preview memory bounded at `job_count * MAX_PREVIEW_CONTENT`.
- Truncation is deterministic and tested.

### QualityStaleProtectionV2
- Preview display always matches the currently selected job.
- Late-arriving content for a previously-selected job is discarded.

### QualityNoHardcodedLengthsV1
- Avoid fixed buffers when reading UI text or files; size dynamically.
- For preview truncation, use explicit constants (documented) and test.

### QualityLoggingWithContextV1
- Any file I/O failure (cold path) includes `job_id` and filename in logs.

### QualityTestsLockBehaviorV1
- Every new behavior comes with a unit test (state transitions, content storage, view model projection).

### QualityBackwardsCompatibilityV2
- Persistence format changes use `Option<T>` for new fields.
- Include a fixture test that deserializes the old format (without new fields) successfully.

---

## Testing strategy

### TestPureStateMachineV2
The hot path is entirely within `harvester_core` (pure state), making it trivially testable:
1. Send `Msg::JobDone` with content → verify stored.
2. Send `Msg::JobSelected` → verify `PreviewState` transition.
3. Call `state.view()` → verify view model has preview text.

No mocking of file I/O needed for the primary path.

### TestStateTransitionCoverageV2
Enumerate all `(PreviewState, Msg)` pairs:
- `Empty` + `JobSelected(with content)` → `Available`
- `Empty` + `JobSelected(no content)` → `Unavailable`
- `Available(A)` + `JobSelected(B)` → `Available(B)` or `Unavailable(B)`
- `Available(A)` + `JobSelected(A)` → no-op (no dirty flag)
- `InProgress(A)` + `JobProgress(A, content)` → `InProgress(A, updated)`
- `InProgress(A)` + `JobDone(A)` → `Available(A)`
- Any + `JobSelected(different)` → replaces

### TestTruncationV2
- Content at exactly `MAX_PREVIEW_CONTENT` → stored as-is (no marker).
- Content at `MAX_PREVIEW_CONTENT + 1` → truncated with `\n…[truncated]` marker.
- Empty content → stored as empty string.
- Output length never exceeds `MAX_PREVIEW_CONTENT + marker.len()`.

### TestFrontmatterStrippingV2
- Standard frontmatter (between `---` lines) → stripped.
- No frontmatter → content unchanged.
- Malformed (no closing `---`) → content preserved as-is.
- Frontmatter with complex YAML (multiline values) → correctly stripped.

### TestPersistenceFixtureV2
- Save a fixture file of the current RON format (before changes) in `tests/fixtures/`.
- Assert it deserializes correctly after schema changes.
- This is a regression guard for backwards compatibility.

---

## Future phases (after MVP)

### FutureColdPathFileLoadingV2
For jobs restored from persistence (no content in memory):
- Extend `CompletedJobSnapshot` with `output_filename: Option<String>`.
- Add `Effect::LoadPreview { job_id, output_filename }`.
- Add `Msg::PreviewLoaded { job_id, content }` / `Msg::PreviewLoadFailed { job_id, error }`.
- Effect executor reads file from `output_dir.join(output_filename)` with size limit.
- On success → store content, transition to `Available`.
- Handle missing files gracefully: "File not found — may have been archived or deleted."
- Read as UTF-8, replace invalid sequences with U+FFFD.

### FutureDiscardWorkflowV2
- Add a "Discarded" state in core:
  - `Msg::DiscardSelected`
  - `Effect::DiscardArtifacts { output_filename }` to delete/move files
- Keep discard reversible (move to `discarded/` folder first).
- Batch triage mode: arrow through jobs, `D` to discard, `K` to keep, confirm at end.

### FutureHeuristicSignalsV2
Compute cheap signals to help discard decisions:
- `token_count < 50` → `[stub]`
- Contains "subscribe" + "premium" near top → `[paywall?]`
- Contains "accept cookies" / "consent" in first 500 chars → `[cookie wall?]`
- Content hash for duplicate detection → `[duplicate of #N]`

Show as indicators in preview header.

### FutureRichPreviewV2
- Optional RichEdit-based rendering with basic styling.
- Use `pulldown-cmark` to apply heading emphasis, bold, code formatting.
- Keep raw markdown preview as the default (it serves the inspection use case better).

### FuturePreviewCachingV2
- LRU cache in app layer: `LruCache<JobId, String>` (capacity ~20).
- Re-selecting a job with cached content skips any cold-path file read.
- Invalidation: on job re-run or file deletion.

### FutureSearchWithinPreviewV2
- `Ctrl+F` find box for the preview pane.
- Highlight matches by scrolling to first occurrence.

### FutureSideBySideComparisonV2
- Select two jobs and show previews side-by-side.
- Simple diff highlighting (longest common subsequence) for detecting near-duplicate pages from the same site.

### FutureOutlineNavigationV2
- Extract headings into a mini-outline list.
- Clicking a heading scrolls the preview to that section.

### FuturePDFPipelineV1
- If/when PDF ingestion exists, reuse the same content-through-events pattern for extracted markdown.
