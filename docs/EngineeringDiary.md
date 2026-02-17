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
