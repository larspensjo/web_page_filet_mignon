# Instructions to consider
Some instructions here doesn't hold for the CommanDuctUI submodule.

## Workflow
* Build with "cargo build".
* At the end of complete plan implementation, test with "cargo clippy --all-targets -- -D warnings". Don't do that for itermediary steps as there will be warnings from unused entities.
* When adding a new CLI flag to `harvester_batch`, update `scripts/Start-HarvesterBatch.ps1` in the same change so the launcher supports the flag.
*
* Maintain an engineering diary in `docs/EngineeringDiary.md` (see "Engineering Diary" section below).

## Bugs
* Is there a lessons learned here? A design issue? Lack of robustness? That is, I want to find similar problems and prevent future problems of the same type.
* When fixing bugs, make sure to add a unit test that locks functionality in and prevents it from happening again.
* Avoid hard-coded string/buffer lengths anywhere (UI, I/O, parsing); size dynamically from the data source (e.g., `GetWindowTextLengthW`/`LB_GETTEXTLEN` for Win32, length-prefixed reads elsewhere) and centralize helpers to prevent truncation.

## Unidirectional Data Flow Architecture
Adhere to the Unidirectional Data Flow Architecture.

* All state changes flow in one direction. UI (or other input sources) must not mutate model/state directly. State changes only occur by dispatching actions/events into the update pipeline.

* Single source of truth per feature. Each feature/module owns a single authoritative state structure. Other parts of the system read state via that owner’s public API and do not keep competing “shadow state”.

* Pipeline shape is fixed:

  1. Inputs (UI, timers, IO callbacks) create Actions (intent).
  2. Actions are processed by a Reducer/Update function that produces the next State.
  3. Views render from State (read-only).
  4. Side effects (network/filesystem, background work) are triggered by actions and feed results back as new actions.

* Reducers are pure. Update/Reducer code must be deterministic and free of side effects (no IO, no random, no sleeping, no global mutation). It may compute new state and emit “effect requests” only.

* Effects are isolated. All IO is performed in effect handlers/services. An effect handler receives an effect request and must respond by dispatching a follow-up action (success/failure/progress).

* No back-channels. Views and services must not call into each other to “push” changes. The only way to change state is dispatching an action.

* State is immutable from the outside. Expose state snapshots or read-only views; never return mutable references that allow external mutation. Prefer “replace with new state” semantics internally.

* Traceability is mandatory. Every user-visible change should be explainable as: *Action → (Reducer) → State’ → Render*, with optional *Action → Effect → Action* loops. Add logging/telemetry at action dispatch boundaries.

* Testing expectation. Reducers must be unit-testable: given (State, Action) assert resulting State and emitted effects. Effect handlers are tested separately with mocked IO.

## Structs
First and foremost, adhere to the Unidirectional Data Flow Architecture.
* Prefer private members to enforce encapsulation.
* Expose behavior, not structure. Types must provide methods that perform domain operations; avoid exposing fields or providing “raw” getters/setters that make callers assemble logic.
* Keep invariants inside the type. Any update that could break validity must be done through a method that enforces rules (validation, normalization, cross-field consistency).
* No leaking internal representation. Do not return internal collections or references that allow external mutation; return derived values, immutable views, or copies where appropriate.
* Stable contract at the boundary. Public APIs describe what happens, not how data is stored. Internal layouts may change without requiring changes in callers.
* Prefer commands over queries for state changes. Callers request actions (e.g., add_url(...), mark_complete(), apply_filter(...)) rather than fetching state, modifying it, and writing it back.
* Pure data containers are still needed, and can be public. Use names that makes it obvious.

## Testing
* Consider using dependency injection and mock objects to enhance unit testing
* It is very important that all feaures have unit tests to lock-in functionality.
* Don't build the release version.

## General Rust design
* mod.rs, lib.rs and main.rs should be thin wrappers.
* Follow the principle of Correctness-by-construction: Prefer designs and language features that prevent bugs by construction—make illegal states unrepresentable and incorrect usage hard or impossible.
* Instead of liberally use of comments, try to use names on things (functions, variables, etc) that makes the intent clear. But still, comments may be needed eventually.
* When building long prompt strings (system/user templates or `expected_format` literals), prefer `concat!` to split the text into readable pieces while preserving the literal content; do not rely on a single massive inline string with escaped newlines.

## Logging
* Use the `engine_logging` crate for all logging. Import macros: `use engine_logging::{engine_info, engine_warn, engine_error};`
* Available macros: `engine_trace!`, `engine_debug!`, `engine_info!`, `engine_warn!`, `engine_error!`
* Default log level is INFO (debug messages are filtered out)
* Logs are written to both terminal and `./engine.log` in the current working directory
* Log errors with context: include the URL, job_id, or other identifying information
* In unit tests, call `engine_logging::initialize_for_tests();` to enable logging output
* Logs should have a category inside '[' and ']' to make it easy to filter.

## Git submodules
It is fine to update these, if changes are required. If a change is done:
* Increase the version number in the submodule Cargo.toml
* Update CHANGELOG documents, if they exist.
* If the changes are breaking, make that clear.

## Engineering Diary
Use this project diary as long-term memory for AI-assisted coding.

* Diary file path: `docs/EngineeringDiary.md`
* Update the diary in the same change when any of the following happens:
* A noteworthy implementation is completed.
* A bug is fixed.
* A non-trivial architectural, API, or workflow decision is made.
* Keep entries short and high-signal; avoid transcript-style logs.
* Every bug-fix entry must include a "Lessons Learned" line and a "Prevention" line.
* Link to concrete artifacts when available (file paths, test names, commit hashes).
* Prefer append-only history; do not rewrite old entries except for factual corrections.
* If a change is too small to be noteworthy, no diary entry is required.

### Capturing diary entries at plan creation

Every new plan document must include a draft diary entry (at the top or in a dedicated section) with at least `Context` and `Change` fields pre-filled. At plan-creation time the motivation and goals are freshest — this is the golden opportunity to document *why* the work is being done. The `Change` field can be written as the intended outcome; it will be confirmed or adjusted when the plan is completed and the diary entry is finalized.

### Async/Burst feature planning checklist

For features that react to streams/batches of events (e.g. `JobDone`, timers, polling, callbacks), the plan must include:

* Burst behavior / backpressure — What happens if many events arrive quickly? (coalesce, throttle, queue, dedupe)
* Async result safety — How stale/out-of-order async results are rejected (request IDs/versioning/cancellation)
* Performance envelope — Expected cost per event and whether any path is `O(N)` / full rebuild
* Observability — Timing logs/spans that prove batching and identify bottlenecks
* Failure semantics — What fails on background errors (local feature vs wider workflow)
* Starvation/livelock guard — Maximum wait before progress is forced
* Burst test case — At least one test scenario that asserts exact dispatch/rebuild count during a burst

### What to record in diary entries

* Context — the motivation, goal, or problem being solved. Focus on *why*, not *how*. This is the most valuable part; without it, history becomes a meaningless list of changes.
* Change — name the subsystems or crates affected (e.g., "harvester_engine, harvester_core"), not individual files. Keep it to one or two sentences. File-level detail belongs in commit messages, not the diary.
* Bug fixes — only record a diary entry when the fix reveals an insight that changes how you would design or review code in the future. Ask: *"Would knowing this prevent a whole category of future bugs?"* If yes, document the lesson. If it was a simple typo, wrong variable, or misreading of docs, skip the diary entry.

### When a plan is completed and deleted

Before deleting a completed plan file, finalize its draft diary entry: confirm the `Change` field reflects what was actually built, add `Evidence`, and copy the entry into `docs/EngineeringDiary.md`. This can be coodinated with the extraction of future ideas.

Recommended entry format:

```md
## YYYY-MM-DD - Short title
Type: Implementation | Bug Fix | Decision
Context: Why this change happened.
Change: What was implemented/changed (name subsystems, not files).
Evidence: Tests, logs, or validation performed.
Lessons Learned: (required for Bug Fix, only when insightful)
Prevention: (required for Bug Fix) How we reduce recurrence.
Refs: crate/module names, test_name, commit abc1234
```
