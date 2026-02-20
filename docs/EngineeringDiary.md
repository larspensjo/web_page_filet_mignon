# Engineering Diary

Purpose: durable project memory for AI-assisted development.

How to use:
- Add an entry when a noteworthy implementation lands.
- Add an entry for every bug fix, including lessons learned and prevention.
- Add an entry for important decisions and tradeoffs.
- Keep entries concise and reference concrete artifacts.

## Entry Template

## YYYY-MM-DD - Short title
Type: Implementation | Bug Fix | Decision
Context: Why this change happened.
Change: What was implemented/changed.
Lessons Learned: (required for Bug Fix)
Prevention: (required for Bug Fix)
Refs: path/to/file.rs, test_name, commit abc1234

## 2026-02-17 - Diary initialized
Type: Decision
Context: Need persistent memory across AI-assisted sessions.
Change: Added explicit diary workflow in AGENTS.md and created this file.
Refs: AGENTS.md, docs/EngineeringDiary.md

## 2026-01-21 - Plan.Main.md

Type: Implementation
Period: 2026-01-21 to 2026-02-13
StartCommit: `a6dd12da`
Context: UDF loop: PlatformEvent -> Msg -> update(state,msg) -> effects -> render(state)-> PlatformCommands. update() is pure. Encapsulation: no “getters” that expose internal struct state broadly; expose capabilities (methods) and small immutable snapshots when needed (e.g., JobRowView). Prefer pub(crate) over pub. Use module facades to keep internals private. * Determinism for logic/tests: deterministic IDs, ordering (BTreeMap/sorted vectors), stable file naming, stable export format. * Finishing intake policy (locked 2026-01-21): SessionState::Finishing keeps the intake closed; drop/ignore paste or start-style messages while draining. Auto-resume from Finishing/Finished is deferred and must be feature-flagged if added later.
Refs: docs/Plan.Main.md

## 2026-01-21 - Plan.Dropbox.md

Type: Implementation
Period: 2026-01-21 to 2026-02-08
StartCommit: `5fe6d2d6`
Context: Goal: make pasted URLs immediately enqueue jobs, restart a finished/idle session automatically, and clear the input box to support rapid paste -> alt-tab flow. Keep the app runnable after every step; each step lists what to expect and how to QA.
Refs: docs/Plan.Dropbox.md

## 2026-01-25 - Plan.ExtractLinksAndCleanMarkdown.md

Type: Implementation
Period: 2026-01-25 to 2026-02-08
StartCommit: `e09719c8`
Context: Reduce output Markdown size by removing bulky URLs (especially share/related links and <img> tags). Extract and keep links in runtime state for future features (tree expansion, link browsing, "download next"). Persist extracted links in the app's saved state (.harvester_state.ron) so they are restored when loading state from the output folder. Keep long-term "main text only" in mind with a clean architecture that can later host additional reducers (boilerplate removal, nav pruning, etc.).
Refs: docs/Plan.ExtractLinksAndCleanMarkdown.md

## 2026-01-27 - Plan.LinksTreeViewMarkers.md

Type: Implementation
Period: 2026-01-27 to 2026-02-08
StartCommit: `a9ce3cf9`
Context: Expose extracted links under each downloaded page/job node in the TreeView, so the user can quickly browse and decide what to download next.
Refs: docs/Plan.LinksTreeViewMarkers.md

## 2026-01-29 - Plan.BarSplitter.md

Type: Implementation
Period: 2026-01-29 to 2026-02-08
StartCommit: `eabee907`
Context: Add a draggable vertical splitter bar between the left panels (PANEL_INPUT + PANEL_JOBS) and the preview panel (PANEL_PREVIEW). Users can drag the splitter to resize the total left width. PANEL_INPUT and PANEL_JOBS resize proportionally; left_panel_width is the sum of both columns (default 600 = 320 + 280).
Refs: docs/Plan.BarSplitter.md

## 2026-02-07 - Plan.Phase0.SecurityPosture.md

Type: Implementation
Period: 2026-02-07 to 2026-02-08
StartCommit: `ba51d068`
Context: The project is transitioning from a manual URL-download tool into an automated RSS + LLM curation pipeline. Phase 0 establishes security foundations before LLM integration begins (Phase 1+). The codebase has a clean Elm-like architecture (pure reducer + declarative effects + async engine), which provides natural insertion points for security enforcement.
Refs: docs/Plan.Phase0.SecurityPosture.md

## 2026-02-08 - Plan.Phase1.LlmFoundation.md

Type: Implementation
Period: 2026-02-08 to 2026-02-08
StartCommit: `4adc0321`
Context: Provider-agnostic LLM abstraction — a trait-based design that supports OpenAI, Anthropic, and Google, with one concrete implementation (OpenAI) to validate the design. Versioned prompt registry — compile-time prompt templates with version tracking, A/B testing support, and injection-resistant document delimiting. Typed DTO outputs with strict validation — structured result types for triage, summary, and briefing, validated fail-closed before any state change. Cost tracking and quota enforcement — per-session limits on LLM calls, tokens, and dollar cost, preventing denial-of-wallet. Replay/evaluation harness — persist every LLM call (input hash, prompt metadata, raw output, validated result) for offline prompt iteration without re-calling the API. Elm-architecture integration — new Msg, Effect, and result variants that route LLM requests through the existing effect system, keeping reducers pure.
Refs: docs/Plan.Phase1.LlmFoundation.md

## 2026-02-08 - Plan.Phase2.ContentPreparation.md

Type: Implementation
Period: 2026-02-08 to 2026-02-09
StartCommit: `be13081f`
Context: Deterministic clean text derivation — a pure pipeline that transforms raw downloaded markdown (with frontmatter, boilerplate, inconsistent formatting) into clean, hashable text with provenance metadata
Refs: docs/Plan.Phase2.ContentPreparation.md

## 2026-02-09 - Plan.Phase3.ExecutiveBriefing.md

Type: Implementation
Period: 2026-02-09 to 2026-02-10
StartCommit: `cec08a29`
Context: Manual-trigger briefing generation
Refs: docs/Plan.Phase3.ExecutiveBriefing.md

## 2026-02-10 - Plan.Phase4.TriageRanking.md

Type: Implementation
Period: 2026-02-10 to 2026-02-10
StartCommit: `6f1e3527`
Context: The Phase 1 LLM foundation already includes TriageResult DTO, PromptId::ArticleTriage, validate_triage(), and LlmConfig::triage_model routing — all ready to use.
Refs: docs/Plan.Phase4.TriageRanking.md

## 2026-02-10 - Plan.Phase5.AutomatedSources.md

Type: Implementation
Period: 2026-02-10 to 2026-02-11
StartCommit: `27403957`
Context: Manual URL pasting doesn't scale for daily briefing workflows - Users want automated intake from trusted sources without full RSS complexity - Establishes foundation for Phase 6 (RSS) by creating pluggable source registry
Refs: docs/Plan.Phase5.AutomatedSources.md

## 2026-02-11 - Plan.Phase6.RssIngestion.md

Type: Implementation
Period: 2026-02-11 to 2026-02-12
StartCommit: `aac326bc`
Context: RSS feeds are the natural scalable intake for content curation (the project's name includes "RSS") - Title/description metadata from feeds enables future "RSS-first triage" (pre-filter before download)
Refs: docs/Plan.Phase6.RssIngestion.md

## 2026-02-11 - Plan.DynamicPromptContext.md

Type: Implementation
Period: 2026-02-11 to 2026-02-11
StartCommit: `494f1b06`
Context: Decouple frequently-changing analyst context (holdings, themes, exclusions) from versioned prompt templates by loading context variables from TOML files on disk and injecting them through the existing but unused context: Vec<(String, String)> plumbing that already runs end-to-end:
Refs: docs/Plan.DynamicPromptContext.md

## 2026-02-12 - Plan.FetchRobustness.md

Type: Implementation
Period: 2026-02-12 to 2026-02-12
StartCommit: `f2b43bf2`
Context: The RSS article downloader has a ~25% failure rate (10 failures out of ~40 jobs across two runs). All failures are HTTP 403 or 401 errors from specific sites:
Refs: docs/Plan.FetchRobustness.md

## 2026-02-12 - Plan.SummaryReuse.md

Type: Implementation
Period: 2026-02-12 to 2026-02-12
StartCommit: `39f5c092`
Context: Current behavior recomputes all article summaries each time the user clicks Generate Briefing, even when most articles were already summarized in a previous run. This increases latency and cost.
Refs: docs/Plan.SummaryReuse.md

## 2026-02-12 - Plan.MarkdownPreviewPane.md

Type: Implementation
Period: 2026-02-12 to 2026-02-12
StartCommit: `021070d4`
Context: Add a preview pane that displays a job's extracted Markdown so the user can: Assess extraction quality — judge whether the download pipeline effectively captured relevant text. Decide keep/skip — quickly determine if a page's content is interesting or should be discarded. Monitor in-progress jobs — see partial content during slow downloads to decide whether to wait for more.
Refs: docs/Plan.MarkdownPreviewPane.md

## 2026-02-12 - Plan.SummaryCacheReuse.MetadataIndependence.md

Type: Implementation
Period: 2026-02-12 to 2026-02-13
StartCommit: `e0cf0c81`
Context: Summary cache lookup fails during Generate Briefing even when inputs are unchanged. Cache entries are stored, but lookups return key unavailable because lookup-time prompt_version and model_id are missing.
Refs: docs/Plan.SummaryCacheReuse.MetadataIndependence.md

## 2026-02-13 - Plan.ConcurrentLlmProcessing.md

Type: Implementation
Period: 2026-02-13 to 2026-02-13
StartCommit: `643dd755`
Context: Triage and briefing currently process article-level LLM requests largely in serial order, which makes end-to-end latency grow roughly linearly with article count. The dominant cost is remote API wait time, not local CPU.
Refs: docs/Plan.ConcurrentLlmProcessing.md

## 2026-02-13 - Plan.MarkdownRenderingImplementation.md

Type: Implementation
Period: 2026-02-13 to 2026-02-13
StartCommit: `7d80a542`
Context: Implement robust markdown rendering in the preview pane using native Win32 Rich Edit, while preserving unidirectional data flow and keeping reducers pure.
Refs: docs/Plan.MarkdownRenderingImplementation.md

## 2026-02-14 - Plan.Step6.PromptLab.TemplateDrafts.md

Type: Implementation
Period: 2026-02-14 to 2026-02-15
StartCommit: `7904f581`
Context: Implement Prompt Tuning Workflow B (Template Drafts) so Prompt Lab can edit prompt templates safely at runtime, validate them before dispatch, run with draft templates without touching production defaults, and save drafts as explicit file-based versions.
Refs: docs/Plan.Step6.PromptLab.TemplateDrafts.md

## 2026-02-14 - Plan.Step4.PromptLab.MinimalUiManualUrlWorkflow.md

Type: Implementation
Period: 2026-02-14 to 2026-02-15
StartCommit: `7904f581`
Context: Goal: Add a first usable Prompt Lab UI in the manual URL workflow using existing CommanDuctUI primitives, while preserving UDF boundaries and production workflow isolation.
Refs: docs/Plan.Step4.PromptLab.MinimalUiManualUrlWorkflow.md

## 2026-02-14 - Plan.Step7.PromptLab.CompareModeCheapEnoughSelectionLoop.md

Type: Implementation
Period: 2026-02-14 to 2026-02-15
StartCommit: `7904f581`
Context: - Implement Step 7 from docs/Plan.Rough.PromptLab.TriageSummaryBriefing.md: run multiple (prompt_version, model, context, template) candidates against one fixed input snapshot and support deterministic "cheap-enough" winner selection. - Preserve existing Unidirectional Data Flow: Msg -> update (pure) -> State -> View, all IO in Effect handlers. - Prompt Lab state must remain isolated from production triage/briefing state mutation.
Refs: docs/Plan.Step7.PromptLab.CompareModeCheapEnoughSelectionLoop.md

## 2026-02-14 - Plan.Step1.PromptLab.DomainSlice.md

Type: Implementation
Period: 2026-02-14 to 2026-02-15
StartCommit: `7904f581`
Context: Introduce a robust, isolated Prompt Lab domain in harvester_core so Prompt Lab actions and LLM runs can be modeled and tested end-to-end in UDF, without changing existing triage/briefing behavior.
Refs: docs/Plan.Step1.PromptLab.DomainSlice.md

## 2026-02-14 - Plan.Step2.PromptLab.RunMetadataContract.md

Type: Implementation
Period: 2026-02-14 to 2026-02-15
StartCommit: `7904f581`
Context: _Last updated: 2026-02-15_
Refs: docs/Plan.Step2.PromptLab.RunMetadataContract.md

## 2026-02-14 - Plan.Step3.PromptLab.PerRunOverrides.md

Type: Implementation
Period: 2026-02-14 to 2026-02-15
StartCommit: `7904f581`
Context: Goal: Allow every Prompt Lab run to choose prompt version and model independently of stage defaults while keeping the UDF pipeline pure and traceable.
Refs: docs/Plan.Step3.PromptLab.PerRunOverrides.md

## 2026-02-15 - Plan.TriageResultPersistence.md

Type: Implementation
Period: 2026-02-15 to 2026-02-15
StartCommit: `b5618412`
Context: Persist triage outcomes so that after app restart, identical articles can reuse prior triage results and skip repeated LLM triage work.
Refs: docs/Plan.TriageResultPersistence.md

## 2026-02-15 - Plan.BriefingDependsOnTriage.md

Type: Implementation
Period: 2026-02-15 to 2026-02-15
StartCommit: `cdc79794`
Context: This plan changes the briefing workflow so briefing input is always triage-filtered first.
Refs: docs/Plan.BriefingDependsOnTriage.md

## 2026-02-15 - Plan.Step5.PromptLab.PromptTuningWorkflow.md

Type: Implementation
Period: 2026-02-15 to 2026-02-15
StartCommit: `bdbd0a99`
Context: Implement Step 5 from docs/Plan.Rough.PromptLab.TriageSummaryBriefing.md: Prompt tuning workflow A for context editing in Prompt Lab.
Refs: docs/Plan.Step5.PromptLab.PromptTuningWorkflow.md

## 2026-02-15 - Plan.PreTriageManualFiltering.md

Type: Implementation
Period: 2026-02-15 to 2026-02-15
StartCommit: `e46ac6df`
Context: Add a manual-assisted filter gate before LLM triage to reduce low-signal articles (video shells, tiny pages, boilerplate-heavy stubs) while keeping operator override and strict UDF flow.
Refs: docs/Plan.PreTriageManualFiltering.md

## 2026-02-15 - Plan.BriefingPreviewPresentationUpgrade.md

Type: Implementation
Period: 2026-02-15 to 2026-02-15
StartCommit: `de739947`
Context: Improve briefing readability and presentation in the preview pane while preserving the unidirectional data flow architecture and keeping rendering robust on the existing Win32 text control.
Refs: docs/Plan.BriefingPreviewPresentationUpgrade.md

## 2026-02-16 - Plan.PreviewBestAvailableInfo.md

Type: Implementation
Period: 2026-02-16 to 2026-02-16
StartCommit: `4e6c0178`
Context: When a user selects a job, the preview pane should always show the best available information in strict priority order:
Refs: docs/Plan.PreviewBestAvailableInfo.md

## 2026-02-16 - Plan.ComboBoxAndRadioButton.md

Type: Implementation
Period: 2026-02-16 to 2026-02-17
StartCommit: `9b87e5b8`
Context: Implement support for Combobox and radio button
Refs: docs/Plan.ComboBoxAndRadioButton.md

## 2026-02-16 - Plan.ComboBoxModelSelectorHardening.md

Type: Implementation
Period: 2026-02-16 to 2026-02-17
StartCommit: `07923490`
Context: Make Prompt Lab model selection reliably visible and usable across lifecycle transitions and Win32 layout/theming edge cases, with clear diagnostics and regression tests.
Refs: docs/Plan.ComboBoxModelSelectorHardening.md

## 2026-02-18 - Unified persistence paths
Type: Bug Fix
Context: `harvester_batch` was saving caches/state with `.json` names while `harvester_app` expected `.ron`, so the GUI never picked up batch-generated caches.
Change: Updated `RuntimePaths` to produce `.ron` files, removed the app-local cache persistence modules, and redirected the UI to `harvester_io`’s load/save APIs; added regression tests to ensure the same path is used end-to-end.
Lessons Learned: Allowing multiple codepaths to own file naming leads to silent divergence of persisted data.
Prevention: Centralize filenames/formats in `harvester_io::RuntimePaths` and cover the shared persistence API with regression tests.
Refs: crates/harvester_io/src/runtime_paths.rs, crates/harvester_app/src/platform/app.rs, cargo test -p harvester_io runtime_paths::tests

## 2026-02-18 - Fix workspace crate coverage in project stats
Type: Bug Fix
Context: `scripts/project-stats.ps1` hard-coded four crates, so newly added workspace crates were omitted from the Rust section and totals.
Change: Replaced hard-coded crate enumeration with workspace-driven discovery from root `Cargo.toml`, fixed crate-level `tests/` lookup to use each crate root, and added a Pester regression test that compares reported crates with `cargo metadata`.
Lessons Learned: Hard-coded project topology in reporting tooling quickly drifts from workspace reality and silently under-reports.
Prevention: Derive crate inventory from workspace metadata and keep a regression test that cross-checks script output against `cargo metadata`.
Refs: scripts/project-stats.ps1, scripts/tests/project-stats.Tests.ps1

## 2026-02-18 - Per-model LLM token usage visibility in app and batch
Type: Implementation
Context: Operators need session-scoped visibility into LLM token usage by resolved model in both GUI and headless batch flows, using the existing `Msg::LlmCompleted` pipeline without introducing side-channels.
Change: `harvester_core` gained a reducer-owned per-model usage ledger (`BTreeMap<String,(u64,u64)>` in `AppState`, updated in `update()` for `CacheStatus::Miss` completions only, exposed via `AppViewModel.llm_usage_by_model`). `harvester_batch` prints compact per-model lines after each cycle row. `harvester_app` extends the footer status bar with the same snapshot. No local accumulators in either binary.
Refs: crates/harvester_core/src/state.rs, crates/harvester_core/src/update.rs, crates/harvester_core/src/view_model.rs, crates/harvester_core/tests/llm_usage.rs, crates/harvester_batch/src/runner.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-02-18 - Token usage display architecture decision
Type: Decision
Context: A draft plan for per-model token usage proposed binary-local accumulators and render-state mutation, which conflicts with the project's unidirectional data flow constraints and risks inconsistent behavior.
Change: Locked direction to reducer-owned per-model usage tracking in `harvester_core`, consumed read-only by both `harvester_app` and `harvester_batch`; replay cache hits are excluded from session consumption totals to avoid overcounting.
Refs: docs/Plan.TokenUsageDisplay.md, crates/harvester_core/src/update.rs, crates/harvester_engine/src/llm/handle.rs

## 2026-02-19 - Prompt Lab templates section layout collapse/width fix
Type: Bug Fix
Context: In Prompt Lab advanced mode with Templates open, the template toggle button label was truncated and a stray white template input field could remain visible even when the template editor was not opened.
Change: Updated `harvester_app` Prompt Lab layout to compute visibility from a canonical predicate set and use the same `show_template_editor_rows` condition for both panel height and row sizing. Introduced shared Prompt Lab layout size constants and a reusable collapsed-row helper for zero-height top-docked sections. Added reducer-side state normalization in `harvester_core::PromptLabState` so `template_editor_open` cannot remain true when Prompt Lab is hidden, advanced mode is disabled, or the template section is closed.
Lessons Learned: In docked Win32 layouts, every conditional branch must explicitly size hidden rows; relying on previous layout state causes stale controls to leak into the visible UI.
Prevention: Derive layout visibility from one canonical state predicate set, enforce Prompt Lab visibility invariants in reducer-owned state transitions, and maintain matrix-style layout tests that cover toggle combinations rather than single happy paths.
Refs: crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_core/src/prompt_lab.rs

## 2026-02-20 - Prompt Lab section toggles migrated from RadioButton to CheckBox
Type: Implementation
Context: Prompt Lab section controls (`Compare`, `Context`, `Templates`, `Run details`) were implemented as `BS_AUTORADIOBUTTON` controls even though they are independent booleans. This semantic mismatch repeatedly caused dark-theme regressions when new button-like controls were added, because the split between creation-time dark-mode enablement and style-application-time classic rendering was easy to miss.
Change: `commanductui` gained a new `CheckBox` control type.
Refs: src/CommanDuctUI/src/types.rs, src/CommanDuctUI/src/window_common.rs, src/CommanDuctUI/src/controls/checkbox_handler.rs, src/CommanDuctUI/src/controls/paint_router.rs, crates/harvester_app/src/platform/ui/constants.rs, crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs, crates/harvester_app/src/platform/app.rs

## 2026-02-20 - Render function section extraction and diff helper consolidation
Type: Implementation
Context: `harvester_app` UI rendering had grown into a single oversized `render` function with long, repetitive `tree_state` change-detection chains, making it harder to reason about and safely extend.
Change: Refactored `harvester_app` render orchestration into section-level functions (`layout`, `status`, `token progress`, `main controls`, `prompt lab`, `preview`) and introduced a shared `emit_if_changed` helper for repeated state-diff patterns while preserving command emission behavior and ordering.
Refs: crates/harvester_app/src/platform/ui/render.rs
