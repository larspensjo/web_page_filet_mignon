# Split `state.rs` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Break the 5,251-line `crates/harvester_core/src/state.rs` into focused modules, each with a single responsibility, while preserving all existing behavior and public API.

**Architecture:** Extract six cohesive concerns into separate files, leaving `state/mod.rs` as the core `AppState` struct with its direct methods. Each new module is declared inside the `state/` directory module.

**Tech Stack:** Rust, `cargo build`, `cargo clippy`, `cargo test`

---

## Wrapper policy

Per `Agents.md`: "Keep `mod.rs` and `lib.rs` files as thin wrappers only."

- **`state/mod.rs`**: contains `AppState`, its `Default` impl, core `impl AppState` methods, `mod` declarations, and a small number of `pub(crate) use` re-exports for items that sibling modules (`update/`) import via `crate::state::*`. Does not barrel-export every extracted helper.
- **`lib.rs`**: keeps its curated public re-exports. If an extracted submodule defines a type that `lib.rs` currently re-exports (e.g. `SessionState`, `Stage`), `state/mod.rs` re-exports it with `pub use` so `lib.rs` continues to find it at `state::TypeName`. Small `lib.rs` re-export adjustments are acceptable if they keep `state/mod.rs` thinner.
- **Crate-internal access**: `PromptLabPendingRunRegistration` and `TriageCacheLookupResult` stay defined in `state/mod.rs` because `update/` imports them directly via `crate::state::`. Moving them out later is fine but not in scope for this plan.

---

## File Structure

After all tasks are complete, the layout will be:

| File | Responsibility | Approx lines |
|------|---------------|-------------|
| `state/mod.rs` | `AppState` struct, `Default`, core `impl AppState` methods, `mod` declarations, narrow re-exports | ~1,800 |
| `state/job_state.rs` | `JobState`, `PreviewQuality`, link attachment/snapshot helpers | ~250 |
| `state/ui_state.rs` | `UiState`, `PreviewState`, `PreviewMode`, `MetricsState` | ~200 |
| `state/indirect_links.rs` | `IndirectLink`, `IndirectLinkPool`, blocklist constants, `should_collect_indirect_link`, `host_matches_indirect_blocklist` | ~180 |
| `state/view_builder.rs` | `AppState::view()`, `layout_view()`, `build_right_pane_view()`, `build_left_pane_header_view()`, `build_preview_context_view()`, formatting helpers | ~550 |
| `state/link_helpers.rs` | `normalize_extracted_link`, `format_lab_*_markdown`, `domain_from_url`, `build_link_rows`, `link_label_for_record`, `truncate_link_url`, `map_job_filter_status`, link constants | ~200 |
| `state/tests.rs` | All `#[cfg(test)]` modules currently in `state.rs` | ~1,660 |

### Conversion to directory module

`state.rs` must become `state/mod.rs` to host submodules. Rust requires this structural change. The steps:

1. Create `crates/harvester_core/src/state/` directory
2. Move `state.rs` → `state/mod.rs`
3. Verify `cargo build` still compiles (module path unchanged from `lib.rs` perspective)

---

## Task 1: Convert `state.rs` to directory module

**Files:**
- Move: `crates/harvester_core/src/state.rs` → `crates/harvester_core/src/state/mod.rs`

- [ ] **Step 1: Create the directory and move the file**

```bash
mkdir -p crates/harvester_core/src/state
git mv crates/harvester_core/src/state.rs crates/harvester_core/src/state/mod.rs
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: success, no errors

- [ ] **Step 3: Run the test suite**

Run: `cargo test -p harvester_core`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_core/src/state/mod.rs
git commit -m "refactor: convert state.rs to state/mod.rs directory module"
```

---

## Task 2: Extract `job_state.rs`

This moves `JobState` (line 3163–3304) and `PreviewQuality` (line 3362–3405) into their own file. These types are private to the `state` module and used only within `state/mod.rs` and `state/view_builder.rs` (later).

**Files:**
- Create: `crates/harvester_core/src/state/job_state.rs`
- Modify: `crates/harvester_core/src/state/mod.rs`

- [ ] **Step 1: Create `job_state.rs`**

Move these items from `mod.rs` into the new file:
- `struct PreviewQuality` and its `impl Default`, `impl PreviewQuality` (lines 3362–3405)
- `struct JobState` and its full `impl JobState` block (lines 3163–3304)

Starting imports (verify with compiler — additions may be needed):

```rust
use super::{
    normalize_extracted_link, build_link_rows, JobId, JobOrigin, JobResultKind,
    LinkDownloadState, LinkRecord, Stage, MAX_EXTRACTED_LINKS,
};
use crate::preview::PreviewContentKind;
use crate::url_age::{guess_age_from_url, AgeEstimate};
use crate::view_model::{JobRowView, LinkRowView};
use harvester_engine::ExtractedLink;
use std::collections::HashSet;
use std::path::PathBuf;
```

Note: `build_link_rows` is still in `mod.rs` at this point (moved to `link_helpers.rs` in Task 5). Import it via `super::build_link_rows` for now.

Add `pub(super)` visibility to `JobState`, its fields, and all methods that `mod.rs` calls directly (check each call site). Keep internal helpers `fn` (no `pub`).

- [ ] **Step 2: Add module declaration in `mod.rs`**

At the top of `mod.rs`, after existing `use` statements, add:

```rust
mod job_state;
use job_state::{JobState, PreviewQuality};
```

Remove the moved items from `mod.rs`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: success

- [ ] **Step 4: Run tests**

Run: `cargo test -p harvester_core`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_core/src/state/job_state.rs crates/harvester_core/src/state/mod.rs
git commit -m "refactor: extract JobState and PreviewQuality into state/job_state.rs"
```

---

## Task 3: Extract `ui_state.rs`

Moves `MetricsState` (line 3407–3411), `PreviewState` + impl (lines 3413–3456), `PreviewMode` (lines 3458–3463), `UiState` + `Default` + impl (lines 3465–3567) into their own file.

**Files:**
- Create: `crates/harvester_core/src/state/ui_state.rs`
- Modify: `crates/harvester_core/src/state/mod.rs`

- [ ] **Step 1: Create `ui_state.rs`**

Move the four types listed above. Starting imports (verify with compiler):

```rust
use super::JobId;
use crate::preview::PreviewContentKind;
use crate::view_model::{DEFAULT_JOBS_PANEL_WIDTH, DEFAULT_WINDOW_WIDTH};
```

Add `pub(super)` visibility to `UiState`, `MetricsState`, `PreviewState`, `PreviewMode`, and all methods called from `mod.rs`.

- [ ] **Step 2: Add module declaration in `mod.rs`**

```rust
mod ui_state;
use ui_state::{MetricsState, PreviewMode, PreviewState, UiState};
```

Remove moved items from `mod.rs`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: success

- [ ] **Step 4: Run tests**

Run: `cargo test -p harvester_core`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_core/src/state/ui_state.rs crates/harvester_core/src/state/mod.rs
git commit -m "refactor: extract UiState, PreviewState, MetricsState into state/ui_state.rs"
```

---

## Task 4: Extract `indirect_links.rs`

Moves `IndirectLink`, `IndirectLinkPool`, blocklist constants, `host_matches_indirect_blocklist`, and `should_collect_indirect_link` (lines 41–430) into their own file.

**Files:**
- Create: `crates/harvester_core/src/state/indirect_links.rs`
- Modify: `crates/harvester_core/src/state/mod.rs`

- [ ] **Step 1: Create `indirect_links.rs`**

Move these items:
- `INDIRECT_LINK_BLOCKED_HOSTS` (line 41)
- `INDIRECT_LINK_BLOCKED_PATH_PREFIXES` (line 55)
- `INDIRECT_LINK_BLOCKED_PATH_CONTAINS` (line 72)
- `struct IndirectLink` (line 299)
- `struct IndirectLinkPool` + impl (lines 305–354)
- `fn host_matches_indirect_blocklist` (line 356)
- `fn should_collect_indirect_link` (line 362)

Starting imports (verify with compiler):

```rust
use super::{normalize_url_for_dedupe, JobId};
use std::collections::HashSet;
use url::Url;
```

`IndirectLink` and `IndirectLinkPool` are crate-internal — they are **not** part of the `lib.rs` public surface. Use `pub(crate)` on the structs and `pub(super)` on helper functions called from `mod.rs`. Keep `host_matches_indirect_blocklist` private (only used within the module).

- [ ] **Step 2: Add module declaration in `mod.rs`**

```rust
mod indirect_links;
use indirect_links::{IndirectLink, IndirectLinkPool, should_collect_indirect_link};
```

Remove moved items from `mod.rs`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: success

- [ ] **Step 4: Run tests**

Run: `cargo test -p harvester_core`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_core/src/state/indirect_links.rs crates/harvester_core/src/state/mod.rs
git commit -m "refactor: extract IndirectLinkPool and blocklist logic into state/indirect_links.rs"
```

---

## Task 5: Extract `link_helpers.rs`

Moves pure link-related utility functions and constants.

**Files:**
- Create: `crates/harvester_core/src/state/link_helpers.rs`
- Modify: `crates/harvester_core/src/state/mod.rs`

- [ ] **Step 1: Create `link_helpers.rs`**

Move these items:
- `const LINK_ROW_LIMIT` (line 37)
- `const LINK_LABEL_MAX` (line 38)
- `const LINK_LABEL_TRUNCATE_MARKER` (line 39)
- `fn normalize_extracted_link` (line 3052)
- `fn format_lab_triage_markdown` (line 3074)
- `fn format_lab_summary_markdown` (line 3094)
- `fn format_lab_briefing_markdown` (line 3113)
- `fn domain_from_url` (line 3117)
- `fn build_link_rows` (line 3322)
- `fn link_label_for_record` (line 3337)
- `fn truncate_link_url` (line 3350)
- `fn map_job_filter_status` (line 3306)

Starting imports (verify with compiler):

```rust
use crate::view_model::{JobFilterStatus, LinkRowView};
use super::{LinkDownloadState, LinkRecord};
use harvester_engine::truncate_to_char_boundary;
use url::Url;
```

All functions are `pub(super)` or less — they're module-internal helpers.

- [ ] **Step 2: Add module declaration in `mod.rs`**

```rust
mod link_helpers;
use link_helpers::{
    build_link_rows, domain_from_url, format_lab_briefing_markdown, format_lab_summary_markdown,
    format_lab_triage_markdown, map_job_filter_status, normalize_extracted_link,
};
```

Remove moved items from `mod.rs`.

Also update `job_state.rs` if it imports `normalize_extracted_link` via `super::` — it will now come through the re-export, so no change needed if using `super::normalize_extracted_link`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: success

- [ ] **Step 4: Run tests**

Run: `cargo test -p harvester_core`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_core/src/state/link_helpers.rs crates/harvester_core/src/state/mod.rs
git commit -m "refactor: extract link utility functions into state/link_helpers.rs"
```

---

## Task 6: Extract `view_builder.rs`

This is the largest extraction. Moves all view-building methods out of `impl AppState` and the free functions that support them.

**Files:**
- Create: `crates/harvester_core/src/state/view_builder.rs`
- Modify: `crates/harvester_core/src/state/mod.rs`

- [ ] **Step 1: Create `view_builder.rs`**

Move these items:
- `impl AppState` methods: `view()` (line 793), `format_briefing_preview_header()` (line 1007), `format_trends_preview_header()` (line 1035), `layout_view()` (line 1039), `build_right_pane_view()` (line 1076), `build_indirect_link_summary()` (line 1493)
- Free functions: `build_left_pane_header_view` (line 2937), `build_preview_context_view` (line 3014)

These become a new `impl AppState` block in the new file (Rust allows multiple impl blocks across files within the same module).

Starting imports (verify with compiler — the list below covers known dependencies but may need additions like `ArticleTriageState`, `normalize_url_for_dedupe`, `PreTriagePhase`):

```rust
use super::{
    format_lab_briefing_markdown, format_lab_summary_markdown, format_lab_triage_markdown,
    domain_from_url, build_link_rows, map_job_filter_status, normalize_url_for_dedupe,
    AppState, JobResultKind, PreviewMode, Stage,
};
use crate::briefing::BriefingPhase;
use crate::pre_triage_filter::PreTriagePhase;
use crate::preview::{self, PreviewContentKind};
use crate::tabs::{AppTab, JobListScope, LeftTab, TrendCategory};
use crate::triage::{ArticleTriageState, TriagePhase};
use crate::view_model::{
    AppViewModel, IndirectLinkPhase, IndirectLinkSummary, JobFilterStatus, JobRowView,
    LayoutViewModel, LeftPaneHeaderView, OperationProgress, PreviewContextView, PreviewHeaderView,
    RightPaneView, TriageAnnotationView, TOKEN_LIMIT,
};
```

The `view()` and `layout_view()` methods access many `AppState` fields — that's fine, they're in the same module so private fields are accessible.

- [ ] **Step 2: Add module declaration in `mod.rs`**

```rust
mod view_builder;
```

No `use` needed — the methods are `impl AppState` and automatically available.

Remove moved items from `mod.rs`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: success

- [ ] **Step 4: Run tests**

Run: `cargo test -p harvester_core`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_core/src/state/view_builder.rs crates/harvester_core/src/state/mod.rs
git commit -m "refactor: extract view-building methods into state/view_builder.rs"
```

---

## Task 7: Extract tests

Move all `#[cfg(test)]` modules (lines 3587–5251) into `state/tests.rs`.

**Files:**
- Create: `crates/harvester_core/src/state/tests.rs`
- Modify: `crates/harvester_core/src/state/mod.rs`

- [ ] **Step 1: Create `tests.rs`**

Move the three test modules:
- `mod tests` (line 3588–5124)
- `mod briefing_history_state_tests` (line 5125–5174)
- `mod poll_stats_view_tests` (line 5175–5207)
- `mod trends_view_tests` (line 5208–end)

Wrap all of them under a single `#[cfg(test)]` gate in the new file. Use `use super::*;` at the top of each sub-module to access the parent module's items.

- [ ] **Step 2: Add module declaration in `mod.rs`**

Replace the `#[cfg(test)]` block(s) at the bottom of `mod.rs` with:

```rust
#[cfg(test)]
mod tests;
```

- [ ] **Step 3: Verify tests compile and pass**

Run: `cargo test -p harvester_core`
Expected: all tests pass (same count as before)

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_core/src/state/tests.rs crates/harvester_core/src/state/mod.rs
git commit -m "refactor: move state tests into state/tests.rs"
```

---

## Task 8: Final verification

- [ ] **Step 1: Full build and lint**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings, no errors

- [ ] **Step 2: Full test suite**

Run: `cargo test`
Expected: all tests pass

- [ ] **Step 3: Verify line counts**

Run: `wc -l crates/harvester_core/src/state/*.rs`
Expected: `mod.rs` is approximately 1,800 lines or fewer. All new files are under 700 lines.

- [ ] **Step 4: Commit any fixups**

If clippy or tests required small adjustments, commit them:

```bash
git commit -m "refactor: fixup lint and visibility after state.rs split"
```

---

## Ordering rationale

The tasks are sequenced to minimize merge conflicts between steps:

1. **Directory conversion first** — structural prerequisite for all other tasks
2. **JobState** — extracted early because `view_builder.rs` (Task 6) needs to reference it; establishing the pattern for later extractions
3. **UiState** — similar private types, no dependency on later tasks
4. **IndirectLinkPool** — self-contained, no dependencies on other extracted modules
5. **Link helpers** — pure functions, but `job_state.rs` imports `normalize_extracted_link` so this must come after Task 2
6. **View builder** — depends on link helpers and job_state being in place so `use super::` imports resolve correctly; includes `build_indirect_link_summary` to keep all view-assembly logic in one place
7. **Tests last** — tests reference everything; moving them last avoids churn from updating test imports repeatedly
8. **Final verification** — catch any accumulated issues

## Notes

- `lib.rs` re-exports a curated subset from `state`. `state/mod.rs` re-exports only what `lib.rs` and sibling modules need — it does not barrel-export every submodule item. Small `lib.rs` adjustments are acceptable to keep `mod.rs` thin (see Wrapper Policy above).
- All new files use `pub(super)` or `pub(crate)` visibility — no new public API surface.
- The `update/` module imports `crate::state::PromptLabPendingRunRegistration` and `crate::state::TriageCacheLookupResult` — these stay defined in `mod.rs`, so no breakage.
- Import lists in each task are starting points. The compiler is the source of truth — expect to add or remove imports during implementation.
- Each task is independently compilable and testable. If a task fails, the prior commit is a safe rollback point.
