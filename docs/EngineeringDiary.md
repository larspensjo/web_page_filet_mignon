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
Evidence: Tests, logs, or validation performed.
Lessons Learned: (required for Bug Fix)
Prevention: (required for Bug Fix)
Refs: path/to/file.rs, test_name, commit abc1234

---

## 2026-02-17 - Diary initialized
Type: Decision
Context: Need persistent memory across AI-assisted sessions.
Change: Added explicit diary workflow in AGENTS.md and created this file.
Evidence: AGENTS.md updated with Engineering Diary rules and template.
Refs: AGENTS.md, docs/EngineeringDiary.md

## 2026-02-18 - Unified persistence paths
Type: Bug Fix
Context: `harvester_batch` was saving caches/state with `.json` names while `harvester_app` expected `.ron`, so the GUI never picked up batch-generated caches.
Change: Updated `RuntimePaths` to produce `.ron` files, removed the app-local cache persistence modules, and redirected the UI to `harvester_io`’s load/save APIs; added regression tests to ensure the same path is used end-to-end.
Evidence: `cargo test -p harvester_io runtime_paths::tests`
Lessons Learned: Allowing multiple codepaths to own file naming leads to silent divergence of persisted data.
Prevention: Centralize filenames/formats in `harvester_io::RuntimePaths` and cover the shared persistence API with regression tests.
Refs: crates/harvester_io/src/runtime_paths.rs, crates/harvester_app/src/platform/app.rs, cargo test -p harvester_io runtime_paths::tests

## 2026-02-18 - Fix workspace crate coverage in project stats
Type: Bug Fix
Context: `scripts/project-stats.ps1` hard-coded four crates, so newly added workspace crates were omitted from the Rust section and totals.
Change: Replaced hard-coded crate enumeration with workspace-driven discovery from root `Cargo.toml`, fixed crate-level `tests/` lookup to use each crate root, and added a Pester regression test that compares reported crates with `cargo metadata`.
Evidence: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/project-stats.ps1`; `Invoke-Pester -Path scripts/tests/project-stats.Tests.ps1`; `cargo build`; `cargo clippy --all-targets -- -D warnings`
Lessons Learned: Hard-coded project topology in reporting tooling quickly drifts from workspace reality and silently under-reports.
Prevention: Derive crate inventory from workspace metadata and keep a regression test that cross-checks script output against `cargo metadata`.
Refs: scripts/project-stats.ps1, scripts/tests/project-stats.Tests.ps1
