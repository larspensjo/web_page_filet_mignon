# Design: Tab System and Entity Trend Chart

**Date:** 2026-02-24  
**Status:** Reviewed; changes incorporated, ready for implementation planning  
**References:** `crates/harvester_app/src/platform/ui/layout.rs`, `crates/harvester_app/src/platform/ui/render.rs`, `crates/harvester_core/src/state.rs`, `crates/harvester_core/src/update.rs`, `crates/harvester_engine/src/llm/validation.rs`, `docs/FutureIdeas.md`

---

## Draft Diary Entry

Context: The right pane is overloaded and mixes multiple responsibilities (selected-article preview, briefing preview fallback, and Prompt Lab preview override). We need a scalable content-area navigation model and a trend view for archive-level signal detection without breaking UDF boundaries or existing cache/persistence behavior.

Change: Define a reviewed implementation design for right-pane tabs plus entity/theme trend data, including reducer-owned tab/trend state, prompt/schema upgrades for summary entities, a rebuildable sidecar index with race-safe updates, and a staged UI delivery that acknowledges CommanDuctUI chart/event-framework blockers.

---

## Baseline Checked Against Current Source

- The app still has a single right-side preview container (`PANEL_PREVIEW`) with one header label + one RichEdit (`VIEWER_PREVIEW`) in `crates/harvester_app/src/platform/ui/layout.rs`.
- Prompt Lab is currently rendered inside the left input panel (`PANEL_PROMPT_LAB`) and also overrides the right preview content when a Prompt Lab run completes (`prompt_lab_preview_override(...)` in `crates/harvester_app/src/platform/ui/render.rs`).
- `AppViewModel` exposes a single `preview_text`, `preview_header`, and `briefing_preview`; there is no tab model yet (`crates/harvester_core/src/view_model.rs`, `crates/harvester_core/src/state.rs`).
- `harvester_core::Msg` / `Effect` contain no tab or trend messages/effects today (`crates/harvester_core/src/msg.rs`, `crates/harvester_core/src/effect.rs`).
- Summary DTO/validation currently support only `title`, `summary`, `key_points` (`crates/harvester_engine/src/llm/dto.rs`, `crates/harvester_engine/src/llm/validation.rs`).
- Active summary prompt is V3, not V4 (`crates/harvester_engine/src/llm/prompts/mod.rs`).
- Summary and triage caches are persisted with custom IO DTOs (`crates/harvester_io/src/summary_cache_store.rs`, `crates/harvester_io/src/triage_cache_store.rs`) and will need explicit schema updates if `ArticleSummaryResult` changes.
- `LoadedArticle` does not carry `fetched_utc` today (`crates/harvester_engine/src/briefing.rs`), which matters for trend indexing.
- CommanDuctUI currently has no generic chart control command/event surface and no mouse-move `AppEvent` for hover tooltips (`src/CommanDuctUI/src/types.rs`).

---

## Design Corrections From Review

1. `active_tab` must be reducer-owned state (preferably in `UiState` / `AppState`), not only an `AppViewModel` field.
2. `Msg::TrendTabOpened` is optional/redundant.
   Prefer `Msg::TabSelected { tab }` and let the reducer emit load/rebuild effects on transition to `Trends`.
3. Entity-index updates must be race-safe.
   Separate `UpdateEntityIndex` and `UpdateEntityIndexThemes` effects can lose data under concurrent summary/triage completions because the current effect runner spawns threads per effect.
4. Rebuild fallback cannot recover themes from markdown frontmatter alone.
   Triage tags are not stored in article markdown today; rebuild must join with persisted triage data (or explicitly accept missing themes).
5. Cache-hit paths must update the entity index too.
   Current triage/summary cache reuse paths bypass LLM completion handlers in `update.rs`.
6. Hover tooltip is a framework blocker for v1 charting.
   CommanDuctUI lacks mouse-move events and a chart control; tooltip support should be deferred or scoped as a framework sub-slice.

---

## Goals (Unchanged)

1. Add right-pane tabs so Triage, Summary, Briefing, Trends, and Prompt Lab can coexist cleanly.
2. Add archive trend visibility for four categories:
   `Companies`, `Technologies`, `Products`, `Themes`.
3. Keep v1 minimal and deterministic:
   fixed ~13-week window, no weighting, no drilldown, no velocity arrows.

---

## Architecture Decisions (Reviewed)

## 1. Right-Pane Tabs (Reducer-Owned)

### State ownership

Add tab state to reducer-owned UI state (single source of truth), then mirror it in the view model:

```rust
enum AppTab {
    Triage,
    Summary,
    Briefing,
    Trends,
    PromptLab,
}
```

Recommended placement:

- `harvester_core::state::UiState` owns `active_tab`
- `AppViewModel` exposes `active_tab` read-only for rendering

This avoids view-only state drift and keeps tab changes traceable (`Msg -> update -> state -> render`).

### Message model

Use one explicit message:

```rust
Msg::TabSelected { tab: AppTab }
```

Reducer behavior:

- updates `active_tab`
- if transitioning into `Trends` and trend data is missing/stale, emits effect(s) to load/rebuild index
- if selecting `PromptLab`, optionally also emits/follows existing Prompt Lab setup behavior (context editor initialization) without using the old left-panel visibility semantics

### Prompt Lab migration compatibility

Current Prompt Lab state (`prompt_lab.visible`) is tied to left-panel layout and preview override logic. Do not directly reuse it as the new tab source of truth.

Recommended transition strategy:

1. Introduce `active_tab` first.
2. Keep `PromptLabOpenRequested/CloseRequested` temporarily, but redefine them to map to tab selection.
3. Remove `prompt_lab_preview_override(...)` once Prompt Lab has its own tab content.
4. Later, simplify `PromptLabState.visible` if it becomes redundant.

This reduces regressions in existing Prompt Lab tests while migrating UI composition.

## 2. Tab Content View Model (Avoid Single Preview Overloading)

The current `AppViewModel` is preview-centric (`preview_text`, `briefing_preview`, `preview_header`) and the renderer decides some content switching. Tabs will become simpler and more testable if core exposes explicit tab payloads.

Recommended shape (conceptual):

```rust
pub struct RightPaneView {
    pub active_tab: AppTab,
    pub triage_markdown: Option<String>,
    pub summary_markdown: Option<String>,
    pub briefing_markdown: Option<String>,
    pub trends: TrendsTabView,
    pub prompt_lab: PromptLabView,
}
```

This keeps formatting/selection logic in `harvester_core` and makes `harvester_app` render code mostly mechanical.

## 3. Entity Extraction (Summary Prompt V4) and Validation

### Summary schema extension

Extend summary DTOs with bounded, validated entity lists:

```rust
pub struct SummaryEntities {
    pub companies: Vec<String>,
    pub technologies: Vec<String>,
    pub products: Vec<String>,
}
```

Review recommendations:

- Add strict validation limits in `validate_summary(...)` for:
  - max items per category
  - max string length per entity
  - string type checks
- Deduplicate within each category during validation or immediately after validation.
- Keep empty arrays valid.

### Cache/persistence impact (important)

`ArticleSummaryResult` is persisted in summary cache via custom IO DTOs. Adding `entities` requires:

- `crates/harvester_core/src/briefing.rs` (`ArticleSummaryResult`)
- `crates/harvester_io/src/summary_cache_store.rs` persisted DTO schema changes
- backward-compatible loading (`#[serde(default)]`-style behavior in persisted DTO conversion)
- tests for old-cache compatibility

### Active prompt version update

When V4 is added:

- register V4 in `crates/harvester_engine/src/llm/prompts/mod.rs`
- set active `ArticleSummary` prompt to V4
- update tests that assert active/default prompt versions

## 4. Entity Index Persistence (Race-Safe, Rebuildable Sidecar)

### Sidecar format

`output/.entity_index.ron` is a good fit, but the update mechanism must prevent lost updates.

### Blocker: current proposed split effects can race

Current effect runner spawns threads for persistence effects. If summary and triage completions write the same entry concurrently, one write can overwrite the other.

Do not ship v1 with unsynchronized read-modify-write effects.

### Recommended write design

Option A (preferred for v1): single upsert effect with patch merge semantics

```rust
Effect::UpsertEntityIndexEntry {
    url: String,
    fetched_utc: Option<String>,
    content_hash: Option<String>,
    summary_entities: Option<SummaryEntities>,
    themes: Option<Vec<String>>,
}
```

Effect runner behavior:

- load latest index
- merge per-entry fields (only overwrite fields present in the patch)
- write atomically
- serialize these updates through one worker/mutex per runner
- this is an explicit `EffectRunner` behavioral constraint for the `EntityIndex` effect family:
  `UpsertEntityIndexEntry` must not execute concurrently with another `UpsertEntityIndexEntry`
  in the same process/runner

Option B: reducer-owned in-memory entity index + `Effect::PersistEntityIndex { snapshot }`

- simpler race model
- more reducer state churn
- requires load/rebuild first and consistent update sequencing

Either is valid. Option A is less invasive to existing state and matches current cache persistence patterns.

### Cache-hit coverage (must include)

Index updates must occur for:

- fresh summary completions
- fresh triage completions
- summary cache hits
- triage cache hits

Current cache hit paths in `dispatch_next_summary_step(...)` and `dispatch_next_triage_step(...)` complete articles without going through `Msg::LlmCompleted`; they will otherwise be invisible to the entity index.

## 5. Rebuild Strategy (Corrected)

### Original claim that needs correction

Rebuilding from markdown frontmatter alone cannot restore themes, because triage tags are not stored in markdown frontmatter today.

### Recommended rebuild strategy

Rebuild should use a join strategy:

1. Scan markdown archive to obtain:
   - URL
   - `fetched_utc`
   - content hash (derived from markdown via existing content-prep path or a lighter hash helper)
2. Join with triage cache by `content_hash` to recover `tags` (themes), when available.
3. Join with summary cache by `content_hash` to recover entities, but only after summary cache schema includes entities (V4+ cached entries).
4. Emit entries with partial fields when data is missing.

This preserves the “sidecar is rebuildable” principle while acknowledging current storage reality.

### Practical enabler

Strongly consider extending `harvester_engine::briefing::LoadedArticle` (or introducing a sibling archive metadata DTO) to carry `fetched_utc`.

Without `fetched_utc` in loaded article metadata, live upserts and rebuild logic both need extra archive scans/parsing.

### EntityIndex entry shape (add `content_hash`)

Because rebuild and cache-hit update paths join through summary/triage caches by `content_hash`, `EntityIndexEntry` should include `content_hash` from the first implementation.

```rust
pub struct EntityIndexEntry {
    pub fetched_utc: String,   // RFC3339, for weekly bucketing
    pub content_hash: String,  // cache join key (summary_cache / triage_cache)
    pub companies: Vec<String>,
    pub technologies: Vec<String>,
    pub products: Vec<String>,
    pub themes: Vec<String>,
}
```

## 6. Trend Computation (Pure Core Module)

Implement trend math in a new pure module (recommended: `crates/harvester_core/src/trends.rs`) with unit tests.

### Counting rules (clarify in design)

- Count at most one mention per entity per article per category (presence, not repeated mention count).
- Deduplicate entity lists within an entry before bucketing.
- Use UTC dates from `fetched_utc`.
- Bucket by ISO week start (Monday UTC) and store/display bucket start dates explicitly.
- Deterministic tie-breakers for top-N:
  1. total count desc
  2. latest-week count desc (optional)
  3. display label asc

### Normalization (improve beyond lowercase-only)

Lowercasing-only grouping is insufficient for long-term robustness.

Recommended v1 normalizer:

- normalize key for grouping (trim, collapse whitespace, lowercase)
- preserve a display label chosen deterministically (most frequent original form, tie -> lexical)
- keep category-local namespaces (same string can exist in different categories)

Add `normalizer_version` to index schema if normalization rules may evolve.

## 7. Trends UI / Chart Delivery (Staged for Framework Risk)

### Framework blocker (current source)

CommanDuctUI has:

- no `CreateChart` / `SetChartData` platform commands
- no chart control kind
- no mouse move `AppEvent`

This makes hover tooltip and hit-testing a framework feature, not an app-only task.

### Revised delivery recommendation

Split chart work into two sub-slices:

- Slice 3A: Trends tab with category selector + textual/table summary (top entities + weekly counts)
- Slice 3B: Chart control framework work (owner-draw chart + optional hover)

If a line chart is required in v1 anyway, defer hover tooltip to v2 and keep v1 chart static.

### Alternative low-risk v1 visualization

Render trends as a RichEdit/text panel with compact sparklines or aligned weekly columns first. This validates:

- entity extraction
- indexing
- bucketing
- top-N selection

before taking on CommanDuctUI framework changes.

---

## Reviewed Messages and Effects

## Messages

Recommended core additions:

- `Msg::TabSelected { tab: AppTab }`
- `Msg::TrendCategorySelected { category: TrendCategory }`
- `Msg::EntityIndexLoaded { index: EntityIndex }`
- `Msg::EntityIndexLoadFailed { reason: String }`
- `Msg::EntityIndexRebuilt { index: EntityIndex }`
- `Msg::EntityIndexRebuildFailed { reason: String }`

Optional:

- `Msg::EntityIndexRefreshRequested` (if you prefer explicit refresh over tab-transition inference)

## Effects

Recommended additions:

- `Effect::LoadEntityIndex`
- `Effect::RebuildEntityIndex`
- `Effect::UpsertEntityIndexEntry { ... }` (single merge patch effect, race-safe)

If implementing cache-join rebuild:

- `harvester_io` needs access to markdown archive + triage cache + summary cache through `RuntimePaths`.

---

## Revised Delivery Slices

## Slice 1 — Tab shell and Prompt Lab relocation (no trend data yet)

- Add right-pane tab bar + tab panels.
- Move Prompt Lab UI from left panel to Prompt Lab tab.
- Keep left input panel behavior unchanged except Prompt Lab removal.
- Remove Prompt Lab preview override from the shared preview renderer.
- Triage/Summary/Briefing tabs render explicit content (placeholder allowed for gaps).
- Trends tab renders placeholder + status text.

Tests:

- reducer tests for `TabSelected`
- layout/render tests for tab creation, checked states, and active panel sizing
- regression tests confirming Prompt Lab no longer hijacks preview content

## Slice 2 — Summary V4 entities + cache/persistence compatibility

- Add `SummaryEntities` to engine DTO + validation + prompt V4.
- Extend core `ArticleSummaryResult`.
- Update summary cache store schema with backward-compatible load.
- Ensure prompt registry default active summary version becomes V4.

Tests:

- `validate_summary` accepts/rejects entity payloads correctly
- old summary cache file loads with empty entities
- new summary cache round-trip preserves entities

## Slice 3 — Entity index write path (live + cache hit coverage)

- Add `EntityIndex` store and IO functions.
- Add race-safe upsert effect and effect-runner handling.
- Add a dedicated serialized persistence lane in `EffectRunner` for `EntityIndex` upserts
  (worker queue or mutex-guarded merge/write path). Other effect families may remain concurrent.
- Wire updates from:
  - summary completion
  - triage completion
  - summary cache hit
  - triage cache hit

Tests:

- upsert merge preserves fields across partial updates
- repeated updates are idempotent
- concurrent update simulation (or serialized worker invariant test) proving no lost updates under
  rapid summary+triage completions

## Slice 4 — Rebuild + trend computation (pure core)

- Add rebuild effect and archive scan/join logic.
- Add pure trend bucketing/top-N module.
- Load/rebuild on first Trends tab open.
- Render text/table trend view (or chart if framework work is already done).

Tests:

- rebuild from synthetic markdown + caches
- week bucketing across month/year boundaries
- top-N deterministic tie breaking
- malformed timestamps are skipped with warnings

## Slice 5 — CommanDuctUI chart control (optional for v1, recommended v1.1)

- Add chart control commands/events to CommanDuctUI
- Add static line chart rendering first
- Add hover tooltip only after mouse-move event plumbing exists

Tests:

- command routing / control creation
- paint routing / redraw invalidation
- optional hit-test tests (pure geometry helpers)

---

## Files Expected to Change (Reviewed)

Likely additions/changes beyond the original list:

- `crates/harvester_core/src/msg.rs`
- `crates/harvester_core/src/effect.rs`
- `crates/harvester_core/src/state.rs`
- `crates/harvester_core/src/view_model.rs`
- `crates/harvester_core/src/update.rs`
- `crates/harvester_core/src/preview.rs` (tab-specific formatting helpers)
- `crates/harvester_core/src/trends.rs` (new, recommended)
- `crates/harvester_core/src/briefing.rs` (`ArticleSummaryResult`)
- `crates/harvester_app/src/platform/ui/constants.rs`
- `crates/harvester_app/src/platform/ui/layout.rs`
- `crates/harvester_app/src/platform/ui/render.rs`
- `crates/harvester_app/src/platform/app.rs` (map tab radio events to `Msg::TabSelected`)
- `crates/harvester_engine/src/llm/dto.rs`
- `crates/harvester_engine/src/llm/validation.rs`
- `crates/harvester_engine/src/llm/prompts/summary.rs`
- `crates/harvester_engine/src/llm/prompts/mod.rs`
- `crates/harvester_engine/src/briefing.rs` (if `LoadedArticle` or scan helpers are extended)
- `crates/harvester_io/src/runtime_paths.rs` (entity index path)
- `crates/harvester_io/src/lib.rs`
- `crates/harvester_io/src/effect_runner.rs`
- `crates/harvester_io/src/persistence.rs` or new `crates/harvester_io/src/entity_index_store.rs`
- `crates/harvester_io/src/summary_cache_store.rs` (summary entities persistence)
- `crates/harvester_io/src/triage_cache_store.rs` (rebuild join support may reuse/load path)
- `src/CommanDuctUI/src/types.rs` (if chart control is implemented)
- `src/CommanDuctUI/src/app.rs` / `window_common.rs` / new control handler (if chart control is implemented)

---

## Blockers and Risks (Current Source)

1. Chart control + hover tooltip require CommanDuctUI framework work.
   Current command/event surface does not support charts or mouse-move hover.
2. Entity index update race risk is real with the current effect-runner threading model.
   Must serialize/merge writes.
3. Rebuild completeness is limited by current persisted data shape.
   Themes are not reconstructable from markdown frontmatter alone.
4. Summary cache schema change is cross-cutting.
   Missing backward compatibility will silently drop cached summaries or fail loads.
5. Prompt Lab relocation touches established render/layout/test assumptions.
   The current implementation is deeply integrated into left-panel layout and preview override logic.

---

## Verification and Testing Plan (Reviewed)

## Manual verification

1. Tab switching shows correct content and preserves current job selection.
2. Prompt Lab menu action selects the Prompt Lab tab (and no longer toggles a left-panel Prompt Lab container).
3. Summary V4 entity extraction populates entities on new summaries.
4. Cache-hit summaries/triages also update trends data (no “missing trends on cache hits” behavior).
5. Deleting `.entity_index.ron` and opening Trends triggers rebuild and yields usable data/clear partial-data messaging.
6. Trends category switch is instant and deterministic.
7. If chart slice ships: chart renders without flicker and does not require hover to be usable.

## Automated tests (high priority)

1. Reducer tests:
   tab selection updates state and emits trend load effect only on relevant transitions.
2. Reducer tests:
   trend category selection is pure and does not emit IO.
3. Summary validation tests:
   entity arrays validated, bounded, deduplicated.
4. Summary cache store tests:
   backward-compatible load from pre-entity schema.
5. Entity index store tests:
   partial upsert merge, idempotency, schema version handling.
6. Rebuild tests:
   markdown scan + cache join reconstructs expected entries.
7. Trend computation tests:
   bucket boundaries, tie-breaks, malformed timestamp handling.
8. UI layout/render tests:
   tab controls created, checked-state updates, inactive panels collapse.

---

## Future Ideas and Extensions (Post-v1)

- Time-window toggles: `4w / 13w / 26w / 1y`
- Smoothing modes: raw counts vs moving average
- Priority-weighted trends (use triage priority as optional weight)
- Drilldown from trend point to filtered article list
- Entity alias management file (manual canonicalization overrides)
- Export trend dataset to CSV/JSON
- Trend snapshot comparison (this week vs previous 13 weeks)
- Annotation markers (operator notes on major events)

### `docs/FutureIdeas.md` mapping

- No existing entries should be closed by this design review alone (nothing is implemented yet).
- Relevant linked backlog items for implementation:
  - `FI-Architecture-UiFramework-0007` (typed selection mapping helpers; useful for tab/category selectors)
  - `FI-Architecture-UiFramework-0008` (paint routing strategy; useful if chart control is added)
  - `FI-Observability-PreviewRendering-0001` / `-0002` (helpful once multiple preview/trend surfaces exist)
