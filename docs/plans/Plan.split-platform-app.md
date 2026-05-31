# Split `platform/app.rs` Implementation Plan

> **For agentic workers:** Use `superpowers:executing-plans` or
> `superpowers:subagent-driven-development` to implement this plan
> phase-by-phase. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce `crates/harvester_app/src/platform/app.rs` (2,669 lines) to a
thin platform-entry module by extracting its inline test module and its
independent concern clusters into sibling files under `platform/app/`.

**Architecture:** `app.rs` is Win32 platform glue: it boots the app (`run_app`),
translates `commanductui::AppEvent`s into `harvester_core::Msg`s, runs them
through the pure `update()` reducer (which lives in `harvester_core`, not here),
and renders. The reducer is *not* in this file, so this is a pure structural
split — move code between files, change no behavior. Each cluster becomes a
sibling module of `app.rs` (Rust allows `app.rs` plus an `app/` directory for its
submodules).

**Tech Stack:** Rust 2021, `commanductui`, `harvester_core`, `harvester_engine`,
`harvester_io`, `engine_logging`.

---

## Why this file is big

Two things dominate the 2,669 lines:

1. **The inline `#[cfg(test)] mod tests` block (lines 1447–2669, ~1,223 lines,
   ~46% of the file).** Extracting it to `app/tests.rs` is the single largest,
   lowest-risk reduction. The crate already uses this pattern (`mod tests;` in
   `crates/harvester_app/src/platform/ui/render.rs:361` and
   `crates/harvester_app/src/platform/ui/layout/mod.rs:88`).
2. **`handle_event` (lines 1039–1387, ~348 lines)** — a single large
   `PlatformEventHandler` dispatch method, the one genuine "god function" here.

The remaining ~1,446 non-test lines fall into clean clusters (startup, config
helpers, render-batching, archive-dialog building, the event handler, and the UI
state provider) that map naturally onto separate files.

## Current map of `app.rs`

| Lines | Region | Items |
|---|---|---|
| 1–57 | Imports + constants | `ARCHIVE_DIALOG_*`, `*_LLM_*`, `VK_*_CODE` |
| 59–207 | Startup helpers | `apply_startup_msg`, `prepare_startup_state`, `assemble_startup_commands` |
| 208–352 | Entry point | `run_app` |
| 353–425 | Config/env helpers | `parse_llm_max_concurrency_requests`, `llm_max_concurrency_requests_from_env`, `effective_model_map`, `llm_quota_limits_from_engine` |
| 426–468 | Core types + small helpers | `SharedState`, `PendingFocus`, `AppEventHandler` (struct), `triage_marker_for_priority`, `pre_triage_toggle_message` |
| 470–536 | Render batching | `RenderMode`, `PendingRender`, `GeometryBatchStats`(+`impl`), `is_geometry_only_message`, `select_render_mode` |
| 537–742 | Archive-dialog helpers | `archive_dialog_context_tag`, `parse_archive_dialog_request_id`, `archive_field_text`, `archive_field_checked`, `format_archive_since_label`, `format_tokens`, `build_archive_form_descriptor` |
| 743–1024 | `impl AppEventHandler` | `new`, `process_pending_messages`, `enqueue_render`, `enqueue_layout_render`, `queue_focus_after_render`, `enqueue_pending_focus_commands` |
| 1025–1037 | Misc | `impl Drop for AppEventHandler`, `msg_for_preview_context_button` |
| 1038–1392 | Event dispatch | `impl PlatformEventHandler for AppEventHandler` (`handle_event`, `try_dequeue_command`) |
| 1393–1446 | UI state provider | `AppUiStateProvider` + `impl UiStateProvider` |
| 1447–2669 | **Inline tests** | `#[cfg(test)] mod tests { ... }` |

## Target structure

```
platform/
  app.rs            -> imports used by run_app + shared structs/helpers,
                       run_app, the AppEventHandler struct, the inherent
                       AppEventHandler impl (unless moved in Phase 7),
                       small shared helpers, mod declarations
  app/
    tests.rs        -> the entire current #[cfg(test)] mod tests body
    startup.rs      -> apply_startup_msg, prepare_startup_state,
                       assemble_startup_commands
    config.rs       -> parse_llm_max_concurrency_requests,
                       llm_max_concurrency_requests_from_env,
                       effective_model_map, llm_quota_limits_from_engine,
                       the LLM_* constants
    render_batch.rs -> RenderMode, PendingRender, GeometryBatchStats(+impl),
                       is_geometry_only_message, select_render_mode
    archive_dialog.rs -> archive_* helpers, format_tokens,
                       build_archive_form_descriptor, ARCHIVE_DIALOG_* constants
    ui_state.rs     -> AppUiStateProvider + impl UiStateProvider
    event_handler.rs-> impl PlatformEventHandler (handle_event,
                       try_dequeue_command), impl Drop, msg_for_preview_context_button
```

Rust note: `app.rs` declares `mod tests; mod config;` etc.; these resolve to
`platform/app/<name>.rs`. `platform/mod.rs` keeps its existing `mod app;`
declaration unchanged. Genuinely shared types/helpers stay in `app.rs`:
`SharedState`, `PendingFocus`, `AppEventHandler` (struct),
`triage_marker_for_priority`, `pre_triage_toggle_message`. Sibling modules reach
those via `use super::{...}`.

## Constraints (from `Agents.md`)

- Build with `cargo build`. Each phase must end **green**: `cargo build`,
  `cargo test -p harvester_app`, then `cargo clippy --all-targets -- -D warnings`
  and `cargo fmt` all clean. There is no "warnings allowed" checkpoint.
- Entry points (`app.rs`, `mod.rs`, `lib.rs`, `main.rs`) stay thin wrappers.
- Keep shared constants/behavior DRY — one home each (move each constant *with*
  the cluster that uses it; do not duplicate).
- Do not commit; changes are reviewed first.
- If `harvester_mcp` processes block building/testing, kill them.

## Sibling-module visibility & import rules (read before any move phase)

These rules resolve the cross-module breakage that a naïve "move the function,
keep `use super::*;`" would cause.

1. **Test access (the critical one).** `app/tests.rs` currently calls many
   helpers *unqualified* under `use super::*;`. That glob imports items from the
   parent (`app.rs`) only — **not** from sibling modules. The moment an item
   moves into `app/config.rs`, `app/render_batch.rs`, or `app/ui_state.rs`, the
   tests stop compiling. Confirmed test references to soon-to-move items include
   `parse_llm_max_concurrency_requests` (~line 1968), `select_render_mode`
   (~line 2059), and `AppUiStateProvider` (~line 1782). (`triage_marker_for_priority`,
   ~line 1922, *stays* in `app.rs`, so it needs no change.)

   **Rule:** in every move phase, after moving items, grep `app/tests.rs` for
   each moved symbol and, for any hit, add an explicit import at the top of
   `app/tests.rs`, e.g. `use super::config::parse_llm_max_concurrency_requests;`.
   Mark those items `pub(super)` in their new module. Do **not** rely on a glob.

2. **`run_app` / cross-module access.** Items the moved code's *callers* still use
   (most live in `run_app` in `app.rs`, e.g. `prepare_startup_state`,
   `assemble_startup_commands`, `effective_model_map`, `llm_quota_limits_from_engine`,
   `llm_max_concurrency_requests_from_env`, `build_archive_form_descriptor`,
   `AppUiStateProvider::new`) must be `pub(super)`. `app.rs` imports them
   explicitly, e.g. `use config::llm_max_concurrency_requests_from_env;`.

3. **Import ownership.** Move each `use` line **with** the code that needs it.
   After a cluster moves, delete the now-unused imports from `app.rs`. The end
   state: `app.rs`'s `use` block lists only what `run_app`, the shared structs,
   and the module declarations actually reference. Prefer explicit imports in the
   child modules over `use super::*;` once the move compiles (a temporary
   `use super::*;` during the move is fine, but narrow it before the phase's
   clippy/fmt gate).

4. **No stub-only phases.** A module file is created in the same phase its code
   moves in, so every checkpoint compiles warning-free and satisfies the repo
   completion rule.

## Standard move procedure (applies to Phases 2–7)

For each cluster:

1. Add `mod <name>;` to `app.rs` and create `crates/harvester_app/src/platform/app/<name>.rs`.
2. Move the listed items **and the `use` lines they need** into the new file.
3. Mark `pub(super)` every moved item referenced from `app.rs` or `app/tests.rs`
   (per visibility rules 1–2 above).
4. Add explicit `use super::<name>::{...}` imports to `app.rs` for items
   `run_app` uses, and to `app/tests.rs` for items the tests use.
5. Remove now-unused `use` lines from `app.rs`; narrow the new module's imports
   (drop any temporary `use super::*;`).
6. Run the full green gate: `cargo build` → `cargo test -p harvester_app` →
   `cargo clippy --all-targets -- -D warnings` → `cargo fmt`.
7. Stop for review (do not commit).

## Note on test strategy

This is a mechanical refactor: behavior must not change, so the existing test
suite (`cargo test -p harvester_app`) **is** the regression net for every phase.
Each phase is a near-pure code move; "verification" means the suite stays green
and clippy/fmt are clean. No new unit tests are written for pure moves (there is
no new behavior to assert). Phase 7's optional decomposition relies on the
existing `handle_event` tests in `app/tests.rs` to prove behavior is preserved.

Run the baseline once before starting and record it:

```
cargo test -p harvester_app
```

Expected: all tests pass. This is the green bar every later phase must restore.

---

## Phase 1: Extract the inline test module (biggest win, zero risk)

**Files:**
- Create: `crates/harvester_app/src/platform/app/tests.rs`
- Modify: `crates/harvester_app/src/platform/app.rs` (lines 1446–2669)

- [ ] **Step 1: Create `app/tests.rs` with the moved test body**

Cut the body *inside* `#[cfg(test)] mod tests { ... }` (everything between the
outer braces, currently lines ~1448–2668) into the new file, including the
existing inner `use super::*;` and the other inner `use` lines verbatim. As a
child module of `app`, `app/tests.rs` reaches all current `app.rs` private items
through `use super::*;`, so **no `pub(super)` changes are needed in this phase** —
every helper the tests call still lives in `app.rs`. (Visibility work starts in
Phase 2 when items begin leaving `app.rs`.)

- [ ] **Step 2: Replace the inline module with a declaration**

In `app.rs`, replace the entire `#[cfg(test)] mod tests { ... }` block with:

```rust
#[cfg(test)]
mod tests;
```

- [ ] **Step 3: Green gate**

Run:
```
cargo build
cargo test -p harvester_app
cargo clippy --all-targets -- -D warnings
cargo fmt
```
Expected: identical pass count to the baseline; no warnings. If the compiler
flags an unresolved name in `app/tests.rs`, it means a `use` line was left behind
in `app.rs` — move it into `app/tests.rs`; do not change item visibility for a
pure parent→child move.

- [ ] **Step 4: Stop for review** (do not commit, per `Agents.md`).

After this phase `app.rs` is ~1,446 lines.

---

## Phase 2: Move the config cluster → `app/config.rs`

Follow the **Standard move procedure**.

**Files:** `app.rs`, create `app/config.rs`, `app/tests.rs`

- [ ] **Step 1: Move items + imports.** Move `parse_llm_max_concurrency_requests`,
  `llm_max_concurrency_requests_from_env`, `effective_model_map`,
  `llm_quota_limits_from_engine`, and the constants
  `DEFAULT_LLM_MAX_CONCURRENT_REQUESTS`, `LLM_MAX_CONCURRENT_REQUESTS_ENV`,
  `MAX_LLM_CONCURRENT_REQUESTS`. Move the LLM `use` lines they need
  (`harvester_engine::llm::{LlmConfig, LlmQuotas, ModelId, PromptId, ...}`,
  `std::collections::HashMap`) into `app/config.rs`.
- [ ] **Step 2: Visibility.** Mark all four functions `pub(super)` (`run_app`
  calls `llm_max_concurrency_requests_from_env`, `effective_model_map`,
  `llm_quota_limits_from_engine`; tests call `parse_llm_max_concurrency_requests`).
- [ ] **Step 3: Importers.** In `app.rs` add
  `use config::{llm_max_concurrency_requests_from_env, effective_model_map, llm_quota_limits_from_engine};`.
  In `app/tests.rs` add `use super::config::parse_llm_max_concurrency_requests;`
  (covers `parse_llm_max_concurrency_uses_default_when_missing_or_invalid` and
  `parse_llm_max_concurrency_clamps_to_valid_range`).
- [ ] **Step 4: Cleanup + green gate** (build, test, clippy, fmt). Remove the now
  app-unused LLM imports from `app.rs` only if nothing else in `app.rs` uses them.
- [ ] **Step 5: Stop for review.**

---

## Phase 3: Move the render-batching cluster → `app/render_batch.rs`

Follow the **Standard move procedure**.

**Files:** `app.rs`, create `app/render_batch.rs`, `app/tests.rs`

- [ ] **Step 1: Move items + imports.** Move `RenderMode`, `PendingRender`,
  `GeometryBatchStats` (+ its `impl`), `is_geometry_only_message`,
  `select_render_mode`. Move the `use` lines they need
  (`harvester_core::{AppViewModel, LayoutViewModel, Msg}`).
- [ ] **Step 2: Visibility.** Mark `pub(super)` what `AppEventHandler` /
  `process_pending_messages` use (`RenderMode`, `PendingRender`,
  `GeometryBatchStats`, `is_geometry_only_message`, `select_render_mode`) and what
  tests use. Confirmed test references (in `app/tests.rs`,
  `geometry_only_batches_use_layout_only_render_mode` ~line 2059): `select_render_mode`
  (3×) and the `RenderMode` enum (3×). `is_geometry_only_message`,
  `GeometryBatchStats`, and `PendingRender` have **no** test references, so they
  need `pub(super)` only for `app.rs`'s inherent-impl use, not for tests.
- [ ] **Step 3: Importers.** In `app.rs` add the explicit
  `use render_batch::{RenderMode, PendingRender, GeometryBatchStats, is_geometry_only_message, select_render_mode};`
  (drop any the inherent `AppEventHandler` impl does not actually reference; the
  clippy gate will flag unused). In `app/tests.rs` add
  `use super::render_batch::{select_render_mode, RenderMode};`.
- [ ] **Step 4: Cleanup + green gate.**
- [ ] **Step 5: Stop for review.**

---

## Phase 4: Move the archive-dialog cluster → `app/archive_dialog.rs`

Follow the **Standard move procedure**.

**Files:** `app.rs`, create `app/archive_dialog.rs`, `app/tests.rs`

- [ ] **Step 1: Move items + imports.** Move `archive_dialog_context_tag`,
  `parse_archive_dialog_request_id`, `archive_field_text`, `archive_field_checked`,
  `format_archive_since_label`, `format_tokens`, `build_archive_form_descriptor`,
  and the `ARCHIVE_DIALOG_*` constants. Move the form-dialog `use` lines
  (`commanductui::types::{FormButtons, FormDialogDescriptor, FormField,
  FormFieldValue, FormFileExistsWarning, FormRow, FormTextValidation, ...}`,
  `chrono::Utc`) into `app/archive_dialog.rs`.
- [ ] **Step 2: Visibility.** Mark `pub(super)` the items the event handler /
  `run_app` use (notably `build_archive_form_descriptor`,
  `parse_archive_dialog_request_id`, `archive_field_text`, `archive_field_checked`,
  and the `ARCHIVE_DIALOG_*` field-id constants used in `handle_event`).
- [ ] **Step 3: Importers.** Add explicit `use archive_dialog::{...}` to `app.rs`
  where the event handler still lives (until Phase 7). Add any test imports the
  grep turns up.
- [ ] **Step 4: Cleanup + green gate.** Remove the form-dialog imports from
  `app.rs` if `run_app`/structs no longer reference them.
- [ ] **Step 5: Stop for review.**

---

## Phase 5: Move the startup cluster → `app/startup.rs`

Follow the **Standard move procedure**.

**Files:** `app.rs`, create `app/startup.rs`, possibly `app/tests.rs`

- [ ] **Step 1: Move items + imports.** Move `apply_startup_msg`,
  `prepare_startup_state`, `assemble_startup_commands`. Move the
  `harvester_core::{...}` / `harvester_io::{...}` / `harvester_engine::{...}` and
  `ui::render::*` `use` lines those bodies need.
- [ ] **Step 2: Visibility.** Mark `prepare_startup_state` and
  `assemble_startup_commands` `pub(super)` (called by `run_app`).
  `apply_startup_msg` can stay private to `startup.rs` if only the other two call
  it. Check `app/tests.rs` for `prepare_startup_state_schedules_metadata_load_once`
  and `assembled_startup_commands_render_before_reveal` — add
  `use super::startup::{prepare_startup_state, assemble_startup_commands};` if
  they reference these unqualified.
- [ ] **Step 3: Importers.** Add `use startup::{prepare_startup_state, assemble_startup_commands};`
  to `app.rs`.
- [ ] **Step 4: Cleanup + green gate.**
- [ ] **Step 5: Stop for review.**

---

## Phase 6: Move the UI state provider → `app/ui_state.rs`

Follow the **Standard move procedure**.

**Files:** `app.rs`, create `app/ui_state.rs`, `app/tests.rs`

- [ ] **Step 1: Move items + imports.** Move the `AppUiStateProvider` struct,
  `impl AppUiStateProvider`, and `impl UiStateProvider for AppUiStateProvider`
  (`is_tree_item_new`, `tree_item_marker`). Move the `use` lines they need
  (`commanductui::{UiStateProvider, WindowId, TreeItemId, ...}`,
  `commanductui::types::TreeItemMarkerKind`,
  `ui::tree_item_ids::{decode_tree_item_id, TreeItemKind}`).
- [ ] **Step 2: Visibility.** `AppUiStateProvider` and its `new` must be
  `pub(super)` (constructed in `run_app` and referenced by tests). It references
  `SharedState` and `triage_marker_for_priority` (both stay in `app.rs`) — mark
  those `pub(super)` and import them in `ui_state.rs` via
  `use super::{SharedState, triage_marker_for_priority};`.
- [ ] **Step 3: Importers.** In `app.rs` add `use ui_state::AppUiStateProvider;`.
  In `app/tests.rs` add `use super::ui_state::AppUiStateProvider;` (referenced by
  the `tree_item_marker_*` tests, ~line 1782 onward).
- [ ] **Step 4: Cleanup + green gate.**
- [ ] **Step 5: Stop for review.**

After this phase `app.rs` holds: imports used by `run_app`/structs, the
`SharedState`/`PendingFocus`/`AppEventHandler` structs, the inherent
`AppEventHandler` impl, `impl Drop`, `msg_for_preview_context_button`, the trait
impl `PlatformEventHandler`, the shared helpers, `run_app`, and `mod` decls.

---

## Phase 7: Move the event handler impl → `app/event_handler.rs`

Follow the **Standard move procedure**. This phase touches the most code.

**Files:** `app.rs`, create `app/event_handler.rs`, `app/tests.rs`

- [ ] **Step 1: Move items + imports.** Move `impl PlatformEventHandler for
  AppEventHandler` (`handle_event`, `try_dequeue_command`),
  `impl Drop for AppEventHandler`, and `msg_for_preview_context_button`.
  Optionally also move the inherent `impl AppEventHandler` block (`new`,
  `process_pending_messages`, `enqueue_render`, `enqueue_layout_render`,
  `queue_focus_after_render`, `enqueue_pending_focus_commands`) here if they are
  used only by event handling — this is what makes `app.rs` truly thin. The
  `AppEventHandler` **struct** definition stays in `app.rs`. Move the `use` lines
  these bodies need (`commanductui::{AppEvent, ControlId, PlatformCommand,
  PlatformEventHandler, ...}`, `windows::...::{VK_ESCAPE, VK_RETURN}` and the
  `VK_*_CODE` consts, `ui::groups::*`, `ui::constants`, the `render_batch`/
  `archive_dialog` re-imports it now calls).
- [ ] **Step 2: Visibility.** Mark `pub(super)` the struct fields the moved impls
  touch if they were private and `app.rs` no longer sits in the same module — but
  note a sibling module does **not** get private-field access automatically.
  Because the impls move out of `app.rs`, every `self.<field>` access requires the
  fields of `AppEventHandler` to be `pub(super)`. Mark all `AppEventHandler`
  fields `pub(super)`. Mark `AppEventHandler::new` `pub(super)` (called by
  `run_app`). The shared helper `pre_triage_toggle_message` (used by the `X`-key
  arm) stays in `app.rs`; mark it `pub(super)` and import it in
  `event_handler.rs`.
- [ ] **Step 3: Importers.** Add `use event_handler::*;`-free explicit imports as
  needed in `app.rs` (`run_app` constructs `AppEventHandler` and boxes it as
  `dyn PlatformEventHandler`; the trait is in scope already). Update
  `app/tests.rs`: many tests construct `AppEventHandler` and call
  `process_pending_messages`/`queue_focus_after_render`/`handle_event`; if those
  methods moved, the tests still see them because they are inherent/trait methods
  on a type imported via `use super::*;` — confirm with a build and add
  `use super::event_handler::msg_for_preview_context_button;` only if a test calls
  that free function directly.
- [ ] **Step 4: Cleanup + green gate.** Rely on the existing event tests
  (`handle_event`, jobs-search, prompt-lab, listbox-key, archive-footer) to prove
  behavior is unchanged.
- [ ] **Step 5: Stop for review.**

After this phase `app.rs` should be roughly 250–450 lines: imports for
`run_app` + structs, the three structs, the shared helpers, `run_app`, and the
`mod` declarations.

---

## Phase 8 (optional): Decompose `handle_event`

Only if review finds `handle_event` still unwieldy. It is one large `match` over
`AppEvent` variants (~31 arms). Extract each non-trivial arm body into a private
`fn handle_<event>(&mut self, ...)` on `AppEventHandler` in
`app/event_handler.rs`, leaving `handle_event` a thin dispatch `match`.

**Files:** `app/event_handler.rs`

- [ ] **Step 1:** Extract one arm body at a time, verbatim, into a method taking
  the arm's bound fields as parameters.
- [ ] **Step 2:** `cargo test -p harvester_app` after each extraction — the
  existing event tests are the regression net and must stay green.
- [ ] **Step 3:** Clippy + fmt clean.
- [ ] **Step 4: Stop for review.**

---

## Phase 9: Final tidy + diary

**Files:** `crates/harvester_app/src/platform/app.rs`, `docs/EngineeringDiary.md`

- [ ] **Step 1: Confirm `app.rs` is thin** — only imports it still uses, the
  shared structs/helpers, `run_app`, and `mod` declarations. Verify no leftover
  `use` lines for code that now lives in child modules.
- [ ] **Step 2: Final green gate** (build, test, clippy, fmt).
- [ ] **Step 3: Diary entry** in `docs/EngineeringDiary.md`: the split, file
  sizes before/after, and the lesson — *the inline test module was ~46% of the
  file; extracting `mod tests` first gave the largest reduction for the least
  risk, and sibling-module moves require updating the extracted tests' imports
  because `use super::*;` reaches only the parent module.*
- [ ] **Step 4: Stop for review** (do not commit).

---

## Risks and mitigations

- **Extracted tests break when items move to siblings (highest risk).** Mitigated
  by the visibility rules and the per-phase test-import step (grep `app/tests.rs`
  for each moved symbol; add `use super::<module>::<item>;`). See visibility rule 1.
- **`AppEventHandler` field access from a sibling module (Phase 7).** A sibling
  module is not the defining module, so private fields are inaccessible. Mitigated
  by marking all struct fields `pub(super)` in Phase 7 Step 2.
- **`app.rs` retaining imports it no longer owns.** Mitigated by moving `use`
  lines with their code and deleting unused ones each phase (visibility rule 3);
  the clippy gate flags leftovers.
- **Behavior drift in `handle_event` (Phase 8).** Extract byte-for-byte; run the
  event tests after each arm; Phase 8 is optional and last.
- **`mod tests` resolution.** `app.rs` + `app/tests.rs` is valid Rust 2018+;
  confirmed by the same pattern in
  `crates/harvester_app/src/platform/ui/render.rs:361` and
  `crates/harvester_app/src/platform/ui/layout/mod.rs:88`.

## Out of scope

- Any change to `CommanDuctUI`.
- Any change to the `harvester_core` reducer (`update`) — it is not in this file.
- Functional/UX changes to the TUI.
- Introducing new abstractions or sub-structs beyond file moves.
- Performance work.

## Self-review notes

- Every region in the current-map table maps to a phase: tests→P1, config→P2,
  render-batch→P3, archive-dialog→P4, startup→P5, ui-state→P6, event-handler→P7,
  handle_event→P8; residual structs/`run_app`/shared helpers stay in `app.rs`.
- No stub-only phase: each module file is created in the phase its code moves, so
  every checkpoint is warning-free (addresses review finding 2).
- Sibling-module visibility, test imports, and import ownership are spelled out
  in the dedicated rules section and repeated per phase (addresses findings 1 & 3).
- Precedent paths include the `platform/` segment (addresses finding 4).
- Phase 1 is a pure parent→child move with no `pub(super)` churn (addresses
  finding 5).
- No fabricated APIs: all named items are from the confirmed map of the file.
