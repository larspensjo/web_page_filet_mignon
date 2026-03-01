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

## 2026-03-01 - Claude plan loop isolation and UTF-8 CLI encoding
Type: Decision
Context: Plan-review loop runs should be deterministic and not inherit prior Claude session state, and generated markdown had mojibake (`ΓÇö`) from an encoding mismatch in the CLI capture path.
Change: Defaulted Claude CLI args to `--no-session-persistence` in the loop and set process native-command encoding to UTF-8 (`Console` input/output plus `$OutputEncoding`) before invoking model CLIs.
Evidence: `scripts/Invoke-PlanReviewLoop.ps1` syntax parse check passed (`Parser::ParseFile` returned OK).
Refs: scripts/Invoke-PlanReviewLoop.ps1
