# Chunk 7 Unit Test Review Findings

Reviewed scope:
- `crates/harvester_io/src/effect_runner.rs`
- `crates/harvester_io/src/effect_helpers.rs`
- `crates/harvester_io/src/persistence.rs`
- `crates/harvester_io/src/persistence_worker.rs`
- `crates/harvester_io/src/entity_index_store.rs`
- `crates/harvester_io/src/runtime_paths.rs`
- `crates/harvester_io/src/seen_set_store.rs`
- `crates/harvester_io/src/source_loader.rs`
- `crates/harvester_io/src/summary_cache_store.rs`
- `crates/harvester_io/src/triage_cache_store.rs`
- `crates/harvester_app/src/platform/app.rs`
- `crates/harvester_app/src/platform/source_loader.rs`
- `crates/harvester_app/src/platform/seen_set_store.rs`
- `crates/harvester_app/src/platform/ui/layout.rs`
- `crates/harvester_app/src/platform/ui/markdown_to_rtf.rs`
- `crates/harvester_app/src/platform/ui/render.rs`
- `crates/harvester_batch/src/cli.rs`
- `crates/harvester_batch/src/lock.rs`
- `crates/harvester_batch/src/runner.rs`

Review standard:
- prefer reducer behavior
- prefer emitted effects
- prefer public contracts over internal details
- avoid literal-constant and implementation-detail assertions unless they defend a real external contract

## Findings

### 1. `effect_runner.rs` save and archive tests over-assert file representation details

**Files:** `crates/harvester_io/src/effect_runner.rs:1823-1846`, `crates/harvester_io/src/effect_runner.rs:1931-1958`, `crates/harvester_io/src/effect_runner.rs:2210-2256`

These tests are strongest when they stay at the effect boundary:
- an effect is enqueued
- the expected success or failure `Msg` is emitted
- the saved artifact can be loaded back correctly

The weaker assertions pin details that are not the main behavior under review:
- exact initial `version == 1` on first save
- raw template file substrings like `system {{context}}`
- exact YAML quoting style in the generated archive frontmatter

Those checks will fail on harmless serialization or version-seeding changes even if the effect still saves, reloads, and dispatches the right message.

**Recommendation:** Keep the emitted-message assertions and the round-trip load checks. Replace exact serialization checks with semantic checks like “selected URL archived”, “unselected URL excluded”, and “saved template/context reloads with the expected fields”.

### 2. `markdown_to_rtf.rs` freezes the current serializer output instead of the viewer contract

**Files:** `crates/harvester_app/src/platform/ui/markdown_to_rtf.rs:228-287`

This file has several low-stability tests:
- exact surrogate-pair encoding fragments
- exact font-size control words like `\\fs36`
- exact bullet control sequences
- a full-document snapshot in `snapshot_sample_briefing`

The stable contract here is that markdown content renders with the expected visible structure in the RTF viewer: headings remain headings, emphasis survives, lists remain lists, Unicode is preserved, and the produced document is valid RTF.

The exact control-word sequence and whole-document string are serializer internals. A safe refactor of the RTF generator could preserve rendered behavior and still break these tests immediately.

**Recommendation:** Rewrite these around semantic rendering properties and minimal structural markers. Keep only the assertions that defend actual viewer compatibility.

### 3. `render.rs` contains copy-locking tests that assert current label wording rather than render behavior

**Files:** `crates/harvester_app/src/platform/ui/render.rs:2962-2980`, `crates/harvester_app/src/platform/ui/render.rs:3138-3167`, `crates/harvester_app/src/platform/ui/render.rs:3429-3473`

Most render tests in this file are good adapter-boundary checks: they assert enabled state, control selection, idempotence, and emitted platform commands. A smaller set is weaker because it locks in the current copy or exact status-line phrasing:
- template error label tests assert exact validation strings like `missing {{context}}`
- triage-results placeholder tests assert the literal phrase `no triage results yet`
- LLM usage status tests assert exact formatted output strings

Those are user-visible strings, but they are still presentation details rather than the main adapter behavior. Minor copy edits or formatting cleanup would cause churn without changing the actual render logic.

**Recommendation:** Keep the command-target assertions, but relax string checks to semantic content:
- the template status label includes all validation messages
- the empty triage state is signaled
- usage status includes model usage information and collapse behavior when the row count exceeds the limit

### 4. `runner.rs` mixes solid boundary tests with low-value exact logging-format tests

**Files:** `crates/harvester_batch/src/runner.rs:1695-1710`, `crates/harvester_batch/src/runner.rs:2014-2036`

The batch runner has valuable tests around dispatch-loop behavior and batch outcome classification. The weaker tests are the helper-level ones that pin exact log or summary strings:
- `test_summarize_batch_msg_compacts_large_payloads`
- `test_truncate_for_log_appends_ellipsis`
- `format_llm_usage_lines_sorted_and_stable`

These are mostly about current string presentation, not reducer behavior, effect behavior, or a strong CLI contract. `format_llm_usage_lines_sorted_and_stable` also uses concrete OpenAI model constants where synthetic model ids would cover the same sorting and formatting behavior with less churn.

**Recommendation:** Keep only the behaviorally meaningful parts:
- large payloads are summarized rather than dumped
- truncation preserves the prefix and marks truncation
- usage lines are sorted and compacted

Avoid pinning the exact literal message text unless the CLI intentionally treats those strings as a stable user-facing contract.

## Keep As-Is

These areas are mostly aligned with the preferred review standard:
- most of `crates/harvester_io/src/effect_helpers.rs`
- `crates/harvester_io/src/persistence.rs`
- `crates/harvester_io/src/persistence_worker.rs`
- `crates/harvester_io/src/entity_index_store.rs`
- `crates/harvester_io/src/runtime_paths.rs`
- `crates/harvester_io/src/seen_set_store.rs`
- `crates/harvester_io/src/source_loader.rs`
- `crates/harvester_io/src/summary_cache_store.rs`
- `crates/harvester_io/src/triage_cache_store.rs`
- most of `crates/harvester_app/src/platform/app.rs`
- `crates/harvester_app/src/platform/source_loader.rs`
- `crates/harvester_app/src/platform/seen_set_store.rs`
- most of `crates/harvester_app/src/platform/ui/layout.rs`
- `crates/harvester_batch/src/cli.rs`
- `crates/harvester_batch/src/lock.rs`
- most of `crates/harvester_batch/src/runner.rs` outside log-format helper tests

Why:
- they mainly test emitted messages, persisted round trips, path handling, CLI parsing, lock lifecycle, or stable adapter behavior
- most assertions sit on real boundaries rather than private helper structure
- the stronger tests in this chunk already protect effect execution outcomes or public contracts

## Follow-Up Actions For This Chunk

- Rewrite `effect_runner.rs` save/archive tests around emitted messages and round-trip semantics, not raw serialization details.
- Replace `markdown_to_rtf.rs` string snapshots with minimal structural assertions tied to rendered meaning.
- Relax `render.rs` copy-level checks to semantic UI-state assertions.
- Trim `runner.rs` helper tests down to sorting, truncation, and compaction behavior rather than exact log wording.
