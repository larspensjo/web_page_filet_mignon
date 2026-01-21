## Plan - "Drop Box" URL Paste Workflow

Goal: make pasted URLs immediately enqueue jobs, restart a finished/idle session automatically, and clear the input box to support rapid paste -> alt-tab flow. Keep the app runnable after every step; each step lists what to expect and how to QA.

---

### Code Review Findings (2026-01-21)

**Blockers resolved:**
- `PlatformCommand::SetInputText` exists and is fully implemented in CommanDuctUI.
- Deterministic job IDs: `next_job_id` counter in `AppState` already provides monotonic IDs starting at 1.

**Gap identified:**
- `SessionState::Finished` is defined but **no code ever transitions to it**. `Finishing` never becomes `Finished`. Recommendation: defer `Finished` handling to a follow-up phase; implement auto-start from `Idle` only for now.

**Decisions locked:**
- **Finishing policy:** Strict block (ignore paste while `Finishing`).
- **URL normalization:** Trim whitespace + lowercase + strip trailing `/`.
- **Textbox clearing:** App layer (`dispatch_msg` in `app.rs`) detects `EnqueueUrl` effects and emits `SetInputText`.

**Dependency note:**
- `harvester_core/Cargo.toml` has no dependencies; add `engine_logging = { path = "../engine_logging" }` before Step 5.2.

---

### Phase 0 - Decisions and invariants (no behavior change)
- **Step 0.1: Lock policy for `Finishing`**
  - Deliverable: document choice (strict block vs. resume) in code comments and docs. Default recommendation: strict block to avoid half-finished sessions; optionally allow a feature flag for resume later.
  - QA: `cargo test` (workspace) still passes; app builds and runs unchanged.
  - Notes: keep structs encapsulated; prefer `pub(crate)`; no new getters. Logging stays on `engine_logging` macros.

### Phase 1 - Reducer: paste enqueues and auto-resume
- **Step 1.1: Implement `UrlsPasted` semantics in `harvester_core`**
  - Deliverable: `update()` enqueues immediately, transitions `Idle -> Running`, ignores empty paste, blocks paste while `Finishing`.
  - **Deferred:** `Finished -> Running` transition (requires adding `Finishing -> Finished` transition first; out of scope).
  - Files: `crates/harvester_core/src/update.rs` (rewrite `UrlsPasted` handler), `crates/harvester_core/src/state.rs` (add `seen_urls: HashSet<String>`, `last_paste_stats`, helper methods).
  - Effects: emit `StartSession` when session was `Idle`; emit `EnqueueUrl` per parsed URL; mark state dirty for render.
  - QA: `cargo test -p harvester_core` with new or updated reducer tests (see Step 4.1) passes; app runs and enqueues on paste.

### Phase 2 - UI: clear textbox and messaging
- **Step 2.1: Auto-clear URL box after successful enqueue**
  - Deliverable: on paste that produced at least one enqueue, emit `PlatformCommand::SetInputText` for the URL control with empty string. Ensure the follow-up `InputTextChanged` is a no-op (empty paste returns early).
  - File: `crates/harvester_app/src/platform/app.rs` in `dispatch_msg()` — after `self.effect_runner.enqueue(effects)`, check if any `Effect::EnqueueUrl` was emitted and push `SetInputText` command.
  - QA: manual: paste URL, observe job row appears, textbox clears; repeat paste does not duplicate.
  - Tests: platform-free unit test can assert reducer sets a flag or command in a test hook (if available); otherwise cover via integration smoke in `harvester_app` behind `#[cfg(test)]`.

- **Step 2.2: Update input label and placeholder text**
  - Deliverable: UI text to "Paste URL(s) here. Jobs are created immediately." Keep Start/Stop buttons; optional rename Start -> Resume later.
  - QA: manual visual check; ensure no hard-coded lengths in UI constants (derive from strings).

### Phase 3 - Status and button semantics
- **Step 3.1: Start/Stop button state**
  - Deliverable: keep Start enabled for `Idle` only (current behavior is acceptable); Stop for `Running`. **Deferred:** enabling Start for `Finished` until that state is reachable.
  - File: `crates/harvester_app/src/platform/ui/render.rs` (no change needed for Phase 1).
  - QA: manual: paste when Idle -> auto-start happens, Stop enabled once running.

- **Step 3.2: Status bar wording for drop-box model**
  - Deliverable: change status string to "Session: {label} | Jobs: N | Last paste: enqueued X, skipped Y" using view model data; keep derived counts deterministic.
  - File: `crates/harvester_app/src/platform/ui/render.rs`, `crates/harvester_core/src/view_model.rs` (add `LastPasteStats` struct).
  - QA: manual: paste duplicate shows skipped count increment; verify no truncation, avoid fixed buffer sizes.

### Phase 4 - Robustness: dedupe and tests
- **Step 4.1: Add reducer unit tests for paste workflow**
  - Deliverable: tests in `harvester_core` covering: (a) Idle paste -> Running + StartSession + EnqueueUrl; (b) Running paste -> stays Running + EnqueueUrl only; (c) duplicate paste skipped; (d) Finishing paste ignored; (e) empty paste is no-op; (f) URL normalization catches variants.
  - **Deferred:** test (b-alt) Finished paste -> Running (until `Finished` state is reachable).
  - File: `crates/harvester_core/tests/update_behaviour.rs`.
  - QA: `cargo test -p harvester_core` green; use `engine_logging::initialize_for_tests()` in tests.

- **Step 4.2: Introduce URL deduplication**
  - Deliverable: `seen_urls: HashSet<String>` in `AppState`; normalization = trim + lowercase + strip trailing `/`; skip enqueue for existing URL while still clearing textbox; expose skipped count via `LastPasteStats` in view model.
  - Lifecycle: `seen_urls` persists across pastes within a session; cleared only on explicit session reset (future enhancement).
  - QA: unit test ensures second paste produces zero `EnqueueUrl` effects and increments skipped metric; manual paste same URL twice shows skipped in status.

### Phase 5 - App-level smoke and UX polish
- **Step 5.1: App integration smoke test**
  - Deliverable: small integration test in `harvester_app` (feature-gated) that feeds messages into the store, asserts emitted platform commands include `SetInputText`, and view model exposes expected counts.
  - QA: run `cargo test -p harvester_app --features smoke` (or similar); app still launches.

- **Step 5.2: Add `engine_logging` dependency to `harvester_core`**
  - Deliverable: add `engine_logging = { path = "../engine_logging" }` to `crates/harvester_core/Cargo.toml`.
  - QA: `cargo build -p harvester_core` succeeds.

- **Step 5.3: Telemetry and logging improvements**
  - Deliverable: log each paste event with URL count, dedupe count, session state before/after (`engine_info!` with context). Errors during enqueue should use `engine_error!` with URL/job_id.
  - File: `crates/harvester_core/src/update.rs` (add logging at end of `UrlsPasted` handler).
  - QA: manual: run app, paste URLs, check `engine.log` for structured messages; ensure log level defaults to INFO.

### Phase 6 - Optional follow-ups and nice-to-haves
- **`Finished` state transition**: add `Msg::SessionDrained` (or similar) when `Finishing` and all jobs complete, transitioning to `Finished`. Then enable Start button for `Finished` and auto-start from `Finished` on paste.
- **Clear session action**: add button or menu to reset `seen_urls` HashSet and optionally clear jobs.
- **Resume button UX**: if Start is rarely used, repurpose to Resume (only visible when not Running) after drop-box flow proves stable.
- **Paste history or undo**: keep last N pasted URLs in memory to re-enqueue if a run failed.
- **Per-host rate cap**: throttle engine enqueue for the same host to avoid accidental overload.
- **Manifest or export hook**: add count of skipped duplicates and paste events to session manifest for postmortem.
- **Accessibility**: add shortcut (Ctrl+Shift+V) mapped to the URL box focus and paste to streamline flow.

### QA checklist per step (quick runbook)
- Build: `cargo build` (workspace) must pass after every step.
- Unit tests: run crate-level tests after steps that touch logic (`harvester_core`, `harvester_app`).
- Manual smoke:
  - Paste URL when Idle -> job appears, textbox clears, session Running, status updates.
  - Paste duplicate -> no new job, skipped count increments in status.
  - Paste while Running -> job enqueued, no `StartSession` effect.
  - Paste while Finishing -> ignored (strict block).
  - Paste empty/whitespace -> no-op.
  - ~~Paste while Finished -> resumes automatically~~ (deferred to Phase 6).
- Logging: confirm paste events recorded with counts; no truncated strings.

### Blockers and risks to track early
- ~~Current platform event wiring: ensure we can emit `SetInputText`~~ **RESOLVED**: `PlatformCommand::SetInputText` exists in CommanDuctUI.
- ~~Dedupe canonicalization~~ **RESOLVED**: trim + lowercase + strip trailing `/`.
- ~~`Finishing` semantics~~ **RESOLVED**: strict block (ignore paste while `Finishing`).
- ~~Deterministic job IDs~~ **RESOLVED**: `next_job_id` counter already provides monotonic IDs starting at 1.
- **Risk: `seen_urls` HashSet grows unbounded** — acceptable for typical use; add "clear session" action in Phase 6.
- **Risk: `Finished` state unreachable** — deferred to Phase 6; current implementation handles `Idle` only.
- Avoid hard-coded string lengths; size dynamically if needed.
