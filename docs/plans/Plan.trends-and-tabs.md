# Implementation Plan: Tab System and Entity Trend Chart

**Date:** 2026-02-24
**Design doc:** `docs/plans/Design.trends-and-tabs.md`
**Status:** Ready after review fixes

---

## Draft Diary Entry

**Context:** The right-pane preview area had grown into a single overloaded surface serving triage
output, article summary, briefing, and Prompt Lab. There was also no visibility into which
companies, technologies, or products were trending across the article archive. Both gaps pointed to
the same structural need: navigate between content modes and surface archive-level insight.

**Change:** Introduce content-area tabs (Triage, Summary, Briefing, Trends, Prompt Lab) replacing
the single preview pane. Extract structured entities from the summary LLM (V4 prompt). Build a
sidecar entity index (`output/.entity_index.ron`) updated live on every completion and cache hit,
rebuildable from existing caches. Add a Trends tab showing top-10 entity mentions per category over
a rolling 13-week window. Deferred to v1.1: GDI+ line chart (Slice 5).

---

## Goal

Add right-pane tabs (Triage, Summary, Briefing, Trends, Prompt Lab) and archive-level entity trend
visibility. The Trends tab shows line charts for Company, Technology, Product, and Theme mentions
across the article archive, extracted automatically by the summary LLM.

Delivered in five sequential slices. Each slice is independently shippable and testable.

---

## Slice 1 — Tab Shell and Prompt Lab Relocation

**Goal:** Right-pane content-area tabs exist and work. Prompt Lab lives in its own tab.
No change to LLM pipelines, data structures, or persistence.

### 1.1 — Core domain: `AppTab` and `Msg::TabSelected`

**File:** `crates/harvester_core/src/msg.rs`

Add:
```rust
Msg::TabSelected { tab: AppTab }
Msg::TrendCategorySelected { category: TrendCategory }   // also needed later; add now
```

**File:** `crates/harvester_core/src/state.rs` (in `UiState` or at `AppState` root)

Add a private `active_tab` field with a mutation method following project conventions
(prefer private fields with invariant-preserving methods over exposed pub fields):

```rust
// field (private or pub(crate))
active_tab: AppTab,   // Default: AppTab::Summary
```

Expose via method on `UiState`/`AppState`:
```rust
pub fn select_tab(&mut self, tab: AppTab) { self.active_tab = tab; }
pub fn active_tab(&self) -> &AppTab { &self.active_tab }
```

Add enums (new file `crates/harvester_core/src/tabs.rs` or inline in `state.rs`):
```rust
#[derive(Clone, PartialEq, Eq)]
pub enum AppTab { Triage, Summary, Briefing, Trends, PromptLab }

#[derive(Clone, PartialEq, Eq, Default)]
pub enum TrendCategory { #[default] Companies, Technologies, Products, Themes }
```

**File:** `crates/harvester_core/src/update.rs`

Handle `Msg::TabSelected { tab }`:
- Call `state.select_tab(tab)`.
- If `tab == AppTab::Trends` and `state.entity_trend_data().is_none()`:
  emit `Effect::LoadEntityIndex` (no-op stub in Slice 1 EffectRunner — see 1.6).
- If `tab == AppTab::PromptLab`:
  reuse existing Prompt Lab setup behavior without the old left-panel `PromptLabOpenRequested` path.
  Map `Msg::PromptLabOpenRequested` → `Msg::TabSelected { tab: PromptLab }` internally (keep
  existing message for backward compatibility but reroute it).

Handle `Msg::TrendCategorySelected { category }`:
- Call `state.set_active_trend_category(category)`.
- Pure state update; no effects.

### 1.2 — View model

**File:** `crates/harvester_core/src/view_model.rs`

Add `RightPaneView` struct:
```rust
pub struct RightPaneView {
    pub active_tab: AppTab,           // pub in view DTO is fine
    pub triage_markdown: Option<String>,
    pub summary_markdown: Option<String>,
    pub briefing_markdown: Option<String>,
    pub trends_placeholder: String,   // e.g. "Trends data loading…" or counts table
    pub prompt_lab: PromptLabView,    // existing field, move here if not already
}
```

Populate `triage_markdown` from the selected job's triage result (format category, priority, tags,
rationale as a clean markdown block). Use existing triage result access helpers in `state.rs`.

Populate `summary_markdown` from the selected job's summary (title + summary text + key_points).
This replaces the current `preview_text` path for the Summary tab.

Populate `briefing_markdown` from the current briefing output (same as existing `briefing_preview`
path).

Keep `preview_text` and `briefing_preview` alive on `AppViewModel` during this slice to avoid
breaking other render paths; they will be removed in a later cleanup pass.

### 1.3 — UI constants

**File:** `crates/harvester_app/src/platform/ui/constants.rs`

Add (IDs in the 2200 range to avoid collision with existing Prompt Lab 2100 range):
```
PANEL_TAB_BAR          = 2200
BUTTON_TAB_TRIAGE      = 2201
BUTTON_TAB_SUMMARY     = 2202
BUTTON_TAB_BRIEFING    = 2203
BUTTON_TAB_TRENDS      = 2204
BUTTON_TAB_PROMPT_LAB  = 2205
PANEL_TAB_TRIAGE       = 2210
PANEL_TAB_SUMMARY      = 2211
PANEL_TAB_BRIEFING     = 2212
PANEL_TAB_TRENDS       = 2213
PANEL_TAB_PROMPT_LAB   = 2214
```

### 1.4 — Layout

**File:** `crates/harvester_app/src/platform/ui/layout.rs`

Replace the current `PANEL_PREVIEW` single-panel block with:

- `PANEL_TAB_BAR`: fixed height ~28px, docked top of the right region.
- `PANEL_TAB_*`: fill the remainder; all panels are created at startup. Inactive tab panels are
  collapsed via `fixed_size: Some(0)`; the active tab panel fills the remaining space.
  CommanDuctUI has no `SetControlVisible` command; use layout collapse throughout (not
  visibility toggling).
- Tab bar contains five `RadioButton`-style buttons (same pattern as Prompt Lab mode row).

Remove:
- `PANEL_PROMPT_LAB` size contribution from the left panel height calculation.
- Prompt Lab panel contents from the left panel layout block.

**Prompt Lab control parenting (important):** The existing Prompt Lab control tree is created in
`initial_commands(...)` with parent IDs fixed at `PANEL_PROMPT_LAB` in the left panel.
In this slice, refactor that creation so the parent chain originates under `PANEL_TAB_PROMPT_LAB`
instead. Keep all existing Prompt Lab control IDs unchanged to preserve event mapping. Do not
create a second control tree; reparent at creation time only.

### 1.5 — Render

**File:** `crates/harvester_app/src/platform/ui/render.rs`

Render tab bar:
- Create five `RadioButton` controls in `PANEL_TAB_BAR`.
- Set checked state of the button matching `vm.right_pane.active_tab`.

Render tab content:
- Collapse inactive `PANEL_TAB_*` panels via `fixed_size: Some(0)`; set active panel to fill
  available height.
- `PANEL_TAB_SUMMARY`: write `vm.right_pane.summary_markdown` to `VIEWER_PREVIEW` (or a new
  RichEdit inside the tab panel); same RTF conversion as today.
- `PANEL_TAB_TRIAGE`: write `vm.right_pane.triage_markdown` to a RichEdit in the triage tab panel.
- `PANEL_TAB_BRIEFING`: write `vm.right_pane.briefing_markdown` as today.
- `PANEL_TAB_TRENDS`: write placeholder text / table for now.
- `PANEL_TAB_PROMPT_LAB`: render existing Prompt Lab sections here; remove from left-panel render.

Remove `prompt_lab_preview_override(...)` call from the shared preview section.

### 1.6 — Effect runner: stub arms for new Effects

**File:** `crates/harvester_io/src/effect_runner.rs`

`EffectRunner::execute_effect` is exhaustive over `Effect`. Adding new `Effect` variants in Slice 1
requires immediately adding match arms or the crate will not compile.

Add no-op/stub arms in Slice 1 for every new `Effect` variant declared:
```rust
Effect::LoadEntityIndex => { /* full handling in Slice 3 */ }
Effect::RebuildEntityIndex => { /* full handling in Slice 4 */ }
Effect::UpsertEntityIndexEntry { .. } => { /* full handling in Slice 3 */ }
```

These stubs are replaced by real implementations in later slices.

### 1.7 — Event mapping

**File:** `crates/harvester_app/src/platform/app.rs`

Map radio button click events for `BUTTON_TAB_*` → `Msg::TabSelected { tab }`.

Map `Msg::PromptLabOpenRequested` and `Msg::PromptLabCloseRequested` to
`Msg::TabSelected { tab: PromptLab }` / `Msg::TabSelected { tab: Summary }` to preserve
backward compatibility while the caller sites are migrated.

### 1.8 — Tests

- Reducer: `TabSelected` updates `active_tab` and emits no unexpected effects in non-Trends cases.
- Reducer: `TabSelected { Trends }` emits `Effect::LoadEntityIndex` when trend data is absent.
- Reducer: `TabSelected { PromptLab }` works without error when PromptLab state is uninitialized.
- Reducer: `PromptLabOpenRequested` bridge calls `select_tab(PromptLab)` and preserves any existing
  PromptLab side effects (e.g. context editor initialization).
- Reducer: `PromptLabCloseRequested` bridge selects a deterministic fallback tab (document which
  tab is chosen as fallback and assert it in the test).
- Layout/render: tab bar panels created; active tab panel visible (non-zero size), others collapsed.
- Layout/render: switching active tab causes inactive panels to collapse via `fixed_size: Some(0)`,
  not via a visibility command.
- Regression: Prompt Lab open/close still works end-to-end without hijacking preview content.

---

## Slice 2 — Summary V4: Entity Extraction and Cache Compatibility

**Goal:** The summary LLM extracts structured entity lists. Old cached summaries still load cleanly.

### 2.1 — Engine DTO

**File:** `crates/harvester_engine/src/llm/dto.rs`

Add:
```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SummaryEntities {
    pub companies: Vec<String>,
    pub technologies: Vec<String>,
    pub products: Vec<String>,
}
```

Extend `ArticleSummary` with the `entities` field only:
```rust
pub struct ArticleSummary {
    pub title: String,
    pub summary: String,
    pub key_points: Vec<String>,
    pub entities: SummaryEntities,   // new; only addition to engine DTO
}
```

Token fields (`input_tokens`, `output_tokens`) remain in
`harvester_core::briefing::ArticleSummaryResult` only. Do not add them to the engine DTO — they
come from `LlmCompleted` metadata and are added by `harvester_core` when building the result.

### 2.2 — Validation

**File:** `crates/harvester_engine/src/llm/validation.rs`

Extend `validate_summary()`:
- `entities.companies`, `.technologies`, `.products`: max 15 items each.
- Each item: max 100 chars, non-empty string, no internal newlines.
- Deduplicate within each list (case-insensitive; keep first occurrence).
- Return validation error if the `entities` key is present but malformed.
- Accept missing `entities` key (backward compatible with V3 responses — treat as empty).

### 2.3 — Prompt V4

**File:** `crates/harvester_engine/src/llm/prompts/summary.rs`

Add `ArticleSummary` prompt `V4`. Key additions over V3:

Instructions added to the extraction rules section:
```
Extract a structured entity list from the article:
- "companies": named legal organizations mentioned (corporations, government bodies, non-profits).
  Normalize to one canonical display name per entity (prefer the most complete form, e.g.
  "Nvidia" not "NVDA"; "Microsoft" not "MSFT"). Omit if none are clearly named.
- "technologies": named technical concepts, platforms, or methods that are category-level terms
  (e.g. "large language models", "data clean rooms", "custom silicon"). Not brand product names.
- "products": named branded products or software platforms from a specific vendor
  (e.g. "H100", "Cortex XSIAM", "Azure Copilot"). Not generic category names.
Return empty arrays for categories with no clear members.
Do not hallucinate entities not present in the article text.
```

JSON output schema (append after existing fields):
```json
"entities": {
  "companies": ["string", ...],
  "technologies": ["string", ...],
  "products": ["string", ...]
}
```

### 2.4 — Prompt registry

**File:** `crates/harvester_engine/src/llm/prompts/mod.rs`

- Register `ArticleSummaryV4`.
- Set default active `ArticleSummary` prompt to V4.
- Update any test that asserts the current active version is V3.

### 2.5 — Core domain: `ArticleSummaryResult`

**File:** `crates/harvester_core/src/briefing.rs`

Extend `ArticleSummaryResult` (or equivalent result type holding the validated output):
```rust
pub entities: SummaryEntities,
```

Token counts (`input_tokens`, `output_tokens`) remain in `ArticleSummaryResult` as today — they are
not moved to the engine DTO.

### 2.6 — Persistence compatibility

**File:** `crates/harvester_io/src/summary_cache_store.rs`

The persisted DTO must load old entries (no `entities` field) without error.

In the IO DTO:
```rust
#[serde(default)]
pub entities: SummaryEntitiesDto,
```

`SummaryEntitiesDto` defaults to empty arrays. Conversion from IO DTO to core type just copies the
(possibly empty) vectors.

The existing persisted cache `version` field already handles schema evolution. No version bump is
required for this additive change: `#[serde(default)]` is sufficient for backward compatibility.
Only consider a version bump if the change is destructive (removing or renaming fields).

### 2.7 — Tests

- `validate_summary` accepts a V4 response with all three entity arrays populated.
- `validate_summary` accepts a V4 response with empty entity arrays.
- `validate_summary` accepts a V3 response (no `entities` key) — treated as empty entities.
- `validate_summary` rejects entity strings exceeding max length.
- `validate_summary` deduplicates repeated entity strings within a category.
- Summary cache store: load a fixture `.ron` file without `entities` field → entities are empty.
- Summary cache store: round-trip with `entities` → entities preserved exactly.
- Prompt V4 JSON output parses correctly into `ArticleSummary`.

---

## Slice 3 — Entity Index: Write Path (Live + Cache Hits)

**Goal:** Every completed summary and triage (fresh or cache hit) upserts the entity index.
The index file is race-safe.

### 3.1 — Entity index types

New file (recommended): `crates/harvester_core/src/entity_index.rs`

```rust
pub struct EntityIndex {
    pub schema_version: u32,
    pub entries: BTreeMap<String, EntityIndexEntry>,  // key = URL
}

pub struct EntityIndexEntry {
    pub fetched_utc: String,       // RFC3339
    pub content_hash: String,      // SHA256; used as cache join key
    pub companies: Vec<String>,
    pub technologies: Vec<String>,
    pub products: Vec<String>,
    pub themes: Vec<String>,       // from triage tags
}
```

`EntityIndex::default()` → schema_version 1, empty map.

### 3.2 — Runtime paths

**File:** `crates/harvester_io/src/runtime_paths.rs`

Add a field following the existing field-based pattern (do not use only an accessor method,
as `RuntimePaths` is used as a field-accessed struct across the codebase and tests):

```rust
pub entity_index_path: PathBuf,
```

Initialize in `RuntimePaths::new(...)` as `output_dir.join(".entity_index.ron")`.

Also update any test that constructs a `RuntimePaths` struct literal — add the new field to
all literal constructions.

### 3.3 — IO module

New file: `crates/harvester_io/src/entity_index_store.rs`

Functions:
- `load_entity_index(path) -> Result<EntityIndex>` — deserialize RON; return default on missing file.
- `save_entity_index(path, index) -> Result<()>` — serialize via `AtomicFileWriter` (existing utility).
- `upsert_entry(index: &mut EntityIndex, url: &str, patch: EntityIndexPatch)` — merge patch fields;
  never overwrite a Some field with None; deduplicate entity strings in the patch before storing.

`EntityIndexPatch`:
```rust
pub struct EntityIndexPatch {
    pub fetched_utc: Option<String>,
    pub content_hash: Option<String>,
    pub summary_entities: Option<SummaryEntities>,
    pub themes: Option<Vec<String>>,
}
```

Export from `crates/harvester_io/src/lib.rs`.

### 3.4 — Effects

**File:** `crates/harvester_core/src/effect.rs`

Add:
```rust
Effect::LoadEntityIndex,
Effect::RebuildEntityIndex,
Effect::UpsertEntityIndexEntry {
    url: String,
    fetched_utc: Option<String>,
    content_hash: Option<String>,
    summary_entities: Option<SummaryEntities>,
    themes: Option<Vec<String>>,
},
```

**File:** `crates/harvester_core/src/msg.rs`

Add:
```rust
Msg::EntityIndexLoaded { index: EntityIndex },
Msg::EntityIndexLoadFailed { reason: String },
Msg::EntityIndexRebuilt { index: EntityIndex },
Msg::EntityIndexRebuildFailed { reason: String },
```

### 3.5 — Effect runner: serialized persistence lane

**File:** `crates/harvester_io/src/effect_runner.rs`

`UpsertEntityIndexEntry` must **not** execute concurrently with other `UpsertEntityIndexEntry`
calls. All other effect families may remain concurrent.

Implementation: route `UpsertEntityIndexEntry` through a dedicated single-threaded channel
(a `mpsc` sender → worker thread that processes one upsert at a time, loading → merging → atomically
writing the file per message). This is a small bounded worker; no new external deps needed.

Replace the stub arms added in Slice 1 with real handling:
- `LoadEntityIndex`: read file; send `Msg::EntityIndexLoaded` or `Msg::EntityIndexLoadFailed`.
- `UpsertEntityIndexEntry`: forward to the serialized worker channel.

Worker lifecycle:
- The worker thread must be joined or cleanly abandoned when the `EffectRunner` is dropped.
  Use channel close (drop the sender) to signal shutdown to the worker; document the chosen
  Drop behavior explicitly.
- If the worker thread exits unexpectedly, log an error (`[entity-index] worker died`). Do not
  silently discard future upsert requests.

Logging:
- Use the log category `[entity-index]` for all load/save/upsert/rebuild events and failures.
- Log errors at error level; log normal merge/write at debug level.

Test hook:
- For deterministic testing, expose a `flush_entity_index_queue()` helper on `EffectRunner`
  (behind `#[cfg(test)]` or a test feature flag) that blocks until the worker queue drains.
  Alternatively, accept a completion callback per upsert so tests can await the write.

### 3.6 — Reducer: wire upserts

**File:** `crates/harvester_core/src/update.rs`

**Note on `fetched_utc` in this slice:** `LoadedArticle.fetched_utc` is not added until Slice 4.
In this slice, emit `fetched_utc: None` where the field is not yet available from the completion
context. This is acceptable — trend bucketing requires the index to be loaded (Slice 4), and
rebuild (also Slice 4) will backfill timestamps from article frontmatter.

Emit `Effect::UpsertEntityIndexEntry` for:

1. **Fresh summary completion** (`Msg::LlmCompleted` with `PromptId::ArticleSummary`, cache miss):
   - `summary_entities`: from the validated `ArticleSummary.entities`
   - `fetched_utc`: `None` until Slice 4
   - `content_hash`: from the summary cache key

2. **Fresh triage completion** (`Msg::LlmCompleted` with `PromptId::ArticleTriage`, cache miss):
   - `themes`: from `ArticleTriageResult.tags`
   - `fetched_utc`: `None` until Slice 4
   - `content_hash`: from the triage cache key

3. **Summary cache hit** (path in `dispatch_next_summary_step` that bypasses `LlmCompleted`):
   - Same fields as (1), sourced from the cached result.

4. **Triage cache hit** (path in `dispatch_next_triage_step`):
   - Same fields as (2), sourced from the cached result.

### 3.7 — Tests

- `upsert_entry`: partial patch (summary only) preserves existing themes; themes-only patch
  preserves existing entities.
- `upsert_entry`: idempotent — upsert same patch twice yields same state.
- `upsert_entry`: deduplicates company names within the patch.
- Effect runner: simulate rapid summary + triage completions for the same URL; confirm no lost
  updates (both entities and themes present after both messages process).
- Effect runner: simulate rapid alternating upserts for two different URLs; confirm neither
  URL's data overwrites the other.
- Effect runner: corrupt `.entity_index.ron` at load time emits `Msg::EntityIndexLoadFailed`
  (not a panic); upsert path recovers by starting from a default empty index (or fails
  deterministically with a log — whichever is chosen, assert the behavior explicitly).
- Reducer: fresh summary completion emits `UpsertEntityIndexEntry` with entities.
- Reducer: summary cache hit path emits `UpsertEntityIndexEntry`.
- Reducer: triage cache hit path emits `UpsertEntityIndexEntry` with themes.

---

## Slice 4 — Rebuild, Trend Computation, and Trends Tab

**Goal:** Trends tab shows actual trend data. The entity index can be rebuilt from scratch.

### 4.1 — Extend `LoadedArticle` with `fetched_utc`

**File:** `crates/harvester_engine/src/briefing.rs`

Extend `LoadedArticle` (or the article metadata DTO used in archive scans):
```rust
pub fetched_utc: Option<String>,   // from frontmatter; None if missing/unparseable
```

Populate from the `fetched_utc` frontmatter field during archive scanning.
Log a warning (not error) for missing/unparseable values; treat as `None` and include the article.

This unblocks Slice 3 upserts from being able to supply timestamps: after Slice 4 ships, the
reducer can source `fetched_utc` from `LoadedArticle` rather than emitting `None`.

Also export a new public engine API for use by the rebuild effect (see 4.2):

```rust
// harvester_engine/src/briefing.rs (or a new pub module)
pub struct ArchiveArticleMeta {
    pub url: String,
    pub fetched_utc: Option<String>,  // from frontmatter; None if absent
    pub content_hash: Option<String>, // if derivable at scan time; None otherwise
}

pub fn scan_archive_article_metadata(output_dir: &Path) -> Result<Vec<ArchiveArticleMeta>>
```

Export `scan_archive_article_metadata` and `ArchiveArticleMeta` from `harvester_engine/src/lib.rs`.
This wraps the private `scan_and_prepare_articles` helper so `harvester_io` can call it without
accessing the private internals.

### 4.2 — Rebuild effect implementation

**File:** `crates/harvester_io/src/effect_runner.rs` (or a new `rebuild_entity_index.rs` helper)

Replace the stub `RebuildEntityIndex` arm from Slice 1 with real implementation.

`RebuildEntityIndex` procedure:
1. Scan markdown archive via `harvester_engine::scan_archive_article_metadata(output_dir)`;
   collect `Vec<ArchiveArticleMeta>`.
2. Load triage cache (via existing `triage_cache_store`): build a `content_hash → tags` map.
3. Load summary cache (via existing `summary_cache_store`): build a `content_hash → entities` map
   (only entries with V4+ summaries will have entities; older entries → empty).
4. For each URL from step 1: build an `EntityIndexEntry` by joining step 2 + 3 on `content_hash`.
   Fields with no match → empty vec (acceptable; index fills in as new articles are processed).
5. Write resulting `EntityIndex` atomically.
6. Send `Msg::EntityIndexRebuilt { index }`.

### 4.3 — Pure trend module

New file: `crates/harvester_core/src/trends.rs`

```rust
pub struct EntityTrendData {
    pub companies: CategoryTrend,
    pub technologies: CategoryTrend,
    pub products: CategoryTrend,
    pub themes: CategoryTrend,
}

pub struct CategoryTrend {
    pub weeks: Vec<IsoWeek>,              // sorted, 13 entries for ~3 months
    pub top_entities: Vec<EntityLine>,    // top N by total count
    pub total_entity_count: usize,        // full population size ("N of M" label)
}

pub struct EntityLine {
    pub display_label: String,            // canonical display form
    pub weekly_counts: Vec<u32>,          // parallel to CategoryTrend::weeks
    pub total_count: u32,
}

pub struct IsoWeek {
    pub week_start: NaiveDate,            // Monday UTC
    pub label: String,                    // display string, e.g. "Feb 3"
}
```

Public functions:
- `compute_trends(index: &EntityIndex, window_weeks: u32, top_n: usize) -> EntityTrendData`
- `normalize_entity_key(s: &str) -> String`  — trim, collapse whitespace, lowercase
- `choose_display_label(occurrences: &[(String, u32)]) -> String`  — most frequent; tie → lexical

Counting rules (enforced in `compute_trends`):
- Count at most one mention per entity per article per category (presence, not frequency).
- Use `normalize_entity_key` for grouping; record original form for display label election.
- Bucket by ISO week start (Monday UTC). Articles with unparseable `fetched_utc` are skipped.
- Top-N tie-breaking: total count desc → latest-week count desc → display label asc.
- `window_weeks` default = 13.

### 4.4 — AppState: trend fields and accessor methods

**File:** `crates/harvester_core/src/state.rs`

Add private fields (keep private; do not expose as pub fields on reducer-owned state):
```rust
entity_index: Option<EntityIndex>,
entity_trend_data: Option<EntityTrendData>,
active_trend_category: TrendCategory,
```

Expose via methods on `AppState`:
```rust
pub fn set_entity_index(&mut self, index: EntityIndex, window_weeks: u32, top_n: usize) {
    self.entity_trend_data = Some(compute_trends(&index, window_weeks, top_n));
    self.entity_index = Some(index);
}

pub fn set_active_trend_category(&mut self, category: TrendCategory) {
    self.active_trend_category = category;
}

pub fn entity_trend_data(&self) -> Option<&EntityTrendData> { self.entity_trend_data.as_ref() }
pub fn active_trend_category(&self) -> &TrendCategory { &self.active_trend_category }
```

### 4.5 — Reducer: trend state management

**File:** `crates/harvester_core/src/update.rs`

Handle `Msg::EntityIndexLoaded { index }`:
- Call `state.set_entity_index(index, 13, 10)`.

Handle `Msg::EntityIndexRebuilt { index }`:
- Call `state.set_entity_index(index, 13, 10)`.
- Optionally emit a UI notification if desired.

Handle `Msg::EntityIndexLoadFailed { reason }`:
- Log the reason.
- Emit `Effect::RebuildEntityIndex` to attempt recovery.

Handle `Msg::TrendCategorySelected { category }`:
- Call `state.set_active_trend_category(category)`.
- Pure; no effects.

### 4.6 — View model

**File:** `crates/harvester_core/src/view_model.rs`

Add to `RightPaneView`:
```rust
pub trends: TrendsTabView,
```

```rust
pub struct TrendsTabView {
    pub is_loading: bool,
    pub active_category: TrendCategory,
    pub category_data: Option<CategoryTrendView>,
}

pub struct CategoryTrendView {
    pub weeks: Vec<String>,               // display labels
    pub lines: Vec<EntityLineView>,       // top N
    pub total_entity_count: usize,
}

pub struct EntityLineView {
    pub label: String,
    pub weekly_counts: Vec<u32>,
    pub total_count: u32,
}
```

Populate from `state.entity_trend_data()` and `state.active_trend_category()`.

### 4.7 — Trends tab render (text/table for Slice 4)

**File:** `crates/harvester_app/src/platform/ui/render.rs`

Render `PANEL_TAB_TRENDS` as a RichEdit with a formatted text table:

```
[Companies]  [Technologies]  [Products]  [Themes]

Top 10 Companies — last 13 weeks

  Nvidia        ▐▌▌▌▌▐▌▌▌▌▌▌▌  total: 47
  TSMC          ▐▌▌▌▌▐▌▌▌▌       total: 31
  ...

  Showing top 10 of 53 entities.
```

Unicode block characters give rough visual bars (proportional to weekly max).
Category selector rendered as RadioButton row above the RichEdit.

Map radio button events for category selector → `Msg::TrendCategorySelected { category }`.

### 4.8 — Tests

- `compute_trends`: buckets articles correctly across ISO week boundaries.
- `compute_trends`: handles month/year boundary weeks (e.g. Dec 29 → Jan 4 ISO week).
- `compute_trends`: top-N deterministic tie-breaking (same counts → lexical order).
- `compute_trends`: articles with missing `fetched_utc` are skipped; no panic.
- `normalize_entity_key`: trims and lowercases; collapses internal whitespace.
- `choose_display_label`: returns most-frequent form; ties resolved lexically.
- Rebuild: synthetic markdown + triage cache → expected entity index entries.
- Rebuild: missing triage cache → entries with empty themes (no panic).
- Rebuild: two URLs with the same normalized URL string produce two separate entries
  (URL string is used as the exact index key, no normalization at index level — document
  and test the key policy explicitly).
- Rebuild: `content_hash` mismatch between archive metadata and summary/triage caches produces
  a partial entry (empty entities or themes) rather than a panic.
- Reducer: `EntityIndexLoaded` → `entity_trend_data` populated; `entity_trend_data.companies.weeks`
  has 13 entries.
- Reducer: `EntityIndexLoadFailed` → `Effect::RebuildEntityIndex` emitted.
- Reducer: `TrendCategorySelected` → `active_trend_category` updated, no effects emitted.

---

## Slice 5 — CommanDuctUI Chart Control (v1.1)

**Goal:** Replace the text/table trend view with a GDI+ line chart.
This slice is a framework extension; plan it as a separate implementation session.

### 5.1 — Control type

**File:** `src/CommanDuctUI/src/types.rs`

Add `ControlKind::Chart`.
Add platform commands:
- `PlatformCommand::CreateChart { id, parent }` — creates an owner-draw HWND.
- `PlatformCommand::SetChartData { id, data: ChartData }` — sends data to the control.
- `PlatformCommand::SetChartColors { id, colors: Vec<u32> }`.

Add `AppEvent::ChartHovered { id, entity: String, week: String, count: u32 }` (optional, deferred).

### 5.2 — Chart control handler

New file: `src/CommanDuctUI/src/controls/chart_handler.rs`

WM_PAINT implementation (GDI+):
1. Fill background with theme dark color.
2. Draw Y-axis: auto-scale to max weekly count in visible set; label N evenly-spaced Y gridlines.
3. Draw horizontal gridlines (dashed, `#3A3F47`).
4. Draw X-axis: week start date labels at the bottom.
5. For each `EntityLine` in top-10:
   - Compute pixel positions for each data point (affine transform: week index → X, count → Y).
   - `GdipDrawLines` with the entity's assigned color.
6. Draw legend below chart: colored dot + label + "(total)".

Optional WM_MOUSEMOVE hit-testing for tooltip:
- Check within ±5px of each data point.
- Show a `TrackMouseEvent`-style tooltip balloon.
- Wire to `AppEvent::ChartHovered`.

### 5.3 — Paint routing

**File:** `src/CommanDuctUI/src/controls/paint_router.rs`

Register chart HWND in the paint strategy map
(relevant to `FI-Architecture-UiFramework-0008`).

### 5.4 — App integration

**File:** `crates/harvester_app/src/platform/ui/render.rs`

Replace the RichEdit text table in `PANEL_TAB_TRENDS` with:
- `PlatformCommand::CreateChart { id: CHART_TRENDS, parent: PANEL_TAB_TRENDS }`.
- `PlatformCommand::SetChartData { id: CHART_TRENDS, data: build_chart_data(&vm.right_pane.trends) }`.
- Category selector RadioButton row remains unchanged.

### 5.5 — Tests

- Chart control creation: `CreateChart` command produces a valid HWND; no panic.
- `SetChartData` with empty series: no paint crash.
- `SetChartData` with 10 series × 13 weeks: no paint crash.
- Paint routing: chart HWND is registered and routed correctly.
- Hit-test geometry helpers (pure functions, no Win32 needed): given a point and a set of data
  positions, returns the nearest data point within radius or `None`.

### 5.6 — Submodule version and changelog

If `src/CommanDuctUI` API changes in this slice:
- Bump the crate version in `src/CommanDuctUI/Cargo.toml`.
- Update the CommanDuctUI changelog with the new control types and platform commands.
- Mark any breaking changes (new `ControlKind` variants, new `AppEvent` variants) prominently
  so downstream crates know to update their exhaustive matches.

---

## Files Changed by Slice

| File | Slices |
|---|---|
| `crates/harvester_core/src/msg.rs` | 1, 3 |
| `crates/harvester_core/src/state.rs` | 1, 4 |
| `crates/harvester_core/src/update.rs` | 1, 3, 4 |
| `crates/harvester_core/src/view_model.rs` | 1, 4 |
| `crates/harvester_core/src/tabs.rs` *(new)* | 1 |
| `crates/harvester_core/src/entity_index.rs` *(new)* | 3 |
| `crates/harvester_core/src/trends.rs` *(new)* | 4 |
| `crates/harvester_app/src/platform/ui/constants.rs` | 1 |
| `crates/harvester_app/src/platform/ui/layout.rs` | 1 |
| `crates/harvester_app/src/platform/ui/render.rs` | 1, 4, 5 |
| `crates/harvester_app/src/platform/app.rs` | 1 |
| `crates/harvester_engine/src/llm/dto.rs` | 2 |
| `crates/harvester_engine/src/llm/validation.rs` | 2 |
| `crates/harvester_engine/src/llm/prompts/summary.rs` | 2 |
| `crates/harvester_engine/src/llm/prompts/mod.rs` | 2 |
| `crates/harvester_engine/src/briefing.rs` | 4 |
| `crates/harvester_engine/src/lib.rs` | 4 (export `scan_archive_article_metadata`) |
| `crates/harvester_io/src/runtime_paths.rs` | 3 |
| `crates/harvester_io/src/lib.rs` | 3 |
| `crates/harvester_io/src/effect_runner.rs` | 1 (stubs), 3, 4 |
| `crates/harvester_io/src/entity_index_store.rs` *(new)* | 3 |
| `crates/harvester_io/src/summary_cache_store.rs` | 2 |
| `crates/harvester_io/src/triage_cache_store.rs` | 4 (rebuild join) |
| `src/CommanDuctUI/src/types.rs` | 5 |
| `src/CommanDuctUI/src/controls/chart_handler.rs` *(new)* | 5 |
| `src/CommanDuctUI/src/controls/paint_router.rs` | 5 |

---

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Summary cache schema change drops existing entries | `#[serde(default)]` on entities; explicit backward-compat fixture test |
| Entity index upsert race under concurrent effects | Dedicated serialized worker lane; concurrent update simulation test |
| Rebuild misses themes (not in markdown frontmatter) | Join with triage cache; partial entries (empty themes) are valid and fill in later |
| Prompt Lab relocation breaks existing tests | Keep `PromptLabOpenRequested` ↔ `TabSelected` bridge; reparent control tree at creation time (same IDs); add regression tests for open/close/preview |
| GDI+ chart introduces paint flicker or crashes | Start with static repaint; no animation; test `SetChartData` with zero/max data first |
| `fetched_utc` absent in many older articles | Treat as `None`; skip article in trend bucketing; log warning; no panic |
| Slice 1 `Effect` variants break `EffectRunner` compile | Add stub arms in Slice 1 effect_runner.rs for all new `Effect` variants |

---

## Verification Checklist

### Slice 1
- [ ] App launches; tab bar visible with 5 tabs.
- [ ] Clicking each tab shows correct content.
- [ ] Inactive tab panels collapse to zero size (not hidden via visibility command).
- [ ] Prompt Lab opens in the Prompt Lab tab; does not hijack preview content.
- [ ] Selecting a job updates Triage and Summary tabs.
- [ ] Generate Briefing → Briefing tab shows updated output.
- [ ] Trends tab shows placeholder text.
- [ ] `cargo test` passes.

### Slice 2
- [ ] Process one new article; inspect `ArticleSummaryResult.entities`.
- [ ] Old summary cache loads without error; entities are empty.
- [ ] New summary cache round-trips entities correctly.
- [ ] `cargo test` passes.

### Slice 3
- [ ] Process one article; inspect `.entity_index.ron` to confirm entry written.
- [ ] Process the same article twice; inspect index for idempotency.
- [ ] Process an article from cache hit; confirm index is still updated.
- [ ] Triage-only path writes `themes` field to the entry.
- [ ] `cargo test` passes.

### Slice 4
- [ ] Delete `.entity_index.ron`; open Trends tab; confirm rebuild completes and data is shown.
- [ ] With index present; open Trends tab; data appears without rebuild.
- [ ] Switch between Companies / Technologies / Products / Themes; content changes.
- [ ] Entities with unparseable timestamps do not crash the tab.
- [ ] `cargo test` passes.

### Slice 5
- [ ] Trends tab shows a line chart instead of the text table.
- [ ] Lines are colored distinctly; legend is visible.
- [ ] Category switch updates the chart.
- [ ] Resizing the window repaints the chart without flicker.
- [ ] (Optional) Hover over a data point shows a tooltip.
- [ ] CommanDuctUI crate version bumped; changelog updated.
- [ ] `cargo test` passes.

---

## Future Ideas and Extensions

### Near-term (v1.1)

- **Time window toggles:** `[4 weeks] [13 weeks] [26 weeks] [1 year]` above the chart.
  Wire to `Msg::TrendWindowChanged { weeks: u32 }` and recompute `entity_trend_data`.
- **Trend velocity indicators:** Week-over-week change arrows (▲/▼) next to entity names in the
  legend. Computed as `(last_week_count - prev_week_count)` with a neutral band around ±1.
- **Source attribution drilldown:** Click a data point on the chart → dispatch
  `Msg::TrendDrilldownRequested { entity, week }` → filter the job list on the left to show only
  articles mentioning that entity in that week's window.

### Medium-term (v1.2)

- **Priority-weighted trend counts:** Weight each mention by the article's triage priority
  (P5 = 5 weight, P1 = 1 weight). Add a toggle: `[Counts] [Weighted]`.
- **Entity alias management:** `contexts/entity_aliases.toml` maps variant spellings to canonical
  names (e.g. `["NVDA", "Nvidia Corporation"] → "Nvidia"`). Applied during upsert normalization.
- **Export trend data:** `File → Export Trends → CSV` writes
  `date,category,entity,count` rows for the current window.
- **Trend snapshot comparison:** "Compare to last period" button overlays the prior 13-week window
  as a dashed line behind the current lines.

### Long-term (v2+)

- **Smoothing modes:** 3-week moving average overlay to reduce noise on noisy categories.
- **New entrant highlight:** Mark entities with zero mentions in all prior weeks with a ⭐ label.
  Useful for catching emerging players before they become obvious.
- **Annotation markers:** Operator can pin a note to a specific week on the chart
  (e.g. "NVIDIA earnings call"). Stored in `output/.trend_annotations.ron`.
- **Cross-entity co-occurrence matrix:** Separate view showing which entities frequently appear
  together in the same article (entity × entity heat-map). Computed from entity index per window.
- **"Surprise score" ranking:** Surface entities whose current-week count is significantly higher
  than their trailing average (z-score based). Alternative top-N sort mode.
- **Multi-device sync:** The entity index is a small RON file; it syncs naturally if the archive is
  managed via a bare git repo (see `Discussion.BriefingAndArchive.md`, Section 6).

### FutureIdeas.md mappings

The following backlog entries become relevant during implementation:

- `FI-Architecture-UiFramework-0007` — typed selection mapping helpers: apply for the tab and
  category RadioButton rows (avoids duplicated index ↔ value offset math).
- `FI-Architecture-UiFramework-0008` — paint routing strategy: apply if/when the chart control
  adds new auxiliary HWNDs.
- `FI-Observability-PreviewRendering-0001 / -0002` — preview truncation telemetry: relevant once
  multiple content surfaces (triage, summary, briefing, trends) exist.
- `FI-Storage-BriefingHistory-0001` — briefing history: architecturally similar to the entity index
  (sidecar, atomic writes, rebuild fallback); patterns can be shared.
- `FI-Architecture-BatchOrchestration-0007` — checkpoint CLI flags: unrelated but same delivery
  wave; consider shipping together with Slice 1 to keep operator tooling current.
