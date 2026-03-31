# Chunk 4 Unit Test Review Findings

Reviewed scope:
- `crates/harvester_engine/tests/extract_convert.rs`
- `crates/harvester_engine/tests/extraction_pipeline_integration.rs`
- `crates/harvester_engine/tests/content_prep_integration.rs`
- `crates/harvester_engine/tests/triage_loader_integration.rs`
- `crates/harvester_engine/tests/briefing_loader_integration.rs`
- `crates/harvester_engine/tests/fetch.rs`
- `crates/harvester_engine/tests/persist.rs`
- `crates/harvester_engine/tests/output.rs`
- `crates/harvester_engine/tests/converter_links.rs`
- `crates/harvester_engine/tests/frontmatter_security.rs`
- `crates/harvester_engine/tests/security.rs`
- `crates/harvester_engine/tests/path_policy.rs`
- `crates/harvester_engine/tests/url_policy.rs`
- `crates/harvester_engine/tests/quota.rs`

Review standard:
- prefer reducer behavior
- prefer emitted effects
- prefer public contracts over internal details
- avoid literal-constant and implementation-detail assertions unless they defend a real external contract

## Findings

### 1. `content_prep_integration.rs` locks normalization internals through exact hash and diagnostic literals

**Files:** `crates/harvester_engine/tests/content_prep_integration.rs:109-123`, `crates/harvester_engine/tests/content_prep_integration.rs:157-163`

Two assertions in this suite reach past the durable pipeline behavior:
- `pipeline_cleans_normalizes_and_hashes` requires `boilerplate_patterns` to contain a token with `"cookie"`
- `determinism_lock_in_hash_constant` requires the exact hash `58ccfdbffd99b383a3159c4393430d8485687283547e9dfb8aed9e2da2b6e445`

The stronger contract is:
- boilerplate is removed
- the content hash is deterministic
- prepared input respects budgets
- the same cleaned content yields the same hash

The exact diagnostic pattern text and exact hash output are implementation details unless they are intentionally part of a compatibility boundary.

**Recommendation:** Keep the assertions on removal, determinism, and budget behavior. Drop the exact hash golden and relax the diagnostic-pattern assertion unless downstream tooling depends on those exact strings.

### 2. `briefing_loader_integration.rs` over-asserts the internal collection string format

**Files:** `crates/harvester_engine/tests/briefing_loader_integration.rs:58-60`, `crates/harvester_engine/tests/briefing_loader_integration.rs:201-210`, `crates/harvester_engine/tests/briefing_loader_integration.rs:329-333`

Several tests verify article selection through exact collection-text markers such as:
- `collection.contains("--- Article 1")`
- `collection.matches("--- Article").count()`
- `collection.contains("--- Article 1: Title 0 ---")`

That makes the tests depend on the current serialization shape of the LLM input collection rather than on the behavior the loader is supposed to protect:
- which articles were selected
- whether ordering is preserved
- whether budget trimming keeps the head and drops the tail

The loaded article list is already available in the API, so exact prompt-collection delimiters are a weak boundary for most of these tests.

**Recommendation:** Prefer assertions over loaded article URLs, order, and count. If collection formatting itself needs coverage, isolate that into a narrower formatting test rather than using it as the main evidence in loader integration tests.

### 3. `fetch.rs` pins exact browser-header literals rather than the fetch policy behavior

**Files:** `crates/harvester_engine/tests/fetch.rs:380-399`

`sends_browser_headers` checks the exact `accept` and `accept-language` header strings, including the precise q-values and locale ordering.

The meaningful contract is that the fetcher sends browser-like headers suitable for normal HTML retrieval. The exact literal header values are likely to change during harmless tuning, and these tests will fail even if behavior remains correct.

If the exact strings are truly important for compatibility with hostile sites, that should be treated as an explicit policy contract. Otherwise this test is too tightly coupled to the current implementation.

**Recommendation:** Assert the presence of the key headers and the essential semantic tokens, or centralize the exact header values behind named constants and treat those constants as the deliberate contract.

### 4. `output.rs` validates manifest JSON through raw substring matches instead of the manifest contract

**Files:** `crates/harvester_engine/tests/output.rs:86-88`, `crates/harvester_engine/tests/output.rs:102-104`, `crates/harvester_engine/tests/output.rs:129-136`

The export tests read the manifest file and then assert raw JSON substrings like:
- `"doc_count":2`
- `"total_tokens":5`
- `"url":"https://root"`

The manifest is a persisted public artifact, so the contract is the data shape and values, not the exact whitespace or serialization layout of the JSON text.

These tests would create noise if the manifest writer changed formatting, ordering, or pretty-printing while keeping the same manifest data.

**Recommendation:** Parse the manifest JSON and assert the semantic fields instead of raw string snippets.

## Keep As-Is

These suites are mostly aligned with the preferred review standard:
- `crates/harvester_engine/tests/extract_convert.rs`
- most of `crates/harvester_engine/tests/extraction_pipeline_integration.rs`
- most of `crates/harvester_engine/tests/triage_loader_integration.rs`
- most of `crates/harvester_engine/tests/fetch.rs` outside the exact header literals
- `crates/harvester_engine/tests/persist.rs`
- most of `crates/harvester_engine/tests/output.rs` outside the raw-manifest substring checks
- `crates/harvester_engine/tests/converter_links.rs`
- `crates/harvester_engine/tests/frontmatter_security.rs`
- `crates/harvester_engine/tests/security.rs`
- `crates/harvester_engine/tests/path_policy.rs`
- `crates/harvester_engine/tests/url_policy.rs`
- `crates/harvester_engine/tests/quota.rs`

Why:
- they mainly test public pipeline outputs, persisted artifacts, security boundaries, retry outcomes, or policy enforcement
- most assertions defend observable semantics like extracted content, selected URLs, path confinement, quota blocking, or failure categories
- the strongest suites already focus on integration behavior instead of helper internals

## Follow-Up Actions For This Chunk

- Remove or relax exact normalization-hash and diagnostic-token assertions in `content_prep_integration.rs`.
- Rework `briefing_loader_integration.rs` tests so loader behavior is asserted via selected articles and order, not collection marker strings.
- Relax `fetch.rs` header assertions unless exact browser-header literals are intentionally policy-locked.
- Parse export manifests as JSON in `output.rs` tests instead of asserting raw string fragments.
