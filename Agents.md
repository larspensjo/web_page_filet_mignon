# Instructions to consider

## Bugs
* Is there a lessons learned here? A design issue? Lack of robustness? That is, I want to find similar problems and prevent future problems of the same type.
* When fixing bugs, make sure to add a unit test that locks functionality in and prevents it from happening again.
* Avoid hard-coded string/buffer lengths anywhere (UI, I/O, parsing); size dynamically from the data source (e.g., `GetWindowTextLengthW`/`LB_GETTEXTLEN` for Win32, length-prefixed reads elsewhere) and centralize helpers to prevent truncation.

## Unidirectional Data Flow Architecture
Adhere to the Unidirectional Data Flow Architecture.

* **All state changes flow in one direction.** UI (or other input sources) must not mutate model/state directly. State changes only occur by dispatching actions/events into the update pipeline.

* **Single source of truth per feature.** Each feature/module owns a single authoritative state structure. Other parts of the system read state via that owner’s public API and do not keep competing “shadow state”.

* **Pipeline shape is fixed:**

  1. **Inputs** (UI, timers, IO callbacks) create **Actions** (intent).
  2. Actions are processed by a **Reducer/Update** function that produces the next **State**.
  3. **Views** render from State (read-only).
  4. **Side effects** (network/filesystem, background work) are triggered by actions and feed results back as new actions.

* **Reducers are pure.** Update/Reducer code must be deterministic and free of side effects (no IO, no random, no sleeping, no global mutation). It may compute new state and emit “effect requests” only.

* **Effects are isolated.** All IO is performed in effect handlers/services. An effect handler receives an effect request and must respond by dispatching a follow-up action (success/failure/progress).

* **No back-channels.** Views and services must not call into each other to “push” changes. The only way to change state is dispatching an action.

* **State is immutable from the outside.** Expose state snapshots or read-only views; never return mutable references that allow external mutation. Prefer “replace with new state” semantics internally.

* **Traceability is mandatory.** Every user-visible change should be explainable as: *Action → (Reducer) → State’ → Render*, with optional *Action → Effect → Action* loops. Add logging/telemetry at action dispatch boundaries.

* **Testing expectation.** Reducers must be unit-testable: given (State, Action) assert resulting State and emitted effects. Effect handlers are tested separately with mocked IO.

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

## General Rust design
* mod.rs, lib.rs and main.rs should be thin wrappers.

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
