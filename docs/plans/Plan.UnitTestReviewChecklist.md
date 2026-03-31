# Unit Test Review Checklist Plan

**Goal:** Review the repo's existing tests against the preferred standard:
- prioritize reducer behavior
- prioritize emitted effects
- prioritize public contracts over internal details
- de-emphasize tests that only pin literal constants unless those literals are externally meaningful

**How to use this plan:**
- Work chunk-by-chunk.
- For each file, review whether the tests primarily protect behavior at a stable boundary.
- Flag tests that would break under safe refactors without a behavior change.
- Flag tests that only restate constants, private helper logic, or exact internal representation.

**Review questions for every chunk:**
- Does this test assert a state transition, emitted effect, or public contract?
- Would the test still pass after an internal refactor with unchanged behavior?
- Is any exact literal being asserted truly part of the external contract?
- Is the test focused on observable semantics rather than private structure?
- Should a helper-level test be replaced by a higher-level boundary test?

## Chunk 1: `harvester_core` reducer regression suites

**Focus:** Highest priority. These tests should be strongest on `Msg -> update -> (State, Effects)`.

- [x] `crates/harvester_core/tests/update_behaviour.rs`
- [x] `crates/harvester_core/tests/update_jobs.rs`
- [x] `crates/harvester_core/tests/update_noop.rs`
- [x] `crates/harvester_core/tests/triage_orchestration.rs`
- [x] `crates/harvester_core/tests/brave_integration.rs`
- [x] `crates/harvester_core/tests/left_tab_scope_integration.rs`
- [x] `crates/harvester_core/tests/persistence.rs`
- [x] `crates/harvester_core/tests/llm_usage.rs`

## Chunk 2: `harvester_core` inline reducer-adjacent domain modules

**Focus:** Keep tests centered on stable domain behavior, not helper structure.

- [x] `crates/harvester_core/src/update.rs`
- [x] `crates/harvester_core/src/state.rs`
- [x] `crates/harvester_core/src/source_state.rs`
- [x] `crates/harvester_core/src/triage.rs`
- [x] `crates/harvester_core/src/pre_triage_coordinator.rs`
- [x] `crates/harvester_core/src/pre_triage_filter.rs`
- [x] `crates/harvester_core/src/working_corpus.rs`
- [x] `crates/harvester_core/src/prompt_lab.rs`

## Chunk 3: `harvester_core` view, briefing, preview, and cache contracts

**Focus:** Public contract and output-shape tests are valid here if the output is user-visible or persisted.

- [x] `crates/harvester_core/src/briefing.rs`
- [x] `crates/harvester_core/src/view_model.rs`
- [x] `crates/harvester_core/src/preview.rs`
- [x] `crates/harvester_core/src/context_draft.rs`
- [x] `crates/harvester_core/src/summary_cache.rs`
- [x] `crates/harvester_core/src/triage_cache.rs`
- [x] `crates/harvester_core/src/cache_utils.rs`
- [x] `crates/harvester_core/src/tabs.rs`
- [x] `crates/harvester_core/src/trends.rs`
- [x] `crates/harvester_core/src/ui_geometry.rs`
- [x] `crates/harvester_core/src/url_age.rs`

## Chunk 4: `harvester_engine` content pipeline and safety integration tests

**Focus:** Public pipeline behavior, persisted output, policy enforcement, and security boundaries.

- [x] `crates/harvester_engine/tests/extract_convert.rs`
- [x] `crates/harvester_engine/tests/extraction_pipeline_integration.rs`
- [x] `crates/harvester_engine/tests/content_prep_integration.rs`
- [x] `crates/harvester_engine/tests/triage_loader_integration.rs`
- [x] `crates/harvester_engine/tests/briefing_loader_integration.rs`
- [x] `crates/harvester_engine/tests/fetch.rs`
- [x] `crates/harvester_engine/tests/persist.rs`
- [x] `crates/harvester_engine/tests/output.rs`
- [x] `crates/harvester_engine/tests/converter_links.rs`
- [x] `crates/harvester_engine/tests/frontmatter_security.rs`
- [x] `crates/harvester_engine/tests/security.rs`
- [x] `crates/harvester_engine/tests/path_policy.rs`
- [x] `crates/harvester_engine/tests/url_policy.rs`
- [x] `crates/harvester_engine/tests/quota.rs`

## Chunk 5: `harvester_engine` inline content and policy modules

**Focus:** Prefer tests of exported behavior and policy outcomes over helper-by-helper assertions.

- [x] `crates/harvester_engine/src/blocker_page.rs`
- [x] `crates/harvester_engine/src/brave_poll.rs`
- [x] `crates/harvester_engine/src/brave_seen_set.rs`
- [x] `crates/harvester_engine/src/frontmatter.rs`
- [x] `crates/harvester_engine/src/import.rs`
- [x] `crates/harvester_engine/src/preview.rs`
- [x] `crates/harvester_engine/src/rss_parse.rs`
- [x] `crates/harvester_engine/src/rss_seen_set.rs`
- [x] `crates/harvester_engine/src/since_filter.rs`
- [x] `crates/harvester_engine/src/source_config.rs`
- [x] `crates/harvester_engine/src/source_poll.rs`
- [x] `crates/harvester_engine/src/text_safety.rs`
- [x] `crates/harvester_engine/src/content_extraction/candidate_select.rs`
- [x] `crates/harvester_engine/src/content_extraction/dom_prune.rs`
- [x] `crates/harvester_engine/src/content_extraction/markdown_cleanup.rs`
- [x] `crates/harvester_engine/src/content_extraction/pipeline.rs`
- [x] `crates/harvester_engine/src/content_prep/boilerplate.rs`
- [x] `crates/harvester_engine/src/content_prep/budget.rs`
- [x] `crates/harvester_engine/src/content_prep/derive.rs`
- [x] `crates/harvester_engine/src/content_prep/normalize.rs`
- [x] `crates/harvester_engine/src/content_prep/truncation.rs`
- [x] `crates/harvester_engine/src/content_prep/types.rs`

## Chunk 6: `harvester_engine` LLM public contracts

**Focus:** Validate provider mapping, prompt contracts, replay/metadata behavior, and typed outputs. Constant-only tests are suspect unless the constant is protocol-facing.

- [x] `crates/harvester_engine/tests/llm_handle.rs`
- [x] `crates/harvester_engine/tests/llm_mock.rs`
- [x] `crates/harvester_engine/tests/llm_openai.rs`
- [x] `crates/harvester_engine/tests/llm_pricing.rs`
- [x] `crates/harvester_engine/tests/llm_prompt.rs`
- [x] `crates/harvester_engine/tests/llm_quota.rs`
- [x] `crates/harvester_engine/tests/llm_replay.rs`
- [x] `crates/harvester_engine/tests/llm_types.rs`
- [x] `crates/harvester_engine/tests/llm_validation.rs`
- [x] `crates/harvester_engine/tests/prompt_context.rs`
- [x] `crates/harvester_engine/src/llm/handle.rs`
- [x] `crates/harvester_engine/src/llm/prompt.rs`
- [x] `crates/harvester_engine/src/llm/prompt_context.rs`
- [x] `crates/harvester_engine/src/llm/providers/openai.rs`
- [x] `crates/harvester_engine/src/llm/run_metadata.rs`
- [x] `crates/harvester_engine/src/llm/template_validation.rs`
- [x] `crates/harvester_engine/src/llm/validation.rs`
- [x] `crates/harvester_engine/src/llm/prompts/mod.rs`
- [x] `crates/harvester_engine/src/llm/prompts/briefing.rs`

## Chunk 7: IO, app boundary, and batch CLI

**Focus:** Effect execution outcomes, persistence contracts, CLI contract, and UI-facing adapter behavior.

- [x] `crates/harvester_io/src/effect_runner.rs`
- [x] `crates/harvester_io/src/effect_helpers.rs`
- [x] `crates/harvester_io/src/persistence.rs`
- [x] `crates/harvester_io/src/persistence_worker.rs`
- [x] `crates/harvester_io/src/entity_index_store.rs`
- [x] `crates/harvester_io/src/runtime_paths.rs`
- [x] `crates/harvester_io/src/seen_set_store.rs`
- [x] `crates/harvester_io/src/source_loader.rs`
- [x] `crates/harvester_io/src/summary_cache_store.rs`
- [x] `crates/harvester_io/src/triage_cache_store.rs`
- [x] `crates/harvester_app/src/platform/app.rs`
- [x] `crates/harvester_app/src/platform/source_loader.rs`
- [x] `crates/harvester_app/src/platform/seen_set_store.rs`
- [x] `crates/harvester_app/src/platform/ui/layout.rs`
- [x] `crates/harvester_app/src/platform/ui/markdown_to_rtf.rs`
- [x] `crates/harvester_app/src/platform/ui/render.rs`
- [x] `crates/harvester_batch/src/cli.rs`
- [x] `crates/harvester_batch/src/lock.rs`
- [x] `crates/harvester_batch/src/runner.rs`

## Chunk 8: `CommanDuctUI` generic infrastructure

**Focus:** Keep tests generic. Remove or rewrite any tests that accidentally encode Harvester-specific assumptions.

- [x] `src/CommanDuctUI/src/app.rs`
- [x] `src/CommanDuctUI/src/command_executor.rs`
- [x] `src/CommanDuctUI/src/types.rs`
- [x] `src/CommanDuctUI/src/window_common.rs`
- [x] `src/CommanDuctUI/src/controls/button_handler.rs`
- [x] `src/CommanDuctUI/src/controls/chart_handler.rs`
- [x] `src/CommanDuctUI/src/controls/checkbox_handler.rs`
- [x] `src/CommanDuctUI/src/controls/combobox_handler.rs`
- [x] `src/CommanDuctUI/src/controls/dialog_handler.rs`
- [x] `src/CommanDuctUI/src/controls/label_handler.rs`
- [x] `src/CommanDuctUI/src/controls/menu_handler.rs`
- [x] `src/CommanDuctUI/src/controls/paint_router.rs`
- [x] `src/CommanDuctUI/src/controls/panel_handler.rs`
- [x] `src/CommanDuctUI/src/controls/radiobutton_handler.rs`
- [x] `src/CommanDuctUI/src/controls/tab_bar_handler.rs`
- [x] `src/CommanDuctUI/src/controls/treeview_handler.rs`

## Chunk 9: PowerShell and auxiliary TUI modules

**Focus:** Reducer/render behavior and script contracts, not literal formatting constants unless they are part of the public interface.

- [x] `scripts/tests/HarvesterLauncher.Tests.ps1`
- [x] `scripts/tests/project-stats.Tests.ps1`
- [x] `ministry-of-future-plans/tests/Reducer.Tests.ps1`
- [x] `ministry-of-future-plans/tests/Render.Tests.ps1`
- [x] `ministry-of-future-plans/tests/Layout.Tests.ps1`
- [x] `ministry-of-future-plans/tests/Filtering.Tests.ps1`
- [x] `ministry-of-future-plans/tests/IdeaDocCore.Tests.ps1`

## Expected outputs from the review

- A list of tests to tighten because they currently over-assert internal details.
- A list of tests to rewrite upward to a more stable boundary.
- A list of literal-constant assertions to delete unless they protect a real external contract.
- A short follow-up plan for gaps where the repo lacks reducer/effect regression coverage.
