# Engineering Diary

Purpose: durable project memory for AI-assisted development.

How to use:
- Add an entry when a noteworthy implementation lands.
- Add an entry for every bug fix, including lessons learned and prevention.
- Add an entry for important decisions and tradeoffs.
- Keep entries concise and reference concrete artifacts.
- New entries goes to the end of the file.

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

## 2026-02-21 - Harvester batch launcher design hardening
Type: Decision
Context: The initial launcher design conflicted with the requirement that ENTER runs immediately, mixed side effects into reducer responsibilities, and relied on fragile command-string and stderr parsing patterns that would reduce robustness.
Change: Updated the launcher design to enforce UDF with explicit effect requests, argv-based command execution, startup capability probing for checkpoint flags, dynamic layout sizing, and in-scope Pester coverage for reducer/render/effects.
Evidence: Reviewed against `crates/harvester_batch/src/cli.rs` and updated `docs/plans/Design.harvester-batch-tui-launcher.md`.
Refs: docs/plans/Design.harvester-batch-tui-launcher.md, crates/harvester_batch/src/cli.rs, ministry-of-future-plans/browser/Input.psm1

## 2026-02-21 - Delta briefing design hardening
Type: Decision
Context: The initial delta-briefing draft captured the core idea but left key robustness gaps around prompt variable injection, render-size control, and centralized path ownership, which could cause noisy prompts and long-term maintenance drift.
Change: Revised the design to use reducer-owned history with capped retention, explicit load/save effects, dedicated extra template variables for `previous_briefings`, rendered-size safeguards, and a `RuntimePaths`-owned history path.
Evidence: Cross-checked against current briefing orchestration and LLM rendering flow, then updated `docs/plans/Design.delta-briefing-design.md`.
Refs: docs/plans/Design.delta-briefing-design.md, crates/harvester_core/src/update.rs, crates/harvester_engine/src/llm/handle.rs, crates/harvester_io/src/runtime_paths.rs

## 2026-02-21 - Delta briefing implementation plan hardening
Type: Decision
Context: The initial delta-briefing implementation plan contained several brittle implementation details (context-variable duplication risk, incomplete prompt-size safeguards, and fragmented path ownership) that could cause regressions during coding.
Change: Rewrote the plan to align with current reducer/effect/LLM plumbing, added explicit `extra_template_vars` and rendered-size guard requirements, centralized history path ownership in `RuntimePaths`, expanded regression tests, and added backlog status guidance for `FI-Storage-BriefingHistory-0001`.
Evidence: Updated `docs/plans/Plan.delta-briefing.md` after source cross-check with `harvester_core`, `harvester_io`, and `harvester_engine`.
Refs: docs/plans/Plan.delta-briefing.md, crates/harvester_core/src/update.rs, crates/harvester_engine/src/llm/handle.rs, crates/harvester_io/src/runtime_paths.rs, docs/FutureIdeas.md

## 2026-02-21 - Harvester batch TUI launcher implementation
Type: Implementation
Period: 2026-02-21
Context: Implement a PowerShell TUI launcher (Elm/Redux UDF) for `harvester_batch`: key input → action → pure reducer → effects → follow-up actions. Fully covered by Pester 5 unit tests across all layers.
Change: Six modules shipped: `Data.psm1` (action/param defs), `Reducer.psm1` (pure state management), `Effects.psm1` (IO/process calls), `Input.psm1` (key mapper wrapping submodule), `Render.psm1` (frame-diff rendering), `Start-HarvesterBatch.ps1` (main loop). 140 Pester tests.
Key bugs fixed during implementation: (1) Pester 5 BeforeAll/Describe scope isolation — helpers need `function script:Name` inside `BeforeAll`. (2) `Import-Module -Global` required inside modules to expose dependencies to global scope. (3) `Set-StrictMode -Version Latest` + `ConvertFrom-Json` — must use `$json.PSObject.Properties['Key']` not dot notation for undefined properties. (4) Pester `-ModuleName` in `Mock` must match the .psm1 filename, not a logical alias. (5) `[Console]::CursorVisible` throws in headless environments — wrap in `try/catch`. (6) PowerShell `-ne` is case-insensitive by default — use `-cne` in frame-diff comparisons. (7) `List[object[]]::Add()` fails when pipeline-enumerated output is passed — use `List[object]` instead.
Refs: scripts/harvester_launcher/, scripts/tests/HarvesterLauncher.Tests.ps1, scripts/Start-HarvesterBatch.ps1, docs/plans/Plan.harvester-batch-tui-launcher.md

## 2026-02-21 - Pester 5 module scoping in PowerShell TUI tests
Type: Bug Fix
Context: While implementing the harvester_batch TUI launcher, Pester tests were failing with `CommandNotFoundException` for functions that should have been in scope after `Import-Module` calls in `BeforeAll`.
Change: Two fixes applied: (1) `Import-Module` inside a module's own script body imports as a *nested module*, scoping exported functions to the parent module only — not to the global session. Fixed by adding `-Global` to `Reducer.psm1`'s `Import-Module Data.psm1` call so Data's functions are always in global scope. (2) Functions defined at `Describe` scope (outside `BeforeAll`) are not accessible from `It` blocks in Pester 5 due to discovery-vs-execution phase isolation. Fixed by moving helper functions into `BeforeAll` with `function script:FuncName { ... }` so they're in the test script's scope and accessible from all `It` blocks.
Lessons Learned: Pester 5 runs `Describe` body code during discovery (before module imports run), so helper functions defined there cannot call imported cmdlets. `It` blocks run in a child scope that does not inherit from the `Describe` discovery scope. Module-internal `Import-Module` without `-Global` creates a nested module visible only within that module.
Prevention: Always define test helper functions inside `BeforeAll` using `function script:Name`. When a module requires another module's exports to be globally accessible (e.g., because tests import them independently), use `Import-Module <path> -Force -Global` inside the depending module.
Refs: scripts/harvester_launcher/Reducer.psm1, scripts/tests/HarvesterLauncher.Tests.ps1

## 2026-02-24 - Trends and tabs: design and implementation planning
Type: Decision
Context: The right-pane preview area had grown into a single overloaded surface handling four
distinct responsibilities: selected-article preview, briefing preview, Prompt Lab output override,

## 2026-04-02 - Footer operation progress bar
Type: Implementation
Context: Polling previously exposed only a disabled button, and triage/summary progress lived only in status text. The UI needed one shared, compact progress surface for the active operation without breaking the reducer-owned state model or footer height.
Change: Added reducer-owned `OperationProgress` projection in `AppViewModel`, tracked poll totals/completions in `SourceStateIndex` via new `Msg::PollStarted { total }`, auto-switched to `PollStats` when polling completed, and rendered a collapsible footer label + progress bar using the existing diff-based layout/render pipeline.
Refs: crates/harvester_core/src/source_state.rs, crates/harvester_core/src/state.rs, crates/harvester_core/src/update.rs, crates/harvester_io/src/effect_runner.rs, crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs
and future content types. Separately, there was no way to see how coverage of companies,
technologies, products, or themes was evolving across the archive over time. The user requested
a tab system to give each content type its own surface, plus an entity trend chart that the LLM
populates automatically from summaries (no hard-coded keyword lists).
Change: Produced `docs/plans/Design.trends-and-tabs.md` (brainstormed, reviewed, and hardened
against current source) and `docs/plans/Plan.trends-and-tabs.md` (five-slice implementation plan).
Key design decisions locked: content-area tabs using existing RadioButton-row pattern; entity
extraction added to summary prompt V4 (companies, technologies, products); themes reuse existing
triage tags; entity sidecar index `.entity_index.ron` with race-safe serialized upsert lane;
rebuild from markdown scan + cache join; GDI+ chart control deferred to Slice 5 with text/table
view in Slice 4.
Refs: docs/plans/Design.trends-and-tabs.md, docs/plans/Plan.trends-and-tabs.md

## 2026-02-24 - Harvester batch launcher TUI rendering fixes for Windows console
Type: Bug Fix
Context: `Start-HarvesterBatch.ps1` rendered malformed borders and shifted rows in some Windows terminals due to console encoding defaults, a wide/ambiguous selection glyph, and unbounded preview text pushing pane borders.
Change: Updated the PowerShell harvester launcher TUI (`scripts/harvester_launcher` + startup script) to force UTF-8 console output, render explicit top borders in both panes, use a narrow selection marker glyph, clamp right-pane preview text before padding, and raise layout minimum height to account for the added border row. Added Pester regression tests for top borders, marker glyph choice, preview border preservation, and min-height sizing.
Evidence: `Invoke-Pester scripts/tests/HarvesterLauncher.Tests.ps1`
Lessons Learned: Console TUI layout correctness depends on three separate invariants at once (encoding, glyph display width, and explicit string truncation); fixing only one can leave visually similar border corruption.
Prevention: Keep renderer regression tests that assert pane border characters and preview-row right borders under long input, and treat non-ASCII TUI glyphs as width-sensitive choices (prefer known narrow glyphs).
Refs: scripts/Start-HarvesterBatch.ps1, scripts/harvester_launcher/Render.psm1, scripts/harvester_launcher/Reducer.psm1, scripts/tests/HarvesterLauncher.Tests.ps1

## 2026-02-24 - Harvester launcher checkpoint show strict-mode optional property fix
Type: Bug Fix
Context: Choosing `Show current checkpoint` in `scripts/Start-HarvesterBatch.ps1` crashed before running the command because the effect dispatcher assumed every `RunCheckpointCommand` effect object carried a `CustomDate` property.
Change: Hardened the launcher effects dispatcher to read `CustomDate` as an optional field for both hashtable and PSCustomObject effects, defaulting to an empty string for non-date checkpoint actions. Added a Pester regression test that exercises the `cp-show` path through `Invoke-LauncherEffects`.
Evidence: `Invoke-Pester -Path scripts/tests/HarvesterLauncher.Tests.ps1 -CI` (141 passed).
Lessons Learned: Under `Set-StrictMode -Version Latest`, dot-accessing a missing property/key in heterogeneous effect payloads is a runtime error; optional effect fields must be read via explicit existence checks at the effect boundary.
Prevention: Treat effect payloads as versioned/partial contracts, centralize optional-field extraction in dispatcher code, and add dispatcher-level tests for action variants that intentionally omit optional fields.
Refs: scripts/harvester_launcher/Effects.psm1, scripts/tests/HarvesterLauncher.Tests.ps1

## 2026-02-24 - Briefing coverage window injected into aggregate briefing prompt and preview
Type: Implementation
Context: Time-limited briefing checkpoints filter article inputs, but the generated briefing did not explicitly tell the recipient what period was covered, making it easy to misread an intentionally filtered briefing as all-time coverage.
Change: `harvester_core` now snapshots a per-run briefing coverage window label from the active checkpoint, passes it to aggregate briefing requests as a `briefing_time_window` extra template variable, and includes the same label in briefing preview session metadata. `harvester_engine` adds aggregate briefing prompt `v6` (set active) so the coverage window is rendered and the model is instructed to mention it in the executive summary without mutating `v5` semantics.
Evidence: `cargo test -p harvester_core aggregate_briefing_effect_includes_checkpoint_time_window_extra_var -- --nocapture`; `cargo test -p harvester_core briefing_format_preview_includes_coverage_window_when_present -- --nocapture`; `cargo test -p harvester_engine aggregate_briefing_active_version_is_v6 -- --nocapture`; `cargo test -p harvester_engine v6_system_template_contains_briefing_time_window_slot -- --nocapture`; `cargo build`; `cargo clippy --all-targets -- -D warnings`.
Refs: crates/harvester_core/src/briefing.rs, crates/harvester_core/src/update.rs, crates/harvester_engine/src/llm/prompts/briefing.rs, crates/harvester_engine/src/llm/prompts/mod.rs

## 2026-02-25 - Preview tab ghost square from multiple Fill panels
Type: Bug Fix
Context: After adding preview tabs, a persistent small white square appeared at the top-left of the preview pane. The artifact remained even with no selected job, indicating a hidden control/layout issue rather than header text content.
Change: Updated `harvester_app` tab-panel layout rules so only the active preview tab panel uses `DockStyle::Fill`; inactive tab panels now collapse with zero-height `Top` docking. Added a layout regression test that enforces exactly one Fill tab panel under `PANEL_PREVIEW`.
Evidence: `cargo test -p harvester_app preview_tab_panels_use_single_fill_rule -- --nocapture`
Lessons Learned: A UI layout workaround that depends on unsupported toolkit semantics (multiple sibling Fill docks with “collapsed” sizes) can fail as visual artifacts far from the feature code.
Prevention: Encode toolkit layout constraints directly in app layout builders (one Fill child per parent) and add tests that assert structural layout invariants, not only resulting sizes.
Refs: crates/harvester_app/src/platform/ui/layout.rs, preview_tab_panels_use_single_fill_rule

## 2026-02-25 - CommanDuctUI hard-fails invalid multi-Fill layouts
Type: Bug Fix
Context: The preview-tab artifact exposed that `CommanDuctUI` accepted unsupported layouts with multiple sibling `DockStyle::Fill` rules, logged a warning, and proceeded with degraded rendering instead of failing at the boundary.
Change: `commanductui` now validates `DefineLayout` rules and returns a hard error when any parent has more than one `DockStyle::Fill` child. Added unit tests for both rejection and valid one-Fill-per-parent layouts, and released the submodule as `0.4.1`.
Evidence: `cargo test --manifest-path src/CommanDuctUI/Cargo.toml define_layout_validation -- --nocapture`
Lessons Learned: Silent degradation in foundational UI infrastructure obscures the true fault domain and turns contract violations into expensive visual debugging.
Prevention: Treat layout rule sets as validated input contracts at `DefineLayout` boundaries and prefer explicit errors over best-effort behavior for unsupported docking combinations.
Refs: src/CommanDuctUI/src/window_common.rs, src/CommanDuctUI/src/command_executor.rs, src/CommanDuctUI/Cargo.toml, src/CommanDuctUI/CHANGELOG.md

## 2026-02-25 - Briefing loader URL alias matching for redirected/mobile variants
Type: Bug Fix
Context: `engine.log` showed repeated `[briefing-loader] selected url missing from corpus` warnings during startup because persisted briefing selections used URL variants (e.g., `www`/`m`/`edition`, `http` vs `https`, query-tagged URLs, and Cisco newsroom path variants) that did not exactly match archived markdown frontmatter URLs.
Change: Hardened `harvester_engine` briefing URL lookup alias generation to match across host-prefix variants (`www.`, `eu.`, `m.`, `edition.`), `http`/`https` scheme variants, query/no-query forms, and the Cisco `newsroom.cisco.com/content/r/...` to `/c/r/...` path shape. Added integration tests covering each matching case and cleaned an unrelated unused import warning in `harvester_io`.
Evidence: `cargo test -p harvester_engine --test briefing_loader_integration filtered_loader_matches`; `cargo build`
Lessons Learned: URL equality in cross-stage pipelines is a contract boundary, not a string-compare detail; if one stage stores fetched/canonicalized URLs while another stores selected/source URLs, alias matching must explicitly model common transformations.
Prevention: Keep URL-lookup normalization/aliasing centralized in the briefing loader and add regression tests for every new real-world mismatch observed in logs before adjusting warning policy.
Refs: crates/harvester_engine/src/briefing.rs, crates/harvester_engine/tests/briefing_loader_integration.rs, crates/harvester_io/src/effect_runner.rs

## 2026-02-25 - Block interstitial pages from entering the article archive
Type: Bug Fix
Context: Some fetches were succeeding technically but landing on consent/captcha/interstitial pages (e.g., Yahoo consent and site captcha challenge endpoints). Those pages were exported as markdown and later caused briefing-loader corpus mismatches and noisy warnings when selected article URLs no longer matched archived interstitial URLs.
Change: Added a pure blocker-page classifier in `harvester_engine` (URL and narrow content heuristics) and invoked it in `run_job` before export so interstitial pages fail with `FailureKind::BlockedContent` instead of being written to the archive or persisted as completed jobs. Added unit tests for Yahoo consent, captcha challenge URLs, content-based verification pages, and a false-positive regression case.
Evidence: `cargo test -p harvester_engine blocker_page -- --nocapture`; `cargo build`
Lessons Learned: “Successful HTTP fetch” is not the same as “valid article acquisition”; pipelines need an explicit post-fetch content validity gate before persistence, especially when redirects can land on interstitial products.
Prevention: Keep interstitial detection as a centralized pure classifier with real-world regression fixtures and extend it from observed logs before adding ad hoc per-site exceptions downstream.
Refs: crates/harvester_engine/src/blocker_page.rs, crates/harvester_engine/src/engine.rs, crates/harvester_engine/src/types.rs

## 2026-02-25 - Prompt Lab advanced tab mode ignored legacy visibility flag
Type: Bug Fix
Context: After moving Prompt Lab into its own right-pane tab, clicking `Advanced` selected the radio button but did not reveal advanced sections because the layout still gated Prompt Lab visibility on the old left-panel `prompt_lab.visible` flag.
Change: Updated `harvester_app` Prompt Lab layout rendering to treat Prompt Lab as visible when the active tab is `PromptLab`, independent of the legacy visibility flag. Added a render regression test that asserts advanced layout rows expand when the Prompt Lab tab is active even if `prompt_lab.visible` is false.
Evidence: `cargo test -p harvester_app prompt_lab_tab_advanced_layout_does_not_depend_on_legacy_visible_flag -- --nocapture`
Lessons Learned: UI refactors that relocate a feature can leave hidden gating booleans behind; duplicated “visibility” concepts must be re-derived from the new owner state (here: active tab) instead of preserved by coincidence.
Prevention: When migrating a panel into a tab/surface, audit render and layout code for all legacy visibility predicates and add regression tests that intentionally keep old flags false while the new navigation state is active.
Refs: crates/harvester_app/src/platform/ui/render.rs, prompt_lab_tab_advanced_layout_does_not_depend_on_legacy_visible_flag

## 2026-02-25 - Generate Briefing now switches directly to Briefing tab
Type: Bug Fix
Context: After introducing right-pane tabs, clicking `Generate Briefing` left the UI on the default `Summary` tab even though the briefing workflow had started, which made the user manually switch tabs to follow briefing progress/output.
Change: Updated `harvester_core` reducer handling for `Msg::GenerateBriefingClicked` to select `AppTab::Briefing` immediately before starting briefing orchestration. Extended the reducer test to assert the tab switch alongside the existing emitted effects and briefing phase transition.
Evidence: `cargo test -p harvester_core generate_briefing_emits_load_effect -- --nocapture`
Lessons Learned: Feature actions that previously relied on a single shared preview surface need explicit navigation updates after tabbed UI refactors; preserving old side effects alone is not enough to preserve user-visible flow.
Prevention: For tabbed surfaces, add reducer tests that assert both domain state changes and `active_tab` transitions for primary workflow entry actions (e.g., Generate Briefing, Prompt Lab open).
Refs: crates/harvester_core/src/update.rs, generate_briefing_emits_load_effect

## 2026-02-25 - Button-like controls missing dark theme style — recurring pattern
Type: Bug Fix
Context: Trends category radio buttons (BUTTON_TREND_COMPANIES/TECHNOLOGIES/PRODUCTS/THEMES) appeared with light/white backgrounds because they were created via `CreateRadioButton` but never assigned `StyleId::RadioButton` via `ApplyStyleToControl`. Two Prompt Lab source buttons (BTN_SOURCE_FROM_TRIAGE, BTN_SOURCE_TYPE_URL) were also found unstyled when the regression test ran. Without a style, `handle_wm_ctlcolorbtn` returns `None` and Windows falls back to system-default (light) colors. The same pattern had silently affected other controls in earlier slices.
Change: Added `ApplyStyleToControl` for the four trend category buttons and two source buttons. Added `every_button_like_control_has_a_style_applied` unit test in `layout.rs` that collects all `CreateButton`/`CreateRadioButton`/`CreateCheckBox` control IDs and asserts each has a corresponding `ApplyStyleToControl` command — the test fails at CI time the moment a new button is added without styling.
Lessons Learned: `initial_commands()` in layout.rs has a two-step, spatially separated pattern: control creation (~line 170) and style application (~line 2100), far apart in the same file. There is nothing in the type system to enforce the pairing. The omission is silent at compile time and only visible as a visual regression at runtime on Windows.
Prevention: `every_button_like_control_has_a_style_applied` test in `crates/harvester_app/src/platform/ui/layout.rs` now acts as the guard. Any future `CreateButton`/`CreateRadioButton`/`CreateCheckBox` without a matching `ApplyStyleToControl` will fail this test immediately.
Refs: crates/harvester_app/src/platform/ui/layout.rs, every_button_like_control_has_a_style_applied

## 2026-02-25 - Summary tab no longer falls back to briefing/shared preview content
Type: Bug Fix
Context: After switching to the Briefing tab during briefing generation, returning to the Summary tab with no selected article could display briefing output because Summary still fell back to the legacy shared `preview_text` content path.
Change: Updated `harvester_app` summary-tab rendering to use only `right_pane.summary_markdown` and show an explicit empty-state message when no article is selected. Added a render regression test that prevents briefing text from leaking into the Summary viewer in the no-selection case.
Evidence: `cargo test -p harvester_app summary_tab_without_selected_article_shows_empty_state_not_briefing_preview -- --nocapture`
Lessons Learned: Tab migrations require strict ownership of displayed content; retaining generic fallback content paths inside tab renderers reintroduces cross-tab leakage when selection state is absent.
Prevention: For each tab renderer, define a tab-specific empty state and add tests that set conflicting legacy preview fields to ensure tab content is sourced only from the tab’s view model fields.
Refs: crates/harvester_app/src/platform/ui/render.rs, summary_tab_without_selected_article_shows_empty_state_not_briefing_preview

## 2026-02-26 - Prompt Lab moved to left panel tab bar
Type: Implementation
Context: Prompt Lab occupied a right-pane tab alongside Triage/Summary/Briefing/Trends. This meant the user could not view lab configuration and lab results simultaneously, and lab results (`output_json`) had no viewer at all. Moving Prompt Lab to the left panel gives the user side-by-side config+result visibility by routing lab output into the existing right-pane viewers.
Change: Added `LeftTab { JobList, PromptLab }` enum and `LeftPaneView` to `harvester_core`. Left panel now has a tab bar with two tabs; right panel drops the PromptLab entry. When `left_tab == PromptLab` the right-pane viewers show lab results instead of production content. Auto-switch after lab run completion routes the right pane to the matching content tab. Affected subsystems: `harvester_core`, `harvester_app`.
Evidence: `cargo nextest run` — 850/850 passed; `cargo clippy --workspace --all-targets -- -D warnings` — clean.
Refs: crates/harvester_core/src/tabs.rs, crates/harvester_core/src/view_model.rs, crates/harvester_core/src/state.rs, crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs, crates/harvester_app/src/platform/app.rs

## 2026-06-11 - Custom TabBar widget replacing radio-button tab bars
Type: Implementation
Context: The three tab bars (right-pane tabs, left-pane tabs, trend-category selector) were built from Win32 panels + radio buttons, which gave no hover feedback, no accent underline, and no easy styling. A custom TabBar widget was needed to provide a polished native look consistent with the dark theme.
Change: Added custom `HarvesterTabBarControl` Win32 WndProc widget in `commanductui` (v0.7.0). Implements hover fill, 3 px accent underline for the active tab, and 40 %-blended inactive text color. Introduced `ControlKind::TabBar`, `StyleId::TabBar`/`TabBarAccent`, `AppEvent::TabBarSelectionChanged`, and four new `PlatformCommand` variants (`CreateTabBar`, `SetTabBarItems`, `SetTabBarSelection`, `SetTabBarStyle`). Added `from_index`/`to_index` conversion methods to `AppTab`, `LeftTab`, `TrendCategory` in `harvester_core`. Migrated all three tab-bar sites in `harvester_app` to use the new widget; removed dead radio-button panel/button constants. Affected subsystems: `commanductui`, `harvester_core`, `harvester_app`.
Evidence: `cargo nextest run` — all tests pass; `cargo clippy --workspace --all-targets -- -D warnings` — clean.
Refs: src/CommanDuctUI/src/controls/tab_bar_handler.rs, crates/harvester_core/src/tabs.rs, crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs, crates/harvester_app/src/platform/app.rs

## 2026-06-11 - Bug: tab click did not trigger content change
Type: Bug Fix
Context: After the TabBar widget was introduced, clicking a tab visually updated the selection (accent line moved) but the content pane did not change. Programmatic switches (e.g., auto-switch after briefing generation) worked correctly.
Change: Fixed `tab_bar_handler` to send `WM_APP_TAB_SELECTED` to the root ancestor window (`GetAncestor(hwnd, GA_ROOT)`) instead of the direct parent (`GetParent`). The tab bars are grandchildren of the main window (nested inside panel HWNDs), so the direct parent is a plain panel whose WndProc calls `DefWindowProcW` and silently drops the unrecognised message.
Evidence: Clicking tabs now dispatches `AppEvent::TabBarSelectionChanged` and switches content correctly.
Lessons Learned: Custom controls that send `WM_APP_*` notifications must target the root-ancestor window, not the immediate parent, when they may be nested inside intermediate panels. `GetParent` is only safe when the control is a direct child of the main window (as the splitter is).
Prevention: When adding a new custom WndProc control that raises a `WM_APP_*` notification, always use `GetAncestor(hwnd, GA_ROOT)` as the message target.
Refs: src/CommanDuctUI/src/controls/tab_bar_handler.rs, commit 155031f

## 2026-02-26 - Coalesced pre-triage refreshes during source polling
Type: Implementation
Context: `Poll Sources` enqueues many jobs in quick succession, and each `JobDone` previously triggered a full pre-triage article reload (`LoadArticlesForTriage`) that reloaded persisted completed jobs and re-ran `content_prep` across the corpus. This produced repeated expensive work and made polling feel much slower than the RSS fetch itself.
Change: Added a serialized, debounced pre-triage refresh worker in `harvester_io` that coalesces repeated `LoadArticlesForTriage` requests and runs only the latest refresh request per burst. Added timing logs for source polling and article-loading effects (`briefing`, `briefing-prereq`, and pre-triage refresh) to expose elapsed time and coalescing behavior. Affected subsystems: `harvester_io`.
Evidence: `cargo build`; `cargo test -p harvester_io drain_latest_triage_refresh_requests_keeps_latest_batch -- --nocapture`; `cargo clippy --all-targets -- -D warnings`.
Refs: crates/harvester_io/src/effect_runner.rs, drain_latest_triage_refresh_requests_keeps_latest_batch

## 2026-02-27 - Reducer-owned pre-triage refresh coordinator
Type: Implementation
Context: The IO-layer debounce worker from 2026-02-26 coalesced bursts within a 300 ms window, but the reducer still dispatched one `LoadArticlesForTriage` effect per `JobDone`. Spaced job completions (e.g. after a `Poll Sources` run) therefore produced multiple sequential refreshes. The debounce worker also held the only guard against stale results arriving out of order.
Change: Replaced per-`JobDone` dispatch with a reducer-owned `PreTriageRefreshCoordinator` state machine. Demand is recorded on `JobDone`/`RestoreCompletedJobs` and dispatched from `Msg::Tick` (75 ms cadence) after a configurable quiet window: `QUIET_TICKS_NORMAL=4` (~300 ms) for single job completions, `QUIET_TICKS_AFTER_POLL=16` (~1200 ms) after a poll burst. Poll-burst gating blocks dispatch until `poll_sources_ended && jobs_in_flight==0`. `MAX_WAIT_TICKS=80` (~6 s) prevents starvation. Added `request_id: u64` to `Effect::LoadArticlesForTriage`, `Msg::TriageArticlesLoaded`, and `Msg::TriageArticlesLoadFailed`; the reducer silently discards results whose ID does not match the current in-flight request. Background refresh failures no longer poison the active triage session. Removed the interim IO-layer debounce worker; `LoadArticlesForTriage` now spawns a direct IO thread per dispatch. Affected subsystems: `harvester_core`, `harvester_io`.
Evidence: `cargo nextest run` — 437+ tests pass; `cargo clippy --all-targets -- -D warnings` — clean on `harvester_core`, `harvester_io`, `harvester_batch`.
Refs: crates/harvester_core/src/pre_triage_coordinator.rs, crates/harvester_core/src/update.rs, crates/harvester_core/src/state.rs, crates/harvester_io/src/effect_runner.rs, docs/plans/Plan.pretriage-refresh-coordinator.md

## 2026-02-27 - Batched app-loop post-processing and async latest-wins persistence
Type: Implementation
Context: Pre-triage load IO was already fast, but user-visible readiness lagged because the app thread still did burst-multiplied post-processing (`state.view()`/render work and repeated refresh URL snapshots) plus synchronous persistence writes.
Change: `harvester_app` now drains and reduces messages in batches, then performs one post-processing pass per batch: one render for dirty batches, one input-clear command, and one pre-triage refresh evaluation action with a single completed-URL snapshot. `harvester_core` now records refresh-evaluation intent on `JobDone`/`RestoreCompletedJobs` and schedules coordinator refresh from `Msg::EvaluatePreTriageRefresh`, removing per-message URL snapshot cloning from the reducer hot path. Added `harvester_io::PersistenceWorker`, a bounded latest-wins worker (`Mutex<Option<_>> + Condvar`) with debounce + max-flush interval and atomic writes via `persist_runtime_state`; app-thread synchronous state writes were removed. Scope check: `harvester_batch` has an independent dispatch loop, so batch-parity handling was added there by draining message bursts and applying a single refresh evaluation per drained batch.
Evidence: `cargo build`; `cargo test -p harvester_io persistence_worker -- --nocapture`; `cargo test -p harvester_core multiple_job_dones_within_quiet_window_emit_exactly_one_triage_load -- --nocapture`; `cargo test -p harvester_core restore_completed_jobs_schedules_and_dispatches_after_quiet_window -- --nocapture`; `cargo clippy --all-targets -- -D warnings`.
Refs: crates/harvester_app/src/platform/app.rs, crates/harvester_core/src/update.rs, crates/harvester_core/src/state.rs, crates/harvester_core/src/msg.rs, crates/harvester_batch/src/runner.rs, crates/harvester_io/src/persistence_worker.rs, crates/harvester_io/src/persistence.rs

## 2026-03-01 - Archive action now exports triage-filtered archive.md
Type: Implementation
Context: `Archive` previously triggered engine `export.txt` for all markdown files, which did not match briefing/triage workflows. The feature needed to export only triage-eligible articles (priority > cutoff), preserve briefing checkpoint filtering, and keep deterministic triage ordering.
Change: `harvester_core` now emits `Effect::ArchiveRequested { ordered_urls, since_utc }` from reducer state (`briefing_triage_policy`, triage session, briefing checkpoint). `harvester_io` handles this effect by spawning a dedicated archive writer that calls new `harvester_engine::build_triage_archive`, writing `archive.md` with delimiter blocks plus full raw markdown documents ordered by triage priority (via reducer-provided URL order). Affected subsystems: `harvester_core`, `harvester_engine`, `harvester_io`.
Evidence: `cargo test -p harvester_core`; `cargo test -p harvester_engine output`; `cargo test -p harvester_io archive_requested_writes_archive_markdown_for_selected_urls`; `cargo clippy --all-targets -- -D warnings`.
Refs: crates/harvester_core/src/effect.rs, crates/harvester_core/src/update.rs, crates/harvester_engine/src/export.rs, crates/harvester_engine/tests/output.rs, crates/harvester_io/src/effect_runner.rs, crates/harvester_core/tests/update_behaviour.rs

## 2026-03-01 - Clarified updater prompt as text-return contract
Type: Decision
Context: The updater prompt wording implied direct file overwrite, which could trigger tool-write behavior in CLI agents despite this script being designed as a text-return pipeline with local atomic writes.
Change: Refined the updater contract in the plan-review loop prompt to explicitly require returned Markdown text and forbid file edits/permission requests. This aligns model behavior with script ownership of writes.
Evidence: `scripts/Invoke-PlanReviewLoop.ps1` syntax parse check passed (`Parser::ParseFile` returned OK).
Refs: scripts/Invoke-PlanReviewLoop.ps1

## 2026-04-09 - harvester_core state module split
Type: Implementation
Context: `crates/harvester_core/src/state.rs` had grown past 5,000 lines and mixed reducer-owned state, link helpers, view-building logic, and tests into one file, which made navigation and safe refactoring harder.
Change: Converted `state.rs` into a directory module and extracted focused submodules for job state, UI state, indirect-link handling, pure link helpers, view building, and state tests while preserving the existing `AppState` API and crate-internal import surface.
Refs: crates/harvester_core/src/state/mod.rs, crates/harvester_core/src/state/job_state.rs, crates/harvester_core/src/state/ui_state.rs, crates/harvester_core/src/state/indirect_links.rs, crates/harvester_core/src/state/link_helpers.rs, crates/harvester_core/src/state/view_builder.rs, crates/harvester_core/src/state/tests.rs

## 2026-03-01 - Claude plan loop isolation and UTF-8 CLI encoding
Type: Decision
Context: Plan-review loop runs should be deterministic and not inherit prior Claude session state, and generated markdown had mojibake (`ΓÇö`) from an encoding mismatch in the CLI capture path.
Change: Defaulted Claude CLI args to `--no-session-persistence` in the loop and set process native-command encoding to UTF-8 (`Console` input/output plus `$OutputEncoding`) before invoking model CLIs.
Evidence: `scripts/Invoke-PlanReviewLoop.ps1` syntax parse check passed (`Parser::ParseFile` returned OK).
Refs: scripts/Invoke-PlanReviewLoop.ps1

## 2026-03-01 - Updater prompt now requires independent validation of review claims
Type: Decision
Context: Review outputs can include incorrect or out-of-scope suggestions, and blindly applying them degrades plan quality.
Change: Strengthened the plan-updater prompt contract to require claim-by-claim validation against the current plan and source code, apply only correct/relevant suggestions, and document rejected suggestions with rationale in `Notes`.
Evidence: `scripts/Invoke-PlanReviewLoop.ps1` syntax parse check passed (`Parser::ParseFile` returned OK).
Refs: scripts/Invoke-PlanReviewLoop.ps1

## 2026-03-02 - Since Checkpoint Tab
Type: Implementation
Context: Users needed a way to see only articles fetched since the last briefing checkpoint without scrolling through older jobs. Archive and Briefing already had this filter; the UI had no equivalent.
Change: Added a new `SinceCheckpoint` left-pane tab. Threaded `fetched_utc` from the engine through `JobOutcome` → `Msg::JobDone` → `JobState` → `JobRowView`. Persisted via `PersistedJob` for restart correctness. Fixed reducer tab-selection logic that was overriding any non-PromptLab tab to `JobList` by splitting `close_prompt_lab()` into `close_prompt_lab_internals()` (tab-selection concern) and `set_left_tab()`. Extracted shared `passes_since_filter_dt` / `passes_since_filter_str` helpers into `harvester_engine/src/since_filter.rs` to keep semantics consistent across Archive, Briefing, and the new tab while respecting the crate dependency graph.
Inclusion semantics: Since Checkpoint intentionally includes in-flight and failed jobs (fetched_utc = None → include by default), unlike Archive/Briefing which only see completed on-disk articles. This gives users visibility into work in progress.
Evidence: `cargo build --all-targets` clean. 280 harvester_core tests pass (1 pre-existing failure in `poll_burst_waits_for_engine_jobs_to_drain_before_dispatching` unrelated to this change). 95 harvester_engine, 59 harvester_io, 0 harvester_app lib test failures. `cargo clippy --all-targets -- -D warnings` clean.
Lessons Learned: `JobState` constructors exist in multiple inline test functions scattered across state.rs — track all callsites when extending a struct. The `close_prompt_lab()` function conflated two concerns (tab selection + lab state); splitting them made `Msg::LeftTabSelected` correctly handle any tab without side-effect overrides.
Refs: crates/harvester_engine/src/since_filter.rs, crates/harvester_core/src/tabs.rs, crates/harvester_core/src/state.rs, crates/harvester_core/src/update.rs, crates/harvester_app/src/platform/ui/render.rs, docs/plans/Plan.since-checkpoint-tab-design.md

## 2026-03-03 - PromptLab URL resolve now avoids corpus-wide content prep
Type: Bug Fix
Context: Selecting a single article triggered `ResolvePromptLabInputFromUrl`, but the loader path performed a full archive scan and `content_prep` derivation for every markdown file before filtering to one URL. This produced large INFO bursts and unnecessary work/latency for a single click.
Change: Added a single-URL fast path in `harvester_engine` filtered loading: when exactly one URL is requested, the loader now scans markdown frontmatter in deterministic order, resolves URL aliases, and derives content only for the first matching article instead of deriving all articles. Multi-URL behavior remains unchanged. Affected subsystems: `harvester_engine`, `harvester_io`.
Evidence: `cargo test -p harvester_engine filtered_loader_single_selection_ignores_unrelated_invalid_markdown -- --nocapture`; `cargo test -p harvester_io resolve_effect_success_emits_ok_msg -- --nocapture`; `cargo build`; `cargo clippy --all-targets -- -D warnings`.
Lessons Learned: Reusing a bulk-oriented data path for single-item interactions can silently turn O(1)-intent UI actions into O(N)-workload side effects, which then look like "log noise" but are real compute amplification.
Prevention: Add explicit single-item code paths (or indexed lookups) for URL-targeted workflows and require tests that include unrelated malformed corpus entries so single-item resolve remains isolated from archive-wide scanning failures.
Refs: crates/harvester_engine, crates/harvester_io, filtered_loader_single_selection_ignores_unrelated_invalid_markdown

## 2026-03-03 - Left-tab jobs/triage reorganization completion pass
Type: Implementation
Context: The Jobs/Triage left-tab IA plan had landed partially, but key operator-facing pieces were still missing: a reducer-wired scope toggle in the jobs pane, explicit tab/scope event coverage, and integration-style burst safety coverage for tab/scope switches.
Change: Completed the missing slice across `harvester_app` and `harvester_core`: added a `Since checkpoint only` jobs-pane checkbox control wired to `Msg::JobListScopeSet { scope }`, synchronized checkbox render state from `job_list_scope`, added neutral jobs-header placeholder messaging for empty triage/review conditions, and added new app/layout/render/core integration tests for left-tab mapping, scope events, jobs-pane visibility on all job-oriented tabs, and burst updates with tab/scope switching.
Evidence: `cargo build`; `cargo test -p harvester_app`; `cargo test -p harvester_core --test left_tab_scope_integration`; `cargo clippy --all-targets -- -D warnings`.
Refs: crates/harvester_app/src/platform/ui/constants.rs, crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs, crates/harvester_app/src/platform/app.rs, crates/harvester_core/tests/left_tab_scope_integration.rs, docs/plans/Plan.left-tabs-jobs-triage-reorganization.md

## 2026-03-03 - CommanDuctUI layout/checkbox hardening
Type: Implementation
Context: A visible UI regression risk remained in any app using CommanDuctUI with docked header rows: layout rules could silently accept invalid dock sizing, and checkbox rows relied on fixed pixel heights that were brittle under DPI/font scaling.
Change: Hardened `commanductui` by validating docked edge rules (`Top/Bottom/Left/Right`) require non-negative `fixed_size`, added DPI-aware minimum checkbox height helpers, and enforced minimum native checkbox height in layout application for `ControlKind::CheckBox`. Added regression tests for header+checkbox+fill non-overlap and new validation failure cases. Bumped submodule version to `0.7.2` and updated its changelog.
Evidence: `cargo test -p commanductui`; `cargo build`; `cargo clippy --all-targets -- -D warnings`.
Refs: src/CommanDuctUI/src/window_common.rs, src/CommanDuctUI/src/controls/checkbox_handler.rs, src/CommanDuctUI/Cargo.toml, src/CommanDuctUI/CHANGELOG.md

## 2026-03-04 - Token progress now respects Since checkpoint scope
Type: Bug Fix
Context: The jobs-pane `Since checkpoint only` scope correctly filtered visible rows, but the top token progress bar continued to use all-time totals, which made the scoped view misleading and inconsistent with operator intent.
Change: Updated `harvester_app` token-progress rendering to compute the numerator from scope-filtered jobs when `JobListScope::SinceCheckpoint` is active, while preserving all-time behavior for `JobListScope::All`. Added a render regression test that asserts both label text and progress bar position use only since-checkpoint token totals in scoped mode.
Evidence: `cargo test -p harvester_app token_progress_uses_since_checkpoint_scope_total_when_enabled -- --nocapture`; `cargo clippy --all-targets -- -D warnings`.
Lessons Learned: Scope toggles that filter rows should also drive summary metrics in the same surface, otherwise the UI can present internally inconsistent state even when each piece is individually correct.
Prevention: For each new scope/filter control, add explicit render tests for aggregate labels and progress indicators (not only row visibility) so scoped metrics regressions are caught early.
Refs: crates/harvester_app/src/platform/ui/render.rs, token_progress_uses_since_checkpoint_scope_total_when_enabled

## 2026-03-05 - Batch loop tick injection and output simplification
Type: Bug Fix
Context: `harvester_batch` cycles were settling with pre-triage/triage/summary counters at zero because the dispatch loop never emitted `Msg::Tick`, so reducer-owned pre-triage refresh dispatch never ran.
Change: Updated `harvester_batch` dispatch loop to emit cadence-guarded ticks (75 ms) before AI orchestration, skip same-iteration settlement when orchestration enqueues a follow-up action, and block settlement during active pre-triage work phases (`LoadingArticles`, `Reviewing`). AI orchestration/ticks are now enabled only when `OPENAI_API_KEY` is present and non-empty, preventing no-LLM runs from stalling in triage orchestration and restoring regular cycle completion/output cadence. Simplified cycle output to `Jobs(new/done/fail)`, `Triage(ok/fail)`, and `Summaries(ok/fail)`, and changed final summary to a single line including total new articles.
Evidence: `cargo test -p harvester_batch`; `cargo build`; `cargo clippy --all-targets -- -D warnings`.
Lessons Learned: Batch/headless loops that rely on reducer-owned time coordination must inject periodic ticks explicitly; omitting the tick path silently disables downstream pipelines even when all other orchestration code is present.
Prevention: Add a batch-runner checklist item and regression tests that verify `Action -> Tick -> Effect -> Action` loops execute under headless dispatch (including orchestration handoff settlement behavior).
Refs: crates/harvester_batch/src/runner.rs, runner::tests::test_dispatch_loop_ticks_drive_pretriage_from_restore_signal

## 2026-03-06 - Article click defaults to triage when summary is missing
Type: Bug Fix
Context: Clicking an article always switched the right pane to the Summary tab, which produced an empty/placeholder summary view for unsummarized articles even when triage details existed.
Change: Updated `harvester_core` selection reducer behavior to choose the right-pane tab by selected-article summary availability: `Summary` when a completed summary exists, otherwise `Triage`. Added reducer tests for both no-summary and summary-present paths.
Evidence: `cargo test -p harvester_core job_selected_without_summary_selects_triage_tab_and_requests_resolve`; `cargo test -p harvester_core job_selected_with_summary_selects_summary_tab`; `cargo build`; `cargo clippy --all-targets -- -D warnings`.
Lessons Learned: Selection actions that imply view focus should derive from content availability in reducer state, not fixed tab defaults, to avoid empty-first UX paths.
Prevention: For each selection-driven tab switch, add paired reducer tests that assert behavior for both data-present and data-missing states.
Refs: crates/harvester_core/src/update.rs, crates/harvester_core/src/state.rs

## 2026-03-06 - Launcher tests shifted from constant literals to behavior checks
Type: Decision
Context: Launcher Pester tests contained many assertions that hard-coded configuration literals (default values/counts), causing maintenance churn without improving confidence in runtime behavior.
Change: Updated the launcher test strategy to focus on behavior/data flow (state to argv/effects, clamping, and value propagation) and removed constant-only assertions from the data/default sections.
Evidence: `Invoke-Pester -Path scripts/tests/HarvesterLauncher.Tests.ps1` (130 passed).
Refs: scripts/tests/HarvesterLauncher.Tests.ps1

## 2026-03-07 - Triage click now opens Triage Results tab
Type: Bug Fix
Context: Clicking `Triage articles` from the Jobs tab could leave the left pane on `Jobs`, making it unclear that triage mode is a global workflow transition and not only a background operation.
Change: Updated `harvester_core` reducer handling for `Msg::TriageClicked` to select `LeftTab::TriageResults` once triage preconditions are satisfied (before metadata-load fallback), and added a reducer test that locks the Jobs -> Triage Results transition when triage starts.
Evidence: `cargo test -p harvester_core triage_clicked_switches_to_triage_results_tab_when_triage_can_start -- --nocapture`; `cargo test -p harvester_core --test left_tab_scope_integration -- --nocapture`; `cargo clippy --all-targets -- -D warnings`; `cargo build` attempted but blocked by locked `target/debug/harvester_app.exe` (os error 5).
Lessons Learned: Workflow-trigger actions should explicitly drive companion navigation state in the same reducer path; otherwise users can end up in a view that contradicts the active mode.
Prevention: Add reducer tests for every workflow entry action asserting both effect dispatch and expected pane/tab focus transitions.
Refs: crates/harvester_core/src/update.rs, update::tests::triage_clicked_switches_to_triage_results_tab_when_triage_can_start

## 2026-03-07 - Prompt Lab briefing runs now use history snapshot without mutating history
Type: Bug Fix
Context: Prompt Lab A/B testing for aggregate briefings needs prompt parity with production (`previous_briefings`/time-window guidance) while remaining state-isolated so one candidate run does not alter inputs for subsequent candidates.
Change: Updated `harvester_core` Prompt Lab aggregate-briefing dispatch to inject `previous_briefings` and `briefing_time_window` as read-only `extra_template_vars` snapshots, and added reducer tests proving Prompt Lab aggregate completion does not emit `SaveBriefingHistory` nor mutate briefing history state.
Evidence: `cargo test -p harvester_core prompt_lab_aggregate_request_includes_previous_briefings_extra_var -- --nocapture`; `cargo test -p harvester_core prompt_lab_aggregate_completion_does_not_update_history -- --nocapture`; `cargo build`; `cargo clippy --all-targets -- -D warnings`.
Lessons Learned: A/B labs must mirror production prompt inputs via explicit snapshot injection; otherwise quality comparisons are confounded by prompt mismatch rather than model/prompt differences.
Prevention: For each Prompt Lab stage, add explicit parity tests asserting required production template vars are present and isolation tests asserting no workflow-state mutations (history/checkpoints) on completion.
Refs: crates/harvester_core/src/update.rs, update::tests::prompt_lab_aggregate_request_includes_previous_briefings_extra_var

## 2026-03-07 - Jobs toolbar now defaults to checkpoint scope and co-locates token bar
Type: Implementation
Context: Operators requested a safer default for job scope and a denser top layout so `Since checkpoint` and token usage are visible in one row without scanning multiple header bands.
Change: Updated `harvester_core` default `JobListScope` to `SinceCheckpoint`, set the UI toggle initial state to enabled, and moved token text/progress controls into the same top toolbar row in `harvester_app` layout rules. Added layout/default regression tests to lock these contracts.
Evidence: `cargo test -p harvester_core job_list_scope_set_to_since_checkpoint_updates_state -- --nocapture`; `cargo test -p harvester_core job_list_scope_set_same_value_is_noop -- --nocapture`; `cargo test -p harvester_app toolbar_contains_scope_and_token_controls_on_same_row -- --nocapture`; `cargo test -p harvester_app new_controls_created_in_initial_commands -- --nocapture`; `cargo build` blocked by locked `target/debug/harvester_app.exe` (os error 5); `cargo clippy --all-targets -- -D warnings` currently fails in `src/CommanDuctUI/src/controls/toggle_switch_handler.rs` on pre-existing `clippy::too_many_arguments`.
Refs: crates/harvester_core/src/tabs.rs, crates/harvester_core/src/update.rs, crates/harvester_app/src/platform/ui/layout.rs

## 2026-03-07 - Batch single-shot mode for one poll/triage/persist cycle
Type: Implementation
Context: Scheduled/manual batch runs needed a one-off mode that executes exactly one full cycle (poll, triage orchestration, persistence) and exits without entering the continuous loop.
Change: Added `--single-shot` in `harvester_batch` CLI (conflicting with `--dry-run`), made the batch loop terminate after the first settled cycle when enabled, and added a dedicated launcher TUI action (`Run single-shot (one cycle)`) that forwards `--single-shot` while omitting continuous-mode-only `--poll-interval`.
Evidence: `cargo test -p harvester_batch`; `Invoke-Pester -Path scripts/tests/HarvesterLauncher.Tests.ps1`; `cargo build`; `cargo clippy --all-targets -- -D warnings` still fails on pre-existing `clippy::too_many_arguments` in `src/CommanDuctUI/src/controls/toggle_switch_handler.rs`.
Refs: crates/harvester_batch/src/cli.rs, crates/harvester_batch/src/runner.rs, crates/harvester_batch/src/main.rs, scripts/Start-HarvesterBatch.ps1, scripts/harvester_launcher/Reducer.psm1

## 2026-03-07 - All batch runs now start with a fresh engine.log file
Type: Decision
Context: Operators want each `harvester_batch` invocation to be inspectable in isolation instead of appending onto previous runs and obscuring current-run behavior.
Change: Updated `harvester_batch` startup to truncate `engine.log` before logger initialization for every batch invocation, not only `--single-shot`.
Evidence: `cargo build`.
Refs: crates/harvester_batch/src/main.rs

## 2026-03-07 - Partial batch outcomes no longer return nonzero exit status
Type: Decision
Context: Batch runs can complete useful work while still having some per-job failures; treating that as process failure caused `cargo run` to report an error even when the run completed normally.
Change: Updated `harvester_batch` exit-code policy so `PARTIAL` outcomes return `0`, while nonzero exit remains reserved for total/fatal failure.
Evidence: `cargo test -p harvester_batch`; `cargo build`.
Refs: crates/harvester_batch/src/runner.rs

## 2026-03-08 - Launcher import mode now prompts for input folder
Type: Implementation
Context: The batch launcher exposed import-mode flags in the right pane, but launching `Import saved webpages` still relied on a prefilled `Import dir` value instead of asking for the folder at the moment the import run started.
Change: Updated the PowerShell launcher reducer/effects flow so import activation requests an interactive folder prompt, validates the selected directory, and only then exits the TUI and launches `harvester_batch` in import mode. Added Pester coverage for the prompt effect and the new reducer transition.
Evidence: `Invoke-Pester -Path scripts/tests/HarvesterLauncher.Tests.ps1`; `cargo clippy --workspace --all-targets -- -D warnings`.
Refs: scripts/Start-HarvesterBatch.ps1, scripts/harvester_launcher/Reducer.psm1, scripts/harvester_launcher/Effects.psm1, scripts/tests/HarvesterLauncher.Tests.ps1

## 2026-03-08 - Imported articles now persist into app-visible completed jobs
Type: Bug Fix
Context: Import mode wrote archive markdown files successfully, but `harvester_app` restores its Jobs list from `.harvester_state.ron`. Because import mode never projected imported archive refs into completed-job snapshots, articles imported after a checkpoint were absent from the app and from the `Since checkpoint` view after restart.
Change: Updated `harvester_core` import completion to register imported archive refs as successful completed jobs with `fetched_utc`, and updated `harvester_batch` import-mode shutdown to merge those imported snapshots with the previously persisted completed jobs before writing state.
Evidence: `cargo test -p harvester_core import_completion_projects_imported_entries_into_completed_jobs_snapshot -- --nocapture`; `cargo test -p harvester_batch import_mode_persistence_merge_preserves_existing_jobs_and_appends_imports -- --nocapture`; `cargo clippy --workspace --all-targets -- -D warnings`.
Lessons Learned: Archive persistence and app-visible state persistence are separate contracts; any feature that writes new corpus entries must explicitly update both paths or the UI will silently diverge from disk contents.
Prevention: Add a persistence-focused regression test for every non-poll ingestion path asserting that newly created archive entries appear in `CompletedJobSnapshot` output and survive app restart through `.harvester_state.ron`.
Refs: crates/harvester_core/src/update.rs, crates/harvester_core/src/state.rs, crates/harvester_batch/src/runner.rs

## 2026-03-09 - Briefing output now uses top stories on gpt-5-mini
Type: Implementation
Context: The aggregate briefing needed to stay concise at the top while becoming more operationally useful below the summary. The previous `themes` section was too abstract for day-to-day review compared with a ranked list of concrete stories.
Change: Added a new aggregate briefing prompt version that keeps the executive summary but returns up to five ranked `top_stories` with a 150-word cap per story, updated the validation/render/history pipeline to use that schema while remaining compatible with older `themes` outputs/history, and set the default briefing model to `gpt-5-mini` in both app and batch flows.
Evidence: `cargo test -p harvester_engine`; `cargo test -p harvester_core`; `cargo test -p harvester_io`; `cargo test -p harvester_batch`; `cargo check -p harvester_app`; `cargo clippy --all-targets -- -D warnings`; `cargo build` still blocked by locked `target/debug/harvester_app.exe` (os error 5).
Refs: crates/harvester_engine/src/llm/prompts/briefing.rs, crates/harvester_engine/src/llm/validation.rs, crates/harvester_core/src/briefing.rs, crates/harvester_app/src/platform/app.rs, crates/harvester_batch/src/runner.rs

## 2026-03-09 - Briefing failures now surface in the UI and nano is the default dev model
Type: Bug Fix
Context: A requested aggregate briefing could fail at the provider layer and leave the Briefing pane blank, which looked like an empty successful run instead of an error. During prompt and UI iteration, the default briefing model also needed to be cheaper than `gpt-5-mini`.
Change: Updated `harvester_core` briefing state/rendering so aggregate briefing failures surface an explicit failure message in both progress text and preview markdown, added reducer coverage that successful aggregate briefing completions contribute model token usage to the status bar view data, and switched the default briefing model in `harvester_app` and `harvester_batch` to `gpt-5-nano`.
Evidence: `cargo test -p harvester_core`; `cargo test -p harvester_io`; `cargo test -p harvester_batch`; `cargo check -p harvester_app`; `cargo clippy --all-targets -- -D warnings`.
Lessons Learned: For async LLM features, "no content" is not a safe fallback UI state because transport failures become indistinguishable from valid empty results unless the reducer preserves failure information.
Prevention: Add flow-level tests for both success and failure branches of each LLM-backed feature, including assertions on user-visible status text and usage accounting rather than only internal completion state.
Refs: crates/harvester_core/src/briefing.rs, crates/harvester_core/src/update.rs, crates/harvester_app/src/platform/app.rs, crates/harvester_batch/src/runner.rs

## 2026-03-11 - Resize responsiveness restored without blanking the jobs tree
Type: Bug Fix
Context: Dragging the main window border or pane splitter became nearly unresponsive after the TreeView selection-styling update, especially with real job data loaded. Early redraw-suspension mitigations improved throughput but caused blank-pane artifacts, which showed the true fix required separating geometry work from paint-time data costs rather than freezing the control.
Change: Updated `harvester_app` to keep live TreeView redraw enabled during resize while preserving erase suppression in `commanductui`, kept the lighter geometry-only render path for resize batches, and updated `harvester_core`/`harvester_app` so TreeView marker lookup reads pre-triage state directly from `AppState` instead of rebuilding the full view model during paint. Added unit coverage for the live-drag policy and for the direct marker-state lookup path.
Evidence: `cargo test` in `src/CommanDuctUI`; `cargo test -p harvester_core job_filter_status_reads_pre_triage_state_without_building_view -- --nocapture`; `cargo test -p harvester_app tree_item_marker_updates_with_link_state -- --nocapture`; `cargo build`; `cargo clippy --all-targets -- -D warnings`.
Lessons Learned: Interactive resize regressions can come from ordinary read-only helpers if they are invoked inside native paint loops; the right fix is usually to narrow the hot-path query, not to hide the cost by suspending redraw.
Prevention: Treat paint and custom-draw callbacks as performance-sensitive APIs, reject `view()` construction or broad collection scans in those paths during review, and validate resize behavior with realistic loaded data instead of empty-state fixtures.
Refs: crates/harvester_core/src/state.rs, crates/harvester_app/src/platform/app.rs, crates/harvester_app/src/platform/ui/render.rs, src/CommanDuctUI/src/window_common.rs, src/CommanDuctUI/CHANGELOG.md, state::tests::job_filter_status_reads_pre_triage_state_without_building_view

## 2026-03-11 - HTML-to-markdown cleanup hardening
Type: Implementation
Context: Large fetched and imported articles were carrying site chrome (nav, social widgets, newsletter blocks, recirculation cards, hydration payloads, CSS-in-JS) into markdown, wasting tokens and degrading preview, triage, and briefing quality. The prior extractor simply picked the first `<article>` or fell back to `<body>`, with no DOM pruning and no post-conversion cleanup. Fetch, import, and linked-page download paths each used different extractors and converters, causing divergent quality.
Change: Introduced a shared `content_extraction` module in `harvester_engine` with a full extraction pipeline: typed DOM prune policy, scored candidate selection (article/main/content classes with semantic bonus over body), DOM-pruned markdown conversion via an extended `LinkExtractingConverter`, block-oriented markdown cleanup, retention safeguards with pre-cleanup fallback, and per-stage diagnostics. Unified fetch (`engine.rs`), import (`import.rs`), and linked-page download (`effect_helpers.rs`) through one `ExtractionPipeline`. Import now uses `LinkExtractingConverter` instead of `Html2MdConverter`, eliminating converter divergence.
Evidence: 1038 tests pass; `cargo clippy --all-targets -- -D warnings` clean. 7 new integration fixture tests in `crates/harvester_engine/tests/extraction_pipeline_integration.rs` covering clean article preservation, nav/footer chrome exclusion, newsletter DOM-prune removal, CSS payload suppression, long-article non-regression, diagnostics integrity, and determinism.
Refs: harvester_engine::content_extraction, harvester_engine::links, harvester_engine::engine, harvester_engine::import, harvester_io::effect_helpers, extraction_pipeline_integration (test suite)

## 2026-03-11 - Remove --import-action and --trusted-manual-selection flags
Type: Implementation
Context: The `--import-action` flag (values: `import-only`, `summaries`, `briefing`) and `--trusted-manual-selection` were never used beyond `import-only`. Dead options added cognitive overhead to the CLI and the launcher TUI right-pane.
Change: Removed both CLI flags and all downstream infrastructure: `ImportActionArg` enum, `ImportAction` core enum, `action`/`trusted_manual_selection` fields from `ImportSessionState` and `Msg::ImportSavedWebpagesRequested`, the trusted-gate and action-dispatch block in the completed-import handler, `RunImportedCorpusSummaries`/`RunImportedCorpusBriefing` effect variants and their runner handlers, and the `TrustedManualSel`/`ImportAction` rows from the PowerShell launcher TUI. Affected crates: harvester_batch, harvester_core, harvester_io, harvester_launcher (PowerShell).
Evidence: 1012 tests pass; `cargo clippy --all-targets -- -D warnings` clean.
Refs: crates/harvester_batch/src/cli.rs, crates/harvester_core/src/import_session.rs, crates/harvester_core/src/msg.rs, crates/harvester_core/src/effect.rs, crates/harvester_io/src/effect_runner.rs, scripts/harvester_launcher/Data.psm1, scripts/harvester_launcher/Reducer.psm1

## 2026-03-13 - LLM auto-retry and cached-token observability
Type: Implementation
Context: AggregateBriefing call timed out at 60s, wasting all prior summary work. No retry logic existed. OpenAI prompt caching discount was not being tracked or applied to internal cost estimates. gpt-5-nano had no pricing entry so cost tracking was a silent no-op for the main briefing path.
Change: harvester_engine LLM subsystem — retry loop (MAX_ATTEMPTS=2, 1-based logging) in shared worker; HTTP timeout raised to 120s; gpt-5-nano added to default pricing registry; ModelPricing::new now accepts dollars per million tokens; TokenUsage extended with cached_input_tokens (clamped at builder); OpenAI response parsing extracts prompt_tokens_details.cached_tokens; exact 50% cached-token discount in pricing (ceil / 2_000_000); both [llm-run] log lines emit cached_input_tokens={}; LlmRunMetadata carries the field for replay/observability.
Evidence: 1055 tests pass; cargo clippy --all-targets -- -D warnings clean. 17 new tests across llm_types, llm_pricing, llm_openai, llm_handle (including retry_under_concurrency_pressure), llm_replay.
Refs: harvester_engine::llm::types, harvester_engine::llm::pricing, harvester_engine::llm::providers::openai, harvester_engine::llm::run_metadata, harvester_engine::llm::handle, commit 5fd83c7

## 2026-03-22 - Manual triage now respects the briefing checkpoint window
Type: Bug Fix
Context: After polling a small batch of new URLs, clicking `Triage` still prepared and triaged the full historical corpus because the pre-triage refresh path rebuilt from all completed-job URLs without carrying the active briefing checkpoint filter. This made manual triage inconsistent with Archive, Briefing, and the `Since checkpoint` UI scope.
Change: Threaded `briefing_since_utc` through `Effect::LoadArticlesForTriage` and into `harvester_io`'s pre-triage article loader so manual triage/pre-triage refresh now loads only articles on or after the active checkpoint. Added reducer and IO regression tests to lock the checkpoint-filtered dispatch and filtered load behavior.
Evidence: `cargo test -p harvester_core pre_triage_refresh_dispatch_includes_briefing_checkpoint_since_utc -- --nocapture`; `cargo test -p harvester_io load_articles_for_triage_respects_since_utc_filter -- --nocapture`.
Lessons Learned: Reusing a shared corpus-refresh pipeline is not enough if one workflow silently drops a governing filter parameter; temporal scope must be carried explicitly through every effect boundary that reconstructs article sets.
Prevention: For every feature that depends on the briefing checkpoint, add parity tests across Archive, Briefing, pre-triage refresh, and manual Triage so all paths prove they apply the same `since_utc` semantics.
Refs: crates/harvester_core/src/effect.rs, crates/harvester_core/src/update.rs, crates/harvester_io/src/effect_runner.rs, update::tests::pre_triage_refresh_dispatch_includes_briefing_checkpoint_since_utc, effect_runner::tests::load_articles_for_triage_respects_since_utc_filter

## 2026-03-22 - Archive export now confirms naming before writing
Type: Implementation
Context: Archive exports needed an explicit confirmation step so users could choose the output basename, see whether it already exists, and decide whether the briefing checkpoint should advance only after a successful export. The old flow exported immediately with a date-derived filename, which made overwrite intent and completion timing too implicit.
Change: Updated `harvester_engine`, `harvester_core`, `harvester_io`, `harvester_app`, and `CommanDuctUI` so archive export now flows through a request-id gated modal dialog, live basename validation, overwrite detection, and explicit submit/completion/failure messages. Added the generic form dialog primitive to `CommanDuctUI`, threaded archive basenames through engine export, and bumped the toolkit version/changelog.
Evidence: `cargo build`; `cargo test -p harvester_core archive_ -- --nocapture`; `cargo test -p harvester_io archive_requested_writes_archive_markdown_for_selected_urls -- --nocapture`; `cargo clippy --all-targets -- -D warnings`.
Refs: crates/harvester_core/src/update.rs, crates/harvester_core/src/msg.rs, crates/harvester_core/src/effect.rs, crates/harvester_engine/src/export.rs, crates/harvester_io/src/effect_runner.rs, crates/harvester_app/src/platform/app.rs, src/CommanDuctUI/src/types.rs, src/CommanDuctUI/src/controls/dialog_handler.rs

## 2026-03-22 - Archive dialog now follows ready pre-triage selection
Type: Bug Fix
Context: After a checkpoint-scoped pre-triage refresh, the jobs pane and token bar correctly reflected the current ready corpus, but archive export still counted URLs from the older `TriageSession`. That made the archive dialog claim there were no matching URLs even when pre-triage had already prepared includable articles.
Change: Updated `harvester_core` archive dialog open/submit handling to prefer `PreTriageSession::resolved_included_urls()` whenever pre-triage is ready, falling back to triage-policy results only when no ready pre-triage corpus exists. Added reducer regression tests covering both dialog article counts and submitted archive URL selection.
Evidence: `cargo test -p harvester_core archive_clicked_uses_ready_pre_triage_urls_for_article_count -- --nocapture`; `cargo test -p harvester_core archive_dialog_submitted_uses_ready_pre_triage_urls -- --nocapture`.
Lessons Learned: When two workflows render from adjacent but different state slices, any action that claims to operate on the visible corpus must be bound to the same authoritative slice as the UI, or it will drift into stale-session behavior.
Prevention: Add explicit parity tests for every corpus-derived action path so dialog counts, button enablement, and emitted effects all prove they use the same state source as the visible jobs/pre-triage UI.
Refs: crates/harvester_core/src/update.rs, update::tests::archive_clicked_uses_ready_pre_triage_urls_for_article_count, update::tests::archive_dialog_submitted_uses_ready_pre_triage_urls

## 2026-03-22 - Unify current working corpus selection
Type: Implementation
Context: Multiple workflows needed the answer to "what article set is current right now?", but that answer was derivable from adjacent state slices (PreTriageSession, TriageSession, checkpoint-scoped job views). The prior archive bug (dialog showed ready pre-triage corpus while action used stale triage data) exposed the weakness: two plausible answers could coexist silently, with divergence only discovered at runtime.
Change: harvester_core — introduced CurrentWorkingCorpus selector with explicit source enum (PreTriageReady > PreTriageReviewing > TriageComplete > Unavailable), stable FNV-1a fingerprint, and AppState::current_working_corpus() entry point. Archive actions migrated to use the selector with pinned dialog-open snapshot to eliminate the open/submit TOCTOU race. All other corpus-derived call sites audited and either migrated or documented as intentional exceptions. Parity tests added to prove UI-facing counts and emitted effect payloads agree. Observability logs added at selector and action dispatch boundaries.
Evidence: 389 harvester_core tests pass; all 11 plan test cases covered across working_corpus.rs and update.rs; `cargo clippy --all-targets -- -D warnings` clean.
Refs: harvester_core::working_corpus, harvester_core::state, harvester_core::update, working_corpus::tests::pre_triage_ready_wins_over_stale_triage, update::tests::parity_a_pre_triage_ready_corpus_count_dialog_count_urls_match, update::tests::refresh_between_open_and_submit_uses_pinned_snapshot, commit 561428b..046d61b

## 2026-03-22 - Archive exports triage-only; warns when pre-triage articles are excluded
Type: Bug Fix + Implementation
Context: After introducing the CurrentWorkingCorpus selector, ArchiveClicked used current_working_corpus() which returns PreTriageReady articles when available. This was wrong: pre-triage articles have not been through triage and should not be exported. Scenario: 5 triaged articles from a previous session + poll sources → Archive dialog showed the pre-triage articles, not the 5 triaged ones.
Change: harvester_core — added CurrentWorkingCorpus::select_for_archive (triage-only, ignores pre-triage entirely) and AppState::archive_corpus(). ArchiveClicked now uses archive_corpus(); pending_pre_triage_count computed from pre_triage().resolved_included_urls().len() unconditionally. harvester_app — warning row shown in archive dialog when pending_pre_triage_count > 0: "N articles await triage and are not included in this export."
Evidence: 1089 tests pass; cargo clippy --all-targets -- -D warnings clean.
Lessons Learned: current_working_corpus() is the right selector for the triage workflow (what to work on next), but archive has different semantics (what has been curated). Sharing one selector for both obscured the difference and silently exported un-triaged articles.
Prevention: When adding a new corpus-derived action, explicitly ask whether it should use the live working corpus or a domain-specific subset. Archive = triage-only is now enforced by a separate selector rather than relying on source-enum checks at call sites.
Refs: harvester_core::working_corpus::select_for_archive, harvester_core::state::archive_corpus, update::tests::archive_clicked_with_triage_complete_and_pre_triage_ready_sets_pending_count, update::tests::archive_clicked_with_only_pre_triage_ready_has_zero_article_count, commits 87818c9..8e21f7e

## 2026-03-22 - Centralize production OpenAI model IDs and move defaults to GPT-5.4
Type: Implementation
Context: Production model names had started to drift across app, batch, IO, and test helper code, which made pricing changes and default-model upgrades riskier than they should be. The undotted GPT-5 aliases also no longer matched the intended published GPT-5.4 family names.
Change: harvester_engine, harvester_app, harvester_batch, harvester_io, harvester_core — added shared OpenAI model-name constants in `harvester_engine::llm`, switched production/default model selection to those constants, and moved the GPT-5 defaults from undotted aliases to `gpt-5.4-*`. Intentional raw fixture strings in compatibility/provider tests were left literal where the test is about parsing external model identifiers.
Evidence: `cargo build`; `cargo test -p harvester_engine --test llm_pricing`; `cargo test -p harvester_core --test llm_usage`; `cargo test -p harvester_io build_local_model_catalog_uses_effective_models_with_dedup_and_sort -- --nocapture`; `cargo test -p harvester_app status_bar_includes_llm_usage_segment -- --nocapture`; `cargo clippy --all-targets -- -D warnings`.
Refs: harvester_engine::llm::mod, harvester_engine::llm::pricing, harvester_app::platform::app, harvester_batch::runner, harvester_io::effect_runner

## 2026-03-23 - Consume pre-triage when manual triage starts
Type: Bug Fix
Context: Manual triage could start from `PreTriageReady` while leaving the pre-triage session
in the same action-ready state afterward. That stale state forced archive-warning logic to
subtract URLs already present in triage and left the working-corpus selector vulnerable to
reporting an already-consumed pre-triage corpus as current.
Change: harvester_core — manual triage start now atomically consumes pre-triage articles and
resets the session to Idle via `consume_ready_pre_triage_articles_for_triage()`. Archive
pending-count logic simplified to rely on this invariant instead of set subtraction.
Evidence: Tests: triage_clicked_consumes_ready_pre_triage_into_triage_session,
triage_clicked_sets_current_working_corpus_to_unavailable_until_triage_completes,
archive_clicked_after_triage_start_has_zero_pending_pre_triage_count,
pre_triage_refresh_after_triage_start_repopulates_pre_triage_without_mutating_active_triage,
consume_ready_pre_triage_articles_for_triage_rejects_non_ready_phase,
consume_ready_pre_triage_articles_for_triage_returns_articles_and_resets_to_idle.
cargo build, cargo clippy --all-targets -- -D warnings passed.
Lessons Learned: Lifecycle handoff bugs are best fixed at the producer/consumer boundary;
downstream subtraction logic hides the symptom but leaves the state model inconsistent.
Prevention: Introduce domain-level consume/reset helpers for workflow handoffs and require
parity tests for every selector that reads corpus state after such transitions.
Refs: harvester_core::state, harvester_core::update, harvester_core::working_corpus

## 2026-03-24 - Briefing top-story body paragraphs need explicit list-item paragraph handling
Type: Bug Fix
Context: The briefing viewer rendered each top-story headline in bold but collapsed the following body paragraph onto the same visual line, reducing scanability in the highest-value section of the briefing.
Change: harvester_app — updated RichEdit markdown-to-RTF conversion so loose paragraphs inside ordered and unordered list items preserve list-item indentation while forcing later paragraphs onto their own line. Added a renderer regression test for the `1. **Headline**` plus body layout.
Evidence: `cargo test -p harvester_app loose_ordered_list_item_body_starts_on_new_paragraph -- --nocapture`; `cargo build`; `cargo clippy --all-targets -- -D warnings`.
Lessons Learned: Markdown list rendering cannot rely on list markers alone; once generated content uses loose list items, the renderer also needs item-local paragraph state to keep structure readable.
Prevention: Keep regression tests for every markdown shape emitted by app-generated views, especially ordered-list items with multiple paragraphs and mixed emphasis.
Refs: harvester_app::platform::ui::markdown_to_rtf, loose_ordered_list_item_body_starts_on_new_paragraph

## 2026-03-24 - Persist window size across launches
Type: Implementation
Context: The main window opened at hardcoded 960x720 every launch, requiring manual resize each session.
Change: Persist outer window dimensions in `.harvester_state.ron` via two new `Option<i32>` fields. Save triggers on `WM_EXITSIZEMOVE` (once per drag, not continuous `WM_SIZE`). Restore at startup with a minimum-size guard (both dimensions must be >= 960x720). Fixed `persist_runtime_state` to carry forward window size fields so job/override saves don't clobber persisted dimensions. CommanDuctUI 0.9.0 adds `AppEvent::WindowResizeCompleted` with outer dimensions from `GetWindowRect`.
Refs: harvester_io::persistence::{load_window_size, persist_window_size}, CommanDuctUI 0.9.0 CHANGELOG, docs/superpowers/specs/2026-03-24-persist-window-size-design.md

## 2026-03-25 - Remove duplicate archive overwrite warning
Type: Bug Fix
Context: The archive export dialog showed the overwrite warning twice when the default output file already existed, once as a top-level note and again as the filename field's live warning.
Change: harvester_app — removed the redundant top-level archive dialog warning and renamed the File menu action to `Archive...` so the UI better signals that opening the dialog does not immediately export.
Evidence: `cargo build`; `cargo clippy --all-targets -- -D warnings`.
Refs: harvester_app::platform::app, harvester_app::platform::ui::layout

## 2026-03-25 - TreeView selection accent restored
Type: Bug Fix
Context: The jobs tree still defined `TreeViewSelectionAccent`, but the blue left-edge accent was no longer visible for selected rows.
Change: Fixed CommanDuctUI TreeView post-paint selection accent drawing to resolve the selected row from the caret item after pre-paint suppresses the native selected state; added regression coverage for the accent draw decision and updated the CommanDuctUI changelog/version.
Lessons Learned: When custom draw mutates native state flags to suppress default rendering, later paint stages cannot safely reuse those flags as the source of truth.
Prevention: Keep draw-stage decisions in small pure helpers and add tests around the selection/accent gating logic whenever TreeView custom draw changes.
Refs: src/CommanDuctUI/src/controls/treeview_handler.rs, src/CommanDuctUI/CHANGELOG.md, docs/EngineeringDiary.md

## 2026-03-25 - TreeView keyboard selection updates preview
Type: Bug Fix
Context: Arrow-key navigation moved the TreeView selection, but only mouse clicks emitted the selection event that updates the preview pane.
Change: Routed user-driven TreeView selection changes through `TVN_SELCHANGEDW` and removed duplicate label-click selection dispatch from `NM_CLICK`, so mouse and keyboard navigation now share the same selection event path.
Lessons Learned: If equivalent user interactions travel through different Win32 notifications, UI behavior will drift unless the app chooses one authoritative event path.
Prevention: Prefer native selection-changed notifications over hit-test click inference for controls that support keyboard navigation, and keep mouse-only handlers focused on non-selection interactions such as checkbox toggles.
Refs: src/CommanDuctUI/src/controls/treeview_handler.rs, src/CommanDuctUI/src/window_common.rs, src/CommanDuctUI/CHANGELOG.md

## 2026-03-25 - Briefing tab header no longer leaks selected article metadata
Type: Bug Fix
Context: The preview header kept showing the last selected tree item while the Briefing tab displayed aggregate briefing content, which conflicted with the actual pane content.
Change: Added a mode-scoped preview header text override so the Briefing tab now renders an aggregate header (`Executive Briefing | N articles | scope | status`) while article-specific headers remain on Triage and Summary.
Lessons Learned: Shared chrome above a multi-mode pane must be driven by the visible mode, not by preserved selection context from another mode.
Prevention: When a pane can override selected-item content, expose explicit header text in the view model instead of implicitly deriving all headers from selection.
Refs: crates/harvester_core/src/state.rs, crates/harvester_core/src/view_model.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-03-25 - Trends tab high-value pass
Type: Implementation
Context: Trends chart lacked axis labels, showed 10 undifferentiated lines, and ranked by stale total counts. Implemented Plan.TrendsHighValuePass.md.
Change: (A) CommanDuctUI 0.10.0 — ChartLineData gains end_label/emphasis; ChartDataPacket gains show_x/y_axis_labels + show_end_labels; chart_handler renders axis tick labels with measured left margin and stride-skipped x labels. (B) CommanDuctUI 0.10.1 — place_end_labels helper with deterministic y-spread and x/y clamping; Secondary lines muted via mute_color + 1px pen; single-point series drawn as dots; legend replaced by endpoint labels when show_end_labels=true. (C) harvester_core — recency score = latest*100 + prev1*40 + prev2*20 + total (u64); compute_category_trend sorted by score desc, display_label asc tie-break. (D) harvester_app layout — static "Top 5 products by recent activity, last 13 weeks" label above chart.
Refs: src/CommanDuctUI/src/controls/chart_handler.rs, src/CommanDuctUI/src/types.rs, crates/harvester_core/src/trends.rs, crates/harvester_app/src/platform/ui/render.rs, docs/plans/Plan.TrendsHighValuePass.md

## 2026-03-27 - Checkpoint persistence now has explicit success/failure acknowledgments
Type: Bug Fix
Context: Archive export and other checkpoint updates changed the reducer-owned in-memory checkpoint immediately, while the disk write still happened asynchronously without an acknowledgment. That allowed split-brain state: the UI could show a new checkpoint even if persistence failed, leaving restart behavior inconsistent with the live app.
Change: Added request-scoped checkpoint save acknowledgments from `harvester_io` back into the reducer, tracked a single pending checkpoint save in `harvester_core`, cleared the pending state only on matching success, and reverted the in-memory checkpoint plus surfaced a status-bar error on matching failure. Added reducer and IO regression tests for success, failure, and stale-ack handling.
Lessons Learned: For persisted workflow state, optimistic reducer updates need a correlated completion protocol; otherwise success and failure can no longer be reasoned about from state alone.
Prevention: Treat async persistence that affects user-visible workflow scope as a request/ack state machine with correlation ids, stale-ack tests, and explicit rollback behavior on failure.
Evidence: `cargo test -p harvester_core briefing_checkpoint_save -- --nocapture`; `cargo test -p harvester_io save_briefing_checkpoint_dispatches -- --nocapture`; `cargo build`; `cargo clippy --all-targets -- -D warnings`.
Refs: crates/harvester_core/src/update.rs, crates/harvester_core/src/state.rs, crates/harvester_core/src/msg.rs, crates/harvester_core/src/effect.rs, crates/harvester_io/src/effect_runner.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-03-30 - Brave Search News API integration (feature/brave-slice-a)
Type: Implementation
Context: Pipeline was passive (RSS/file/curated). Added Brave News API as an active source type so it can hunt by query, not only consume feeds.
Change: New `SourceType::BraveNews(BraveNewsSourceConfig)` slots into the existing poll loop. Pure parse in `harvester_engine::brave_poll`; HTTP fetch and API-key resolution in `harvester_io::effect_helpers`. Two dedup layers: `BraveSeenSet` (cross-cycle, URL-keyed, bounded FIFO eviction) in `harvester_engine` for Brave-specific dedup; `AppState.ingest_urls` (in-session) for reducer-level dedup. `BravePollContext` struct groups the 4+ mutable args to `handle_brave_source_poll` to stay within clippy's argument-count limit. Brave metadata (title, description, age) persisted in a sidecar store (`.brave_metadata.ron`) for future triage/preview. Per-source poll stats (`parsed → dedup-filtered → emitted`) added to dry-run and `--single-shot` stdout, grouped by source type. `SourcePollStat` recorded by reducer after `ingest_urls` so `emitted` reflects actual enqueued count. `normalize_url_for_dedupe` moved to `harvester_engine::brave_seen_set` (canonical home); `harvester_core` re-exports it to avoid circular dependency. `reqwest::blocking` does not support `.query()` without additional feature flags — use `Url::query_pairs_mut()` to build query strings manually. Lock behavior improved: stale-lock error now printed to stderr (was log-only); Ctrl-C removes the lock file and exits immediately via signal handler.
Refs: crates/harvester_engine/src/brave_poll.rs, crates/harvester_engine/src/brave_seen_set.rs, crates/harvester_io/src/effect_helpers.rs, crates/harvester_io/src/seen_set_store.rs, crates/harvester_core/src/source_state.rs, crates/harvester_batch/src/lock.rs, crates/harvester_core/tests/brave_integration.rs, docs/plans/Plan.BraveSearchIntegration.md

## 2026-03-31 - Poll stats viewer now emits markdown-shaped sections
Type: Bug Fix
Context: The Poll Stats tab rendered one dense paragraph because the RichEdit markdown path treats single newlines as soft wraps, so the formatter's plain newline-separated output lost its intended line breaks.
Change: Updated the shared poll-stats formatter to emit markdown headings, blank lines, bold summary lines, and bullet rows per source; added formatter regression tests for the sectioned output.
Lessons Learned: When a view is rendered through markdown, formatter output must encode structure with markdown paragraphs/lists instead of relying on raw newline placement.
Prevention: Add formatter-level regression tests for every app-owned markdown shape and treat single-line newline joins as suspicious in markdown-backed panes.
Refs: crates/harvester_core/src/poll_stats_fmt.rs, poll_stats_fmt::tests::formats_groups_as_markdown_sections_and_bullets

## 2026-03-31 - Headings after lists now force a new paragraph in RichEdit markdown
Type: Bug Fix
Context: In the Poll Stats tab, the next section heading could attach to the final bullet line because the markdown-to-RTF converter ended lists without emitting a paragraph break.
Change: Updated list-end handling in the RichEdit markdown renderer to emit `\\par` after a list and added a regression test for a heading immediately following bullet items; refreshed the renderer snapshot accordingly.
Lessons Learned: Block-level markdown boundaries need explicit paragraph termination in the RTF layer or unrelated sections can visually merge despite correct markdown input.
Prevention: Keep renderer regression tests for adjacent block shapes such as list-to-heading and list-to-paragraph transitions, not only isolated element snapshots.
Refs: crates/harvester_app/src/platform/ui/markdown_to_rtf.rs, platform::ui::markdown_to_rtf::tests::heading_after_list_starts_on_new_paragraph

## 2026-03-31 - Rewrite engine tests toward typed contracts instead of serialization details
Type: Bug Fix
Context: Chunk 4 unit-test review found several `harvester_engine` tests asserting exact hash literals, prompt collection markers, browser-header strings, and raw manifest JSON substrings. Those tests failed under harmless refactors because they were pinned to representations rather than behavior.
Change: Reworked the affected tests to assert cleaned-content removal and hash determinism, collection selection via unique article bodies and prefix retention, browser-header semantics instead of exact q-values, and manifest fields via parsed JSON.
Lessons Learned: Tests that observe behavior through serialized intermediates drift toward implementation locks unless the representation is itself a public compatibility boundary.
Prevention: Prefer typed assertions and semantic fixtures; when representation format must be locked, isolate that in a narrow formatting test with an explicit compatibility rationale.
Refs: crates/harvester_engine/tests/content_prep_integration.rs, crates/harvester_engine/tests/briefing_loader_integration.rs, crates/harvester_engine/tests/fetch.rs, crates/harvester_engine/tests/output.rs, docs/plans/Findings.UnitTestReviewChunk4.md

## 2026-03-31 - Relax helper-heavy engine tests to stable behavioral contracts
Type: Bug Fix
Context: Chunk 5 unit-test review found several `harvester_engine` tests asserting detector label strings, prompt wrapper text, selector score/tie-break internals, and upstream parser wording. Those checks created churn under safe heuristic refactors without protecting user-visible behavior.
Change: Reworked the affected tests to assert boilerplate removal plus recorded detection, prepared-collection article count/content ordering, candidate-selection outcomes and fallback behavior, and RSS parse failure category with non-empty context rather than dependency-specific wording.
Lessons Learned: The root cause was asserting whichever internal diagnostic was easiest to read from helper tests instead of first choosing the narrowest stable contract. Heuristic modules are especially prone to this because labels, scores, and selector order feel observable while still being tuning details.
Prevention: Use correctness by construction for tests by separating stable outcomes from diagnostics: expose typed outcomes/variants for durable contracts, keep diagnostic strings and numeric scores as optional observability, and require each heuristic test to answer "would this still pass after safe tuning with identical extraction output?" before locking an assertion.
Refs: crates/harvester_engine/src/content_prep/boilerplate.rs, crates/harvester_engine/src/content_prep/budget.rs, crates/harvester_engine/src/content_extraction/candidate_select.rs, crates/harvester_engine/src/rss_parse.rs, docs/plans/Findings.UnitTestReviewChunk5.md

## 2026-03-31 - Candidate selection now separates contract from diagnostics
Type: Implementation
Context: Follow-up to the Chunk 5 test review. `candidate_select` returned one struct mixing durable selection facts with tuning diagnostics, which made both production code and tests treat heuristic score details like part of the contract.
Change: Introduced `CandidateSelection` and `CandidateSelectionDiagnostics` in `candidate_select`, with `SelectedCandidate` bundling the DOM element plus the split data. The stable contract now answers "what kind of container was selected?" and "was body fallback used?" while the numeric score remains diagnostic-only. `ExtractionPipeline` was updated to consume the split result and still populate extraction diagnostics/logging.
Refs: crates/harvester_engine/src/content_extraction/candidate_select.rs, crates/harvester_engine/src/content_extraction/pipeline.rs

## 2026-03-31 - Rework LLM tests around stable contracts instead of shipped revisions and catalog snapshots
Type: Bug Fix
Context: Chunk 6 unit-test review found LLM tests asserting exact prompt revision counts, prompt wording, current OpenAI model ids, and default price-table literals. Those checks created churn under safe prompt/catalog updates and also exposed a validation gap: aggregate briefing template validation did not include the runtime vars for `previous_briefings` and `briefing_time_window`.
Change: Reworked the affected `harvester_engine` tests to assert registry consistency, active-latest prompt behavior, older-version accessibility, schema/variable validation, and provider category filtering with synthetic ids. Extended `validate_template()` synthetic aggregate-briefing vars so built-in briefing prompts validate through the same contract used by Prompt Lab.
Lessons Learned: The root cause was letting mutable policy data and copy revisions masquerade as unit contracts. When tests observe whichever literal is easiest to assert, prompt/version churn and provider catalog churn become false regressions.
Prevention: Use correctness by construction for prompt/catalog tests by separating stable contracts from mutable data: validate built-in prompts through one shared template-validation boundary with all supported runtime vars, express default-registry tests in terms of repo-owned defaults and exported aliases, and use synthetic fixtures for provider category rules instead of live catalog names.
Refs: crates/harvester_engine/tests/llm_pricing.rs, crates/harvester_engine/tests/llm_prompt.rs, crates/harvester_engine/tests/llm_openai.rs, crates/harvester_engine/src/llm/prompts/briefing.rs, crates/harvester_engine/src/llm/prompts/mod.rs, crates/harvester_engine/src/llm/template_validation.rs, docs/plans/Findings.UnitTestReviewChunk6.md

## 2026-03-31 - Blocked-page detection now uses typed blocker kinds
Type: Implementation
Context: `blocker_page` exposed free-form reason strings as its primary result, which made wording look like the contract even though the durable fact is the blocker class.
Change: Replaced `Option<String>` with `Option<BlockedPageKind>` in `blocker_page`, implemented `Display` for log/user text, updated engine handling to format the enum into `FailureKind::BlockedContent`, and added regression coverage for generic consent and captcha-content cases.
Refs: crates/harvester_engine/src/blocker_page.rs, crates/harvester_engine/src/engine.rs

## 2026-03-31 - Rewrite chunk 7 tests around boundary semantics instead of formatting details
Type: Bug Fix
Context: Chunk 7 unit-test review found IO, UI, and batch tests asserting serializer details, exact copy, and log-string wording that could change under safe refactors without any behavioral regression.
Change: Reworked the affected tests to assert prompt save round-trips, archive export completion plus selected-document inclusion/exclusion, markdown viewer structure via minimal RTF markers, UI state changes via rendered control semantics, and batch log helpers via compaction/truncation behavior rather than exact phrasing.
Lessons Learned: The root cause was treating the easiest observable string as the contract instead of first choosing the narrowest stable boundary; once tests lock raw representations, harmless formatting drift looks like a correctness failure.
Prevention: Use correctness by construction in tests by preferring typed round-trip loads, request/ack messages, and semantic render helpers over raw file substrings or full snapshots; if a format string is truly a contract, isolate it in one compatibility-focused test and document why it is stable.
Refs: crates/harvester_io/src/effect_runner.rs, crates/harvester_app/src/platform/ui/markdown_to_rtf.rs, crates/harvester_app/src/platform/ui/render.rs, crates/harvester_batch/src/runner.rs, docs/plans/Findings.UnitTestReviewChunk7.md

## 2026-03-31 - Rework CommanDuctUI tests around generic infrastructure contracts
Type: Bug Fix
Context: Chunk 8 unit-test review found `CommanDuctUI` tests encoding Harvester-flavored menu fixtures, exact diagnostic wording, and default palette literals even though the framework contracts are generic action routing, layout invariant rejection, control identification, and dark-theme-safe defaults.
Change: Replaced app-shaped menu ids/labels with neutral semantic fixtures, relaxed layout and HWND-description assertions to invariant-level metadata, and rewrote the default tab-bar palette test to verify dark-background, readable-text, and blue-accent semantics instead of pinned channel constants.
Lessons Learned: The root cause was test data and assertions being chosen for convenience from the hosting app and current debug strings, rather than being derived from the framework invariants first. Infrastructure tests drift toward accidental product coupling unless fixtures and checks are intentionally generic.
Prevention: Use correctness by construction for infrastructure tests by giving generic modules generic fixture builders, asserting typed invariants or invariant-bearing fields before display text, and isolating any true formatting-compatibility requirement in a narrowly named test with an explicit rationale.
Refs: src/CommanDuctUI/src/controls/menu_handler.rs, src/CommanDuctUI/src/window_common.rs, src/CommanDuctUI/src/app.rs, src/CommanDuctUI/src/controls/tab_bar_handler.rs, docs/plans/Findings.UnitTestReviewChunk8.md

## 2026-03-31 - Rework chunk 9 PowerShell/TUI tests toward semantic boundaries
Type: Bug Fix
Context: Chunk 9 unit-test review found launcher and browser TUI tests pinning current status words, checkpoint action ids, selection glyphs, helper serialization strings, and empty-state copy even though the durable contracts are reducer/effect behavior, visible selection, layout shape, and user-supplied render data.
Change: Relaxed the affected Pester tests to assert runtime state transitions, effect emission, forwarded custom dates, visible selection markers, command-preview/checkpoint values derived from state, and non-empty empty-state rendering instead of current copy or helper string formats.
Lessons Learned: The root cause was using whichever string or glyph was easiest to observe from a TUI test, which turned transient presentation and internal dispatch ids into accidental contracts.
Prevention: Use correctness by construction for these suites by deriving expectations from reducer-owned state and source-of-truth data tables, adding semantic helper assertions for visible selection/empty states, and only locking exact text or ids when they are intentional public protocols.
Refs: scripts/tests/HarvesterLauncher.Tests.ps1, ministry-of-future-plans/tests/Render.Tests.ps1, docs/plans/Findings.UnitTestReviewChunk9.md

## 2026-04-02 - AI unavailable UX for triage and briefing
Type: Bug Fix
Context: Starting the app without OPENAI_API_KEY left the UI in a misleading dead-end where triage could not run but Triage Results still looked like a normal empty state.
Change: Added explicit reducer-owned AI availability state, fed startup missing-key and metadata-derived no-model signals into it, disabled triage/briefing actions from the view model, surfaced a warning in the status bar, and changed triage/briefing empty-state behavior to show configuration-aware messaging.
Lessons Learned: Configuration blockers need first-class reducer state rather than being inferred from logs or scattered metadata checks in render.
Prevention: Keep operator-visible availability reasons in core state and require render tests for blocked-action copy and warning severity when adding new feature gates.
Refs: crates/harvester_core/src/state.rs, crates/harvester_core/src/update.rs, crates/harvester_app/src/platform/app.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-04-02 - Deferred triage-results resorting to stop tree flicker
Type: Bug Fix
Context: While triage was running, the Triage Results left-pane view kept reordering rows by priority as each result arrived. That changed TreeView sibling order on nearly every update, so the current render path fell back to full `PopulateTreeView`, visibly clearing and rebuilding the pane mid-run.
Change: Updated `harvester_app` tree rendering so Triage Results keeps stable job order while triage is in flight and only applies priority sorting after triage settles. Added render regression tests covering stable in-flight order and in-place text updates without full repopulation.
Lessons Learned: When a UI control lacks cheap item reordering, continuously changing sort order turns innocuous data updates into full-structure repaints and visible flicker.
Prevention: Treat in-flight list ordering as a rendering contract; for TreeView-backed surfaces, prefer stable order during streaming updates and add tests that fail if progress-only updates emit `PopulateTreeView`.
Refs: crates/harvester_app/src/platform/ui/render.rs

## 2026-04-02 - Removed tiny job tree status dots
Type: Bug Fix
Context: Jobs in the treeview showed very small colored markers for review and exclusion states. The markers were visually ambiguous and looked like dead pixels while the row text already carried the same state.
Change: Stopped assigning tree-item markers to job rows and kept markers only for link download state, where compact color cues still add signal.
Lessons Learned: Tiny unlabeled status markers can read as rendering defects rather than useful feedback when the row already includes explicit text.
Prevention: Prefer text for job-level state in tree rows and add focused UI regression tests before introducing compact markers.
Refs: crates/harvester_app/src/platform/app.rs, docs/EngineeringDiary.md

## 2026-04-03 - Warm dark visual redesign foundation
Type: Implementation
Context: The UI redesign shifted Harvester from a cool blue-gray palette to warm neutrals with a terracotta accent while preserving the existing Win32/CommanDuctUI architecture.
Change: Reworked `harvester_app` theme definitions, inline tab/toggle colors, reading-pane RTF typography, chart palette, and pane spacing. Added `StatusMeter`, `SectionTitle`, `PrimaryButton`, and `DestructiveButton` to CommanDuctUI so token usage, section headers, and button hierarchy can be styled semantically instead of overloading existing roles. Updated CommanDuctUI chart/dialog hardcoded colors to match the host theme; those surfaces remain hardcoded because the current infrastructure style model does not yet theme owner-drawn chart paint or modal dialog template rendering. Kept the redesign within Win32 constraints by using tonal contrast, typography, and margins instead of unsupported radius, shadow, hover, or alpha effects.
Refs: crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/markdown_to_rtf.rs, crates/harvester_app/src/platform/ui/render.rs, src/CommanDuctUI/src/styling_primitives.rs, src/CommanDuctUI/src/controls/chart_handler.rs, src/CommanDuctUI/src/controls/dialog_handler.rs

## 2026-04-03 - Screenshot follow-up pass for scan hierarchy and editorial reading
Type: Implementation
Context: The first warm-palette pass still left the jobs list too URL-dominant, the footer actions too visually even, and the reading pane too much like a utility block.
Change: Reworked jobs rows to lead with a short title or compact host/path label and push status/source/size metadata into a quieter trailing segment. Tuned the layout/theme layer to demote secondary buttons, soften section labels, widen viewer spacing, and quiet the splitter/footer chrome. Updated the markdown-to-RTF renderer to use softer body text, fixed editorial indents, stronger heading contrast, and roomier paragraph/list spacing. Added render and RTF regression tests for the new row-compaction and reading-surface contracts.
Refs: crates/harvester_app/src/platform/ui/render.rs, crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/markdown_to_rtf.rs

## 2026-04-03 - Triage results rows now scan by priority, title, and source
Type: Implementation
Context: Screenshot review showed the Triage Results tab still reading like a dense tag dump, with long tag lists pushing the title and source out of the scan path.
Change: Reformatted Triage Results rows to `P# · Category · title/slug · host · compact tags`, deriving a readable title from the URL slug when no summary title exists and collapsing long tag sets to two visible tags plus a remainder count. Updated render regression tests to lock the new in-place update and row-shape behavior.
Refs: crates/harvester_app/src/platform/ui/render.rs

## 2026-04-03 - Triage preview now reads like an editorial note
Type: Implementation
Context: After the results list was cleaned up, the right-hand Triage pane still surfaced raw diagnostics first, with inline category/priority/tags text and a header crowded by low-value document metrics.
Change: Reformatted triage preview content into a short markdown document with a title, a single compact tag line, and `Why It Matters`, reused that formatter for Prompt Lab triage previews, and simplified the preview header to emphasize source plus state instead of tokens/bytes/headings. Updated preview, state, and render tests to lock the new reading-surface contract.
Refs: crates/harvester_core/src/preview.rs, crates/harvester_core/src/state.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-04-03 - Token budget meter no longer dominates the header
Type: Implementation
Context: After the list and reading-surface cleanup, the top token budget strip had become the loudest element in the UI, competing with active tabs, selection, and primary actions.
Change: Reduced the toolbar height, capped the token meter width to a short fixed measure, compacted the count label, and kept the bar on muted styling until the budget is actually exhausted. Updated layout and render regression tests to lock the fixed-width meter, compact label copy, and muted-below-limit styling.
Refs: crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-04-03 - Triage hierarchy now leads with the article subject
Type: Implementation
Context: After the token strip was quieted, the remaining scan problem was triage hierarchy: both the results rows and the right-hand Triage pane still led with category/priority metadata instead of the article topic.
Change: Reformatted Triage Results rows to `P# Category: Title — host · tags` so the title is the sentence focus, and updated triage previews to use the article source title when available, with URL-slug fallback before showing category/priority metadata. Added regression coverage for row copy, preview headings, and source-title fallback behavior.
Refs: crates/harvester_app/src/platform/ui/render.rs, crates/harvester_core/src/preview.rs, crates/harvester_core/src/state.rs, crates/harvester_core/src/triage.rs

## 2026-04-03 - Triage priority dots anchored to the text lane
Type: Implementation
Context: The Triage Results marker plan needed a stable geometry pass after the earlier tiny-dot regression, and priority markers had to stay out of the Jobs and Triage Review tabs.
Change: Reworked CommanDuctUI TreeView marker placement to anchor from the label lane rather than a fixed offset, increased the dot size, and warmed the shared palette. Harvester now reads triage priority directly from reducer state for `TriageResults` job rows only, with regression tests covering the marker mapping and tab gating.
Refs: src/CommanDuctUI/src/controls/treeview_handler.rs, crates/harvester_app/src/platform/app.rs, crates/harvester_core/src/state.rs

## 2026-04-03 - TreeView marker lane no longer needs a checkbox underneath
Type: Bug Fix
Context: The new triage priority dots and link-status markers were visually colliding with the TreeView checkbox lane everywhere except interactive pre-triage review.
Change: Added `CheckState::Hidden` to CommanDuctUI so tree rows can reserve the state-image lane without rendering a checkbox, taught the handler to ignore clicks on hidden rows, and switched Harvester job/link rows outside `TriageReview` to that blank-lane mode. Also resized markers from the text height so the dot reads as intentional UI.
Refs: src/CommanDuctUI/src/types.rs, src/CommanDuctUI/src/controls/treeview_handler.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-04-03 - Hidden TreeView rows now keep a real marker lane
Type: Bug Fix
Context: Hiding the checkbox glyph by removing the state image entirely caused triage dots to slide left onto the expand button instead of occupying a clean lane.
Change: `CheckState::Hidden` rows now keep the TreeView state-image slot for layout, and CommanDuctUI erases the checkbox glyph in postpaint so the reserved gap remains available for markers.
Refs: src/CommanDuctUI/src/controls/treeview_handler.rs

## 2026-04-03 - Header hierarchy split into title and metadata rows
Type: Implementation
Context: The left pane collection title and the right selected-item strip were still competing with viewer content and with each other.
Change: Split the left job header into stable title/meta controls, demoted the right strip into source/status metadata, and kept page-level overrides only for briefing and poll-stats views.
Refs: crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs, crates/harvester_core/src/state.rs, crates/harvester_core/src/view_model.rs

## 2026-04-03 - Left pane header no longer repeats the tab name
Type: Bug Fix
Context: The new split header still duplicated the active left tab name below the tab bar, which made the hierarchy flatter rather than clearer.
Change: Collapsed the left title row and left only the contextual meta line under the tab bar so the tab itself carries the collection name.
Refs: crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-04-03 - Triage rows now lead with the article title
Type: Implementation
Context: Screenshot review showed the Triage Results list still scanning as metadata-first because `P# category` led each row while the title and host were pushed later in the line.
Change: Reworked the row formatter so Triage Results now leads with the article title or slug, followed by compact triage metadata (`P# category`, host, tags). Kept the change in `render.rs` and updated render tests to lock the new title-first ordering and no-triage placeholder copy.
Refs: crates/harvester_app/src/platform/ui/render.rs

## 2026-04-03 - Secondary footer actions and readers now recede more cleanly
Type: Implementation
Context: After the row-hierarchy pass, the remaining screenshot gap was footer competition and a reading pane that still felt too wide and utility-like.
Change: Added a semantic `SecondaryButton` style in CommanDuctUI so Harvester could demote `Triage Articles`, `Poll Sources`, and `Open in Browser` without muting every generic button. Follow-up screenshot review also changed disabled button painting to mute the fill itself, made `Stop / Finish` neutral by default and destructive only while a session is actually running, and increased the footer row height so the primary action no longer reads like a short chip. Tightened footer grouping, softened footer text and splitter chrome, widened preview-side margins, and increased RichEdit indents and spacing so long-form panes read more like editorial documents.
Refs: src/CommanDuctUI/src/styling_primitives.rs, crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/markdown_to_rtf.rs

## 2026-04-04 - Trends header no longer leaks selected-article source text
Type: Bug Fix
Context: The right-pane header split left `Trends` on the selected-article fallback path, so a host like `epochai.substack.com` could appear above the trends tabs even though the chart is collection-level data.
Change: Routed `AppTab::Trends` through a page-level preview header override, hid the selected-article metadata row for that tab, and added state regression coverage locking the trends header behavior.
Refs: crates/harvester_core/src/state.rs

## 2026-04-05 - Left pane now routes through the owner-drawn listbox slice
Type: Implementation
Context: The first migration slice from `Plan.OwnerDrawnListboxDesign.md` replaced the old left-pane tree path for the job-oriented tabs with the new structured owner-drawn listbox flow, while Prompt Lab remained on its separate panel path.
Change: Wired the Harvester left pane to populate and select through the new listbox commands, including per-row badge/title/metadata descriptors and badge-column sizing for Jobs, Triage Review, and Triage Results.
Refs: docs/plans/Plan.OwnerDrawnListboxDesign.md, crates/harvester_app/src/platform/ui/render.rs, crates/harvester_app/src/platform/ui/layout.rs

## 2026-04-05 - Indirect-link polling stage button and UI visibility
Type: Implementation
Context: Stage gating requires a visible footer action for the new indirect-link collection feature, and the button must reuse the secondary footer styling when the indirect pool is empty/enabled so the warm theme stays consistent.
Change: Added the layout rule, style application, and footer enablement hook for `Poll Indirect Links`, wired the button into the event handler/render pipeline, and extended the view model status line so the readiness text (collecting/ready/empty) appears alongside session metadata.
Refs: docs/visual_design/VisualDesignSpec.md, crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs, crates/harvester_app/src/platform/app.rs, crates/harvester_core/src/update.rs, crates/harvester_core/src/state.rs, crates/harvester_core/src/view_model.rs

## 2026-04-05 - Indirect-link polling now drops navigation/share noise before enqueue
Type: Bug Fix
Context: Field testing showed `Poll Indirect Links` ballooning into large low-signal batches dominated by site chrome, social share URLs, legal pages, and redirect wrappers.
Change: Filtered indirect links upstream in reducer state collection, switched pool-local dedupe to normalized URLs, and added regression tests covering noise rejection and normalized duplicate suppression.
Refs: crates/harvester_core/src/state.rs, engine.log

## 2026-04-05 - Indirect-link polling UI hidden pending redesign
Type: Bug Fix
Context: Fresh-start polling still surfaced large low-signal indirect link pools, so the footer action and status copy encouraged expensive noisy follow-up runs.
Change: Removed the `Poll Indirect Links` footer button from layout/rendering and stopped showing indirect-link counts in the footer, while leaving the underlying code path commented and intact for a future redesign.
Refs: crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-04-06 - Owner-drawn listbox now claims focus and dialog-navigation keys
Type: Bug Fix
Context: The new left-pane owner-drawn listbox selected rows on mouse click but did not reliably react to arrow/page navigation because the custom child window was not taking focus on click and did not advertise that it wanted dialog-managed navigation keys.
Change: On `WM_LBUTTONDOWN`, the listbox now calls `SetFocus(hwnd)` before hit-testing, and `WM_GETDLGCODE` returns `DLGC_WANTARROWS | DLGC_WANTCHARS` so Up/Down/Page/Home/End and future character shortcuts route to the control.
Refs: src/CommanDuctUI/src/controls/listbox_handler.rs, docs/plans/Plan.OwnerDrawnListboxDesign.md

## 2026-04-06 - Treeview marker geometry test now matches centered 14px marker
Type: Bug Fix
Context: `cargo nextest run` started failing after the treeview marker sizing logic clamped markers to the current 14px maximum, but one unit test still asserted the older 12px geometry.
Change: Updated the `tree_item_marker_rect_anchors_before_text_lane` expectation to the centered 14px rect produced by the live layout math.
Refs: src/CommanDuctUI/src/controls/treeview_handler.rs

## 2026-04-06 - Render tests now assert list-box output and explicit left-pane header input
Type: Bug Fix
Context: Several `harvester_app` render tests were still expecting `PopulateTreeView` commands and implicit header-copy derivation after the left-pane renderer switched to `PopulateListBox` while keeping tree helpers only for marker/link hierarchy logic.
Change: Rebased the stale assertions onto `PopulateListBox` and `build_job_tree` where appropriate, and seeded `left_pane_header` explicitly in tests that validate header meta copy.
Refs: crates/harvester_app/src/platform/ui/render.rs

## 2026-04-06 - Listbox footer and keyboard exclude path moved fully onto the new control
Type: Bug Fix
Context: The owner-drawn left-pane listbox still lacked the indirect-link footer action, the footer status copy, and the `X` shortcut plumbing needed to replace the old tree checkbox flow.
Change: Restored the `Poll Indirect Links` footer button and footer status summary, added structured badges for triage rows, and routed listbox key presses through a generic `ListBoxItemKeyDown` event so Harvester can toggle pre-triage include/exclude from the selected row.
Refs: crates/harvester_app/src/platform/app.rs, crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs, src/CommanDuctUI/src/controls/listbox_handler.rs

## 2026-04-07 - Triage can start from interactive review
Type: Bug Fix
Context: Pressing `X` in Triage Review could move pre-triage from `ReadyToTriage` to `Reviewing`, which disabled the `Triage Articles` button even though unresolved review rows are tentatively included.
Change: Allowed triage start and pre-triage handoff from both `Reviewing` and `ReadyToTriage` when at least one article is currently included. Added a regression test for the `X`-then-`Triage Articles` path.
Refs: crates/harvester_core/src/state.rs, crates/harvester_core/src/update.rs

## 2026-04-07 - Pre-triage phase now derives from lifecycle plus entries
Type: Hardening
Context: `Reviewing` and `ReadyToTriage` were stored as mutable pre-triage phases, allowing load and manual-decision paths to drift into different readiness interpretations.
Change: Replaced the stored phase with a smaller internal lifecycle (`Idle`, `LoadingArticles`, `Loaded`, `Failed`) and made `phase()` derive `Reviewing` vs `ReadyToTriage` from the current entries. Updated tests to assert that review-verdict articles derive `Reviewing` immediately.
Refs: crates/harvester_core/src/pre_triage_filter.rs, crates/harvester_core/src/update.rs, crates/harvester_core/src/working_corpus.rs, crates/harvester_core/tests/pre_triage_filter.rs

## 2026-04-08 - Jobs list selection now uses warm listbox palette tokens
Type: Bug Fix
Context: The left-pane Jobs UI had moved from the old TreeView to the owner-drawn ListBox, but the control still painted from a legacy cool blue-gray fallback palette instead of the visual-spec warm accent system.
Change: Added `ListBoxSelectionAccent`, wired the Jobs list to the listbox-specific style IDs, and taught `CommanDuctUI` list boxes to consume the host palette so the selected row fill and left accent bar match the shared warm accent token.
Refs: crates/harvester_app/src/platform/ui/layout.rs, src/CommanDuctUI/src/controls/listbox_handler.rs, src/CommanDuctUI/src/styling_primitives.rs

## 2026-04-08 - Listbox hover repaint no longer invalidates the full client
Type: Bug Fix
Context: Moving the mouse across article rows repainted the entire owner-drawn listbox on each hover transition, producing a visible flicker/flash along the left edge.
Change: Narrowed hover invalidation to only the previous and current row rectangles in `CommanDuctUI`'s listbox handler and added a row-geometry regression test.
Refs: src/CommanDuctUI/src/controls/listbox_handler.rs

## 2026-04-08 - Startup now emits a deterministic first layout
Type: Bug Fix
Context: `harvester_app` could show a malformed first frame because the initial render reused default render-cache values, started from the default window width, and enqueued startup metadata twice.
Change: Forced the first render to emit `DefineLayout`, seeded app state with the restored startup width before the first view snapshot, and removed the duplicate `LoadLlmMetadata` bootstrap enqueue. Added a render regression test for the default first frame.
Refs: crates/harvester_app/src/platform/app.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-04-08 - Startup preparation and reveal ordering now have app-layer helpers
Type: Refactor
Context: `run_app()` still mixed synchronous startup state seeding, startup hydration scheduling, first-view assembly, and reveal sequencing in one brittle block with repeated `take/update/restore` patterns.
Change: Extracted `prepare_startup_state()` and `assemble_startup_commands()` in `harvester_app` so synchronous startup reads as one explicit sequence before the first `view()` snapshot, and added app-layer tests that lock the render-before-reveal contract plus the single `LoadLlmMetadata` startup scheduling invariant.
Refs: crates/harvester_app/src/platform/app.rs, docs/plans/Plan.StartupRefactor.md

## 2026-04-08 - Prompt Lab update handlers moved into a sub-module
Type: Refactor
Context: Task 2 of `Plan.RefactorUpdateIntoModules` split the largest self-contained branch set out of `harvester_core::update`, but the Prompt Lab reducer arms and run-dispatch helpers were still embedded in `update/mod.rs`.
Change: Moved the Prompt Lab match-arm bodies plus run/template helpers into `update/prompt_lab.rs`, left `update()` as a forwarding table for `Msg::PromptLab*`, and updated reducer tests to call the moved helper through the new sub-module.
Refs: crates/harvester_core/src/update/mod.rs, crates/harvester_core/src/update/prompt_lab.rs, docs/plans/Plan.RefactorUpdateIntoModules.md

## 2026-04-08 - Briefing orchestration moved behind an update sub-module
Type: Refactor
Context: Task 3 of `Plan.RefactorUpdateIntoModules` moved briefing-specific reducer branches out of `harvester_core::update`, but the summary orchestration and checkpoint handlers were still mixed into `update/mod.rs`.
Change: Extracted briefing click/load/checkpoint handlers plus the briefing-step orchestration helpers into `update/briefing.rs`, left shared summary-cache helpers in `update/mod.rs`, and rewired existing triage/LLM paths to call the new sub-module boundary.
Refs: crates/harvester_core/src/update/mod.rs, crates/harvester_core/src/update/briefing.rs, docs/plans/Plan.RefactorUpdateIntoModules.md

## 2026-04-08 - Triage refresh and triage-run orchestration moved into an update sub-module
Type: Refactor
Context: Task 4 of `Plan.RefactorUpdateIntoModules` moved the triage-specific reducer work out of `harvester_core::update`, but the pre-triage refresh coordinator, triage dispatch loop, and cache-run logging were still embedded in `update/mod.rs`.
Change: Extracted the triage/pre-triage message handlers plus `dispatch_pre_triage_if_due()` and `dispatch_next_triage_step()` into `update/triage.rs`, kept `update/mod.rs` as the routing table, and made the briefing module call the triage dispatcher through the explicit sub-module boundary instead of `super::*`.
Refs: crates/harvester_core/src/update/mod.rs, crates/harvester_core/src/update/triage.rs, crates/harvester_core/src/update/briefing.rs, docs/plans/Plan.RefactorUpdateIntoModules.md

## 2026-04-08 - Archive, polling, and import reducer branches moved behind update sub-modules
Type: Refactor
Context: Tasks 5, 6, and 8 of `Plan.RefactorUpdateIntoModules` still left archive-dialog flow, source polling, and saved-webpage import handling embedded in `harvester_core::update/mod.rs`, making the dispatcher harder to scan.
Change: Extracted the archive handlers into `update/archive.rs`, source-poll handlers into `update/polling.rs`, and saved-webpage import handlers into `update/import.rs`; kept `update/mod.rs` as a forwarding table and moved `is_safe_archive_basename()` with the archive flow so the helper stays local to the domain.
Refs: crates/harvester_core/src/update/mod.rs, crates/harvester_core/src/update/archive.rs, crates/harvester_core/src/update/polling.rs, crates/harvester_core/src/update/import.rs, docs/plans/Plan.RefactorUpdateIntoModules.md

## 2026-04-08 - LLM completion routing moved behind an update sub-module
Type: Refactor
Context: Task 7 of `Plan.RefactorUpdateIntoModules` still left the cross-domain `Msg::LlmCompleted` branch inline in `harvester_core::update/mod.rs`, even after briefing, triage, and Prompt Lab had their own extracted orchestration modules.
Change: Extracted the full `Msg::LlmCompleted` flow into `update/llm_completed.rs`, split it into summary, triage, aggregate-briefing, and prompt-lab completion helpers, and kept `update/mod.rs` as a forwarding table that only delegates completion routing.
Refs: crates/harvester_core/src/update/mod.rs, crates/harvester_core/src/update/llm_completed.rs, docs/plans/Plan.RefactorUpdateIntoModules.md

## 2026-04-08 - Footer workflow order now matches Poll -> Triage -> Briefing
Type: Bug Fix
Context: The footer action row presented the main workflow out of sequence, and `Poll Indirect Links` had been re-enabled despite the earlier decision to keep that path disabled pending redesign.
Change: Reordered the footer buttons to `Poll Sources`, `Run Triage`, then `Generate Briefing`, and forced `Poll Indirect Links` to stay disabled in the renderer while leaving the underlying indirect-link pipeline intact. Added layout/render regression coverage for the workflow order and disabled state.
Refs: crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-04-08 - Indirect-link footer affordance hidden again pending redesign
Type: Bug Fix
Context: Even while disabled, the visible `Poll Indirect Links` footer control and readiness copy still advertised a dormant workflow stage and suggested a supported next step.
Change: Removed the `Poll Indirect Links` footer button from layout/rendering again, dropped the indirect-link footer status copy, and removed the app-layer button handler while keeping the indirect-link pipeline code intact behind the UI boundary.
Refs: crates/harvester_app/src/platform/app.rs, crates/harvester_app/src/platform/ui/layout.rs, crates/harvester_app/src/platform/ui/render.rs

## 2026-04-08 - Update reducer tests moved out of mod.rs
Type: Refactor
Context: `harvester_core::update::mod.rs` still embedded two large unit-test modules, which made the wrapper file much longer and harder to scan even after the reducer branches had been extracted into sub-modules.
Change: Replaced the in-file test blocks with `#[cfg(test)] mod tests;`, moved the main reducer unit tests into `update/tests/mod.rs`, and moved the import/polling-specific reducer tests into `update/tests/import_tests.rs` while keeping them under `src/update/` so they still exercise private reducer helpers.
Refs: crates/harvester_core/src/update/mod.rs, crates/harvester_core/src/update/tests/mod.rs, crates/harvester_core/src/update/tests/import_tests.rs, docs/plans/Plan.RefactorUpdateIntoModules.md

## 2026-04-08 - Update helper clusters moved out of mod.rs
Type: Refactor
Context: After the reducer branch extraction and test move, `harvester_core::update::mod.rs` still owned pasted-URL parsing plus the shared summary-cache helper/logging functions, so the wrapper file still mixed dispatch with domain helper logic.
Change: Moved URL parsing into `update/url_input.rs`, moved the shared summary-cache warmup/key/logging helpers into `update/summary_cache_support.rs`, and updated briefing, triage, LLM-completion, and reducer-test code to import those helpers explicitly instead of relying on `mod.rs` imports leaking through `super::*`.
Refs: crates/harvester_core/src/update/mod.rs, crates/harvester_core/src/update/url_input.rs, crates/harvester_core/src/update/summary_cache_support.rs, crates/harvester_core/src/update/briefing.rs, crates/harvester_core/src/update/llm_completed.rs, crates/harvester_core/src/update/triage.rs, crates/harvester_core/src/update/tests/mod.rs, docs/plans/Plan.RefactorUpdateIntoModules.md

## 2026-04-08 - Update submodules now compile with explicit imports
Type: Bug Fix
Context: After removing `super::*` from the `harvester_core::update` submodules, the reducer stopped compiling because those files had been relying on names imported only in `update/mod.rs`.
Change: Added explicit imports for reducer macros, state/effect types, tabs, and prompt-lab/LLM identifiers in the affected `update/*` files, and replaced `ToOwned`-style snapshot cloning with direct `str::to_owned` calls where needed.
Lessons Learned: `super::*` hid real module dependencies and let submodules compile only because `mod.rs` happened to import the right names.
Prevention: Keep `update` submodules on explicit imports so future extractions fail locally and visibly instead of depending on parent-module import leakage.
Refs: crates/harvester_core/src/update/archive.rs, crates/harvester_core/src/update/briefing.rs, crates/harvester_core/src/update/import.rs, crates/harvester_core/src/update/llm_completed.rs, crates/harvester_core/src/update/polling.rs, crates/harvester_core/src/update/prompt_lab.rs, crates/harvester_core/src/update/triage.rs

## 2026-04-09 - state mod.rs follow-up extraction
Type: Refactor
Context: The first `harvester_core::state` split left `state/mod.rs` above the rough target size, with cache metadata and briefing orchestration still mixed into the wrapper.
Change: Moved summary/triage cache metadata and metrics into `state/cache_state.rs`, moved briefing orchestration into `state/briefing_orchestration.rs`, and kept `state/mod.rs` focused on `AppState`, core job/session state, and the remaining transitions. The follow-up brought `state/mod.rs` down to 1792 lines.
Refs: crates/harvester_core/src/state/mod.rs, crates/harvester_core/src/state/cache_state.rs, crates/harvester_core/src/state/briefing_orchestration.rs

## 2026-04-10 - Archive checkpoint export avoids stale list selection
Type: Bug Fix
Context: Exporting an archive with "set checkpoint" enabled could move the selected job outside the active Since Checkpoint list scope, then the renderer still asked CommanDuctUI to select the filtered-out list item and logged an invalid handle error.
Change: Derived list-box items and selected item together in a render model so selection commands are emitted only for items present in the populated list, and added a render regression test for a filtered selected job.
Refs: crates/harvester_app/src/platform/ui/render.rs, crates/harvester_app/src/platform/ui/render_tests.rs, engine.log

## 2026-04-10 - render.rs Steps 1+2 — extract tests, delete legacy tree-view code
Type: Refactor
Context: `render.rs` had reached 4,783 lines housing five responsibility clusters: render orchestration, legacy tree-view code (dead since the owner-drawn listbox migration), viewer-shaping helpers, tests, and active section renderers. `Plan.RenderRsRefactor.md` called for nine extraction steps; Steps 1 and 2 tackled the two lowest-risk chunks first.
Change: Step 1 — moved the entire inline `#[cfg(test)]` block (~2,230 lines) into a sibling file `render_tests.rs` and wired it back via `#[cfg(test)] #[path = "render_tests.rs"] mod tests;` at the end of `render.rs`. Step 2 — deleted all legacy tree-view symbols: `TreeStructureItem`, `TreeSnapshot`, `TreeRenderState` fields (`initialized`, `structure`, `text_by_id`, `check_state_by_id`), `append_tree_commands`, `build_job_tree`, `build_link_children`, `JobRowPresentation`, `job_row_check_policy`, `job_row_style_policy`, `format_job_row_legacy`, `format_job_row_triage_review`, `format_job_row_triage_results`, `stage_label`, `format_preview_context`, `normalize_windows_newlines`, `shape_for_viewer` plus sub-helpers, `truncate_for_viewer`, `VIEWER_TRUNCATE_MARKER`, and a cluster of now-dead display-label helpers. Removed 398 matching test lines from `render_tests.rs`. Fixed a clippy orphan doc comment and scope-adjusted `PreviewContextView`/`PreviewHeaderView` imports in the test file. Result: `render.rs` 4,783 → 1,977 lines; `render_tests.rs` 2,228 → 1,831 lines; 1,249 tests pass; `cargo clippy --workspace --all-targets -- -D warnings` clean.
Lessons Learned: PowerShell `[System.IO.File]::WriteAllLines` silently truncated files to 0 bytes in this terminal session on every attempt. Python (`python script.py`) was the only reliable large-file mutation path in this workspace.
Prevention: For large Rust file deletions (> ~100 lines), prefer a Python helper script over PowerShell file-write patterns; keep the script in `scripts/` temporarily and delete after validation.
Refs: crates/harvester_app/src/platform/ui/render.rs, crates/harvester_app/src/platform/ui/render_tests.rs, docs/plans/Plan.RenderRsRefactor.md, commit 0bbdcd8 (Step 1)

## 2026-04-10 - render.rs Steps 3–5 — extract render_text, render_list_box, render_controls
Type: Refactor
Context: Following Steps 1+2, `render.rs` was still 2,000 lines. The next three lowest-risk extractions targeted pure text helpers (no platform side effects), list-box building (row labels, badges, sort policy), and generic control section renderers (status, progress, button enablement).
Change: Created three sibling modules under `platform/ui/`: `render_text.rs` (144 lines) — pure format helpers: `format_compact_tokens`, `compact_url_label`, `title_case_label`, `domain_from_url`, `truncate_with_ellipsis`, `format_compact_bytes`, `strip_leading_h1`, `truncate_markdown_for_preview`, `MAX_VIEWER_CHARS`. `render_list_box.rs` (294 lines) — `ListBoxRenderModel`, `append_list_box_commands`, `build_list_box_items`, `build_list_box_item`, badge/style helpers, row-label helpers, `compute_list_box_badge_column_width`. `render_controls.rs` (353 lines) — `render_status_section`, `render_operation_progress_section`, `render_token_progress_section`, `render_main_controls_section`, `render_tab_bar_section`, `render_left_tab_bar_section`, `format_left_pane_header_meta`, `format_llm_usage_status`. Made `emit_if_changed` and `TreeRenderState` fields `pub(super)` so sibling modules can access them. Added `render_tests.rs` explicit imports for moved types. Declared the three new modules as `mod` (not `pub mod`) in `platform/ui/mod.rs`. Result: `render.rs` 2,000 → 1,245 lines; 1,250 tests pass; clippy clean; `cargo fmt` clean.
Lessons Learned: Extracting a function that was inside an `impl` block (here `ListBoxRenderModel::from_view`) with a generic AST brace-counter produced a duplicate impl when the script also separately extracted the bare `from_view` marker; the fixup required deleting the trailing duplicate. Python scrip logic must extract `impl` blocks as whole units rather than individual methods inside them.
Prevention: When writing extraction scripts, prefer extracting the enclosing `impl` block rather than individual methods, and add an overlap/duplicate check before writing output files.
Refs: crates/harvester_app/src/platform/ui/render_text.rs, crates/harvester_app/src/platform/ui/render_list_box.rs, crates/harvester_app/src/platform/ui/render_controls.rs, crates/harvester_app/src/platform/ui/render.rs, crates/harvester_app/src/platform/ui/render_tests.rs, docs/plans/Plan.RenderRsRefactor.md
