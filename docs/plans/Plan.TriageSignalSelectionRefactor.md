# Triage Signal Selection Refactor Plan

## Context

Commit `d9e0e49e613da062aa92c43b7483cacd85c3875b` changed article summaries and aggregate briefings from thesis-confirmation prompts to neutral business-signal detection. The triage process still uses older thesis-weighted language in both:

- `contexts/article_triage.toml`
- `crates/harvester_engine/src/llm/prompts/triage.rs`

That creates a front-door mismatch. Summary and briefing now ask the model to surface business-significant change, counter-signals, and commercially meaningful product/platform updates, but triage can still suppress articles before they reach those later stages because the current scoring rules prefer thesis-confirming infrastructure, SaaS deflation, and advertising-reset signals.

There is also a technical mismatch: `TRIAGE_PROMPT_V3` does not include `{{context}}`, so `contexts/article_triage.toml` is not reaching the triage model today. The V3 analytical framework is hardcoded in the built-in prompt. Adding `{{context}}` in V4 is therefore not just a wording change; it is the first triage prompt version that will consume the editable TOML context at runtime.

## Goal

Refactor article triage so it admits a wider, more useful corpus for an AI-focused portfolio analyst. Triage should score for business-significant selection value, not for confirmation of a fixed investment thesis.

The desired behavior is:

- Let through articles that could matter commercially or strategically.
- Include evidence that strengthens, weakens, or complicates existing assumptions.
- Treat major product, platform, model, pricing, distribution, policy, partnership, capex, adoption, and competitive changes as first-class signals.
- Preserve enough low-friction filtering to keep generic hype, listicles, minor UX updates, and low-signal PR from crowding the corpus.

## Non-Goals

- Do not change the triage JSON schema in the first implementation slice.
- Do not add Harvester-specific behavior to `CommanDuctUI`.
- Do not change reducer orchestration unless prompt-version rollout exposes a concrete bug.
- Do not remove historical prompt versions.
- Do not change the `PromptId` enum or the context-hash computation.
- Do not rebuild the entire archive automatically.
- Do not add a `harvester_batch` CLI flag in the initial prompt-only slice.

## Current State

### Prompt Context

`contexts/article_triage.toml` currently says:

- `STRATEGIC MANDATE: "The Great Bifurcation"`
- prioritize confirmation of resource-grab, physical-wall, and SaaS-deflation themes
- round up when a core holding is mentioned
- round down product/feature releases
- cap high scores unless hard numbers are present

This conflicts with the current summary and briefing contexts, which treat prior theses as background rather than preferred conclusions.

Because V3 does not reference `{{context}}`, changing this TOML alone has no runtime effect. Slice 1 must ship with Slice 2 before triage behavior changes.

### Built-In Prompt

`TRIAGE_PROMPT_V3` currently frames the task around:

- AI Super-Cycle resource grab
- SaaS deflation
- advertising reset
- entity escalation for a fixed list
- down-ranking generic software updates

The schema is stable and should remain:

```json
{
  "category": "string",
  "priority": 1,
  "tags": ["string"],
  "rationale": "string"
}
```

### Cache Behavior

Triage cache keys already include:

- content hash
- prompt id
- prompt version
- model id
- context hash

Activating a new triage prompt version and updating `contexts/article_triage.toml` will naturally make old cached triage results miss for the new scoring policy. This gives a clean rollout path without manual cache invalidation.

## Recommended Implementation

### Slice 1 - Neutralize Triage Context

Update `contexts/article_triage.toml`:

- bump `version` from `1` to `2`
- set `updated` to the implementation date
- change the description to business-significant triage selection
- replace thesis-confirming instructions with neutral selection guidance

Suggested context structure:

- `TRIAGE OBJECTIVE`
- `ANALYST FOCUS`
- `ADMISSION PRIORITIES`
- `PRIORITY SCALE`
- `SIGNALS TO ADMIT`
- `DOWN-RANK GUIDANCE`
- `TIE-BREAKER LOGIC`
- `TAG GUIDANCE`

`ANALYST FOCUS` should anchor the task on AI and adjacent markets without reintroducing thesis confirmation:

- Focus on AI and its commercial ecosystem: foundation-model labs, AI chips and compute substrate, hyperscaler capex, enterprise AI adoption, AI-driven product/platform changes, developer tooling, data-center supply chains, and policy or regulation shaping AI markets.
- Within this focus, treat supporting and contradicting signals as equally valuable.
- Outside this focus, require stronger commercial or strategic implications before assigning priority 3 or higher.

The priority scale should mean:

- `5`: highly material, specific, decision-relevant development with clear commercial, strategic, regulatory, financial, operational, or competitive implications.
- `4`: strong business signal that may affect portfolio assumptions, enterprise adoption, developer behavior, pricing power, distribution, capex, supply, demand, or competitive position.
- `3`: potentially relevant signal, early evidence, notable product/platform update, competitor move, or useful context that deserves review but lacks enough specificity for priority 4 or 5.
- `2`: background, commentary, minor update, or weakly actionable context.
- `1`: generic hype, listicle, low-signal PR, consumer gadget coverage without market-structure implications, or advice content with no new data.

Important wording changes:

- Treat prior theses as background, not as preferred conclusions.
- Do not over-weight articles because they fit existing assumptions.
- Round up when the article contains concrete business impact, novelty, contradiction, named actors, dates, numbers, or strategic optionality.
- Do not automatically round down major product/platform/model releases; score them by commercial impact.
- Treat a major foundation-model release, hyperscaler platform launch, or developer-tool repricing as a priority-4 candidate when enterprise adoption, workflow change, distribution, pricing power, or competitive position is implicated.
- Prefer at least one of named actor, date, number, or specific commercial mechanism before scoring above 3, but do not require hard financial metrics for every high-value product or strategy signal.
- If implications are ambiguous, use the rationale to say what is unknown.

### Slice 2 - Add `TRIAGE_PROMPT_V4`

Add `TRIAGE_PROMPT_V4` to `crates/harvester_engine/src/llm/prompts/triage.rs`.

Design constraints:

- Keep the exact existing schema.
- Keep document-instruction isolation language.
- Include `BACKGROUND CONTEXT:\n{{context}}` so `article_triage.toml` is explicitly treated as input framing.
- Say the context is optional framing, not a thesis to confirm.
- Define priority as "selection value for business-signal review."
- Ask the model to explain both inclusion and down-ranking decisions in `rationale`.
- Keep tags short, stable, and useful for search.

Implementation checks:

- Verify the `{{context}}` render path is wired end-to-end for `ArticleTriage` in `crates/harvester_engine/src/llm/handle.rs`. The shared render path currently builds a `context` template variable for all prompt IDs; keep that behavior covered.
- Verify `crates/harvester_engine/src/llm/template_validation.rs` accepts `{{context}}` for `ArticleTriage`. It currently does through `synthetic_vars`; keep or add a test that locks this contract.
- Verify `crates/harvester_engine/src/llm/prompt_context.rs` continues to accept `ArticleTriage` context files.

Recommended prompt description:

`Business-significant triage with neutral signal admission and thesis-challenging evidence`

Add prompt tests in the inline `#[cfg(test)]` block:

- `v4_template_validates_triage_variables`
- `v4_expected_format_preserves_triage_schema`
- a V4 assertion that the system template contains `{{context}}` or `BACKGROUND CONTEXT`

### Slice 3 - Activate the New Prompt Version

Update `crates/harvester_engine/src/llm/prompts/mod.rs`:

- export `TRIAGE_PROMPT_V4 as TRIAGE_PROMPT`
- include `TRIAGE_PROMPT_V4` in the version exports
- register `TRIAGE_PROMPT_V4`
- set active `ArticleTriage` to version 4

Existing tests in `mod.rs` should continue to assert that exported aliases match active defaults.

### Slice 4 - Verify Runtime and Cache Behavior

No reducer changes are expected. Verify the existing flow still holds:

- input article text is loaded by effects
- reducer starts triage and dispatches LLM requests
- LLM completion returns as `Msg::LlmCompleted`
- reducer validates and stores `ArticleTriageResult`
- state renders updated triage results
- side effects persist triage cache and entity index

Expected cache behavior:

- old `TRIAGE_PROMPT_V3` cache entries remain persisted
- new `TRIAGE_PROMPT_V4` runs use a different prompt version
- changed `article_triage.toml` creates a different context hash
- old results are not reused for the new selection policy

### Slice 5 - Documentation and Diary

Update `docs/EngineeringDiary.md` after implementation, not just after planning, with a short note referencing:

- `contexts/article_triage.toml`
- `crates/harvester_engine/src/llm/prompts/triage.rs`
- `crates/harvester_engine/src/llm/prompts/mod.rs`

The commit message should describe the code change, for example:

`Shift triage scoring from thesis confirmation to signal admission`

Do not make the commit message about this plan document.

## Optional Slice - Bounded Stale Triage Refresh

Only add this if normal cache misses are operationally too slow or too expensive to handle opportunistically.

The summary refresh commits show the right pattern:

- load prompt defaults and saved overlays
- load the current context file
- scan archive metadata
- identify content hashes missing the current cache key
- select up to a caller-provided limit
- dispatch bounded LLM requests
- persist cache and entity index
- produce a report with successes, failures, token usage, cost, and selected URLs

For triage, this would likely become:

`harvester_batch --refresh-stale-triage-limit N`

If this CLI flag is added, `Agents.md` requires updating:

- `scripts/Start-HarvesterBatch.ps1`

Suggested report locations:

- `output/triage_refresh_reports/`
- `output/.triage_refresh_last.json`

Exit-code policy should mirror summary refresh:

- `0` for all success
- `0` for partial success with persisted useful work
- non-zero for total failure or dispatch failure

This optional slice should be a separate change from the prompt refactor unless there is a strong operational reason to combine them.

## Test Plan

Run after Slice 1-3 implementation:

```powershell
cargo test -p harvester_engine --test llm_prompt
cargo test -p harvester_engine --quiet
cargo test -p harvester_core --quiet
cargo clippy --all-targets -- -D warnings
cargo fmt
```

If only prompt constants and context files change, the highest-value tests are:

- prompt registry tests
- template validation tests
- LLM prompt rendering tests
- triage cache reuse tests, if any failure suggests cache-key drift

If the optional stale-triage refresh CLI is implemented, also test:

```powershell
cargo test -p harvester_batch --quiet
Invoke-Pester -Path 'scripts/tests/HarvesterLauncher.Tests.ps1'
```

## Rollout Notes

- Existing triage cache entries should not be deleted.
- The first run after activation should show triage cache misses for articles without a V4/context-v2 cache key.
- That first run can be materially more expensive and slower because every in-scope article without a V4/context-v2 key must be re-triaged.
- If the prompt lets in too much background noise, tune the down-rank guidance before adding schema complexity.
- If priority scores are too compressed around 3, sharpen the distinction between priority 3 and 4.
- After activation, inspect roughly 20 real triage outputs before tuning again; score drift is easier to diagnose from actual selected and rejected articles than from predicted prompt behavior.
- If product/platform launches are still under-selected, add examples that focus on enterprise adoption, developer workflow, distribution, pricing, bundling, and competitive position.

## Acceptance Criteria

- `ArticleTriage` context no longer instructs the model to confirm "The Great Bifurcation."
- `ArticleTriage` context instructs the model to select business-significant change in AI and adjacent markets, including supporting and contradicting signals.
- Active `ArticleTriage` prompt consumes `{{context}}` at runtime.
- Built-in active triage prompt is version 4.
- Prompt registry keeps versions 1-3 addressable.
- Triage output schema remains unchanged.
- Cache keys roll forward through prompt version and context hash.
- Tests pass.
- Diary entry records the implementation when the code change is made.
