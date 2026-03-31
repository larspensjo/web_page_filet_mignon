# Chunk 6 Unit Test Review Findings

Reviewed scope:
- `crates/harvester_engine/tests/llm_handle.rs`
- `crates/harvester_engine/tests/llm_mock.rs`
- `crates/harvester_engine/tests/llm_openai.rs`
- `crates/harvester_engine/tests/llm_pricing.rs`
- `crates/harvester_engine/tests/llm_prompt.rs`
- `crates/harvester_engine/tests/llm_quota.rs`
- `crates/harvester_engine/tests/llm_replay.rs`
- `crates/harvester_engine/tests/llm_types.rs`
- `crates/harvester_engine/tests/llm_validation.rs`
- `crates/harvester_engine/tests/prompt_context.rs`
- `crates/harvester_engine/src/llm/handle.rs`
- `crates/harvester_engine/src/llm/prompt.rs`
- `crates/harvester_engine/src/llm/prompt_context.rs`
- `crates/harvester_engine/src/llm/providers/openai.rs`
- `crates/harvester_engine/src/llm/run_metadata.rs`
- `crates/harvester_engine/src/llm/template_validation.rs`
- `crates/harvester_engine/src/llm/validation.rs`
- `crates/harvester_engine/src/llm/prompts/mod.rs`
- `crates/harvester_engine/src/llm/prompts/briefing.rs`

Review standard:
- prefer reducer behavior
- prefer emitted effects
- prefer public contracts over internal details
- avoid literal-constant and implementation-detail assertions unless they defend a real external contract

## Findings

### 1. `llm_pricing.rs` is coupled to live default model IDs and exact default prices

**Files:** `crates/harvester_engine/tests/llm_pricing.rs:7-12`, `crates/harvester_engine/tests/llm_pricing.rs:56-71`

The pricing tests do two different jobs:
- good arithmetic tests using synthetic rates and usages
- catalog-locking tests against the current OpenAI default registry

The weaker assertions are:
- `default_pricing_matches_expected_costs` requiring exact `30_000` microdollars for `OPENAI_MODEL_GPT_5_4_MINI`
- `default_registry_contains_gpt54_nano`
- `default_registry_contains_gpt54_family`

Those tests will fail whenever the default catalog or price table changes, even if the pricing logic remains correct. That is a moving policy/copy problem, not a stable unit-contract problem.

**Recommendation:** Keep the arithmetic tests. Move current-catalog coverage to a narrower “pricing table snapshot” test only if the repo intentionally treats the shipped model/price defaults as a versioned product contract.

### 2. Prompt registry tests pin exact active versions and version counts instead of registry behavior

**Files:** `crates/harvester_engine/tests/llm_prompt.rs:73-95`, `crates/harvester_engine/src/llm/prompts/mod.rs:45-76`

These tests assert exact numbers like:
- active triage version `3`
- active briefing version `7`
- summary version count `4`
- aggregate briefing version count `7`

That freezes the current registry contents rather than the more durable contract:
- defaults register at least one template for each prompt id
- the active template is the latest registered default
- older versions remain addressable when intended

The exact counts and active-version integers are expected to change as prompts evolve.

**Recommendation:** Assert semantic registry behavior and accessibility of intentionally preserved older versions. Avoid hard-coding the total number of prompt revisions unless that count itself matters to a migration contract.

### 3. `prompts/briefing.rs` tests mostly freeze prompt wording and revision numbers

**Files:** `crates/harvester_engine/src/llm/prompts/briefing.rs:214-265`

The briefing prompt tests check:
- exact slot names like `{{previous_briefings}}`
- exact wording like `NEW or CHANGED`
- exact version integers `5`, `6`, and `7`
- exact phrasing like `"150 words or fewer"`

Some of this may be justified for prompt-authoring discipline, but as unit tests they are tightly coupled to prompt text edits. A prompt rewrite that preserves the real contract could still fail these tests immediately.

The durable boundary is usually:
- required variables are present and validated
- the prompt shape matches the expected response contract
- the active prompt can render successfully with the intended variables

**Recommendation:** Keep only the assertions that defend required variables or schema-level response requirements. Move copy-level prompt review to explicit prompt QA or snapshot review if needed.

### 4. OpenAI model-list tests and dated-snapshot helper tests are coupled to the current catalog

**Files:** `crates/harvester_engine/tests/llm_openai.rs:267-342`, `crates/harvester_engine/src/llm/providers/openai.rs:387-421`

Two places lean too hard on current model names:
- `list_models_filters_to_chat_models_only` hard-codes a large allow/deny list of current OpenAI IDs
- `is_dated_snapshot` helper tests use many concrete historical model names

The real provider contract is the filtering behavior:
- dated snapshots are excluded
- non-chat/audio/image/realtime/search/instruct models are excluded
- eligible rolling chat models remain

Pinning specific current IDs makes the suite noisy whenever the provider catalog shifts, which is likely.

**Recommendation:** Use synthetic IDs that represent the relevant categories, or reduce assertions to category behavior. Keep a small number of representative real-model fixtures only where compatibility with a known naming pattern matters.

## Keep As-Is

These suites and modules are mostly aligned with the preferred review standard:
- most of `crates/harvester_engine/tests/llm_handle.rs`
- `crates/harvester_engine/tests/llm_mock.rs`
- most of `crates/harvester_engine/tests/llm_openai.rs` outside current-catalog filtering
- most of `crates/harvester_engine/tests/llm_pricing.rs` outside default-catalog checks
- most of `crates/harvester_engine/tests/llm_prompt.rs` outside registry version/count checks
- `crates/harvester_engine/tests/llm_quota.rs`
- `crates/harvester_engine/tests/llm_replay.rs`
- `crates/harvester_engine/tests/llm_types.rs`
- `crates/harvester_engine/tests/llm_validation.rs`
- `crates/harvester_engine/tests/prompt_context.rs`
- most of `crates/harvester_engine/src/llm/handle.rs`
- `crates/harvester_engine/src/llm/prompt.rs`
- `crates/harvester_engine/src/llm/prompt_context.rs`
- most of `crates/harvester_engine/src/llm/providers/openai.rs` outside current-catalog helpers
- `crates/harvester_engine/src/llm/run_metadata.rs`
- `crates/harvester_engine/src/llm/template_validation.rs`
- `crates/harvester_engine/src/llm/validation.rs`

Why:
- they mainly test public LLM outcomes, cache/replay behavior, retry/error categories, typed validation, or prompt rendering semantics
- most assertions sit on stable boundaries like event delivery, serialized records, quota enforcement, or validation outputs
- the strongest tests in this chunk already defend behavior that matters through the public API rather than internal helper structure

## Follow-Up Actions For This Chunk

- Split pricing logic tests from default-catalog snapshot tests in `llm_pricing.rs`.
- Rewrite prompt-registry tests around active/latest/accessibility behavior rather than exact counts.
- Trim briefing prompt tests down to variable/schema contract checks.
- Decouple OpenAI model-filter tests from the live catalog by using synthetic category fixtures or a much smaller representative set.
