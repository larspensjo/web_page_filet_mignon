# Phase 2 Implementation Plan — Content Preparation Pipeline for Safe Summarization Inputs

Revised: 2026-02-08

## Goals

1. **Deterministic clean text derivation** — a pure pipeline that transforms raw downloaded markdown (with frontmatter, boilerplate, inconsistent formatting) into clean, hashable text with provenance metadata.
2. **Normalization policy** — configurable whitespace/formatting cleanup that is idempotent and deterministic for a given policy.
3. **Boilerplate removal** — rule-based heuristics that remove navigation remnants, cookie banners, and footer noise at document boundaries (never mid-content).
4. **Input bounding** — smart truncation that prefers paragraph > sentence > character boundaries, with budget-aware allocation for multi-article prompts.
5. **Provenance tracking** — every `CleanText` value carries a `PreparationReport` with original/clean sizes, truncation details, boilerplate removals, and a SHA-256 content hash.
6. **Replay cache compatibility** — the content hash of clean text directly serves as the replay harness lookup key, enabling cache hits when the same article is processed again.

Phase 2 is mostly internal — no new user-visible workflow. It ensures that LLM inputs (Phase 3+) are clean, bounded, cost-predictable, and cache-friendly.

---

## Context

Phase 1 built the LLM foundation: provider abstraction, prompt registry with nonce-based document delimiting, typed DTO outputs with fail-closed validation, cost/quota tracking, replay harness, and Elm-architecture integration. The LLM worker (`LlmHandle`) takes `input_content: String` and wraps it in nonce-delimited document tags via `TemplateVars::set_document()`.

But there is currently **no pipeline to prepare that input**. Downloaded pages are stored as markdown with YAML frontmatter, may contain boilerplate that survived HTML extraction, and can be arbitrarily large. Phase 2 provides the deterministic bridge from "downloaded markdown on disk" to "bounded, clean text ready for LLM consumption."

---

## Architecture Decisions

### 1. New module `harvester_engine/src/content_prep/` (not under `llm/`)

Content preparation is logically between the download pipeline and the LLM pipeline. It follows the crate's pattern where each concern has its own module (`extract.rs`, `convert.rs`, `preview.rs`). The `llm/` module is the consumer; `content_prep/` is the producer.

### 2. Lazy derivation — no disk caching in Phase 2

`CleanText` is derived on-demand when LLM processing is requested. It is a pure function of `(markdown_body, ContentPrepConfig)` so determinism is guaranteed by construction. No storage overhead, easy to iterate on normalization rules, no cache invalidation problem. The content hash enables replay cache hits. Disk caching can be added later if profiling shows derivation is a bottleneck.

### 3. `NormalizationPolicy` is a configuration struct, not hard-coded rules

Follows `Agents.md` constraint: "Avoid hard-coded string/buffer lengths; size dynamically from configuration." All thresholds are fields with sensible defaults. Deterministic for a given policy while remaining tunable.

### 4. Boilerplate removal uses deterministic, rule-based heuristics

Pattern-based rules on already-extracted markdown (the HTML extraction already did the heavy lifting via `<article>` preference). Rules only apply at document boundaries (first/last N lines) — middle content is never touched. Each rule is a named, individually testable function.

### 5. `ContentBudget` as a central concept for input bounding

Rather than passing `max_chars` around, a `ContentBudget` type encapsulates the available space for content. It knows the total budget (from `LlmConfig::max_input_chars`), the overhead (system message, user template chrome, nonce delimiters), and can allocate per-article shares for multi-article prompts.

### 6. Smart truncation prefers paragraph > sentence > character boundaries

Truncation marker `[content truncated]` length is included in the budget so it never pushes content over the limit. A minimum of 20% utilization prevents truncating to near-empty content.

### 7. No changes to `LlmCommand` or the LLM worker

Phase 2 produces `PreparedInput` / `PreparedCollection` types. The caller (Phase 3) extracts `.text()` and passes it as `input_content: String` to `Msg::RequestLlmCompletion`. The preparation guarantees the text is bounded and clean. The LLM worker stays generic.

---

## Prerequisites

### Make `strip_frontmatter` visible within the crate

The function in `preview.rs` is currently private (`fn strip_frontmatter`). It needs to be made `pub(crate)` so `content_prep/derive.rs` can call it. Zero risk — pure function, existing tests continue to pass.

---

## Deliverables (7 Parts)

### Part 1: NormalizationPolicy and Text Normalizer [Foundation]

**New file:** `crates/harvester_engine/src/content_prep/normalize.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationPolicy {
    max_consecutive_blank_lines: usize,    // default: 2
    trim_trailing_whitespace: bool,        // default: true
    normalize_line_endings: bool,          // default: true (CRLF -> LF)
    collapse_horizontal_rules: bool,       // default: true
    strip_html_comments: bool,             // default: true
    normalize_unicode_whitespace: bool,    // default: true (NBSP -> space)
}

/// Pure function: same input + same policy = same output, always.
pub fn normalize_markdown(text: &str, policy: &NormalizationPolicy) -> String
```

Pipeline (applied in order):
1. Normalize line endings: `\r\n` -> `\n`, stray `\r` -> `\n`
2. Normalize Unicode whitespace: NBSP, zero-width spaces -> ASCII equivalents
3. Strip HTML comments: `<!-- ... -->` (may survive markdown conversion)
4. Trim trailing whitespace per line
5. Collapse consecutive blank lines to `max_consecutive_blank_lines`
6. Collapse duplicate horizontal rules
7. Trim leading/trailing whitespace from whole document

Each step is a named private function.

**Tests:** CRLF normalization, blank line collapse, trailing whitespace, HTML comment removal, NBSP replacement, idempotency (`normalize(normalize(x)) == normalize(x)`), empty input, disabled policy returns input modulo line endings.

---

### Part 2: Boilerplate Filter [Foundation]

**New file:** `crates/harvester_engine/src/content_prep/boilerplate.rs`

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct BoilerplatePolicy {
    link_density_threshold: f64,       // default: 0.6
    min_nav_block_lines: usize,        // default: 3
    boundary_scan_lines: usize,        // default: 20
    known_patterns: Vec<String>,       // default: cookie/consent/footer patterns
}

pub fn filter_boilerplate(text: &str, policy: &BoilerplatePolicy) -> BoilerplateResult

pub struct BoilerplateResult {
    pub filtered_text: String,
    pub removed_line_count: usize,
    pub detected_patterns: Vec<String>,
}
```

Detection rules (each a named function, applied only in first/last `boundary_scan_lines`):
1. **Nav blocks**: Consecutive lines where `[...](...)` link syntax dominates (>= `link_density_threshold` ratio, >= `min_nav_block_lines` consecutive)
2. **Cookie patterns**: Lines matching known consent patterns at boundaries
3. **Footer patterns**: "all rights reserved", "copyright", "terms of service", etc.
4. **Share widgets**: Short lines with social media platform names at boundaries

**Critical constraint**: Middle-of-document content is never touched. This prevents false positives on articles *about* cookies or navigation.

**Tests:** Nav block at start removed, cookie banner at end removed, middle-of-document "cookies" preserved, empty document handled, no boilerplate returns unchanged, report reflects matched patterns, link density edge cases.

---

### Part 3: CleanText Type with Provenance [Core Type]

**New files:**
- `crates/harvester_engine/src/content_prep/mod.rs` — thin re-exports
- `crates/harvester_engine/src/content_prep/types.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparationReport {
    source_url: String,
    source_title: Option<String>,
    original_chars: usize,
    original_tokens: u32,
    clean_chars: usize,
    clean_tokens: u32,
    was_truncated: bool,
    truncated_at_boundary: Option<TruncationBoundary>,
    boilerplate_lines_removed: usize,
    boilerplate_patterns: Vec<String>,
    content_hash: String,           // SHA-256 of clean text
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TruncationBoundary { Paragraph, Sentence, Character }

/// Private inner field — can only be created through derive_clean_text().
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanText {
    text: String,
    content_hash: String,
    report: PreparationReport,
}

impl CleanText {
    pub fn text(&self) -> &str
    pub fn content_hash(&self) -> &str
    pub fn report(&self) -> &PreparationReport
    pub fn char_count(&self) -> usize
    pub fn token_count(&self) -> u32
}
```

No public constructor — enforces the invariant that all `CleanText` values passed through the normalization/filtering/hashing pipeline.

**Tests:** Accessor correctness, `PreparationReport` serde round-trip, `TruncationBoundary` coverage.

---

### Part 4: CleanText Derivation Pipeline [Integration]

**New file:** `crates/harvester_engine/src/content_prep/derive.rs`

```rust
#[derive(Debug, Clone)]
pub struct ContentPrepConfig {
    pub normalization: NormalizationPolicy,
    pub boilerplate: BoilerplatePolicy,
    pub token_counter: Arc<dyn TokenCounter>,
}

/// Pure pipeline: strip_frontmatter -> normalize -> filter_boilerplate -> hash -> CleanText
pub fn derive_clean_text(
    markdown: &str,
    source_url: &str,
    source_title: Option<&str>,
    config: &ContentPrepConfig,
) -> CleanText
```

Pipeline steps:
1. `strip_frontmatter(markdown)` — reuse from `preview.rs` (make `pub(crate)`)
2. `normalize_markdown(stripped, &config.normalization)` — Part 1
3. `filter_boilerplate(normalized, &config.boilerplate)` — Part 2
4. Token estimation via `config.token_counter.count()`
5. SHA-256 hash via existing `content_hash()` from `llm/replay.rs`
6. Assemble `PreparationReport` and construct `CleanText`

**Determinism contract**: Given identical `(markdown, source_url, source_title, config)`, the output (including `content_hash`) is always identical. This is the foundation for replay cache hits.

**Tests:** Determinism (same input -> same hash), different inputs -> different hashes, frontmatter stripped before hashing, normalization/boilerplate applied, provenance report accuracy, empty markdown handled, large input handled.

---

### Part 5: Smart Truncation [Input Bounding]

**New file:** `crates/harvester_engine/src/content_prep/truncation.rs`

```rust
pub const TRUNCATION_MARKER: &str = "\n\n[content truncated]";

/// Truncate to fit within max_chars, preferring natural boundaries.
/// Returns (truncated_text, boundary_kind) or original text if it fits.
pub fn truncate_to_budget(
    text: &str,
    max_chars: usize,
) -> (String, Option<TruncationBoundary>)
```

Boundary preference:
1. **Paragraph** — last `\n\n` where text up to that point + marker fits within budget. Minimum 20% utilization.
2. **Sentence** — last `. ` / `! ` / `? ` where text up to that point + marker fits. Minimum 20% utilization.
3. **Character** — fallback via existing `truncate_to_char_boundary` from `text_safety.rs`, minus marker length.

The marker length is subtracted from `max_chars` before searching boundaries, ensuring text + marker never exceeds `max_chars`.

**Tests:** Within-budget unchanged (no marker), paragraph boundary, sentence boundary, character fallback, marker appended and total <= max_chars, Unicode correctness, 20% minimum rule, edge cases (max_chars < marker length, single long paragraph).

---

### Part 6: ContentBudget and PreparedInput [Input Bounding]

**New file:** `crates/harvester_engine/src/content_prep/budget.rs`

```rust
/// Character budget for content within an LLM prompt.
pub struct ContentBudget {
    total_chars: usize,        // from LlmConfig::max_input_chars
    overhead_chars: usize,     // system msg + user template chrome + nonce delimiters
    available_chars: usize,    // total - overhead (saturating)
}

impl ContentBudget {
    pub fn new(total_chars: usize, overhead_chars: usize) -> Self
    pub fn available(&self) -> usize

    /// Split budget equally among N items, each getting at least min_per_item.
    pub fn allocate_equal(&self, n: usize, min_per_item: usize) -> Option<Vec<usize>>

    /// Measure overhead for a prompt template (rendered length minus content placeholder).
    pub fn estimate_overhead(
        template: &PromptTemplate,
        context_vars: &[(String, String)],
        content_placeholder: &str,
    ) -> usize
}

/// Single article prepared for LLM consumption.
pub struct PreparedInput { /* clean_text, bounded_text, budget_chars */ }

impl PreparedInput {
    pub fn text(&self) -> &str          // possibly truncated
    pub fn clean_text(&self) -> &CleanText
    pub fn was_truncated(&self) -> bool
    pub fn from_clean_text(clean_text: CleanText, budget_chars: usize) -> Self
}

/// Multiple articles prepared for a briefing prompt.
pub struct PreparedCollection { /* items, concatenated, total_chars */ }

impl PreparedCollection {
    pub fn text(&self) -> &str          // for {{collection}} placeholder
    pub fn article_count(&self) -> usize
    /// Each item wrapped: "--- Article 1: {title} ---\n{text}\n"
    pub fn from_inputs(inputs: Vec<PreparedInput>) -> Self
}
```

`estimate_overhead` renders the template with all context vars but empty content placeholder, measures the resulting string length, and adds ~40 chars for nonce delimiter wrapping. This accounts for all prompt chrome.

**Tests:** Budget arithmetic, overhead > total gives 0 available, equal allocation with remainder, min_per_item enforcement, PreparedInput truncation, collection concatenation, single-article collection.

---

### Part 7: Module Integration, Re-exports, and Logging [Integration]

**Modified files:**
- `crates/harvester_engine/src/lib.rs` — add `pub mod content_prep;` and re-exports
- `crates/harvester_engine/src/preview.rs` — make `strip_frontmatter` `pub(crate)` (line 18: `fn` -> `pub(crate) fn`)

**`content_prep/mod.rs`:**
```rust
mod boilerplate;
mod budget;
mod derive;
mod normalize;
mod truncation;
mod types;

pub use boilerplate::{BoilerplatePolicy, BoilerplateResult};
pub use budget::{ContentBudget, PreparedCollection, PreparedInput};
pub use derive::{ContentPrepConfig, derive_clean_text};
pub use normalize::NormalizationPolicy;
pub use truncation::{truncate_to_budget, TRUNCATION_MARKER};
pub use types::{CleanText, PreparationReport, TruncationBoundary};
```

**Logging** in `derive.rs`:
```rust
engine_info!(
    "[content-prep] url={} original_chars={} clean_chars={} truncated={} hash={}",
    source_url, report.original_chars, report.clean_chars, report.was_truncated, &content_hash[..8]
);
```

**Integration tests** (`crates/harvester_engine/tests/content_prep_integration.rs`):
1. Raw markdown with frontmatter + boilerplate -> `derive_clean_text` -> `PreparedInput::from_clean_text` -> verify clean, bounded, hashable
2. Multiple articles -> derive each -> allocate budget -> `PreparedCollection::from_inputs` -> verify fits budget
3. `CleanText::content_hash()` matches `content_hash(clean_text.text())` from replay module
4. **Determinism lock-in test**: Fixed input string with fixed expected SHA-256 hash (detects accidental normalization changes)

---

## Implementation Order (Blocker-First)

1. **Part 1** — NormalizationPolicy + `normalize_markdown()` (no dependencies)
2. **Part 2** — BoilerplatePolicy + `filter_boilerplate()` (no dependencies; can parallel with Part 1)
3. **Part 3** — CleanText, PreparationReport types (needs `sha2`/`serde`, already available)
4. **Part 4** — `derive_clean_text()` (depends on Parts 1-3 + `strip_frontmatter` + `content_hash`)
5. **Part 5** — `truncate_to_budget()` (depends on `truncate_to_char_boundary`, already available)
6. **Part 6** — ContentBudget, PreparedInput, PreparedCollection (depends on Parts 3, 5 + `PromptTemplate`)
7. **Part 7** — Module integration + integration tests (depends on all above)

---

## Files Summary

| Action | File | Purpose |
|--------|------|---------|
| **Create** | `crates/harvester_engine/src/content_prep/mod.rs` | Thin re-export module |
| **Create** | `crates/harvester_engine/src/content_prep/types.rs` | `CleanText`, `PreparationReport`, `TruncationBoundary` |
| **Create** | `crates/harvester_engine/src/content_prep/normalize.rs` | `NormalizationPolicy`, `normalize_markdown()` |
| **Create** | `crates/harvester_engine/src/content_prep/boilerplate.rs` | `BoilerplatePolicy`, `filter_boilerplate()` |
| **Create** | `crates/harvester_engine/src/content_prep/derive.rs` | `ContentPrepConfig`, `derive_clean_text()` |
| **Create** | `crates/harvester_engine/src/content_prep/truncation.rs` | `truncate_to_budget()` |
| **Create** | `crates/harvester_engine/src/content_prep/budget.rs` | `ContentBudget`, `PreparedInput`, `PreparedCollection` |
| **Create** | `crates/harvester_engine/tests/content_prep_integration.rs` | Integration tests |
| **Modify** | `crates/harvester_engine/src/lib.rs` | Add `pub mod content_prep;` + re-exports |
| **Modify** | `crates/harvester_engine/src/preview.rs` | `strip_frontmatter`: `fn` -> `pub(crate) fn` |

---

## Test Strategy

### Unit tests per module (inline `#[cfg(test)] mod tests`)

**normalize.rs:** Idempotency, each normalization step in isolation, policy-driven behavior, edge cases (empty, Unicode-heavy, all-whitespace).

**boilerplate.rs:** Each detection rule in isolation, false positive prevention (middle-of-document content preserved), boundary scanning limits, policy tuning.

**types.rs:** Serde round-trip for `PreparationReport`, accessor correctness.

**derive.rs:** Determinism (same input -> same hash), pipeline ordering verification, provenance report accuracy.

**truncation.rs:** All three boundary types, marker inclusion in budget, Unicode safety, minimum percentage rule.

**budget.rs:** Allocation arithmetic, overflow/underflow protection, overhead estimation, collection assembly.

### Integration tests

A top-level integration test in `crates/harvester_engine/tests/content_prep_integration.rs` that:
1. Constructs realistic markdown (with frontmatter, boilerplate, long content)
2. Runs `derive_clean_text`
3. Verifies `content_hash` matches direct SHA-256 of the clean text
4. Verifies truncation at budget boundaries
5. Verifies collection assembly for 3-5 articles
6. Verifies replay cache compatibility

### Determinism lock-in test

A dedicated test with a fixed input string and a fixed expected SHA-256 hash, to detect any accidental normalization changes that would break replay cache compatibility. This test must be updated intentionally when normalization rules change.

---

## Potential Blockers

1. **`strip_frontmatter` visibility** — Currently private in `preview.rs` (line 18). Must be made `pub(crate)`. Zero risk — pure function, existing tests continue to pass.

2. **`content_hash` reuse** — Lives in `llm/replay.rs`, accessible within the crate as `crate::llm::replay::content_hash`. No dependency issue.

3. **`TokenCounter` in `ContentPrepConfig`** — Requires `Arc<dyn TokenCounter>`. Follows existing pattern (`EngineConfig` carries `token_counter: Arc<dyn TokenCounter>`).

4. **Normalization changes invalidate replay cache** — By design: different preparation = different content hash = different LLM input = different result. The determinism lock-in test detects unintentional changes.

5. **No new external dependencies** — All functionality uses existing crate dependencies (`sha2`, `hex`, `serde`). No regex needed — boilerplate patterns use simple string operations (`contains`, `starts_with`, link counting).

---

## Future Extensions (noted for later phases)

- **Tiktoken-accurate token counting**: Replace `WhitespaceTokenCounter` with BPE-based estimator for accurate budget calculation. The `TokenCounter` trait already supports injection.
- **Priority-weighted budget allocation**: Allocate more tokens to higher-priority articles (after Phase 4 provides triage scores).
- **Normalization versioning**: Include policy version hash in replay lookup key for cache-safe policy evolution.
- **Disk caching of CleanText**: Optional persistence alongside markdown for large corpora. Feature-gated.
- **Chunking for very long articles**: Split into overlapping chunks instead of truncation. Requires merge strategy for DTO outputs.
- **`UntrustedContent(String)` newtype**: Trust-wrapper around raw text that can only be unwrapped through content prep. Compile-time enforcement.
- **Summary-as-input for briefings**: Use `ArticleSummary` outputs as briefing collection items instead of raw clean text, reducing token usage.
- **Content fingerprinting for dedup**: Extend content hash to skip re-processing across sessions (same article, different download time).
- **Configurable boilerplate rule sets**: Load patterns from config file instead of compile-time defaults.
- **Retry with smaller excerpt**: If LLM returns MaxTokens finish reason, automatically retry with a smaller content budget.

---

## Verification

1. `cargo build` — workspace compiles with new module
2. `cargo test --workspace` — all existing + new tests pass
3. `cargo clippy --all-targets -- -D warnings` — no warnings
4. **Determinism**: fixed-input test produces expected SHA-256 hash
5. **Idempotency**: `normalize(normalize(x)) == normalize(x)` for all test inputs
6. **Bounding**: no `PreparedInput` or `PreparedCollection` exceeds its budget
7. **Safety**: `truncate_to_budget` never splits a Unicode character
8. **Provenance**: every `CleanText` carries accurate `PreparationReport`
9. **Hash compatibility**: `CleanText::content_hash()` == `content_hash(clean_text.text())`
10. **Logging**: `[content-prep]` category tags present in derivation path
