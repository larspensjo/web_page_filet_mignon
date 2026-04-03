# Repo Instructions

## Workflow
- Build with `cargo build`.
- When a task is complete, run `cargo clippy --all-targets -- -D warnings`.
- When adding a CLI flag to `harvester_batch`, update `scripts/Start-HarvesterBatch.ps1` in the same change.

## Architecture
- Preserve the unidirectional data flow: input -> action -> reducer -> state -> render, with side effects isolated and fed back as actions.
- Reducers must stay pure and unit-testable.

## CommanDuctUI Boundary
- Treat `CommanDuctUI` as generic infrastructure, not Harvester domain code.
- Do not add Harvester-specific terminology or behavior to `CommanDuctUI`.
- If `CommanDuctUI` changes, update its version and changelog, and preserve dark-theme support.

## Testing
- Bug fixes should include a regression test when practical.
- Prefer tests of reducer behavior, emitted effects, and public contracts over internal details.

## Logging
- Use `engine_logging` for runtime logging.
- Include enough context in error logs to identify the failing job, URL, or operation.

## Diary
- Keep `docs/EngineeringDiary.md` up to date for noteworthy implementations, important decisions, and bug fixes with reusable lessons.
- Keep diary entries short and reference concrete artifacts.
- Add new entries to the end.
