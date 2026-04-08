# Harvester Startup Refactor Plan

Reviewed and updated after feedback in `docs/plans/Review.StartupRefactor.md`.

## Goal

Refactor startup so the first visible frame remains correct and the startup path in `harvester_app` is easier to reason about, test, and extend without reintroducing ordering bugs.

This plan assumes several first-frame fixes are already in place and focuses only on the remaining architectural cleanup.

## Already Done

These items are not future work anymore:

1. Reveal commands are already centralized in `app.rs`, not `layout.rs`.
2. `initial_commands()` already excludes `ShowWindow` and `SignalMainWindowUISetupComplete`.
3. A render-level regression test already asserts that the initial render emits `DefineLayout`.
4. The most obvious first-frame geometry/reveal bug has already been fixed.

This plan should not re-solve those problems. It should build on them.

## Real Remaining Problem

The main remaining issue is not “startup phases are missing entirely”. It is that `run_app()` still mixes too many startup concerns in one long function, and the synchronous startup state preparation section is repetitive and brittle.

Today, startup still bundles together:

- window creation
- initial state seeding
- repeated lock / take / update / restore blocks
- startup effect scheduling
- initial view snapshot creation
- initial command assembly
- reveal ordering
- event-loop handoff

The worst part is the state preparation block: multiple sequential `lock -> take state -> update -> enqueue effects -> store state` sections repeat the same pattern. That makes the startup flow harder to read and easier to accidentally break by reordering steps.

## Desired Invariants

These invariants should hold after the refactor:

1. The first visible frame is rendered from a state snapshot that already includes all synchronous local startup facts that are cheap and immediately available.
2. Reveal ordering remains centralized in one place in `app.rs`.
3. The ordering between first render and reveal is covered by an app-layer assembly test, not just by line ordering.
4. Synchronous startup preparation remains reducer-driven where applicable.
5. `CommanDuctUI` stays generic infrastructure; no Harvester-specific startup policy should leak into it.
6. The startup preparation sequence is explicit enough that reordering a step is visibly suspicious.

## Design Direction

Keep the design simple.

Do not introduce type-level staged builders or wrapper types for a single call site.

Instead:

- extract the synchronous startup preparation into a named helper
- keep reveal ownership in `app.rs`
- add one app-layer startup assembly test that locks in the critical ordering
- document the synchronous vs asynchronous startup boundary with comments where the code actually executes

This is enough structure for the current codebase.

## Correctness By Construction Direction

The right “correctness by construction” move here is modest:

- give synchronous startup preparation one named entry point
- give initial command assembly one named entry point
- test the assembled startup sequence directly

That does not make bad orderings impossible in the abstract, but it makes the valid path obvious and the invalid path harder to create accidentally.

In this codebase, that is a better tradeoff than introducing extra startup command wrapper types.

## Proposed Implementation

### Slice 1: Extract synchronous startup preparation

Files:

- `crates/harvester_app/src/platform/app.rs`
- optionally `crates/harvester_app/src/platform/startup.rs`

Tasks:

- extract the repeated synchronous startup state-loading logic into a named helper such as `prepare_startup_state(...)`
- keep the existing reducer/update path for state changes
- preserve the current execution order explicitly inside that helper
- keep startup effect scheduling adjacent to the state transitions that produce those effects
- add brief comments that distinguish:
  - synchronous startup preparation
  - asynchronous startup hydration

Specific startup inputs to keep in this preparation flow:

- restored window width
- startup AI availability
- startup hydration request message
- persisted completed jobs
- summary cache
- triage cache
- pre-triage overrides

Success criteria:

- the first `state.view()` snapshot happens only after the synchronous startup preparation helper completes
- startup preparation reads top-to-bottom as one coherent sequence instead of many repeated blocks

### Slice 2: Add app-layer startup assembly tests

Files:

- `crates/harvester_app/src/platform/app.rs`
- existing test modules as appropriate

Tasks:

- add a test that assembles the startup command list at the app layer
- assert `ShowWindow` appears exactly once
- assert `ShowWindow` appears after the initial render commands
- assert `SignalMainWindowUISetupComplete` appears after the initial render commands
- assert startup effect scheduling does not duplicate metadata loads

This is the missing test coverage today. Layout-only and render-only tests exist, but the assembled startup sequence is not tested as one contract.

Success criteria:

- a future misplaced `push` in `app.rs` fails a focused test

### Slice 3: Small documentation and comment cleanup

Files:

- `crates/harvester_app/src/platform/app.rs`
- optionally `docs/Architecture.md`

Tasks:

- add short comments around the synchronous preparation boundary
- add short comments around the reveal boundary
- document the async startup work only if the code comments are not sufficient

This should stay lightweight. No separate documentation project is needed.

Success criteria:

- the next person reading startup can identify:
  - what must happen before first render
  - what may happen after reveal

## File Map

| File | Action | Reason |
|---|---|---|
| `crates/harvester_app/src/platform/app.rs` | Refactor | Extract startup preparation and test assembled startup ordering |
| `crates/harvester_app/src/platform/ui/layout.rs` | Keep mostly as-is | Shell command builder already excludes reveal commands |
| `crates/harvester_app/src/platform/ui/render.rs` | Keep mostly as-is | First-render `DefineLayout` invariant is already covered |
| `docs/Architecture.md` | Optional small update | Only if a short architecture note adds value |
| `docs/EngineeringDiary.md` | Update when implementation lands | Record the completed refactor and lesson |

## Risks And Mitigations

### Risk: Reordering startup state preparation silently changes behavior

This is the most important additional risk identified in review.

The current repeated `std::mem::take` + replace pattern means a careless reorder could change which state each step sees, or could make a later block overwrite assumptions established by an earlier block.

Mitigation:

- move the sequence into one helper
- keep the sequence linear and explicit
- avoid scattering preparation across multiple unrelated blocks
- add tests for the externally visible startup contract

### Risk: Over-engineering the startup path

Mitigation:

- do not add type-level staged builders
- do not create abstraction layers for one call site unless the code proves they are needed

### Risk: Weakening reducer ownership

Mitigation:

- continue routing state changes through reducer messages where applicable
- keep direct mutation limited to infrastructure setup that is not reducer-owned domain behavior

### Risk: Pushing Harvester policy into `CommanDuctUI`

Mitigation:

- keep this refactor inside `harvester_app` unless a separate generic repaint or invalidation bug is proven

## Test Plan

Keep existing startup-related tests and add the missing app-layer contract coverage:

- `initial_commands()` excludes reveal commands
- initial render emits `DefineLayout`
- full startup assembly emits `ShowWindow` exactly once
- full startup assembly places `ShowWindow` after initial render commands
- full startup assembly places `SignalMainWindowUISetupComplete` after initial render commands
- startup scheduling does not duplicate metadata loads

Prefer testing the assembled startup behavior over introducing more micro-tests for helper internals.

## Recommended End State

The startup code in `app.rs` should read roughly like this:

1. create window
2. prepare synchronous startup state
3. take initial view snapshot
4. assemble initial shell + render + reveal commands
5. enter event loop
6. let asynchronous hydration update the visible UI incrementally

That is small enough to stay pragmatic and structured enough to resist the specific class of bugs that caused the startup glitch.
