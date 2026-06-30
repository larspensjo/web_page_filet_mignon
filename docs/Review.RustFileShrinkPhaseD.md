# Review: RustFileShrink Phase D staged files

Date: 2026-06-30

## Findings

### High: `Invoke-RustFileShrink.ps1` cannot run because required prompt/schema files are missing

Location: `scripts/Invoke-RustFileShrink.ps1:215`

`Resolve-ShrinkContext` requires these files:

- `scripts/prompts/shrink-recommendation.schema.json`
- `scripts/prompts/shrink-recommend.md`
- `scripts/prompts/shrink-extract.md`

They are not present in the staged files or the working tree. A real preflight currently fails before reaching the clean-worktree or no-AI path:

```powershell
pwsh scripts/Invoke-RustFileShrink.ps1 -FilePath crates/openai_provider_kit/tests/openai.rs -PreflightOnly
```

Observed failure:

```text
Required prompt/schema not found: ...\scripts\prompts\shrink-recommendation.schema.json
```

Action: add and stage the three shrink prompt/schema files from phase D, then rerun the preflight command. Add a focused test around `Resolve-ShrinkContext` or preflight so this cannot regress.

### High: staged launcher changes reintroduce removed CLI flags

Locations:

- `scripts/harvester_launcher/Data.psm1:30`
- `scripts/harvester_launcher/Reducer.psm1:136`

The staged launcher diff adds `--trusted-manual-selection` and `--import-action` back into the PowerShell launcher. The Rust CLI no longer appears to support either flag (`git grep` over `HEAD -- *.rs` finds no matches), and `docs/EngineeringDiary.md` has a 2026-03-11 entry documenting their deliberate removal.

Impact: import mode can generate a command line that `harvester_batch` rejects with unknown arguments.

Action: remove these launcher hunks from the staged change unless the Rust CLI support is intentionally being restored in the same change. If restoring them, update the Rust CLI, launcher tests, and diary together so the contract stays consistent.

## Verification Run

- `pwsh -NoProfile` parser check for `scripts/Invoke-RustFileShrink.ps1`: passed.
- `Invoke-Pester -Path scripts/tests/InvokeRustFileShrink.Tests.ps1 -CI`: 19 passed.
- `Invoke-Pester -Path scripts/tests/HarvesterLauncher.Tests.ps1 -CI`: 166 passed.
- `git diff --cached --check`: passed.
- `pwsh scripts/Invoke-RustFileShrink.ps1 -FilePath crates/openai_provider_kit/tests/openai.rs -PreflightOnly`: failed as described above.

