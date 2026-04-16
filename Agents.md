# Repo Instructions

## Workflow
- Build with `cargo build`.
- When a task is complete, run `cargo clippy --all-targets -- -D warnings` and then `cargo fmt`.
- For plan-driven work, write commit messages about the code change, not the plan. Follow recommended practices for teh design of the comment.
- When adding a CLI flag to `harvester_batch`, update `scripts/Start-HarvesterBatch.ps1` in the same change.

## Architecture
- Preserve the unidirectional data flow: input -> action -> reducer -> state -> render, with side effects isolated and fed back as actions.
- Reducers must stay pure and unit-testable.
- Keep entry points (`main.rs`, `mod.rs` and `lib.rs`) files as thin wrappers only.
- Keep shared constants and behavior DRY; prefer one source of truth over duplicated definitions.

## CommanDuctUI Boundary
- Treat `CommanDuctUI` as generic infrastructure, not Harvester domain code.
- Do not add Harvester-specific terminology or behavior to `CommanDuctUI`.
- If `CommanDuctUI` changes, update its version and changelog, and preserve dark-theme support.

## Testing
- Bug fixes should include a regression test when practical.
- Prefer tests of reducer behavior, emitted effects, and public contracts over internal details.
- `use super::*;` is acceptable inside an inline `#[cfg(test)]` block, but extracted test files (e.g. `tests.rs`) must use explicit imports.

## Logging
- Use `engine_logging` for runtime logging.
- Include enough context in error logs to identify the failing job, URL, or operation.

## Diary
- Keep `docs/EngineeringDiary.md` up to date for noteworthy implementations, important decisions, and bug fixes with reusable lessons.
- Keep diary entries short and reference concrete artifacts.
- Add new entries to the end.
