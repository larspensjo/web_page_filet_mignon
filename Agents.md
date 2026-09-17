# Repository Instructions

Keep guidance focused on durable project constraints and user preferences. Trust
agents to choose implementation details and follow existing conventions.

## Working with the user

The user works across many projects and treats code as a black box. Own the
technical investigation, implementation, and verification; explain progress in
terms of observable outcomes.

- At task start, during meaningful progress updates, and at completion, give a
  brief status: **Harvester — [task]: [current state]. Next: [next step or none].**
  Keep this to one or two plain-language sentences so the user can quickly regain
  context when switching projects. Avoid narrating tool calls and file edits.
- Final status states what changed, whether it was verified, and any remaining
  blocker or user decision. Distinguish implemented from verified; do not infer
  overall project health from checks of one change.
- Keep implementation details out of routine summaries unless requested or needed
  to explain a material risk or decision. Be explicit about uncertainty.
- Make routine, reversible implementation decisions independently. Ask when missing
  information materially changes the intended outcome; explain the user-facing
  tradeoff and recommend an option without requiring the user to inspect code.
- Leave changes uncommitted for review unless the user authorizes committing.

## Project boundaries

- Preserve input -> action -> reducer -> state -> render. Reducers are pure;
  side effects return through actions. Keep entry points and orchestration thin.
- Runtime logging uses `engine_logging`, with enough context to identify the
  failing job, URL, or operation.
- Launch scripts encode a fixed launch policy; change them when that policy
  changes, not merely when adding a CLI flag.
- For public corpus-layout changes, update `docs/CorpusFormat.md`, bump
  `CORPUS_SCHEMA_VERSION` when compatibility changes, and synchronize
  `harvester-corpus.json` generation and tests.
- UI work follows `docs/visual_design/VisualDesignSpec.md`: the warm dark-theme
  Tauri desktop UI is the supported desktop host.

## Secrets and verification

- Do not obtain API keys, run Harvester launchers, or iterate against live LLM
  APIs. Use the keyless build, test, Pester, and IPC-probe paths.
- Select checks appropriate to the change. Documentation-only edits do not need
  application builds or tests. Report checks that could not be completed.
- For Rust changes, build with `cargo build`, run relevant tests, and finish with
  `cargo clippy --all-targets -- -D warnings` and `cargo fmt`.
- Root Cargo commands stay Node-free: `harvester_ui` is not a default member.
  Changes to that host additionally require
  `cargo clippy -p harvester_ui --all-targets -- -D warnings`.
- Frontend checks run from `frontend/`: `npm run check`, `npm run build`, and
  `npm run fmt`.
- A running batch locks `target/debug/harvester_batch.exe`. While it runs, build
  shared-crate changes with `cargo build --workspace --exclude harvester_batch`.
- The keyless IPC probe is `cargo run -p harvester_ui -- --probe-ipc`. It needs a
  display and the GUI lock, takes about 150 seconds, writes
  `.local/probe/ipc-report.json`, and exits non-zero on a failed gate.
- Cover behavior fixes with practical regression tests. Focus on reducer behavior,
  emitted effects, and public contracts rather than implementation details.

## Corpus research

Read harvested articles directly; there is no corpus server. Search `output/*.md`
and, when present, `output/linked/*.md`. Article filenames contain titles;
frontmatter contains `url`, `title`, and `fetched_utc`.
`output/harvester-corpus.json` describes layout and schema, not an article index.

## Planning and project memory

- Consult relevant architecture and decision-log entries before planning changes.
  Favor durable solutions; divide complex work into independently testable phases.
  Save plans under `docs/plans/` unless instructed otherwise.
- Keep affected documentation accurate. `docs/DecisionLog.md` is append-only
  memory for settled commitments; record reversals as new entries.
- Update `docs/EngineeringDiary.md` for noteworthy implementations and bug fixes
  with reusable lessons, following its "How to use" section.
