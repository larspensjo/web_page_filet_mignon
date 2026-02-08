# Phase 2 Implementation Plan — Content Preparation Pipeline for Safe Summarization Inputs

Revised: 2026-02-08 (post-review)

## Goals

1. **Deterministic clean text derivation** — a pure pipeline that transforms raw downloaded markdown (with frontmatter, boilerplate, inconsistent formatting) into clean, hashable text with provenance metadata.
2. **Normalization policy** — configurable whitespace/formatting cleanup that is idempotent and deterministic for a given policy.
3. **Boilerplate removal** — rule-based heuristics that remove navigation remnants, cookie banners, and footer noise at document boundaries (never mid-content), with short-document safeguards.
4. **Input bounding** — smart truncation that prefers paragraph > sentence > character boundaries, with budget-aware allocation for multi-article prompts. **All budgets use bytes** (matching the runtime enforcement in `LlmConfig::max_input_chars` and `effects.rs`, which both check `String::len()` — i.e. bytes, not Unicode scalar count).
5. **Provenance tracking** — every `CleanText` value carries a `CleanTextReport` with original/clean sizes, boilerplate removals, and a SHA-256 content hash. Truncation metadata lives on `PreparedInput`, not `CleanText`, since truncation is budget-dependent.
6. **Replay cache compatibility** — the content hash of clean text serves as the **content component** of the composite replay lookup key (which also includes `prompt_id` and `prompt_version`; see `replay.rs:149`).

Phase 2 is mostly internal — no new user-visible workflow. It ensures that LLM inputs (Phase 3+) are clean, bounded, cost-predictable, and cache-friendly.

---

## Context

Phase 1 built the LLM foundation: provider abstraction, prompt registry with nonce-based document delimiting, typed DTO outputs with fail-closed validation, cost/quota tracking, replay harness, and Elm-architecture integration. The LLM worker (`LlmHandle`) takes `input_content: String` and wraps it in nonce-delimited document tags via `TemplateVars::set_document()`.

But there is currently **no pipeline to prepare that input**. Downloaded pages are stored as markdown with YAML frontmatter, may contain boilerplate that survived HTML extraction, and can be arbitrarily large. Phase 2 provides the deterministic bridge from "downloaded markdown on disk" to "bounded, clean text ready for LLM consumption."

### Byte budget alignment

The runtime guard in `handle.rs:168` checks `input_content.len()` (Rust `String::len()` = byte count) against `LlmConfig::max_input_chars`. The effect runner in `effects.rs:323` performs the same byte-based check. Despite the field name saying "chars", both enforcement points measure **bytes**.

Phase 2 uses bytes as its canonical budget unit to match this reality. The naming mismatch (`max_input_chars` vs byte semantics) is noted as a cleanup item for integration.

---

## Architecture Decisions

### 1. New module `harvester_engine/src/content_prep/` (not under `llm/`)

Content preparation is logically between the download pipeline and the LLM pipeline. It follows the crate's pattern where each concern has its own module (`extract.rs`, `convert.rs`, `preview.rs`). The `llm/` module is the consumer; `content_prep/` is the producer.

### 2. Lazy derivation — no disk caching in Phase 2

`CleanText` is derived on-demand when LLM processing is requested. It is a pure function of `(markdown_body, ContentPrepConfig)` so determinism is guaranteed by construction. No storage overhead, easy to iterate on normalization rules, no cache invalidation problem. The content hash enables replay cache hits. Disk caching can be added later if profiling shows derivation is a bottleneck.

### 3. `NormalizationPolicy` is a configuration struct, not hard-coded rules

Follows `Agents.md` constraint: "Avoid hard-coded string/buffer lengths; size dynamically from configuration." All thresholds are fields with sensible defaults. Deterministic for a given policy while remaining tunable.

### 4. Boilerplate removal uses deterministic, rule-based heuristics with short-document safeguards

Pattern-based rules on already-extracted markdown (the HTML extraction already did the heavy lifting via `<article>` preference). Rules only apply at document boundaries (first/last N lines) — middle content is never touched. Each rule is a named, individually testable function.

**Short-document handling**: When the document has fewer than `2 * boundary_scan_lines` lines, the head and tail scan windows overlap. In this case, each window shrinks to `document_lines / 2` to prevent double-processing. Additionally, a `max_removal_ratio` (default 0.5) ensures that boilerplate removal never discards more than half the document's lines, protecting against false positives on very short content.

### 5. `ContentBudget` as a central concept for input bounding (bytes)

Rather than passing raw byte limits around, a `ContentBudget` type encapsulates the available space for content. For single articles, the full budget goes to one article. For multi-article briefing prompts, it divides the budget among articles accounting for article-separator overhead. The caller (Phase 3) sets the budget based on `LlmConfig` max input (bytes).

### 6. Smart truncation prefers paragraph > sentence > byte boundaries

Truncation marker `[content truncated]` byte length is included in the budget so it never pushes content over the limit. All boundary searches respect UTF-8 char boundaries. A minimum of 20% utilization prevents truncating to near-empty content.

### 7. No changes to `LlmCommand` or the LLM worker

Phase 2 produces `PreparedInput` / `PreparedCollection` types. The caller (Phase 3) extracts `.text()` and passes it as `input_content: String` to `Msg::RequestLlmCompletion`. The preparation guarantees `text().len() <= budget_bytes`, satisfying the runtime guard.

### 8. Separate derivation metadata from budget/truncation metadata

`CleanText` carries a `CleanTextReport` with derivation-only information (normalization, boilerplate, hash). Truncation metadata (`was_truncated`, `TruncationBoundary`) lives on `PreparedInput` because truncation is budget-dependent — the same `CleanText` may be truncated differently for different prompt budgets. This preserves the invariant that `CleanText` is immutable and budget-agnostic.

### 9. Frontmatter stripping lives in `frontmatter.rs`, not `preview.rs`

The existing `frontmatter.rs` module handles frontmatter building (`build_markdown_document`). `strip_frontmatter` (currently private in `preview.rs`) is the inverse operation and belongs alongside it. Moving it to `frontmatter.rs` as `pub(crate)` eliminates coupling between content prep and the UI-preview module, and keeps both preview and content prep as consumers of a shared utility.

### 10. Exact prompt overhead computation, not approximation

The nonce wrapping overhead from `TemplateVars::set_document()` is a constant **49 bytes** per document slot (the nonce is always 12 hex chars: `<document-{12}>\n` = 24 bytes, `\n</document-{12}>` = 25 bytes). Template chrome is measurable by rendering with an empty document placeholder. Phase 2 provides a helper to compute exact overhead; Phase 3 uses it to calculate budgets from model context limits.

---

## Prerequisites

### Move `strip_frontmatter` to `frontmatter.rs`

The function in `preview.rs` (line 18) is currently private. Rather than making it `pub(crate)` in a preview-specific module:
1. Move `strip_frontmatter` to `frontmatter.rs` as `pub(crate) fn strip_frontmatter`.
2. Update `preview.rs` to call `crate::frontmatter::strip_frontmatter`.
3. All existing tests continue to pass (pure function, no behavior change).

Both `preview.rs` and `content_prep/derive.rs` then depend on the frontmatter module — the natural home for frontmatter operations.

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
    max_removal_ratio: f64,            // default: 0.5 (never remove > 50% of lines)
    known_patterns: Vec<String>,       // default: cookie/consent/footer patterns
}

pub fn filter_boilerplate(text: &str, policy: &BoilerplatePolicy) -> BoilerplateResult

pub struct BoilerplateResult {
    pub filtered_text: String,
    pub removed_line_count: usize,
    pub detected_patterns: Vec<String>,
}
```

Detection rules (each a named function, applied only in head/tail scan windows):
1. **Nav blocks**: Consecutive lines where `[...](...)` link syntax dominates (>= `link_density_threshold` ratio, >= `min_nav_block_lines` consecutive)
2. **Cookie patterns**: Lines matching known consent patterns at boundaries
3. **Footer patterns**: "all rights reserved", "copyright", "terms of service", etc.
4. **Share widgets**: Short lines with social media platform names at boundaries

**Critical constraint**: Middle-of-document content is never touched. This prevents false positives on articles *about* cookies or navigation.

**Short-document safeguards**:
- When `total_lines < 2 * boundary_scan_lines`, each window shrinks to `total_lines / 2` (no overlap).
- After all rules fire, if `removed_line_count > total_lines * max_removal_ratio`, the removal is rejected entirely (return input unchanged) and `detected_patterns` reports the attempted matches for diagnostics.

**Tests:** Nav block at start removed, cookie banner at end removed, middle-of-document "cookies" preserved, empty document handled, no boilerplate returns unchanged, report reflects matched patterns, link density edge cases, **short document (< 40 lines) with boilerplate at both ends**, **max_removal_ratio trigger returns input unchanged**, **document shorter than 2 lines handled gracefully**.

---

### Part 3: CleanText Type with Provenance [Core Type]

**New files:**
- `crates/harvester_engine/src/content_prep/mod.rs` — thin re-exports
- `crates/harvester_engine/src/content_prep/types.rs`

```rust
/// Derivation-only report. No truncation metadata — truncation is budget-dependent
/// and belongs on PreparedInput.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanTextReport {
    source_url: String,
    source_title: Option<String>,
    original_bytes: usize,
    original_tokens: u32,
    clean_bytes: usize,
    clean_tokens: u32,
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
    report: CleanTextReport,
}

impl CleanText {
    pub fn text(&self) -> &str
    pub fn content_hash(&self) -> &str
    pub fn report(&self) -> &CleanTextReport
    pub fn byte_count(&self) -> usize      // text().len()
    pub fn token_count(&self) -> u32       // from report
}
```

No public constructor — enforces the invariant that all `CleanText` values passed through the normalization/filtering/hashing pipeline. `CleanText` is immutable and budget-agnostic.

**Tests:** Accessor correctness, `CleanTextReport` serde round-trip, `TruncationBoundary` coverage.

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
1. `strip_frontmatter(markdown)` — from `crate::frontmatter` (see Prerequisites)
2. `normalize_markdown(stripped, &config.normalization)` — Part 1
3. `filter_boilerplate(normalized, &config.boilerplate)` — Part 2
4. Token estimation via `config.token_counter.count()`
5. SHA-256 hash via existing `content_hash()` from `llm/replay.rs`
6. Assemble `CleanTextReport` and construct `CleanText`

**Determinism contract**: Given identical `(markdown, source_url, source_title, normalization_policy, boilerplate_policy)`, the `text()` and `content_hash()` are always identical. Token counts are additionally deterministic for a given `TokenCounter` implementation. The content hash does **not** depend on token counting — swapping the token counter changes informational metadata but not the cache key.

**Tests:** Determinism (same input -> same hash), different inputs -> different hashes, frontmatter stripped before hashing, normalization/boilerplate applied, provenance report accuracy, empty markdown handled, large input handled, **swapping token counter does not change content_hash**.

---

### Part 5: Smart Truncation [Input Bounding]

**New file:** `crates/harvester_engine/src/content_prep/truncation.rs`

```rust
pub const TRUNCATION_MARKER: &str = "\n\n[content truncated]";

/// Truncate to fit within max_bytes, preferring natural boundaries.
/// All cuts respect UTF-8 char boundaries.
/// Returns (truncated_text, boundary_kind) or original text if it fits.
pub fn truncate_to_budget(
    text: &str,
    max_bytes: usize,
) -> (String, Option<TruncationBoundary>)
```

Boundary preference:
1. **Paragraph** — last `\n\n` where `text[..pos].len() + TRUNCATION_MARKER.len() <= max_bytes`. Minimum 20% utilization of budget.
2. **Sentence** — last `. ` / `! ` / `? ` where text up to that point + marker fits. Minimum 20% utilization.
3. **Character** — fallback: find the last UTF-8 char boundary at or before `max_bytes - TRUNCATION_MARKER.len()`.

The marker byte length is subtracted from `max_bytes` before searching boundaries, ensuring `result.len() <= max_bytes` always holds. Since `\n\n`, `. `, `! `, `? ` are all single-byte ASCII sequences, boundary search within the byte budget naturally aligns with UTF-8 char boundaries.

**Tests:** Within-budget unchanged (no marker), paragraph boundary, sentence boundary, character fallback, **`result.len() <= max_bytes` for all cases**, Unicode correctness (multi-byte chars near boundary), 20% minimum rule, edge cases (max_bytes < marker length, single long paragraph, all-emoji content).

---

### Part 6: ContentBudget and PreparedInput [Input Bounding]

**New file:** `crates/harvester_engine/src/content_prep/budget.rs`

```rust
/// Byte budget for content within an LLM prompt.
pub struct ContentBudget {
    total_bytes: usize,        // from LlmConfig max_input (bytes)
}

impl ContentBudget {
    pub fn new(total_bytes: usize) -> Self
    pub fn available(&self) -> usize

    /// Split budget equally among N items, subtracting per-item separator overhead.
    /// Returns None if budget cannot fit N items with minimum per_item bytes.
    pub fn allocate_equal(
        &self,
        n: usize,
        separator_bytes_per_item: usize,
        min_per_item: usize,
    ) -> Option<Vec<usize>>
}

/// Nonce wrapping overhead: constant 49 bytes per document slot.
/// <document-{12hex}>\n = 24 bytes, \n</document-{12hex}> = 25 bytes.
pub const NONCE_OVERHEAD_BYTES: usize = 49;

/// Compute exact overhead bytes for a prompt template.
/// Renders the template with all context vars but empty document content,
/// measures the byte length, and adds NONCE_OVERHEAD_BYTES per document slot.
pub fn compute_prompt_overhead(
    template: &PromptTemplate,
    document_key: &str,
    context_vars: &[(String, String)],
) -> usize

/// Single article prepared for LLM consumption.
pub struct PreparedInput {
    clean_text: CleanText,
    bounded_text: String,           // possibly truncated
    budget_bytes: usize,
    was_truncated: bool,
    truncated_at_boundary: Option<TruncationBoundary>,
}

impl PreparedInput {
    pub fn text(&self) -> &str          // possibly truncated, .len() <= budget_bytes
    pub fn clean_text(&self) -> &CleanText
    pub fn was_truncated(&self) -> bool
    pub fn truncation_boundary(&self) -> Option<TruncationBoundary>
    pub fn from_clean_text(clean_text: CleanText, budget_bytes: usize) -> Self
}

/// Multiple articles prepared for a briefing prompt.
pub struct PreparedCollection {
    items: Vec<PreparedInput>,
    concatenated: String,
}

impl PreparedCollection {
    pub fn text(&self) -> &str          // for {{collection}} placeholder
    pub fn article_count(&self) -> usize
    /// Each item wrapped: "--- Article 1: {title} ---\n{text}\n"
    pub fn from_inputs(inputs: Vec<PreparedInput>) -> Self
}
```

**Key guarantee**: `PreparedInput::text().len() <= budget_bytes` and `PreparedCollection::text().len() <= total_budget_bytes`. This satisfies the runtime guard's byte-based check.

`compute_prompt_overhead` uses `TemplateVars` and `render_template` (the same rendering path as production) with empty document content. This gives the exact non-content byte cost of a rendered prompt, ensuring budget calculations stay in lockstep with the worker.

**Tests:** Budget arithmetic, equal allocation with remainder, `min_per_item` enforcement, separator overhead accounting, PreparedInput truncation, **`text().len() <= budget_bytes` invariant**, collection concatenation, single-article collection, **`compute_prompt_overhead` matches actual rendered prompt overhead** (cross-module parity test).

---

### Part 7: Module Integration, Re-exports, and Logging [Integration]

**Modified files:**
- `crates/harvester_engine/src/lib.rs` — add `pub mod content_prep;` and re-exports
- `crates/harvester_engine/src/frontmatter.rs` — add `pub(crate) fn strip_frontmatter` (moved from `preview.rs`)
- `crates/harvester_engine/src/preview.rs` — update to call `crate::frontmatter::strip_frontmatter`

**`content_prep/mod.rs`:**
```rust
mod boilerplate;
mod budget;
mod derive;
mod normalize;
mod truncation;
mod types;

pub use boilerplate::{BoilerplatePolicy, BoilerplateResult};
pub use budget::{
    compute_prompt_overhead, ContentBudget, PreparedCollection, PreparedInput,
    NONCE_OVERHEAD_BYTES,
};
pub use derive::{ContentPrepConfig, derive_clean_text};
pub use normalize::NormalizationPolicy;
pub use truncation::{truncate_to_budget, TRUNCATION_MARKER};
pub use types::{CleanText, CleanTextReport, TruncationBoundary};
```

**Logging** in `derive.rs`:
```rust
engine_info!(
    "[content-prep] url={} original_bytes={} clean_bytes={} hash={}",
    source_url, report.original_bytes, report.clean_bytes, &content_hash[..8]
);
```

**Integration tests** (`crates/harvester_engine/tests/content_prep_integration.rs`):
1. Raw markdown with frontmatter + boilerplate -> `derive_clean_text` -> `PreparedInput::from_clean_text` -> verify clean, bounded, hashable
2. Multiple articles -> derive each -> allocate budget -> `PreparedCollection::from_inputs` -> verify `text().len() <= budget`
3. `CleanText::content_hash()` matches `content_hash(clean_text.text())` from replay module
4. **Determinism lock-in test**: Fixed input string with fixed expected SHA-256 hash (detects accidental normalization changes). Validates content hash only — token counts are excluded from the lock-in.
5. **Byte budget invariant**: `PreparedInput::text().len() <= budget_bytes` for ASCII, multi-byte UTF-8, and mixed content
6. **Cross-module overhead parity**: `compute_prompt_overhead` output + content bytes == actual rendered prompt bytes (validates overhead estimation against the production rendering path)

---

## Implementation Order

```
Part 1: NormalizationPolicy + normalize_markdown()
    ↓
Part 2: BoilerplatePolicy + filter_boilerplate()
    ↓
Part 3: CleanText, CleanTextReport types         ← needs sha2/serde (already available)
    ↓
Part 4: derive_clean_text()                      ← depends on Parts 1-3 + strip_frontmatter + content_hash
    ↓
Part 5: truncate_to_budget()                     ← needs truncate_to_char_boundary (already available)
    ↓
Part 6: ContentBudget, PreparedInput, PreparedCollection  ← depends on Parts 3, 5 + PromptTemplate
    ↓
Part 7: Module integration + integration tests   ← depends on all above
```

Parts 1 and 2 have no mutual dependency and can be built in parallel. Part 5 only depends on `text_safety.rs` and could be built in parallel with Parts 1-2, but logically follows the type definitions in Part 3.

---

## Files Summary

| Action | File | Purpose |
|--------|------|---------|
| **Create** | `crates/harvester_engine/src/content_prep/mod.rs` | Thin re-export module |
| **Create** | `crates/harvester_engine/src/content_prep/types.rs` | `CleanText`, `CleanTextReport`, `TruncationBoundary` |
| **Create** | `crates/harvester_engine/src/content_prep/normalize.rs` | `NormalizationPolicy`, `normalize_markdown()` |
| **Create** | `crates/harvester_engine/src/content_prep/boilerplate.rs` | `BoilerplatePolicy`, `filter_boilerplate()` |
| **Create** | `crates/harvester_engine/src/content_prep/derive.rs` | `ContentPrepConfig`, `derive_clean_text()` |
| **Create** | `crates/harvester_engine/src/content_prep/truncation.rs` | `truncate_to_budget()` |
| **Create** | `crates/harvester_engine/src/content_prep/budget.rs` | `ContentBudget`, `PreparedInput`, `PreparedCollection`, `compute_prompt_overhead()` |
| **Create** | `crates/harvester_engine/tests/content_prep_integration.rs` | Integration tests |
| **Modify** | `crates/harvester_engine/src/lib.rs` | Add `pub mod content_prep;` + re-exports |
| **Modify** | `crates/harvester_engine/src/frontmatter.rs` | Add `pub(crate) fn strip_frontmatter` (moved from `preview.rs`) |
| **Modify** | `crates/harvester_engine/src/preview.rs` | Update to use `crate::frontmatter::strip_frontmatter` |

---

## Test Strategy

### Unit tests per module (inline `#[cfg(test)] mod tests`)

**normalize.rs:** Idempotency, each normalization step in isolation, policy-driven behavior, edge cases (empty, Unicode-heavy, all-whitespace).

**boilerplate.rs:** Each detection rule in isolation, false positive prevention (middle-of-document content preserved), boundary scanning limits, policy tuning, **short documents (< 2 * boundary_scan_lines)**, **max_removal_ratio trigger**, **document with only 1-2 lines**.

**types.rs:** Serde round-trip for `CleanTextReport`, accessor correctness.

**derive.rs:** Determinism (same input -> same hash), pipeline ordering verification, provenance report accuracy, **token counter swap does not change content_hash**.

**truncation.rs:** All three boundary types, marker inclusion in budget, **`result.len() <= max_bytes` for all cases**, Unicode safety (multi-byte chars near boundaries), minimum percentage rule.

**budget.rs:** Allocation arithmetic, overflow/underflow protection, **`compute_prompt_overhead` parity with production rendering**, collection assembly, **byte budget invariant on PreparedInput**.

### Integration tests

A top-level integration test in `crates/harvester_engine/tests/content_prep_integration.rs` that:
1. Constructs realistic markdown (with frontmatter, boilerplate, long content)
2. Runs `derive_clean_text`
3. Verifies `content_hash` matches direct SHA-256 of the clean text
4. Verifies truncation at budget boundaries (byte-level)
5. Verifies collection assembly for 3-5 articles
6. Verifies replay cache compatibility
7. Cross-module overhead parity test

### Determinism lock-in test

A dedicated test with a fixed input string and a fixed expected SHA-256 hash, to detect any accidental normalization changes that would break replay cache compatibility. This test validates the **content hash only** — token counts are informational and depend on the injected `TokenCounter`, so they are excluded from the lock-in. This test must be updated intentionally when normalization rules change.

### Byte-budget lock tests

Every `PreparedInput` and `PreparedCollection` must satisfy `text().len() <= budget_bytes`. Tests cover ASCII content, multi-byte UTF-8 content (CJK, emoji), and mixed content to ensure byte-level safety.

---

## Potential Blockers

1. **`strip_frontmatter` relocation** — Currently private in `preview.rs` (line 18). Must be moved to `frontmatter.rs` as `pub(crate)`. Zero risk — pure function, existing tests continue to pass. `preview.rs` updated to call the new location.

2. **`content_hash` reuse** — Lives in `llm/replay.rs`, accessible within the crate as `crate::llm::replay::content_hash`. No dependency issue.

3. **`TokenCounter` in `ContentPrepConfig`** — Requires `Arc<dyn TokenCounter>`. Follows existing pattern (`EngineConfig` carries `token_counter: Arc<dyn TokenCounter>`).

4. **Normalization changes invalidate replay cache** — By design: different preparation = different content hash = different LLM input = different result. The determinism lock-in test detects unintentional changes.

5. **No new external dependencies** — All functionality uses existing crate dependencies (`sha2`, `hex`, `serde`). No regex needed — boilerplate patterns use simple string operations (`contains`, `starts_with`, link counting).

6. **`render_template` access for overhead computation** — `render_template` is currently private in `handle.rs`. Either make it `pub(crate)` or extract to a shared location (e.g. `llm/prompt.rs`). Small, zero-risk change.

7. **`max_input_chars` naming** — The existing field name suggests characters but the runtime checks bytes. Phase 2 works correctly with the byte semantics. Renaming to `max_input_bytes` is an optional cleanup, not a blocker.

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
- **Near-duplicate detection**: Stable fingerprint of normalized clean text to skip redundant LLM calls before expensive processing.
- **Strict mode**: Refuse to process if prepared input cannot satisfy a minimum retained-content threshold after truncation.

---

## Verification

1. `cargo build` — workspace compiles with new module
2. `cargo test --workspace` — all existing + new tests pass
3. `cargo clippy --all-targets -- -D warnings` — no warnings
4. **Determinism**: fixed-input test produces expected SHA-256 hash
5. **Idempotency**: `normalize(normalize(x)) == normalize(x)` for all test inputs
6. **Byte bounding**: no `PreparedInput::text().len()` or `PreparedCollection::text().len()` exceeds its budget
7. **Safety**: `truncate_to_budget` never splits a UTF-8 character
8. **Provenance**: every `CleanText` carries accurate `CleanTextReport`
9. **Hash compatibility**: `CleanText::content_hash()` == `content_hash(clean_text.text())`
10. **Logging**: `[content-prep]` category tags present in derivation path
11. **Overhead parity**: `compute_prompt_overhead` output matches actual rendered prompt overhead
12. **Short-document safety**: boilerplate filter handles documents < `2 * boundary_scan_lines` without over-removal
