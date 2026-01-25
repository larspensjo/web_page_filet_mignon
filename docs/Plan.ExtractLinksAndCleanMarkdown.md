# Plan: Extract + Clean Links During Download (and Persist in Saved State)

## Goals

1. **Reduce output Markdown size** by removing bulky URLs (especially share/related links and `<img>` tags).
2. **Extract and keep links in runtime state** for future features (tree expansion, link browsing, "download next").
3. **Persist extracted links in the app's saved state** (`.harvester_state.ron`) so they are restored when loading state from the output folder.
4. **Keep long-term "main text only" in mind** with a clean architecture that can later host additional reducers (boilerplate removal, nav pruning, etc.).

## Key Design Decisions

- **Architecture:** Extend the `Converter` trait to extract links during HTML→Markdown conversion (single pass, no markdown post-processing)
- **File content:** Cleaned markdown (URLs stripped, smaller files)
- **Token counting:** Reflects cleaned markdown (consistent with file content)
- **srcset handling:** Skip srcset, extract only main `src` URL from images
- **Implementation:** Use `scraper` crate (already a dependency) instead of adding `pulldown-cmark`

## Constraints from `Agents.md`

- **Unidirectional Data Flow:** Core state changes only via `Msg` → reducer/update; no IO in reducers.
- **Reducers are pure:** link extraction and file writing stay in the engine/effects layer, not in `harvester_core`.
- **Encapsulation:** keep internal state private; add behavior methods on `JobState` instead of exposing fields.
- **Testing:** add unit tests for post-processing and state/persistence to lock in behavior.
- **Thin module entrypoints:** keep `mod.rs`/`lib.rs` thin; place logic in focused modules.

---

## Target Behavior (MVP)

When a job completes successfully:

1. **Engine converts HTML→Markdown with link extraction**:
   - Uses custom `LinkExtractingConverter` that walks the HTML DOM
   - Extracts URLs from `<a href>` tags, emits only link text to markdown
   - Extracts URLs from `<img src>` tags (skipping srcset), removes images from markdown
   - Resolves relative URLs against base URL
   - Returns both cleaned markdown and extracted links
2. **Engine uses the cleaned Markdown for**:
   - preview text
   - token counting
   - writing the final `.md` file
3. **Extracted links are included in `JobOutcome`** and flow to:
   - runtime job state (`harvester_core`)
   - persisted completed jobs (`.harvester_state.ron`)

Non-goals for MVP:
- Perfect "main content extraction"
- Sophisticated nav/boilerplate pruning (architecture supports this for later)
- srcset URL extraction (memory bloat avoidance)

---

## Design Choices

### A. Where link extraction happens
**In `harvester_engine`** (the IO/effect layer), during HTML→Markdown conversion.

Why single-pass converter approach:
- Keeps `harvester_core` reducer pure and deterministic (UDF compliance)
- More efficient than dual parsing (HTML→MD, then MD parsing)
- Full control over link handling and markdown output format
- `scraper` crate already a dependency (uses `html5ever` for robust HTML parsing)

### B. How links are represented
Two layers:

1. **Engine layer (detailed)** for extraction:
   - `ExtractedLink { url, text, kind }` where `kind` is `Hyperlink | Image | Email`
2. **Core/Persistence (lightweight)** to avoid bloating save files:
   - store **deduped list of URLs** as `Vec<String>`, per completed job

### C. How Markdown is generated (MVP)
The custom converter:
- Walks HTML DOM using `scraper`
- For `<a href>`: emits link text only (no URL)
- For `<img src>`: extracts src URL, omits image from markdown (skips srcset)
- Resolves relative URLs against base URL

This gives maximum size reduction and aligns with "keep only main text".

---

## Data Model Changes

### 1) `harvester_engine` (runtime details)

Add in `crates/harvester_engine/src/links.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkKind {
    Hyperlink,  // <a href>
    Image,      // <img src>
    Email,      // mailto: links
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedLink {
    pub url: String,
    pub text: Option<String>,
    pub kind: LinkKind,
}

pub struct ConversionOutput {
    pub markdown: String,          // Cleaned (no URLs)
    pub links: Vec<ExtractedLink>, // Extracted links
}
```

### 2) Update `Converter` trait

In `crates/harvester_engine/src/convert.rs`:

```rust
pub trait Converter: Send + Sync {
    fn to_markdown(&self, html: &str, base_url: Option<&str>) -> ConversionOutput;
}
```

Implement `LinkExtractingConverter` using `scraper` to walk DOM and extract links while generating markdown.

### 3) Extend `JobOutcome`

In `crates/harvester_engine/src/types.rs`, extend:

```rust
pub struct JobOutcome {
    pub final_url: String,
    pub tokens: Option<u32>,
    pub bytes_written: Option<u64>,
    pub content_preview: Option<String>,
    pub extracted_links: Vec<ExtractedLink>, // NEW
}
```

### 4) `harvester_core` job state

In `crates/harvester_core/src/state.rs` extend the private `JobState`:

- Add `extracted_links: Vec<String>`
- Add methods:
  - `fn set_extracted_links(&mut self, links: Vec<String>)` - dedupes and normalizes
  - `fn extracted_links(&self) -> &[String]` (read-only slice)

Keep fields private; expose only necessary read-only views.

### 5) Persisted snapshots

In `crates/harvester_core/src/state.rs` extend `CompletedJobSnapshot`:

```rust
pub struct CompletedJobSnapshot {
    pub url: String,
    pub tokens: Option<u32>,
    pub bytes: Option<u64>,
    pub links: Vec<String>, // NEW (deduped)
}
```

---

## Implementation Steps (Buildable + Testable After Each Step)

### Phase 1 — Extend Converter Trait (Engine)

#### Step 1. Define link types
- Create `crates/harvester_engine/src/links.rs`:
  - `LinkKind` enum (Hyperlink, Image, Email)
  - `ExtractedLink` struct
  - `ConversionOutput` struct
- Keep module focused and thin

#### Step 2. Update Converter trait
- In `crates/harvester_engine/src/convert.rs`:
  - Change trait signature to return `ConversionOutput`
  - Add `base_url` parameter for relative URL resolution

```rust
pub trait Converter: Send + Sync {
    fn to_markdown(&self, html: &str, base_url: Option<&str>) -> ConversionOutput;
}
```

#### Step 3. Implement LinkExtractingConverter
Using `scraper` crate (already a dependency):
- Parse HTML with `Html::parse_document()`
- Walk DOM nodes recursively
- For each node:
  - `<a href>`: extract URL (resolve if relative), emit text content only
  - `<img src>`: extract src URL (skip srcset), omit from output
  - `<a href="mailto:">`: extract as Email kind
  - Other text/elements: emit as-is to markdown
- Maintain encounter order for determinism
- Enforce 5,000 link limit per job (truncate if exceeded)

#### Step 4. URL normalization helper
Implement URL deduplication/normalization:
- Lowercase scheme and host
- Remove default ports (:80, :443)
- Trim whitespace
- Optionally decode percent-encoding for comparison

#### Step 5. Unit tests (converter)
Add `crates/harvester_engine/tests/converter_links.rs`:

- Anchor tag extraction:
  - input HTML: `<a href="https://x">world</a>`
  - output markdown: `world`
  - links: one entry `{ url=https://x, text=Some("world"), kind: Hyperlink }`
- Image extraction:
  - input HTML: `<img src="/path/to/image.jpg" srcset="...">`
  - output markdown: (image removed)
  - links: one entry `{ url=https://base.url/path/to/image.jpg, kind: Image }`
- Relative URL resolution
- Email links
- Link limit enforcement
- Determinism: same HTML → same output and same link order
- Malformed HTML handling (unclosed tags, invalid nesting)

**Acceptance for Phase 1:** converter works; tests pass; no pipeline integration yet.

---

### Phase 2 — Update Engine Pipeline

#### Step 7. Update engine.rs to use new converter
In `crates/harvester_engine/src/engine.rs`, around line 301-316:

Replace:
```rust
let markdown = config.converter.to_markdown(&extracted.content_html)
```

With:
```rust
let conversion = config.converter.to_markdown(
    &extracted.content_html,
    Some(fetch_output.metadata.final_url.as_str())
);
let markdown = conversion.markdown;
let extracted_links = conversion.links;
```

The cleaned markdown is now used for:
- `prepare_preview_content(&markdown)` - line ~318
- token counting on `markdown` - line ~336
- `build_markdown_document(..., &markdown, ...)` - line ~369

#### Step 8. Extend `JobOutcome` and populate it
- Add `extracted_links: Vec<ExtractedLink>` to `JobOutcome` in `types.rs`
- When completing the job successfully in `engine.rs`, include `extracted_links` from conversion output

#### Step 9. Update EngineConfig (optional but recommended)
Add configuration flags:
```rust
pub strip_links_from_output: bool,  // Default: true
pub max_links_per_job: usize,       // Default: 5000
```

#### Step 10. Engine integration test
Add `crates/harvester_engine/tests/integration_links.rs`:
- Create test HTML with known links
- Run full pipeline
- Verify:
  - Final written `.md` doesn't contain `](http` patterns (except frontmatter URL)
  - `JobOutcome.extracted_links.len() > 0`
  - File size is smaller than with old converter

**Acceptance for Phase 2:** Downloaded output `.md` shrinks; job completes with extracted links in outcome.

---

### Phase 3 — Propagate to Core (UDF-Compliant)

#### Step 11. Update platform layer event translation
In `crates/harvester_app/src/platform/effects.rs` (around line 74-90):

In the `EngineEvent::JobCompleted` handler, map `Vec<ExtractedLink>` to `Vec<String>`:
```rust
let extracted_links: Vec<String> = outcome.extracted_links
    .into_iter()
    .map(|link| link.url)
    .collect();
```

Then pass to `Msg::JobDone`.

#### Step 12. Extend `Msg::JobDone`
In `crates/harvester_core/src/msg.rs`:

```rust
JobDone {
    job_id: crate::JobId,
    result: crate::JobResultKind,
    content_preview: Option<String>,
    extracted_links: Vec<String>,  // NEW (simplified from ExtractedLink)
}
```

#### Step 13. Update `JobState` encapsulation
In `crates/harvester_core/src/state.rs`:
- Add private field `extracted_links: Vec<String>` to `JobState`
- Add methods:
  - `fn set_extracted_links(&mut self, links: Vec<String>)` which:
    - Dedupes using normalized comparison
    - Enforces max links per job (5,000 guardrail)
    - Stores in encounter order
  - `fn extracted_links(&self) -> &[String]` (read-only slice)

#### Step 14. Update core reducer
In `crates/harvester_core/src/update.rs`:

In the `Msg::JobDone` match arm, call `state.apply_done()` which should:
- Call `job.set_extracted_links(extracted_links)` for successful jobs
- Set dirty flag
- Keep update pure: no IO

#### Step 15. Unit tests (core)
Add unit tests in `crates/harvester_core/src/state.rs` (test module):

- Given a state with one job, dispatch `Msg::JobDone { extracted_links: vec!["url1", "url2", "url1"] }`
- Assert:
  - Links are stored and deduped
  - `job.extracted_links()` returns `["url1", "url2"]`
  - Reducer remains deterministic
  - Dirty flag set

**Acceptance for Phase 3:** runtime state contains extracted links per job; no IO introduced into core.

---

### Phase 4 — Persistence

#### Step 16. Extend `CompletedJobSnapshot`
In `crates/harvester_core/src/state.rs`:
- Add `links: Vec<String>` to `CompletedJobSnapshot`.
- Update `completed_jobs_snapshot()` to include `job.extracted_links.clone()` for successful jobs.

#### Step 17. Extend persisted state format (backward-compatible)
In `crates/harvester_app/src/platform/persistence.rs`:

- Extend `PersistedJob`:

```rust
#[derive(Serialize, Deserialize)]
struct PersistedJob {
  url: String,
  tokens: Option<u32>,
  bytes: Option<u64>,
  #[serde(default)]
  links: Vec<String>, // NEW
}
```

- When saving:
  - write `links: job.links.clone()`
- When loading:
  - map `PersistedJob.links` into `CompletedJobSnapshot.links`
  - `#[serde(default)]` ensures old state files still load.

#### Step 18. Restore path in core
`Msg::RestoreCompletedJobs(Vec<CompletedJobSnapshot>)` already exists.
- Ensure restore logic sets `JobState.extracted_links` from snapshot.

#### Step 19. Persistence tests
Add `crates/harvester_app/tests/persistence_links.rs` (or unit test module):

- Create `PersistedState` content without `links` → ensure load succeeds and links is empty.
- Create with links → ensure round-trip retains them.

**Acceptance for Phase 4:** Save and load restores links for completed jobs; old `.ron` files remain readable.

---

## Guardrails / Robustness

1. **Limit per-job links** (5,000) and record if truncated.

2. **URL normalization for deduplication**:
   - Lowercase scheme and host
   - Remove default ports (:80, :443)
   - Trim whitespace
   - Later: decode percent-encoding, canonicalize query params

3. **Relative URL resolution**:
   - Resolve in converter (engine layer) using base_url parameter
   - Handle protocol-relative (`//`), absolute paths (`/`), relative paths (`./`, `../`)
   - Skip fragments (`#`) and query-only (`?`) links

4. **Malformed HTML handling**:
   - `scraper` uses `html5ever` which handles most malformed HTML gracefully
   - Wrap extraction logic in defensive handling that logs warnings but doesn't panic
   - Handle unclosed tags, invalid nesting, HTML entities in URLs

5. **srcset skipped**:
   - Extract only main `src` attribute from `<img>` tags
   - Avoids memory bloat from responsive image URL lists

6. **Deterministic output**:
   - Preserve encounter order for extraction
   - Avoid hash-map iteration order in persisted lists
   - Same HTML input → same markdown output and same link order

## Testing Strategy

### Unit Tests
- **Converter tests** (`tests/converter_links.rs`):
  - Link extraction and text-only output
  - Image removal
  - Relative URL resolution
  - Email link handling
  - Link limit enforcement
  - Deterministic output
  - Malformed HTML handling

- **Core state tests** (in `state.rs` test module):
  - Link storage and deduplication
  - Reducer determinism
  - Dirty flag management

- **Persistence tests** (`tests/persistence_links.rs` or in persistence.rs):
  - Backward compatibility (old state without links field)
  - Round-trip save/load preserves links

### Property-Based Tests (recommended)
Using `proptest`:
```rust
proptest! {
    #[test]
    fn conversion_is_deterministic(html in ".*") {
        let converter = LinkExtractingConverter::new();
        let r1 = converter.to_markdown(&html, Some("https://base.url"));
        let r2 = converter.to_markdown(&html, Some("https://base.url"));
        prop_assert_eq!(r1, r2);
    }
}
```

### Real-World Fixture Tests
Create `tests/fixtures/html/`:
- `wikipedia_article.html` - complex structure, many links
- `blog_with_images.html` - many `<img>` tags
- `navigation_heavy.html` - high link density
- `malformed.html` - unclosed tags, invalid nesting

### Integration Tests
- Full pipeline test verifying output `.md` files are smaller and contain no `](http` patterns

---

## Acceptance Criteria (End-to-End)

- Downloaded `.md` file is smaller (verify bytes written decreases on test samples)
- Markdown body does NOT contain `](http` or `](https` patterns (except frontmatter URL field)
- `<img>` tags are removed from output Markdown
- Extracted links are available in runtime state per job via `job.extracted_links()`
- Token count reflects cleaned markdown (lower than before)
- Saving state writes links into `.harvester_state.ron`
- Loading state restores links (including from older save files without links field)
- `PreviewQuality` link density is lower (correct behavior after cleaning)
- All existing tests continue to pass

---

## Future Extensions (Post-MVP)

1. **Main content extraction stage**
   - Heuristics to remove "Related stories", share widgets, nav blocks
   - Build on existing `ReadabilityLikeExtractor`

2. **Link graph UI**
   - Show deduped outgoing links per job; click to enqueue downloads
   - Group links by section (requires `context_heading` field on `ExtractedLink`)
   - Visual link graph showing relationships between downloaded pages

3. **Link classification and filtering**
   - Distinguish internal vs external links
   - Detect social share links (`twitter.com/intent`, `facebook.com/sharer`)
   - Detect archive links (`web.archive.org`, etc.)
   - Group by domain

4. **Reversible exports**
   - Optionally re-materialize clickable reference links for export-only
   - `add_reference_links()` function to append link definitions

5. **Advanced URL canonicalization**
   - Configurable stripping of tracking parameters (`utm_*`, `fbclid`, etc.)
   - Sort query parameters for consistent deduplication
   - Percent-encoding normalization

6. **Content hashing for duplicate detection**
   - Add `content_hash` field to `JobOutcome` (SHA-256 of cleaned markdown)
   - Detect duplicate content from different URLs
   - Skip re-processing unchanged pages

7. **Link position tracking**
   - Add `position: usize` field to `ExtractedLink` (character offset)
   - Enables "jump to link source" in preview
   - Supports extraction quality metrics

---

## Critical Files to Modify

| File | Changes |
|------|---------|
| `crates/harvester_engine/src/links.rs` | **NEW** - Define `LinkKind`, `ExtractedLink`, `ConversionOutput` |
| `crates/harvester_engine/src/convert.rs` | Update `Converter` trait signature; implement `LinkExtractingConverter` |
| `crates/harvester_engine/src/types.rs` | Add `extracted_links: Vec<ExtractedLink>` to `JobOutcome` |
| `crates/harvester_engine/src/engine.rs` | Update converter call (lines ~301-316); use `ConversionOutput` |
| `crates/harvester_core/src/msg.rs` | Add `extracted_links: Vec<String>` to `Msg::JobDone` |
| `crates/harvester_core/src/state.rs` | Add links field to `JobState` and `CompletedJobSnapshot`; add accessor methods |
| `crates/harvester_core/src/update.rs` | Update `apply_done()` to store extracted links |
| `crates/harvester_app/src/platform/effects.rs` | Map `Vec<ExtractedLink>` to `Vec<String>` (lines ~74-90) |
| `crates/harvester_app/src/platform/persistence.rs` | Add `#[serde(default)] links: Vec<String>` to `PersistedJob` |

