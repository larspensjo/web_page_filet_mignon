# Review: Multi-Step Briefing Stream Plan

Reviewed:
- `docs/plans/Plan.MultiStepBriefingStream.md`
- `docs/plans/Design.MultiStepBriefingStream.md`

## Summary

The design is directionally sound: it keeps briefing generation as reducer-driven state, separates the executive summary call from iterative item calls, and avoids adding Harvester-specific behavior to CommanDuctUI. The plan is also detailed enough to implement incrementally.

The main issues to resolve before implementation are:
- the new direct stream-generation path bypasses existing prompt/context/metadata hydration;
- the prompt-cache prefix invariant depends on `HashMap` iteration order once the shared context has more than one variable;
- next-item in-flight work is not clearly represented in the active-work and visible status paths;
- a few designed snapshot fields and contract updates are not fully carried through the plan.

## Findings

### 1. Blocking: Direct stream generation bypasses prompt hydration

Task 3.4 rewrites `handle_generate_clicked` to build a snapshot and emit `Effect::RequestLlmCompletion` directly for `PromptId::BriefingExecutiveSummary`. That loses an important behavior from the existing generate flow in `crates/harvester_core/src/update/briefing.rs`.

The current `begin_briefing_article_load` path emits:
- `LoadPromptContexts`
- `LoadPromptTemplateFiles`
- `LoadLlmMetadata`
- article loading effects

That path guarantees contexts, saved prompt templates, active prompt versions, and model metadata are hydrated before the summarization request is dispatched.

The proposed stream path calls:

```rust
state.context_for(PromptId::BriefingExecutiveSummary)
```

and then dispatches the LLM effect immediately. Startup currently loads prompt contexts, LLM metadata, model catalog, briefing history, and checkpoints, but does not load prompt template files. Saved prompt overlays are therefore usually loaded by the old generate path or explicit Prompt Lab paths. With the new stream path, a first Generate click can miss saved templates or model/version metadata.

The existing `try_start_briefing_with_metadata` only dispatches when the phase is `Summarizing`, so it will not resume a pending stream snapshot after metadata arrives.

Recommendation:
- Add an explicit pre-dispatch hydration step for the stream flow.
- If context/template/metadata are not ready, freeze or defer the snapshot intentionally, emit `LoadPromptContexts`, `LoadPromptTemplateFiles`, and `LoadLlmMetadata`, then resume executive-summary dispatch after all required data is available.
- Add a dedicated pending stream-generation state if that is clearer than reusing the legacy `Summarizing` path.

Suggested regression tests:
- First Generate click with empty metadata emits load effects and no LLM request.
- After `PromptContextsLoaded` and `LlmMetadataLoaded`, executive-summary dispatch uses the aggregate briefing context.
- Saved prompt template overlays affect stream prompts, or the plan explicitly documents that stream generation uses only static prompt files.

### 2. High: Cache-prefix determinism depends on `HashMap` order

The design requires `BriefingExecutiveSummary` and `BriefingNextItem` to share a byte-identical context prefix for prompt caching. The plan currently relies on the aggregate context file having exactly one variable:

> Aggregate context has exactly one key; with one key order is deterministic.

That is fragile. Runtime context loading in `crates/harvester_io/src/effect_runner/dispatch.rs` currently parses TOML into a `HashMap` and then does:

```rust
ctx_file.variables.into_iter().collect()
```

Once the shared context has more than one variable, two loads of the same file into separate `HashMap`s can produce different vector ordering because each map has randomized seeding. `SavePromptContextFile` sorts variables before serializing, but `LoadPromptContexts` does not sort after parsing.

This matters because Prompt Lab can save arbitrary variables. A comment saying to sort later is not enough for a cache-prefix invariant.

Recommendation:
- Sort context variable pairs at load time, or sort in the shared `TemplateVars`/rendering path.
- Apply this to all prompt IDs so rendering is deterministic regardless of TOML parser and map order.
- Add a regression test with a multi-variable shared context file proving both stream prompts render identical context blocks.

### 3. High: Next-item in-flight work is not fully represented in UI/status state

The design says the progress line should show `Fetching next item...` while a next-item call is in flight. Task 3.2 changes `BriefingSession::progress_text`, but the visible operation/status line appears to come from `build_operation_progress`, which currently handles source polling, triage, summarizing, signal candidate work, poll pipeline work, and pre-triage work.

Several active-work paths currently match only the legacy briefing phases:
- `crates/harvester_core/src/state/ui_state.rs::stop_finish_button_state` treats `LoadingArticles`, `Summarizing`, and `GeneratingBriefing` as active work.
- `crates/harvester_core/src/state/view_builder.rs::format_briefing_preview_header` has an exhaustive match on `BriefingPhase` and will need explicit behavior for `Streaming`.

If `next_item_request_id` is set while the phase remains `Streaming`, the Stop/Finish button and visible status can incorrectly look idle unless these paths are updated.

Recommendation:
- Add a helper such as `BriefingSession::next_item_in_flight()` or `BriefingSession::has_active_llm_request()`.
- Use it in stop/finish button state, preview header formatting, and the visible operation/status rendering path.
- Add reducer/view-model tests for a streaming session with `next_item_request_id: Some(_)`.

### 4. Medium: Snapshot budget check omits separator bytes

Task 2.1's snapshot builder draft checks:

```rust
if !text.is_empty() && text.len() + entry.len() > budget_bytes {
    truncated = true;
    break;
}
if !text.is_empty() {
    text.push_str("\n\n");
}
text.push_str(&entry);
```

The check ignores the two separator bytes. If `text.len() + entry.len() == budget_bytes`, the final snapshot exceeds the budget after adding `"\n\n"`.

Recommendation:
- Compute the candidate length with the separator included:

```rust
let separator_len = if text.is_empty() { 0 } else { 2 };
if text.len() + separator_len + entry.len() > budget_bytes {
    truncated = true;
    break;
}
```

- Add an exact-fit regression test.

### 5. Medium: Truncation and dropped counts are designed but not surfaced

The design says `BriefingSnapshot` has `dropped_count` and `truncated`, and that included/skipped/dropped counts are surfaced in Session Info.

The plan's Task 2.1 includes `dropped_count` and `truncated`, and Task 3.4 logs `dropped_count`. But Task 3.1 only stores:
- `snapshot_included_count`
- `snapshot_skipped_count`

Task 3.2 then renders only:

```text
Sources: N article summaries (S skipped: no summary)
```

That means the user will not see when the snapshot was truncated or when sources were dropped because of the byte budget.

Recommendation:
- Store `snapshot_dropped_count` and `snapshot_truncated` in `BriefingSession`.
- Pass them through `BriefingSession::start_stream`.
- Include them in Session Info, ideally only when `dropped_count > 0` or `truncated` is true.
- Add tests for the rendered session info.

### 6. Medium: Test fixture location in Task 2.3 is invalid or underspecified

Task 2.3 proposes:

```rust
let state = crate::state::tests_support::briefed_state_with_duplicate_corpus();
```

There is no obvious `crate::state::tests_support` module today. The existing update test helpers live under `crates/harvester_core/src/update/tests/support.rs` and are `pub(super)`, so they are not available to an inline test in `state/briefing_snapshot_access.rs`.

Recommendation:
- Put the AppState assembly test in the update test area where the existing support helpers are available; or
- create a small local fixture inside `state/briefing_snapshot_access.rs`; or
- add an actual crate-visible `#[cfg(test)]` fixture module and name it explicitly in the plan.

### 7. Low: Prompt-id contract updates should be called out in Phase 1

Adding `BriefingExecutiveSummary` and `BriefingNextItem` affects more than the enum, prompt files, and main registry.

Likely explicit-list updates include:
- `docs/PromptContextFiles.md`, which lists known prompt context files and valid IDs.
- `crates/harvester_engine/src/llm/prompt_context.rs`, whose error text lists valid prompt IDs.
- `crates/harvester_engine/tests/llm_prompt.rs`, which has prompt-id round-trip and default registry contract tests.

Recommendation:
- Add a small Phase 1 task for prompt-id docs, error text, and shared contract tests.

### 8. Low: Completion-routing prose and snippet disagree

Task 3.6 says to add next-item routing before the existing briefing branch, but the snippet checks the executive-summary request first and the next-item request second.

That is probably harmless because `is_briefing_request` checks only `briefing_request_id`, but the mismatch can confuse implementation.

Recommendation:
- Align the prose and snippet.
- Prefer naming the two branches explicitly as executive-summary completion and next-item completion.

## Suggested Plan Adjustments

Before implementation, update the plan with these changes:

1. Add a stream-generation hydration/resume step before the executive-summary LLM request.
2. Make prompt context rendering deterministic by sorting context variables after loading or before rendering.
3. Add explicit active-work/view-model handling for `Streaming` with `next_item_request_id`.
4. Fix the snapshot byte-budget calculation to include separator bytes.
5. Carry `snapshot_dropped_count` and `snapshot_truncated` into `BriefingSession` and Session Info.
6. Replace the invalid Task 2.3 fixture reference with a concrete test location.
7. Include prompt-id documentation, error text, and shared contract test updates in Phase 1.
8. Clean up the completion-routing wording.

