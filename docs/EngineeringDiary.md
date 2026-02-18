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
