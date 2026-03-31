# Chunk 5 Unit Test Review Findings

Reviewed scope:
- `crates/harvester_engine/src/blocker_page.rs`
- `crates/harvester_engine/src/brave_poll.rs`
- `crates/harvester_engine/src/brave_seen_set.rs`
- `crates/harvester_engine/src/frontmatter.rs`
- `crates/harvester_engine/src/import.rs`
- `crates/harvester_engine/src/preview.rs`
- `crates/harvester_engine/src/rss_parse.rs`
- `crates/harvester_engine/src/rss_seen_set.rs`
- `crates/harvester_engine/src/since_filter.rs`
- `crates/harvester_engine/src/source_config.rs`
- `crates/harvester_engine/src/source_poll.rs`
- `crates/harvester_engine/src/text_safety.rs`
- `crates/harvester_engine/src/content_extraction/candidate_select.rs`
- `crates/harvester_engine/src/content_extraction/dom_prune.rs`
- `crates/harvester_engine/src/content_extraction/markdown_cleanup.rs`
- `crates/harvester_engine/src/content_extraction/pipeline.rs`
- `crates/harvester_engine/src/content_prep/boilerplate.rs`
- `crates/harvester_engine/src/content_prep/budget.rs`
- `crates/harvester_engine/src/content_prep/derive.rs`
- `crates/harvester_engine/src/content_prep/normalize.rs`
- `crates/harvester_engine/src/content_prep/truncation.rs`
- `crates/harvester_engine/src/content_prep/types.rs`

Review standard:
- prefer reducer behavior
- prefer emitted effects
- prefer public contracts over internal details
- avoid literal-constant and implementation-detail assertions unless they defend a real external contract

## Findings

### 1. `content_prep/boilerplate.rs` tests lock in diagnostic pattern labels instead of removal behavior

**Files:** `crates/harvester_engine/src/content_prep/boilerplate.rs:268-283`

`detects_cookie_banner_at_end` does two different things:
- it verifies the cookie banner is removed from the filtered text
- it also requires `detected_patterns` to contain exact labels like `"cookie policy"` or `"accept cookies"`

The first part is strong. The second part is weaker because the durable contract is that the banner is detected and removed, not that the detector reports one exact token string from the current pattern list.

Those pattern names are implementation detail and will create noise if the heuristic vocabulary changes while behavior stays correct.

**Recommendation:** Keep the removal assertion. Relax the diagnostic assertion to something behavioral, such as “at least one pattern was recorded,” unless downstream reporting depends on those exact labels.

### 2. `content_prep/budget.rs` tests pin the current collection wrapper format

**Files:** `crates/harvester_engine/src/content_prep/budget.rs:230-238`

`prepared_collection_concatenates_articles` asserts `collection.text().contains("--- Article 1")`.

That freezes the current prompt-assembly wrapper instead of the stronger behavior:
- the collection includes both prepared inputs
- article count is correct
- ordering is preserved

This is the same issue already seen in the briefing loader integration tests, just one level lower in the helper module.

**Recommendation:** Assert item count, relative ordering, and presence of the prepared article content. Only keep the exact wrapper marker if the prompt wrapper format is intentionally a stable contract.

### 3. `content_extraction/candidate_select.rs` overfits the current scoring model and selector priority

**Files:** `crates/harvester_engine/src/content_extraction/candidate_select.rs:176-220`

The candidate-selection tests include helper-level assertions that are tightly coupled to the current implementation:
- `high_link_density_element_gets_penalized` asserts a numeric score relationship derived from the current penalty formula
- `body_fallback_when_no_good_candidates` pins the fallback score to exact `0.0`
- `main_preferred_when_scores_are_similar` depends on the current selector ordering tie-break

These tests are aimed at internal scoring details rather than the public extraction outcome. Safe tuning of the scoring model or candidate priority would break them even if end-to-end extraction quality remained correct.

**Recommendation:** Prefer extraction outcome tests at the pipeline boundary. Keep direct selector/scoring tests only where the exact heuristic rule is intentionally fixed policy.

### 4. `rss_parse.rs` depends on upstream parser error wording

**Files:** `crates/harvester_engine/src/rss_parse.rs:254-265`

`malformed_feed_returns_parse_error` checks that the parse failure reason contains `"unable to parse feed"`.

That message originates from a parsing library and is not the durable contract of this module. The stronger contract is simply that malformed input produces `FeedParseError::ParseFailed`.

This test will create unnecessary churn if the dependency changes its wording while the module still surfaces the same error category correctly.

**Recommendation:** Assert the error variant and, at most, that the reason is non-empty. Do not depend on the exact phrasing from the upstream parser.

## Keep As-Is

These modules are mostly aligned with the preferred review standard:
- `crates/harvester_engine/src/blocker_page.rs`
- `crates/harvester_engine/src/brave_poll.rs`
- `crates/harvester_engine/src/brave_seen_set.rs`
- `crates/harvester_engine/src/frontmatter.rs`
- most of `crates/harvester_engine/src/import.rs`
- `crates/harvester_engine/src/preview.rs`
- most of `crates/harvester_engine/src/rss_parse.rs` outside the parse-error wording check
- `crates/harvester_engine/src/rss_seen_set.rs`
- `crates/harvester_engine/src/since_filter.rs`
- `crates/harvester_engine/src/source_config.rs`
- most of `crates/harvester_engine/src/source_poll.rs`
- `crates/harvester_engine/src/text_safety.rs`
- `crates/harvester_engine/src/content_extraction/dom_prune.rs`
- `crates/harvester_engine/src/content_extraction/markdown_cleanup.rs`
- most of `crates/harvester_engine/src/content_extraction/pipeline.rs`
- most of `crates/harvester_engine/src/content_prep/derive.rs`
- `crates/harvester_engine/src/content_prep/normalize.rs`
- `crates/harvester_engine/src/content_prep/truncation.rs`
- `crates/harvester_engine/src/content_prep/types.rs`

Why:
- they mainly test parsing acceptance/rejection, policy validation, dedup outcomes, truncation behavior, or persisted/public data shapes
- most assertions are about observable semantics rather than helper structure
- the stronger extraction and import tests already sit near stable module boundaries

## Follow-Up Actions For This Chunk

- Relax exact `detected_patterns` assertions in `content_prep/boilerplate.rs`.
- Rewrite `PreparedCollection` tests so they defend ordering/content behavior more than wrapper text.
- Move candidate-selection confidence toward pipeline-outcome tests instead of helper score math.
- Remove dependency on upstream parse-error wording in `rss_parse.rs`.
