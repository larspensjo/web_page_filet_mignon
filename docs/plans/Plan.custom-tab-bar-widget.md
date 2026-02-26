## Plan: Custom TabBar Widget for CommanDuctUI

**Draft Diary Entry**

```md
## 2026-02-26 — Custom TabBar Widget
Type: Implementation
Context: Radio-button tabs look non-standard (visible round indicators). Need a modern,
dark-themed tab bar with bottom accent line and hover effects — industry-standard look.
Change: CommanDuctUI gains a new TabBar custom-WndProc control. harvester_app migrates
all three tab bar sites (right pane, left pane, trend categories) from radio buttons to TabBar.
Evidence: TBD
Refs: docs/plans/Plan.custom-tab-bar-widget.md
```

### TL;DR

Add a first-class `TabBar` widget to the CommanDuctUI library, following the proven custom-WndProc pattern used by `chart_handler`. The widget owns its own painting (`WM_PAINT`), hit-testing (`WM_LBUTTONDOWN`), and hover tracking (`WM_MOUSEMOVE`), sending selection events to the parent via `SendMessageW` (same two-hop pattern as the splitter). Visual style: dark background, off-white text, colored bottom accent line on the active tab, subtle hover highlight. Then migrate all three tab-bar sites in `harvester_app` from radio buttons to the new widget. Keyboard navigation is deferred to a future enhancement.

Implementation is split into **4 independently-shippable slices** — CommanDuctUI types, CommanDuctUI widget, harvester_app migration, cleanup.

---

### Slice 1 — CommanDuctUI: Types & Plumbing (no Win32 calls)

**Goal:** Add all public types, enum variants, and message constants so that later slices can reference them. This slice is purely structural and fully unit-testable.

**Step 1.1** — Add `ControlKind::TabBar` to the `ControlKind` enum in window_common.rs (around line 99).

**Step 1.2** — Add `StyleId::TabBar` and `StyleId::TabBarAccent` to the `StyleId` enum in styling_primitives.rs (around line 62). `TabBar` covers background + text; `TabBarAccent` provides the accent-line color via its `background_color`.

**Step 1.3** — Add new `AppEvent` variant in types.rs (around line 317):
```
TabBarSelectionChanged { window_id: WindowId, control_id: ControlId, selected_index: usize }
```

**Step 1.4** — Add new `PlatformCommand` variants in types.rs (around line 660):
- `CreateTabBar { window_id, control_id, parent_control_id, items: Vec<String> }` — creates the control with initial tab labels.
- `SetTabBarItems { window_id, control_id, items: Vec<String> }` — replaces all tab labels (supports dynamic tab sets).
- `SetTabBarSelection { window_id, control_id, selected_index: usize }` — drives selection from the reducer.
- `SetTabBarStyle { window_id, control_id, background_color: Color, text_color: Color, accent_color: Color, font: Option<FontDescription> }` — pushes resolved palette data into the TabBar's `GWLP_USERDATA` state so the custom WndProc can paint without accessing `Win32ApiInternalState`. See **Style Plumbing** section below.

**Step 1.5** — Add `WM_APP_TAB_SELECTED` constant in window_common.rs (around line 81), following the existing allocation scheme:
```
pub(crate) const WM_APP_TAB_SELECTED: u32 = WM_APP + 0x104;
```

**Step 1.6** — Export `TabBar`-related types from lib.rs. No new public structs needed — only `StyleId::TabBar`, `StyleId::TabBarAccent`, the new `AppEvent` variant, and the new `PlatformCommand` variants, which are already exported.

**Step 1.7** — Register the new module: add `pub(crate) mod tab_bar_handler;` to controls.rs.

**Step 1.8** — Add `ControlKind::TabBar` to the leaf-control invalidation allowlist in window_common.rs (around line 622). This ensures the TabBar repaints after layout changes, matching the treatment of `Chart`, `Splitter`, and other custom-WndProc controls.

**Tests for Slice 1:**
- Unit test in `types.rs`: construct each new `PlatformCommand` and `AppEvent` variant, assert `Debug` output.
- Unit test: `StyleId::TabBar` and `StyleId::TabBarAccent` exist and are distinct.

---

### Style Plumbing — How TabBar Obtains Colors at Paint Time

**Problem:** Custom WndProc controls (`extern "system" fn`) have no reference to `Win32ApiInternalState`. The existing chart and splitter controls hard-code their colors. For TabBar, colors must be style-driven (no hard-coded palette in final state).

**Solution: Explicit `SetTabBarStyle` command** that snapshots palette data into `TabBarState` via `GWLP_USERDATA`.

The flow:
1. App defines `StyleId::TabBar` and `StyleId::TabBarAccent` via `DefineStyle` (already existing mechanism).
2. App sends `SetTabBarStyle { background_color, text_color, accent_color, font }` after creating the TabBar. The command handler in `app.rs` writes these values into the `TabBarState` stored in `GWLP_USERDATA`, then calls `InvalidateRect`.
3. The TabBar's `WM_PAINT` handler reads colors from its own `TabBarState` — no access to `Win32ApiInternalState` needed.
4. If the app never sends `SetTabBarStyle`, the `TabBarState` carries sensible defaults (`#2E3239` background, `#E0E5EC` text, `#0080FF` accent).

**Why not store `Arc<Win32ApiInternalState>` in the per-window state?** That would couple the control's WndProc to the framework's internal locking — risky for a reusable library widget. The snapshot approach keeps the control self-contained, consistent with chart and splitter.

**Why a dedicated command rather than reusing `ApplyStyleToControl`?** `ApplyStyleToControl` stores a `StyleId` in `NativeWindowData` and sends `WM_SETFONT` — it doesn't know how to push color data into `GWLP_USERDATA`. A dedicated command makes the data flow explicit and avoidable bugs minimal. This also allows the TabBar to receive a merged palette (background from one style, accent from another) in a single call.

---

### Slice 2 — CommanDuctUI: TabBar Widget Implementation

**Goal:** Implement the custom-WndProc tab bar control — registration, painting, hit-testing, hover tracking, event emission.

**Step 2.1** — Create src/CommanDuctUI/src/controls/tab_bar_handler.rs.

**Step 2.1a — Internal state struct:**

```rust
struct TabBarPalette {
    background: Color,         // default: #2E3239
    text_active: Color,        // default: #E0E5EC
    text_inactive: Color,      // derived: ~60% blend of text toward background
    hover_fill: Color,         // derived: white at 6% over background
    accent: Color,             // default: #0080FF
}

struct TabBarState {
    items: Vec<String>,          // tab labels
    selected_index: usize,       // active tab
    hover_index: Option<usize>,  // mouse-hover tab (None = no hover)
    tracking_mouse: bool,        // WM_MOUSELEAVE registered?
    item_rects: Vec<RECT>,       // computed per tab in WM_PAINT, reused in hit-test
    palette: TabBarPalette,      // style-driven colors, written by SetTabBarStyle
    font: Option<HFONT>,         // style-driven font, written by SetTabBarStyle
}
```

`TabBarPalette` provides a `new(background, text, accent) -> Self` constructor that derives `text_inactive` and `hover_fill` from the primary colors. This keeps the derivation logic in one place and testable.

Store on the heap via `Box`, pointer in `GWLP_USERDATA` (same pattern as `ChartWindowState` in chart_handler.rs).

**Step 2.1b — Window class registration:**

Follow chart_handler.rs. Class name: `w!("CommanDuctUITabBar")`. Use `OnceLock<()>` for one-time init. Style: `CS_HREDRAW | CS_VREDRAW`. Background brush: null (we paint everything).

**Step 2.1c — WndProc message handling:**

| Message | Behavior |
|---|---|
| `WM_ERASEBKGND` | Return `LRESULT(1)` — suppress flicker. |
| `WM_PAINT` | `BeginPaint` → compute tab rects → draw background → draw each tab label → draw accent line under active tab → draw hover highlight → `EndPaint`. See painting spec below. |
| `WM_SIZE` | `InvalidateRect` to trigger repaint (rects recomputed during paint). |
| `WM_LBUTTONDOWN` | Hit-test cursor against `item_rects`. If a tab is hit and different from `selected_index`, `SendMessageW(parent, WM_APP_TAB_SELECTED, WPARAM(hwnd), LPARAM(index))`. |
| `WM_MOUSEMOVE` | Hit-test cursor. If hover changes, update `hover_index`, `InvalidateRect`. If `!tracking_mouse`, call `TrackMouseEvent` with `TME_LEAVE`. |
| `WM_MOUSELEAVE` | Set `hover_index = None`, `tracking_mouse = false`, `InvalidateRect`. |
| `WM_DESTROY` | Drop the `Box<TabBarState>` from `GWLP_USERDATA`. |
| Default | `DefWindowProcW`. |

**Step 2.1d — Painting spec (dark theme, bottom accent line):**

All colors are read from `TabBarState.palette` — no hard-coded color constants in the painting code.

- **Bar background:** Fill entire client rect with `palette.background`.
- **Tab rects:** Use text-extent-based widths with 16px horizontal padding per tab. Store computed rects in `item_rects` for hit-testing.
- **Active tab text:** Render with `palette.text_active`.
- **Inactive tab text:** Render with `palette.text_inactive`.
- **Hover highlight:** For `hover_index` (if not the active tab), fill the tab rect with `palette.hover_fill`.
- **Accent line:** Draw a 2–3px tall rectangle at the bottom of the active tab's rect, using `palette.accent`.
- **Font:** Use `state.font` if set; otherwise `GetStockObject(DEFAULT_GUI_FONT)`.
- **Double buffering (optional but recommended):** Paint to an off-screen `CreateCompatibleDC` / `CreateCompatibleBitmap` then `BitBlt` to avoid flicker. Alternatively, rely on `WM_ERASEBKGND` suppression + full-rect fill.

**Step 2.1e — Tab rect computation helper:**

Create a pure function `compute_tab_rects(client_width: i32, client_height: i32, items: &[String], hdc: HDC) -> Vec<RECT>` that uses `GetTextExtentPoint32W` to measure each label, adds horizontal padding, and positions tabs sequentially from the left. This function is central to both painting and hit-testing. If total width exceeds the client rect, the excess tabs are clipped (future: scrolling).

**Step 2.2** — Wire up the `CreateTabBar` command in app.rs `execute_platform_command()` (around line 400). Follow the chart handler's 4‑phase create pattern from chart_handler.rs:
1. Read-lock → duplicate check + get parent HWND.
2. Write-lock → register `ControlKind::TabBar`.
3. `CreateWindowExW` (no lock held) → native HWND with the custom class.
4. Write-lock → store HWND.

Initialize `TabBarState` with the provided `items`, `selected_index: 0`, and a default `TabBarPalette`. Store in `GWLP_USERDATA`.

**Step 2.3** — Wire up `SetTabBarItems`, `SetTabBarSelection`, and `SetTabBarStyle` commands in app.rs:
- **`SetTabBarItems`:** Read-lock → get HWND. Get `TabBarState` from `GWLP_USERDATA`, update `items`. `InvalidateRect`.
- **`SetTabBarSelection`:** Read-lock → get HWND. Update `selected_index`. `InvalidateRect`.
- **`SetTabBarStyle`:** Read-lock → get HWND. Construct a `TabBarPalette` from the provided colors, create `HFONT` if font is provided, write both into `TabBarState`. `InvalidateRect`. This is the style-sync entry point — the app calls this after `DefineStyle`/`CreateTabBar` to push resolved palette data into the control.

**Step 2.4** — Handle `WM_APP_TAB_SELECTED` in the parent's `handle_window_message` in window_common.rs (near the `WM_APP_SPLITTER_DRAGGING` match arm around line 1620):
- Extract `ControlId` via `GetDlgCtrlID(HWND(wparam.0 as *mut _))`.
- Extract `selected_index` from `lparam.0 as usize`.
- Return `Some(AppEvent::TabBarSelectionChanged { window_id, control_id, selected_index })`.

**Tests for Slice 2:**
- Unit test `TabBarPalette::new()` — verify derived `text_inactive` and `hover_fill` from known inputs.
- Unit test `TabBarState` default construction and mutation.
- Unit test the `WM_APP_TAB_SELECTED` → `AppEvent::TabBarSelectionChanged` translation in `window_common.rs` (follow the splitter test pattern).
- Unit test that `ControlKind::TabBar` is registered correctly in the 4-phase create pattern (follow existing tests in `app.rs`).

---

### Slice 3 — harvester_app: Migrate All Tab Bars

**Goal:** Replace all three radio-button tab bar sites with the new `TabBar` widget: right-pane tabs (4), left-pane tabs (2), and trend-category tabs (4). Prompt Lab radio buttons remain unchanged.

**Step 3.1** — Add `from_index` / `to_index` methods to `AppTab`, `LeftTab`, and `TrendCategory` in tabs.rs. Use exhaustive `match` arms. `from_index` returns `Option<Self>` and logs a warning on out-of-range (correctness-by-construction — no panicking on bad indices from the UI).

**Step 3.2** — Define styles. In the style-definition section of the app (where `StyleId::RadioButton` is defined), add:
- `StyleId::TabBar` → `background_color: #2E3239`, `text_color: #E0E5EC`, same font as other controls.
- `StyleId::TabBarAccent` → `background_color: #0080FF` (the neon blue consistent with the progress bar palette).

**Step 3.3** — Update layout.rs:

Replace the **right-pane tab bar** creation (around line 208–243):
- Remove the 4 × `CreateRadioButton` commands for `BUTTON_TAB_TRIAGE` through `BUTTON_TAB_TRENDS`.
- Remove `PANEL_TAB_BAR` panel (no longer needed as a container — the TabBar is a single control).
- Add a single `CreateTabBar { control_id: TAB_BAR_RIGHT, parent_control_id: <right pane>, items: vec!["Triage", "Summary", "Briefing", "Trends"] }`.
- Follow with `SetTabBarStyle` using the colors from `StyleId::TabBar` and `StyleId::TabBarAccent`.
- Update the layout rule: the TabBar gets `DockStyle::Top, fixed_size: Some(28)` where `PANEL_TAB_BAR` used to be.

Replace the **left-pane tab bar** (around line 140–180):
- Remove the 2 × `CreateRadioButton` commands for `BUTTON_LEFT_TAB_JOBS` / `BUTTON_LEFT_TAB_PROMPT_LAB`.
- Remove `PANEL_LEFT_TAB_BAR` panel.
- Add `CreateTabBar { control_id: TAB_BAR_LEFT, parent_control_id: <left pane>, items: vec!["Jobs", "Prompt Lab"] }`.
- Follow with `SetTabBarStyle`.
- Layout: `DockStyle::Top, fixed_size: Some(28)`.

Replace the **trend-category tab bar** (search for `BUTTON_TREND_*` constants):
- Same pattern: single `CreateTabBar` replacing multiple radio buttons, plus `SetTabBarStyle`.

**Step 3.4** — Update constants.rs:
- Add new constants: `TAB_BAR_RIGHT`, `TAB_BAR_LEFT`, `TAB_BAR_TRENDS` (new ControlId values).
- Remove the old tab-specific radio constants: `BUTTON_TAB_*`, `PANEL_TAB_BAR`, `BUTTON_LEFT_TAB_*`, `PANEL_LEFT_TAB_BAR`, and trend-category button constants.
- **Keep** all Prompt Lab radio-button constants and `StyleId::RadioButton` — these are not migrated.

**Step 3.5** — Update render.rs:

Replace `render_tab_bar_section()` (around line 442):
- Instead of 4 × `SetRadioButtonChecked`, emit a single `SetTabBarSelection { control_id: TAB_BAR_RIGHT, selected_index: active_tab.to_index() }`.

Replace left-tab rendering:
- Single `SetTabBarSelection { control_id: TAB_BAR_LEFT, selected_index: left_tab.to_index() }`.

Replace trend-category tab rendering (in the trends preview rendering path):
- Single `SetTabBarSelection { control_id: TAB_BAR_TRENDS, selected_index: trend_category.to_index() }`.

**Step 3.6** — Update app.rs event handling (around line 465):

Replace the tab-related `RadioButtonSelected` match arms with `TabBarSelectionChanged` handlers:
```
AppEvent::TabBarSelectionChanged { control_id: TAB_BAR_RIGHT, selected_index, .. } =>
    Msg::TabSelected { tab: AppTab::from_index(selected_index) }
AppEvent::TabBarSelectionChanged { control_id: TAB_BAR_LEFT, selected_index, .. } =>
    Msg::LeftTabSelected { tab: LeftTab::from_index(selected_index) }
AppEvent::TabBarSelectionChanged { control_id: TAB_BAR_TRENDS, selected_index, .. } =>
    Msg::TrendCategorySelected { category: TrendCategory::from_index(selected_index) }
```

Keep all Prompt Lab `RadioButtonSelected` match arms unchanged.

**Tests for Slice 3:**
- Unit tests for `AppTab::to_index()` / `AppTab::from_index()` round-trip for all variants.
- Same for `LeftTab` and `TrendCategory`.
- Test `from_index` with out-of-range values returns `None`.
- Integration: verify that the render output contains `SetTabBarSelection` commands with the correct index for each active tab state.
- Verify no `SetRadioButtonChecked` commands remain in the render output **for migrated tab bars** (Prompt Lab radio buttons may still emit them).
- Update any existing layout/render/app-handler tests that reference the old radio-button tab commands or events to use the new TabBar equivalents.

---

### Slice 4 — Cleanup & Release

**Step 4.1** — Remove dead radio-button code for migrated tabs only:
- Delete the old `BUTTON_TAB_*`, `PANEL_TAB_BAR`, `BUTTON_LEFT_TAB_*`, `PANEL_LEFT_TAB_BAR`, and `BUTTON_TREND_*` constants from constants.rs.
- Remove the `RadioButtonSelected` match arms that were replaced by `TabBarSelectionChanged`.
- **Do not** remove `StyleId::RadioButton` or radio-button infrastructure — Prompt Lab still uses radio buttons.

**Step 4.2** — Update CommanDuctUI version and changelog:
- Bump version in src/CommanDuctUI/Cargo.toml from `0.6.0` to `0.7.0` (new public types & commands = breaking).
- Add `0.7.0` entry in src/CommanDuctUI/CHANGELOG.md documenting the new `TabBar` widget, new `AppEvent::TabBarSelectionChanged`, new `PlatformCommand` variants (`CreateTabBar`, `SetTabBarItems`, `SetTabBarSelection`, `SetTabBarStyle`), new `StyleId` variants (`TabBar`, `TabBarAccent`), and new `ControlKind::TabBar`.

**Step 4.3** — *(Optional — release hygiene only.)* If the harvester_app dependency on CommanDuctUI is path-based (workspace member), no version bump is needed in the consumer. Only update the version reference if the dependency uses an explicit version constraint.

**Step 4.4** — Finalize diary entry in docs/EngineeringDiary.md.

**Step 4.5** — Run full verification (see below).

---

### Verification

- **Visual:** Run `harvester_app` and confirm: tabs render as flat text labels on a dark bar, active tab has a blue bottom accent line, hovering inactive tabs shows a subtle highlight, clicking switches tabs correctly, no radio-button indicators visible. All three tab bar sites (right, left, trend categories) render correctly.
- **Automated — workspace:** `cargo nextest run` passes all new and existing tests.
- **Automated — submodule:** `cd src/CommanDuctUI && cargo test` passes all CommanDuctUI-local tests.
- **Clippy — workspace:** `cargo clippy --workspace --all-targets -- -D warnings` clean.
- **Clippy — submodule:** `cd src/CommanDuctUI && cargo clippy --all-targets -- -D warnings` clean.
- **Regression:** Existing functionality (triage, summary, briefing, trends views, Prompt Lab radio buttons) unchanged.
- **Dark-theme quality bar:**
  - Tab bar background sourced from style, not hard-coded in paint code.
  - Inactive text has sufficient contrast while visibly subordinate to active tab.
  - Hover layer remains subtle and non-flickering under rapid mouse movement.
  - Accent underline color is style-driven and consistent with existing blue highlight palette.
  - Rendering remains stable under resize and long labels.

---

### Decisions

- **Custom WndProc (Option B)** over owner-drawn radio buttons (A) or push buttons (C) — cleanest architecture, full rendering control, reusable widget.
- **Bottom accent line** (Chrome/VS Code style) for active tab indicator.
- **Hover highlight** included in initial implementation.
- **Keyboard navigation deferred** to future enhancement.
- **SendMessageW to parent** pattern (following splitter) for event emission — avoids deadlock risk of calling `send_event()` from a child WndProc.
- **`SetTabBarStyle` command** for style plumbing — snapshots palette into `GWLP_USERDATA` state rather than giving the custom WndProc access to `Win32ApiInternalState`. Consistent with chart/splitter isolation, but style-driven rather than hard-coded.
- **Two style IDs** (`TabBar` + `TabBarAccent`) at the `StyleId` level, merged into a single `TabBarPalette` struct in `TabBarState` — allows per-instance theming and future light-theme support.
- **`from_index` returns `Option`** rather than panicking — correctness-by-construction for untrusted UI indices.
- **Prompt Lab radio buttons kept** — only migrated tab groups are removed; `StyleId::RadioButton` and radio infrastructure remain.

---

### Future Ideas

#### [FI-Architecture-UiFramework-0010] TabBar Keyboard Navigation
Status: Candidate
TopLevel: Architecture
SubLevel: UiFramework
Priority: P2
Effort: M
Risk: L
Origin:
- SourceDoc: docs/plans/Plan.custom-tab-bar-widget.md
- SourceSection: Decisions
- Captured: 2026-02-26
Tags: [accessibility, keyboard, tabbar]
Summary: Add arrow-key and Tab-key navigation to the TabBar widget — `WM_KEYDOWN` handler, `WS_TABSTOP` style, focus rectangle painting.
Rationale: Industry standard for accessibility. Deferred from initial implementation to keep scope manageable.
SuccessCriteria:
- Arrow keys cycle through tabs when focused.
- Tab key moves focus in/out of the tab bar.
- Focus rectangle visible on the active tab.

#### [FI-Architecture-UiFramework-0011] TabBar Close Buttons
Status: Candidate
TopLevel: Architecture
SubLevel: UiFramework
Priority: P3
Effort: M
Risk: L
Origin:
- SourceDoc: docs/plans/Plan.custom-tab-bar-widget.md
- SourceSection: Future Ideas
- Captured: 2026-02-26
Tags: [tabbar, closeable-tabs]
Summary: Optional per-tab close button (×) with `WM_APP_TAB_CLOSE_CLICKED` event, for use cases with dynamic/closeable tabs.
Rationale: Common in IDE-style tab bars. The `WM_APP_TAB_CLOSE_CLICKED` constant slot is already reserved at `WM_APP + 0x105`.
SuccessCriteria:
- Close button renders on hover/active tabs when enabled.
- Clicking close emits `TabBarCloseClicked` event.

#### [FI-Architecture-UiFramework-0012] TabBar Scroll / Overflow
Status: Candidate
TopLevel: Architecture
SubLevel: UiFramework
Priority: P3
Effort: M
Risk: L
Origin:
- SourceDoc: docs/plans/Plan.custom-tab-bar-widget.md
- SourceSection: Step 2.1e
- Captured: 2026-02-26
Tags: [tabbar, overflow, scroll]
Summary: When tabs exceed the available width, add scroll arrows or a dropdown to access overflow tabs.
Rationale: Current implementation clips excess tabs. With dynamic tab counts this could become a usability issue.
SuccessCriteria:
- Overflow indicator visible when tabs exceed width.
- All tabs reachable via scroll or dropdown.

#### [FI-Architecture-UiFramework-0013] TabBar Animated Transitions
Status: Candidate
TopLevel: Architecture
SubLevel: UiFramework
Priority: P3
Effort: S
Risk: L
Origin:
- SourceDoc: docs/plans/Plan.custom-tab-bar-widget.md
- SourceSection: Future Ideas
- Captured: 2026-02-26
Tags: [tabbar, animation, polish]
Summary: Animate the accent-line sliding between tabs on selection change, using a `WM_TIMER`-driven interpolation.
Rationale: Adds visual polish matching modern UI frameworks. Low risk since it's purely cosmetic.
SuccessCriteria:
- Accent line slides smoothly between tabs over ~150ms.
- No visual artifacts or flicker.

#### [FI-Architecture-UiFramework-0014] Style-Sync for Custom WndProc Controls
Status: Candidate
TopLevel: Architecture
SubLevel: UiFramework
Priority: P2
Effort: M
Risk: M
Origin:
- SourceDoc: docs/plans/Review.Plan.custom-tab-bar-widget.feasibility.md
- SourceSection: Style plumbing must be explicit
- Captured: 2026-02-26
Tags: [theming, custom-controls, style-system]
Summary: Generalize the `SetTabBarStyle` pattern into a framework-level mechanism so that all custom-WndProc controls (chart, splitter, future widgets) can receive style data without hard-coding colors. Could be a `WM_APP_STYLE_CHANGED` message or an `ApplyStyleToControl` extension that detects custom controls and pushes palette data to `GWLP_USERDATA`.
Rationale: Currently chart and splitter hard-code colors. The TabBar introduces a per-command style push. A unified approach would prevent theme drift across all custom controls.
SuccessCriteria:
- Chart and splitter colors are style-driven, not hard-coded.
- Adding a new custom control doesn't require a new `Set*Style` command.
