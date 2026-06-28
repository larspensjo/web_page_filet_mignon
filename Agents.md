# Repo Instructions

## Workflow
- Build with `cargo build`.
- When a task is completed with Rust changes, run `cargo clippy --all-targets -- -D warnings` and then `cargo fmt`.
- When adding a CLI flag to `harvester_batch`, update `scripts/Start-HarvesterBatch.ps1` in the same change.
- When creating complex plans, they should be divided into incremental phases that can be tested.
- If harvester_mcp processes block building and testing, kill these processes.
- When implementing a plan, don't commit the changes; they shall first be reviewed.

## Planning & Documentation
- When creating or saving plan documents, always save them to the `docs/plans/` folder unless explicitly told otherwise.
- Prefer plans with proper long term solutions, even if more work or refactoring are required.

## Architecture
- Preserve the unidirectional data flow: input -> action -> reducer -> state -> render, with side effects isolated and fed back as actions.
- Reducers must stay pure and unit-testable.
- Keep entry points (`app.rs`, `main.rs`, `mod.rs` and `lib.rs`) files as thin wrappers only.
- Keep shared constants and behavior DRY; prefer one source of truth over duplicated definitions.

## CommanDuctUI Boundary
- Treat `CommanDuctUI` as generic infrastructure, not Harvester domain code.
- Do not add Harvester-specific terminology or behavior to `CommanDuctUI`.
- If `CommanDuctUI` changes, update its version and changelog, and preserve dark-theme support.

## Testing
- Bug fixes should include a regression test when practical.
- Prefer tests of reducer behavior, emitted effects, and public contracts over internal details.
- `use super::*;` is acceptable inside an inline `#[cfg(test)]` block and extracted test files. Otherwise, prefer specific naming.

## Logging
- Use `engine_logging` for runtime logging.
- Include enough context in error logs to identify the failing job, URL, or operation.

## Skills
- For research questions that should be answered from the local harvested article corpus, use `$harvester-mcp-research`.

## Diary
- Keep `docs/EngineeringDiary.md` up to date for noteworthy implementations, and bug fixes with reusable lessons.
- See the "How to use" section in the beginning.
