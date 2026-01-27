# Plan: Explore extracted links in TreeView with multi-color markers (blue-dot based)

## Goals

GoalExploreLinksInTreeViewV1
Expose extracted links under each downloaded page/job node in the TreeView, so the user can quickly browse and decide what to download next.

GoalDownloadLinksWithoutTopLevelNodesV1
Clicking/toggling a link downloads that page without creating a new top-level root node; the downloaded page remains discoverable from the link entry.

GoalArchiveIncludesDownloadedLinksV1
Final archive creation includes all downloaded linked pages (deduplicated by canonical URL).

GoalFastVisualStatusV1
Provide clear visual status for links and linked pages (downloaded/downloading/failed/old-suspect) using the proven "blue dot" custom-draw approach, upgraded to multiple colors.

GoalHeuristicAgeSignalNotDisableV1
Apply multiple heuristics to estimate link age; do not disable links. Instead, show "old-suspect" visual indications.

GoalArchitectureAndTestabilityV1
Maintain strong encapsulation and strict Unidirectional Data Flow: UI emits actions; reducers are pure; effects handle IO; state is owned by feature modules; no external mutation.

---

## Constraints and key observations from CommanDuctUI

ConstraintNoPerItemIconPipelineV1
CommanDuctUI TreeView insertion does not support per-item image indices; icons are not available without larger changes.

ConstraintBlueDotAlreadyExistsV1
There is existing (currently deactivated) custom draw "blue dot" logic in [treeview_handler.rs:975-995](src/CommanDuctUI/src/controls/treeview_handler.rs#L975-L995); reusing it is low-risk and extendable to multiple colors.

ConstraintNoExpandCollapseEventsV1
CommanDuctUI does not currently emit expand/collapse events, so link nodes should initially be pre-populated (with caps/paging) or updated via repopulation patterns.

ConstraintUiStateProviderLimitedV1
`UiStateProvider` trait in [types.rs:522-528](src/CommanDuctUI/src/types.rs#L522-L528) currently only has `is_tree_item_new()`. Must extend with `tree_item_marker()` method.

ConstraintJobRowViewNoLinksV1
`JobRowView` in [view_model.rs:54-62](crates/harvester_core/src/view_model.rs#L54-L62) does NOT expose `extracted_links`. The renderer only receives `AppViewModel.jobs: Vec<JobRowView>`, so must update the view model.

ConstraintExtractedLinkMetadataDiscardedV1
`ExtractedLink { url, text, kind }` in [links.rs:15-20](crates/harvester_engine/src/links.rs#L15-L20) has anchor text and link kind (Hyperlink/Image/Email), but [effects.rs](crates/harvester_app/src/platform/effects.rs) discards everything except `url`. Preserve `text` for display and `kind` for filtering.

ConstraintExportScansFilesystemV1
`build_concatenated_export()` in [export.rs:55-127](crates/harvester_engine/src/export.rs#L55-L127) scans output_dir for `.md` files. Downloaded link pages must be written to disk in the same format.

ConstraintTreeItemIdIsU64V1
`TreeItemId(pub u64)` in [types.rs:39](src/CommanDuctUI/src/types.rs#L39). Currently job nodes use `TreeItemId(job.job_id)`. Link nodes need an encoding strategy to distinguish job nodes from link nodes.

---

## MVP scope

MvpScopeV1
1) Show extracted links under each page node (`Links (N)` folder).
2) Use checkbox toggle on a link to download (checked) / delete cached download (unchecked).
3) Archive includes downloaded link pages.
4) Add multi-color marker support in CommanDuctUI based on the blue-dot custom draw path.
5) Use markers to indicate link state (downloaded/downloading/failed/old-suspect) without requiring icons.

---

# Phase 0 — Baseline and guardrails (runnable after each step)

## Step 0.1 — Create a feature branch + baseline build/test routine

ChangeSummaryV0_1
- Create a feature branch in the main repo and in the CommanDuctUI submodule.
- Document the local workflow for incremental steps.

ExpectedBehaviorV0_1
- No runtime behavior changes.

HowToTestV0_1
- Developer: `cargo build` succeeds.
- Developer: run existing unit tests (if any) for both repos/submodules.

---

## Step 0.2 — Add "marker plumbing" test seam in CommanDuctUI (no drawing yet)

ChangeSummaryV0_2
- Introduce a small, testable abstraction that maps a TreeItemId → marker kind, without exposing internal TreeView state.
- Do not change painting yet.

DesignV0_2
- Add a public enum (kept small and stable):
  - `TreeItemMarkerKind::{None, Blue, Green, Yellow, Red, Purple, Gray}` (or `Rgb(u8,u8,u8)` if you prefer, but fixed palette is simpler for UX consistency).
- Extend the existing `UiStateProvider` trait in [types.rs](src/CommanDuctUI/src/types.rs) to add:
  - `fn tree_item_marker(&self, window_id: WindowId, id: TreeItemId) -> TreeItemMarkerKind`
- Default implementation returns `None` so existing apps are unaffected.

EncapsulationNotesV0_2
- Marker is a *view concern* derived from app-owned state; CommanDuctUI only *queries* via trait.
- No mutable references to UI internals are exposed.

ExpectedBehaviorV0_2
- No visual changes yet.

HowToTestV0_2
- Unit tests in CommanDuctUI:
  - "default provider returns None"
  - "custom provider returns expected marker for given id"
- Developer: `cargo build` for CommanDuctUI and main app.

---

# Phase 1 — MVP: multi-color dot markers in CommanDuctUI

## Step 1.1 — Re-enable blue-dot custom draw and route it through marker query

ChangeSummaryV1_1
- Re-enable the existing TreeView custom draw path that draws the blue dot (deactivated code at [treeview_handler.rs:43-51, 975-995](src/CommanDuctUI/src/controls/treeview_handler.rs#L975-L995)).
- Replace "is_tree_item_new" style logic with `tree_item_marker(id)` to decide whether/what to draw.

ImplementationNotesV1_1
- Keep the drawing in the TreeView handler only; no changes to tree item insertion.
- Determine item's `TreeItemId` in the paint handler the same way the previous dot logic did.
- Draw a filled ellipse in the requested color; reserve the same left padding offset used historically.

ExpectedBehaviorV1_1
- In the CommanDuctUI demo harness (or the main app if already implementing provider), some rows can show a dot marker (depending on provider).

HowToTestV1_1
- Unit tests:
  - No reliable unit testing for Win32 painting; instead test the pure mapping path:
    - given marker kind, mapping returns expected GDI brush color (if implemented as a function).
- Manual QA:
  - Run an app that returns a marker for a few IDs; verify dot appears aligned and doesn't overlap text.
  - Verify selection highlight still works and dot remains visible.

---

## Step 1.2 — Add multi-color palette support (still MVP)

ChangeSummaryV1_2
- Support multiple colors by mapping `TreeItemMarkerKind` to distinct RGB values (GDI brush).
- Ensure colors are accessible against selected/unselected backgrounds.

ExpectedBehaviorV1_2
- Different rows can show different dot colors.

HowToTestV1_2
- Manual QA:
  - Verify each color renders, including in dark/light themes if applicable.
  - Verify no flicker; scrolling does not leave artifacts.

---

## Step 1.3 — Version + changelog for CommanDuctUI submodule

ChangeSummaryV1_3
- Bump CommanDuctUI version in its Cargo.toml, add CHANGELOG entry.

ExpectedBehaviorV1_3
- No runtime changes beyond prior steps.

HowToTestV1_3
- Developer: `cargo build` of main repo with updated submodule.

---

# Phase 2 — MVP: link nodes in the main application TreeView

## Step 2.1 — Domain model: represent extracted links as first-class "link items" owned by state

ChangeSummaryV2_1
- Replace `extracted_links: Vec<String>` in `JobState` with a `LinkRecord` type that tracks:
  - canonical URL
  - anchor text (preserved from `ExtractedLink.text` for display)
  - link kind (Hyperlink/Image/Email for filtering)
  - download state (NotDownloaded/Downloading/Downloaded/Failed)
  - optional age estimate + confidence
  - error info (if download failed)
- Update [effects.rs](crates/harvester_app/src/platform/effects.rs) to pass full `Vec<ExtractedLink>` to `Msg::JobDone` instead of just URLs.

DesignV2_1
```rust
// In harvester_core:
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkDownloadState {
    NotDownloaded,
    Downloading,
    Downloaded { path: PathBuf },
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRecord {
    pub index: u32,                       // Position in original list, for stable TreeItemId
    pub url: String,                      // Canonical URL
    pub anchor_text: Option<String>,      // From ExtractedLink.text
    pub kind: LinkKind,                   // Hyperlink | Image | Email (from harvester_engine)
    pub download_state: LinkDownloadState,
    pub age_estimate: Option<AgeEstimate>,
}

// In JobState (state.rs):
links: Vec<LinkRecord>,  // Replaces extracted_links: Vec<String>
```

RationaleV2_1
- Enables deterministic reducers and robust UI updates without UI-owned shadow state, per UDF architecture.
- Preserving anchor text improves UX (readable labels vs raw URLs).
- Preserving link kind enables filtering (skip Email/Image for download).

EncapsulationV2_1
- Keep fields private; expose behavior methods:
  - `job.attach_extracted_links(links: Vec<ExtractedLink>)` — creates LinkRecords with indexes
  - `job.mark_link_download_requested(link_index: u32)`
  - `job.mark_link_download_completed(link_index: u32, path: PathBuf)`
  - `job.mark_link_download_failed(link_index: u32, error: String)`
  - `job.mark_link_deleted(link_index: u32)`
- Avoid returning internal Vecs; return derived iterators or copies as needed.

ExpectedBehaviorV2_1
- No UI changes yet. App still builds and runs.

HowToTestV2_1
- Unit tests (core/reducer):
  - Given JobDone with links: state now contains LinkRecords with correct indexes.
  - Duplicate links: dedupe by canonical URL, keeping first occurrence's index.
  - Anchor text and kind are preserved.

---

## Step 2.1.1 — Update ViewModel, TreeItemId encoding, and persistence for link data

ChangeSummaryV2_1_1
- Update `JobRowView` to include link summary data for rendering.
- Define TreeItemId encoding strategy for jobs, folders, and links.
- Update `CompletedJobSnapshot` for persistence of link download states.

DesignViewModelV2_1_1
```rust
// In view_model.rs:
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRowView {
    pub index: u32,
    pub url: String,
    pub label: String,           // anchor_text.unwrap_or(truncated_url)
    pub kind: LinkKind,
    pub download_state: LinkDownloadState,
    pub age_suspect: bool,       // Derived from age_estimate threshold
}

pub struct JobRowView {
    pub job_id: JobId,
    pub url: String,
    pub stage: Stage,
    pub outcome: Option<JobResultKind>,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    // NEW fields:
    pub link_count: usize,
    pub downloaded_link_count: usize,
    pub links: Vec<LinkRowView>,  // First K links for display (K=200)
}
```

DesignTreeItemIdEncodingV2_1_1
```rust
// In render.rs or a shared tree_item_ids module:
const LINK_FLAG: u64 = 0x8000_0000_0000_0000;
const FOLDER_FLAG: u64 = 0x4000_0000_0000_0000;

pub fn job_tree_item_id(job_id: u64) -> TreeItemId {
    TreeItemId(job_id)  // No flag, backward compatible
}

pub fn links_folder_tree_item_id(job_id: u64) -> TreeItemId {
    TreeItemId(FOLDER_FLAG | job_id)
}

pub fn link_tree_item_id(job_id: u64, link_index: u32) -> TreeItemId {
    TreeItemId(LINK_FLAG | ((job_id & 0x7FFF_FFFF) << 32) | link_index as u64)
}

pub enum TreeItemKind {
    Job { job_id: u64 },
    LinksFolder { job_id: u64 },
    Link { job_id: u64, link_index: u32 },
}

pub fn decode_tree_item_id(id: TreeItemId) -> TreeItemKind {
    if id.0 & LINK_FLAG != 0 {
        TreeItemKind::Link {
            job_id: (id.0 >> 32) & 0x7FFF_FFFF,
            link_index: (id.0 & 0xFFFF_FFFF) as u32,
        }
    } else if id.0 & FOLDER_FLAG != 0 {
        TreeItemKind::LinksFolder { job_id: id.0 & !FOLDER_FLAG }
    } else {
        TreeItemKind::Job { job_id: id.0 }
    }
}
```

DesignPersistenceV2_1_1
```rust
// In state.rs - update CompletedJobSnapshot:
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkSnapshotRecord {
    pub url: String,
    pub downloaded_path: Option<String>,  // Some(path) if downloaded
}

pub struct CompletedJobSnapshot {
    pub url: String,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub links: Vec<LinkSnapshotRecord>,  // Changed from Vec<String>
}
```
On restore: reconstruct `LinkDownloadState::Downloaded` for links with `downloaded_path.is_some()`.

ExpectedBehaviorV2_1_1
- No UI changes yet; app builds and runs.
- Persistence roundtrip preserves link download states.

HowToTestV2_1_1
- Unit tests:
  - TreeItemId encoding/decoding roundtrip for all kinds.
  - JobRowView includes correct link_count and links.
  - CompletedJobSnapshot serialization/deserialization preserves link download states.

---

## Step 2.2 — Add actions/messages for link download and delete (pure reducer + effects)

ChangeSummaryV2_2
- Add new actions/messages:
  - `LinkToggleRequested { job_id, link_index, checked }`
  - `LinkDownloadStarted { job_id, link_index }`
  - `LinkDownloadCompleted { job_id, link_index, path }`
  - `LinkDownloadFailed { job_id, link_index, error }`
  - `LinkDeleted { job_id, link_index }`
- Add new effect (separate from job pipeline to satisfy GoalDownloadLinksWithoutTopLevelNodesV1):
  - `Effect::DownloadLinkedPage { job_id, link_index, url }` — writes to `output_dir/linked/<hash>.md`
  - `Effect::DeleteLinkedPage { job_id, link_index, path }`
- Reducer:
  - On toggle checked → set state to Downloading, emit `DownloadLinkedPage` effect
  - On toggle unchecked → emit `DeleteLinkedPage` effect, reset state to NotDownloaded

RationaleV2_2
Using a separate `DownloadLinkedPage` effect (not reusing the job pipeline) ensures downloaded link pages do NOT appear as top-level jobs, per GoalDownloadLinksWithoutTopLevelNodesV1.

ExpectedBehaviorV2_2
- No TreeView changes yet, but link state transitions are now representable and logged (use engine_logging macros).

HowToTestV2_2
- Unit tests:
  - Reducer produces the correct effect and state transitions for each action.
- Integration smoke:
  - Run app; no crash; logs show actions when triggered from temporary harness.

---

## Step 2.3 — TreeView rendering: add `Links (N)` folder and link children (pre-populated with cap)

ChangeSummaryV2_3
- Modify TreeView population in [render.rs:build_job_tree](crates/harvester_app/src/platform/ui/render.rs#L221-L232) to render for each job:
  - job node (now `is_folder: true`)
    - `Links (N)` child folder
      - first K link nodes (K=200, configurable)
      - optional `(show more…)` node if N > K
- Filter out Email/Image links from display (only show Hyperlink kind).

DesignV2_3
```rust
fn build_job_tree(view: &AppViewModel) -> Vec<TreeItemDescriptor> {
    view.jobs
        .iter()
        .map(|job| {
            let link_children = build_link_children(job);
            let links_folder = if job.link_count > 0 {
                vec![TreeItemDescriptor {
                    id: links_folder_tree_item_id(job.job_id),
                    text: format!("Links ({})", job.link_count),
                    is_folder: true,
                    state: CheckState::Unchecked,
                    children: link_children,
                    style_override: None,
                }]
            } else {
                vec![]
            };

            TreeItemDescriptor {
                id: job_tree_item_id(job.job_id),
                text: format_job_row(job),
                is_folder: true,  // Changed: jobs are now folders
                state: CheckState::Unchecked,
                children: links_folder,
                style_override: None,
            }
        })
        .collect()
}

fn build_link_children(job: &JobRowView) -> Vec<TreeItemDescriptor> {
    let mut children: Vec<_> = job.links
        .iter()
        .filter(|link| link.kind == LinkKind::Hyperlink)  // Filter out Image/Email
        .map(|link| TreeItemDescriptor {
            id: link_tree_item_id(job.job_id, link.index),
            text: link.label.clone(),
            is_folder: false,
            state: match link.download_state {
                LinkDownloadState::Downloaded { .. } => CheckState::Checked,
                _ => CheckState::Unchecked,
            },
            children: vec![],
            style_override: None,
        })
        .collect();

    // Add "show more" node if needed
    if job.link_count > job.links.len() {
        children.push(TreeItemDescriptor {
            id: TreeItemId(FOLDER_FLAG | 0xFFFF_FFFF | ((job.job_id & 0xFFFF) << 16)),
            text: format!("(show more... {} remaining)", job.link_count - job.links.len()),
            is_folder: false,
            state: CheckState::Unchecked,
            children: vec![],
            style_override: None,
        });
    }

    children
}
```

ConstraintsV2_3
- Since there are no expand/collapse events, populate children up-front (capped).

ExpectedBehaviorV2_3
- After a page finishes and extracts links, the TreeView shows a `Links (N)` section with link entries.

HowToTestV2_3
- Manual QA:
  - Add a URL with many links; ensure UI stays responsive and respects cap.
  - Verify `Links (N)` count matches extracted number.
- Unit tests (renderer):
  - Given state with N links, output tree has folder + min(N,K) children + show-more node if needed.
  - Email/Image links are filtered out.

---

## Step 2.4 — Interaction wiring: use checkbox toggle for download/delete; selection for preview

ChangeSummaryV2_4
- `TreeViewItemToggledByUser` on a link node: decode TreeItemId, dispatch `LinkToggleRequested { job_id, link_index, checked }`.
- `TreeViewItemSelectionChanged` on a link node: show link details/preview in the main view panel; does not automatically download.

DesignEventHandlingV2_4
```rust
fn handle_tree_view_toggled(item_id: TreeItemId, new_state: CheckState) -> Option<Msg> {
    match decode_tree_item_id(item_id) {
        TreeItemKind::Link { job_id, link_index } => {
            Some(Msg::LinkToggleRequested {
                job_id,
                link_index,
                checked: new_state == CheckState::Checked,
            })
        }
        TreeItemKind::Job { job_id } => {
            // Existing job toggle handling...
        }
        _ => None,
    }
}
```

ExpectedBehaviorV2_4
- Selecting a link shows URL and status.
- Toggling checkbox starts download; toggling off deletes cached download.

HowToTestV2_4
- Manual QA:
  - Toggle on → see download start; once complete, preview becomes available.
  - Toggle off → cached content removed; preview disappears or shows "not downloaded".
- Unit tests:
  - Event-to-action mapping: toggling produces the expected action with correct job_id and link_index.

---

## Step 2.5 — Use multi-color dot markers to show link state (first integration)

ChangeSummaryV2_5
- Implement `UiStateProvider::tree_item_marker()` in the app to return markers based on link state.

DesignV2_5
```rust
fn tree_item_marker(&self, _window_id: WindowId, item_id: TreeItemId) -> TreeItemMarkerKind {
    match decode_tree_item_id(item_id) {
        TreeItemKind::Link { job_id, link_index } => {
            if let Some(link) = self.get_link(job_id, link_index) {
                match link.download_state {
                    LinkDownloadState::Downloaded { .. } => TreeItemMarkerKind::Green,
                    LinkDownloadState::Downloading => TreeItemMarkerKind::Purple,
                    LinkDownloadState::Failed { .. } => TreeItemMarkerKind::Red,
                    LinkDownloadState::NotDownloaded if link.age_suspect => TreeItemMarkerKind::Yellow,
                    LinkDownloadState::NotDownloaded => TreeItemMarkerKind::None,
                }
            } else {
                TreeItemMarkerKind::None
            }
        }
        _ => TreeItemMarkerKind::None,
    }
}
```

ColorMeaningsV2_5
- Green: Downloaded successfully
- Purple: Download in progress
- Red: Download failed
- Yellow: Not downloaded, but flagged as old-suspect
- None: Not downloaded, no age flag

ExpectedBehaviorV2_5
- Link rows show dots by state. Quick scanning becomes practical.

HowToTestV2_5
- Manual QA:
  - Trigger each state and verify color changes.
  - Ensure markers do not show on job or folder rows.

---

## Step 2.6 — Archive: include downloaded link pages

ChangeSummaryV2_6
- Downloaded link pages are written to `output_dir/linked/` in same `.md` format as top-level pages.
- Update archive builder in [export.rs](crates/harvester_engine/src/export.rs) to scan both `output_dir/*.md` and `output_dir/linked/*.md`.
- Deduplicate by canonical URL (if a link URL matches a top-level job URL, don't include twice).

DesignV2_6
```rust
pub fn build_concatenated_export(
    output_dir: &Path,
    options: ExportOptions,
) -> Result<ExportSummary, ExportError> {
    ensure_output_dir(output_dir)?;

    // Collect from root dir
    let mut entries = collect_md_files(output_dir)?;

    // Collect from linked/ subdir
    let linked_dir = output_dir.join("linked");
    if linked_dir.exists() {
        entries.extend(collect_md_files(&linked_dir)?);
    }

    // Deduplicate by canonical URL
    let mut seen_urls = HashSet::new();
    entries.retain(|entry| {
        let url = extract_url_from_frontmatter(&entry);
        seen_urls.insert(normalize_url(&url))
    });

    // ... rest of export logic
}
```

EncapsulationV2_6
- The archive module should consume read-only snapshots/queries from the owning state module; do not reach into internal collections.

ExpectedBehaviorV2_6
- Final archive contains both root pages and downloaded link pages.

HowToTestV2_6
- Integration test with local HTTP fixture:
  - Root page links to B and C; download B only; archive includes Root + B, not C.
- Manual QA:
  - Confirm archive size increases when downloading links; confirm deleted link pages are removed from archive output.

---

# Phase 3 — Heuristic "old-suspect" age estimation (no disabling)

## Step 3.1 — Implement URL-based date heuristics + confidence scoring

ChangeSummaryV3_1
- Parse common patterns in URLs:
  - `/YYYY/MM/DD/`, `YYYY-MM-DD`, `YYYYMMDD`
- Produce `AgeEstimate { date, confidence: High, source: UrlPattern }`.
- Apply during `attach_extracted_links()` so age estimate is available immediately.

ExpectedBehaviorV3_1
- Some link rows gain "old-suspect" yellow dot if older than threshold.

HowToTestV3_1
- Unit tests:
  - Pattern parsing correctness.
  - Threshold comparison.
- Manual QA:
  - Use a news site URL list; verify plausible marking.

---

## Step 3.2 — Optional anchor-context heuristic (if you already have surrounding text)

ChangeSummaryV3_2
- Use preserved `anchor_text` from `LinkRecord` to parse dates with lower confidence.
- Mark low-confidence "old?" via a lighter style override or a different marker (e.g., Gray dot).

ExpectedBehaviorV3_2
- More links get age signals, but low-confidence cases are clearly distinguishable.

HowToTestV3_2
- Unit tests: parse known snippets.
- Manual QA: verify you're not over-flagging.

---

## Step 3.3 — After-download "verified-ish" metadata (optional)

ChangeSummaryV3_3
- After a link is downloaded, attempt:
  - HTTP `Last-Modified` header (low-medium reliability)
  - parse `<time>`/structured metadata in HTML (site-dependent)
- Store as `AgeEstimate` with source `DownloadedMetadata`.

ExpectedBehaviorV3_3
- Some links change from unknown/heuristic to a more confident estimate.

HowToTestV3_3
- Integration tests with fixture pages including `Last-Modified` and `<time datetime=...>`.

---

# Phase 4 — Quality of life and robustness

## Step 4.1 — "Show more…" paging for huge link lists

ChangeSummaryV4_1
- Clicking "Show more…" node dispatches action to increase cap for that job and triggers tree repopulation.
- Track per-job display cap in UI state (not core state).

ExpectedBehaviorV4_1
- User can progressively reveal more links without initial UI freeze.

HowToTestV4_1
- Manual QA: large link page; click show more repeatedly; ensure responsiveness.

---

## Step 4.2 — Add safe deletion affordance (without Win32 key routing)

ChangeSummaryV4_2
- Add a button/menu action "Delete downloaded link" acting on selected link.
- Keep checkbox as primary, but this supports keyboard-only workflows.

ExpectedBehaviorV4_2
- Selected link can be deleted without hunting for checkbox.

HowToTestV4_2
- Manual QA: select link, click delete, verify state.

---

## Step 4.3 — Logging and traceability pass

ChangeSummaryV4_3
- Add structured logs at action boundaries:
  - `[Links] LinkToggleRequested job_id=… link_index=… url=…`
  - `[Links] DownloadStarted job_id=… link_index=…`
  - `[Links] DownloadFailed job_id=… link_index=… error=…`
- Use engine_logging macros only.

HowToTestV4_3
- Manual QA: verify logs are readable and include url/job_id context.

---

# Final hardening step (after full plan)

## Step F.1 — Run clippy with warnings-as-errors

ChangeSummaryVF
- Run `cargo clippy --all-targets -- -D warnings` at the end (not during intermediate steps).

HowToTestVF
- Fix all warnings; ensure tests still pass.

---

# Future ideas (post-MVP)

FutureContextMenuV1
Add context menu on link nodes: Download, Delete cached, Copy URL, Open in browser.

FutureDomainFilterV1
Add domain allowlist/denylist and a "same domain only" view.

FutureLinkSearchSortV1
Search within links and sort by status, domain, or estimated age.

FutureBulkActionsV1
Download all "not old-suspect" links, or "download newest N".

FutureMarkerLegendV1
Small legend panel explaining dot colors; configurable palette.

FutureExpandEventsInCommanDuctUIV1
Add expand/collapse events to enable true lazy link loading for very large pages.

FutureOpenGraphPreviewV1
Show title/description favicon after download; keep it optional to avoid extra requests.

FutureDedupVisualIndicatorV1
Show visual indicator when a link URL matches an already-downloaded top-level job (avoid re-downloading).

FutureSmartLinkOrderingV1
Sort links by relevance: same-domain first, then by path depth, then alphabetically.

FutureKeyboardNavigationV1
Add keyboard shortcuts: Enter to toggle download, Delete to remove, arrow keys for navigation within link list.

FutureConfigurableCapsV1
Make MAX_EXTRACTED_LINKS (5000) and display cap K (200) configurable per-session or globally.
