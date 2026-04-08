# Refactor `update.rs` into Domain Sub-Modules

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the 7,800-line `update.rs` into a `update/` module directory with domain-specific handler files, reducing the main dispatcher to a thin routing table.

**Architecture:** Convert `update.rs` into `update/mod.rs` (thin dispatcher + shared helpers) with sub-modules for each domain. Each sub-module exports one or more `handle_*` functions that take `&mut AppState` and return `Vec<Effect>`. The public API (`pub fn update`) stays identical.

**Tech Stack:** Pure Rust refactor, no new dependencies.

---

## File Structure

After refactoring:

```
crates/harvester_core/src/update/
├── mod.rs                  # Thin dispatcher match + shared utilities (parse_urls, short_hash, summary cache helpers, etc.)
├── prompt_lab.rs           # ~40 Msg::PromptLab* handlers + dispatch helpers
├── briefing.rs             # Briefing lifecycle handlers + dispatch_next_briefing_step + cache logging
├── triage.rs               # Triage + pre-triage handlers + dispatch_next_triage_step
├── archive.rs              # Archive dialog flow handlers
├── polling.rs              # Source polling handlers
├── llm_completed.rs        # The Msg::LlmCompleted mega-handler, decomposed into sub-dispatchers
├── import.rs               # Import saved webpages handlers
└── tests/                  # Test sub-modules (moved from inline #[cfg(test)])
    └── (deferred — not in this plan)
```

### Domain groupings

**`prompt_lab.rs`** (~650 lines of match arms + 4 helper fns):
- All `Msg::PromptLab*` variants (lines 1338–1989, ~40 variants)
- Helper fns: `dispatch_prompt_lab_run`, `PromptLabDispatchRequest`, `dispatch_next_compare_candidate`, `ensure_prompt_lab_template_draft`, `template_draft_texts`, `apply_prompt_lab_template_draft`

**`briefing.rs`** (~300 lines of match arms + ~120 lines helpers):
- `Msg::GenerateBriefingClicked`, `PrepareSummariesClicked`, `BriefingPrereqArticlesLoaded`, `BriefingPrereqLoadFailed`, `BriefingHistoryLoaded`, `BriefingCheckpointLoaded`, `BriefingCheckpointSaveSucceeded`, `BriefingCheckpointSaveFailed`, `BriefingCheckpointSet`, `ArticlesLoaded`, `ArticlesLoadFailed`
- Helper fns: `dispatch_next_briefing_step`, `on_triage_settled_for_briefing`, `try_start_briefing_with_metadata`, `snapshot_briefing_coverage_window`
- Note: `short_hash`, `build_summary_cache_key`, `log_summary_cache_*`, and other summary-cache helpers stay in `mod.rs` (shared across domains)

**`triage.rs`** (~200 lines of match arms + ~130 lines helpers):
- `Msg::TriageClicked`, `TriageArticlesLoaded`, `TriageArticlesLoadFailed`, `PreTriageDecisionSet`, `PreTriageApplyClicked`, `PreTriageResetClicked`, `EvaluatePreTriageRefresh`
- Helper fns: `start_triage_from_pretriage`, `dispatch_next_triage_step`, `schedule_pre_triage_refresh`, `dispatch_pre_triage_if_due`, `log_triage_cache_*` fns

**`archive.rs`** (~110 lines):
- `Msg::ArchiveClicked`, `ArchiveDialogReady`, `ArchiveDialogSubmitted`, `ArchiveExportCompleted`, `ArchiveExportFailed`
- Helper fn: `is_safe_archive_basename`

**`polling.rs`** (~80 lines):
- `Msg::PollSourcesClicked`, `PollIndirectLinks`, `PollStarted`, `SourcePollCompleted`, `SourcePollFailed`, `AllSourcesPollEnded`

**`llm_completed.rs`** (~440 lines):
- The single `Msg::LlmCompleted` arm, which branches into summary/triage/briefing/prompt-lab sub-handlers

**`import.rs`** (~60 lines):
- `Msg::ImportSavedWebpagesRequested`, `ImportSavedWebpagesCompleted`, `ImportSavedWebpagesFailed`, `ImportedCorpusCleared`

**Remaining in `mod.rs`** (~250 lines of match arms):
- Simple/UI messages: `InputChanged`, `StartupHydrationRequested`, `UrlsSubmitted`, `StopFinishClicked`, `ToggleInputPanel`, `JobProgress`, `JobDone`, `LinkToggleRequested`, `LinkDownload*`, `LinkDeleted`, `JobSelected`, `RestoreCompletedJobs`, `SplitterMoved`, `WindowResized`, `WindowResizeCompleted`, `RequestLlmCompletion`, `PromptContextsLoaded`, `PromptContextsLoadFailed`, `LlmMetadataLoaded`, `AiAvailabilityDetected`, `SummaryCacheHydrated`, `TriageCacheHydrated`, `PreTriageOverridesHydrated`, `OpenInBrowserClicked`, `TabSelected`, `LeftTabSelected`, `TrendCategorySelected`, `JobListScopeSet`, `EntityIndex*`, `Tick`, `NoOp`
- `parse_urls` helper

### Cross-cutting concern: `Msg::LlmCompleted`

This handler dispatches to four domains by checking ownership of the `request_id`. It will live in its own file and call into the domain modules:
- Summary completion → calls `briefing::dispatch_next_briefing_step`
- Triage completion → calls `triage::dispatch_next_triage_step`
- Briefing aggregate completion → inline (self-contained)
- Prompt lab completion → calls `prompt_lab::dispatch_next_compare_candidate`

### Visibility approach

Sub-module functions are `pub(super)` so they're accessible from `mod.rs` but not from outside the `update` module. The only public export remains `pub fn update`.

---

## Task 1: Convert `update.rs` to `update/mod.rs` (mechanical rename)

**Files:**
- Rename: `crates/harvester_core/src/update.rs` → `crates/harvester_core/src/update/mod.rs`

This task changes zero code — just the file location. The module system treats them identically.

- [ ] **Step 1: Create directory and move file**

```bash
mkdir crates/harvester_core/src/update
git mv crates/harvester_core/src/update.rs crates/harvester_core/src/update/mod.rs
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles with no errors (module paths unchanged).

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: No new warnings.

- [ ] **Step 4: Run tests**

Run: `cargo test -p harvester_core`
Expected: All existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_core/src/update/ crates/harvester_core/src/update.rs
git commit -m "refactor: convert update.rs to update/mod.rs (mechanical rename)"
```

---

## Task 2: Extract `prompt_lab.rs` sub-module

**Files:**
- Create: `crates/harvester_core/src/update/prompt_lab.rs`
- Modify: `crates/harvester_core/src/update/mod.rs`

This is the largest domain group (~40 Msg variants). Extract all `Msg::PromptLab*` match arms and their helper functions.

- [ ] **Step 1: Create `prompt_lab.rs` with handler functions**

Create `crates/harvester_core/src/update/prompt_lab.rs` containing:

1. Move the `PromptLabDispatchRequest` struct from `mod.rs`
2. Move these helper functions from `mod.rs`:
   - `dispatch_prompt_lab_run`
   - `dispatch_next_compare_candidate`
   - `ensure_prompt_lab_template_draft`
   - `template_draft_texts`
   - `apply_prompt_lab_template_draft`
3. Create a single dispatch function:

```rust
use super::*;  // Inherits all imports from mod.rs

pub(super) fn handle(state: &mut AppState, msg: &Msg) -> Option<Vec<Effect>> {
    // Returns Some(effects) if the message was handled, None if not a PromptLab message.
    // This avoids matching on every Msg variant — only PromptLab ones.
    let effects = match msg {
        Msg::PromptLabOpenRequested => { /* ... */ }
        // ... all PromptLab* arms ...
        _ => return None,
    };
    Some(effects)
}
```

**Note on `msg` ownership:** Some arms destructure owned data from `msg` (e.g., `Msg::PromptLabInputChanged { text }`). Since the Msg enum derives `Clone`, the handler can take `msg: Msg` (owned) instead of `&Msg`. The dispatcher in `mod.rs` will clone when delegating, or — better — pass the owned `msg` and reconstruct the return path. The simplest approach: the handler takes `msg: Msg` by value.

Revised signature:

```rust
pub(super) fn handle(state: &mut AppState, msg: Msg) -> Option<Vec<Effect>> {
```

The dispatcher in `mod.rs` will try prompt_lab first; if it returns `None`, the msg was not consumed and the dispatcher handles it in the remaining match. Since `Msg: Clone`, the dispatcher can `clone()` before passing to the sub-handler, though this is only needed if the fallback match also needs the value. A cleaner pattern: match on a discriminant check first, then delegate.

**Recommended dispatcher pattern in `mod.rs`:**

```rust
pub fn update(mut state: AppState, msg: Msg) -> (AppState, Vec<Effect>) {
    // Try domain-specific handlers first
    if matches!(msg, Msg::PromptLabOpenRequested
        | Msg::PromptLabCloseRequested
        | Msg::PromptLabStageSelected { .. }
        | Msg::PromptLabInputSourceSelected { .. }
        // ... all PromptLab variants listed ...
    ) {
        let effects = prompt_lab::handle(&mut state, msg);
        return (state, effects);
    }

    // ... remaining match ...
}
```

Alternatively, use a simpler approach — just `match msg` with arms that call into the sub-module:

```rust
Msg::PromptLabOpenRequested => prompt_lab::handle_open(&mut state),
Msg::PromptLabCloseRequested => prompt_lab::handle_close(&mut state),
Msg::PromptLabRunRequested => {
    let effects = prompt_lab::handle_run_requested(&mut state);
    return (state, effects);
}
// etc.
```

**Decision: Use the "one handler per arm" approach** (the second pattern above). Rationale:
- The main match remains a readable table of contents
- Each handler function has a clear name and signature
- No need for `Option` wrapping or discriminant pre-checks
- Arms that need `return (state, effects)` for early exit are explicit in `mod.rs`

So `prompt_lab.rs` exports ~40 `pub(super)` functions, one per message variant, plus the private helpers.

- [ ] **Step 2: Wire up in `mod.rs`**

Add `mod prompt_lab;` at the top of `mod.rs`.

Replace each `Msg::PromptLab*` match arm body with a call to the corresponding function in `prompt_lab.rs`. For example:

```rust
Msg::PromptLabOpenRequested => prompt_lab::handle_open(&mut state),
Msg::PromptLabInputChanged { text } => prompt_lab::handle_input_changed(&mut state, text),
Msg::PromptLabContextApplyAndRerunRequested => {
    let effects = prompt_lab::handle_context_apply_and_rerun(&mut state);
    return (state, effects);  // Early return preserved
}
```

**Important:** Some arms use `return (state, effects)` or `return (state, Vec::new())` for early exit. These `return` statements must stay in `mod.rs` — the sub-module function returns a value, and `mod.rs` decides whether to `return` or fall through. Pattern:

```rust
// In mod.rs — for arms that may early-return:
Msg::PromptLabContextSaveRequested => {
    return prompt_lab::handle_context_save_requested(&mut state);
    // The function returns (AppState, Vec<Effect>) directly
}
```

Wait — that changes the function signature. Better: the handler returns `Vec<Effect>`, and `mod.rs` wraps with `return (state, effects)` when needed. But the current code does `return (state, Vec::new())` from within the arm. So the handler should return `Vec<Effect>`, and all early returns become the caller's responsibility.

**Actually,** re-reading the code: the early `return` statements are interleaved with state mutations. For example in `PromptLabContextApplyAndRerunRequested`:

```rust
if !state.prompt_lab_mut().apply_context_draft(prompt_id) {
    return (state, Vec::new());   // <-- early exit
}
```

These can stay inside the handler if the handler signature matches the outer function: `fn(&mut AppState) -> Vec<Effect>`, and the early returns just become `return Vec::new()`. The outer match arm then uses `return (state, effects)` to propagate.

**No — simpler:** just let the handler return `Vec<Effect>`, and the guard returns within the handler naturally become `return Vec::new()`. The outer `update` function always returns `(state, effects)` at the bottom. The only complication is arms that `return (state, ...)` *before* the final `(state, effects)` — but that's just an optimization to skip the bottom. Replacing `return (state, Vec::new())` with just `Vec::new()` achieves the same result since the main match collects into `effects` and returns `(state, effects)`.

**Conclusion: Handler signature is `fn handle_xxx(state: &mut AppState, ...) -> Vec<Effect>`**. All the extracted handlers work naturally — guard clauses return `Vec::new()`, happy paths return the effects vector. The `return (state, ...)` pattern in the original code was only needed because the match is inside the function; once extracted, a normal `return` from the handler suffices.

- [ ] **Step 3: Migrate tests that reference moved symbols**

Any inline tests in `mod.rs` (under `#[cfg(test)] mod tests`) that directly reference `PromptLabDispatchRequest`, `dispatch_prompt_lab_run`, or other symbols moved to `prompt_lab.rs` must be updated to use `super::prompt_lab::*` imports, or moved into a `prompt_lab.rs`-local `#[cfg(test)]` block. Verify by compiling with `cargo test -p harvester_core --no-run`.

- [ ] **Step 4: Verify it compiles**

Run: `cargo build`

- [ ] **Step 5: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`

- [ ] **Step 6: Run tests**

Run: `cargo test -p harvester_core`
Expected: All existing tests pass unchanged.

- [ ] **Step 7: Commit**

```bash
git add crates/harvester_core/src/update/prompt_lab.rs crates/harvester_core/src/update/mod.rs
git commit -m "refactor: extract prompt_lab handlers from update into sub-module"
```

---

## Task 3: Extract `briefing.rs` sub-module

**Files:**
- Create: `crates/harvester_core/src/update/briefing.rs`
- Modify: `crates/harvester_core/src/update/mod.rs`

- [x] **Step 1: Create `briefing.rs` with handler functions**

Move these match arm bodies into handler functions:
- `Msg::GenerateBriefingClicked` → `handle_generate_clicked`
- `Msg::PrepareSummariesClicked` → `handle_prepare_summaries_clicked`
- `Msg::BriefingPrereqArticlesLoaded` → `handle_prereq_articles_loaded`
- `Msg::BriefingPrereqLoadFailed` → `handle_prereq_load_failed`
- `Msg::BriefingHistoryLoaded` → `handle_history_loaded`
- `Msg::BriefingCheckpointLoaded` → `handle_checkpoint_loaded`
- `Msg::BriefingCheckpointSaveSucceeded` → `handle_checkpoint_save_succeeded`
- `Msg::BriefingCheckpointSaveFailed` → `handle_checkpoint_save_failed`
- `Msg::BriefingCheckpointSet` → `handle_checkpoint_set`
- `Msg::ArticlesLoaded` → `handle_articles_loaded`
- `Msg::ArticlesLoadFailed` → `handle_articles_load_failed`

Move these briefing-specific helper functions:
- `dispatch_next_briefing_step` (make `pub(super)` since `llm_completed.rs` calls it)
- `on_triage_settled_for_briefing` (make `pub(super)` since `triage.rs` calls it via `dispatch_next_triage_step`)
- `try_start_briefing_with_metadata`
- `snapshot_briefing_coverage_window`

**Keep these cross-domain helpers in `mod.rs`** (they are used by briefing, triage, and llm_completed — placing them in `briefing.rs` would create cross-domain coupling):
- `short_hash`
- `build_summary_cache_key`
- `summary_cache_key_error_reason`
- `summary_cache_model_ids_compatible`
- `log_summary_cache_warmup_if_needed`
- `log_summary_cache_run_summary`
- `log_summary_cache_lookup_mismatch`
- `log_summary_cache_completion_metadata`

All handlers: `pub(super) fn handle_xxx(state: &mut AppState, ...) -> Vec<Effect>`.

- [x] **Step 2: Wire up in `mod.rs`**

Add `mod briefing;` (the domain module, shadowing the `crate::briefing` — use `self::briefing` or rename to `mod update_briefing` if collision occurs; actually, `mod briefing` within `update/` creates `update::briefing`, not `crate::briefing`, so no collision).

Replace each briefing-related match arm body with a handler call.

- [x] **Step 3: Build + clippy + test**

```bash
cargo build && cargo clippy --all-targets -- -D warnings && cargo test -p harvester_core
```

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_core/src/update/briefing.rs crates/harvester_core/src/update/mod.rs
git commit -m "refactor: extract briefing handlers from update into sub-module"
```

---

## Task 4: Extract `triage.rs` sub-module

**Files:**
- Create: `crates/harvester_core/src/update/triage.rs`
- Modify: `crates/harvester_core/src/update/mod.rs`

- [x] **Step 1: Create `triage.rs` with handler functions**

Move these match arm bodies:
- `Msg::TriageClicked` → `handle_triage_clicked`
- `Msg::TriageArticlesLoaded` → `handle_articles_loaded`
- `Msg::TriageArticlesLoadFailed` → `handle_articles_load_failed`
- `Msg::PreTriageDecisionSet` → `handle_pre_triage_decision_set`
- `Msg::PreTriageApplyClicked` → `handle_pre_triage_apply_clicked`
- `Msg::PreTriageResetClicked` → `handle_pre_triage_reset_clicked`
- `Msg::EvaluatePreTriageRefresh` → `handle_evaluate_pre_triage_refresh`

Move these helper functions:
- `start_triage_from_pretriage`
- `dispatch_next_triage_step` (make `pub(super)` since `llm_completed.rs` calls it)
- `schedule_pre_triage_refresh`
- `dispatch_pre_triage_if_due` (make `pub(super)` since `mod.rs` Tick handler calls it)
- `log_triage_cache_run_start_if_needed`
- `log_triage_cache_run_summary`

**Cross-module dependency:** `dispatch_next_triage_step` calls `on_triage_settled_for_briefing` from the briefing module. Import it as `super::briefing::on_triage_settled_for_briefing`.

- [x] **Step 2: Wire up in `mod.rs`**

Add `mod triage;` and replace match arm bodies. The `Msg::Tick` handler in `mod.rs` will call `triage::dispatch_pre_triage_if_due`.

- [x] **Step 3: Build + clippy + test**

```bash
cargo build && cargo clippy --all-targets -- -D warnings && cargo test -p harvester_core
```

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_core/src/update/triage.rs crates/harvester_core/src/update/mod.rs
git commit -m "refactor: extract triage handlers from update into sub-module"
```

---

## Task 5: Extract `archive.rs` sub-module

**Files:**
- Create: `crates/harvester_core/src/update/archive.rs`
- Modify: `crates/harvester_core/src/update/mod.rs`

- [x] **Step 1: Create `archive.rs` with handler functions**

Move these match arm bodies:
- `Msg::ArchiveClicked` → `handle_archive_clicked`
- `Msg::ArchiveDialogReady` → `handle_dialog_ready`
- `Msg::ArchiveDialogSubmitted` → `handle_dialog_submitted`
- `Msg::ArchiveExportCompleted` → `handle_export_completed`
- `Msg::ArchiveExportFailed` → `handle_export_failed`

Move `is_safe_archive_basename` as a private helper.

- [x] **Step 2: Wire up in `mod.rs`**

- [x] **Step 3: Build + clippy + test**

```bash
cargo build && cargo clippy --all-targets -- -D warnings && cargo test -p harvester_core
```

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_core/src/update/archive.rs crates/harvester_core/src/update/mod.rs
git commit -m "refactor: extract archive handlers from update into sub-module"
```

---

## Task 6: Extract `polling.rs` sub-module

**Files:**
- Create: `crates/harvester_core/src/update/polling.rs`
- Modify: `crates/harvester_core/src/update/mod.rs`

- [x] **Step 1: Create `polling.rs` with handler functions**

Move these match arm bodies:
- `Msg::PollSourcesClicked` → `handle_poll_sources_clicked`
- `Msg::PollIndirectLinks` → `handle_poll_indirect_links`
- `Msg::PollStarted` → `handle_poll_started`
- `Msg::SourcePollCompleted` → `handle_source_poll_completed`
- `Msg::SourcePollFailed` → `handle_source_poll_failed`
- `Msg::AllSourcesPollEnded` → `handle_all_sources_poll_ended`

- [x] **Step 2: Wire up in `mod.rs`**

- [x] **Step 3: Build + clippy + test**

```bash
cargo build && cargo clippy --all-targets -- -D warnings && cargo test -p harvester_core
```

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_core/src/update/polling.rs crates/harvester_core/src/update/mod.rs
git commit -m "refactor: extract polling handlers from update into sub-module"
```

---

## Task 7: Extract `llm_completed.rs` sub-module

**Files:**
- Create: `crates/harvester_core/src/update/llm_completed.rs`
- Modify: `crates/harvester_core/src/update/mod.rs`

This is the most complex extraction because `Msg::LlmCompleted` (lines 330–768, ~440 lines) branches into four domains.

- [ ] **Step 1: Create `llm_completed.rs`**

```rust
use super::*;

pub(super) fn handle(
    state: &mut AppState,
    request_id: u64,
    result: LlmResultKind,
    metadata: Option<LlmRunMetadata>,
) -> Vec<Effect> {
    // Record the LLM result in state (common preamble, lines 335–366)
    // ...

    // Branch by ownership:
    if let Some(article_idx) = state.briefing().find_article_by_request_id(request_id) {
        handle_summary_completion(state, article_idx, &result, &metadata)
    } else if let Some(article_idx) = state.triage().find_article_by_request_id(request_id) {
        handle_triage_completion(state, article_idx, &result)
    } else if state.briefing().is_briefing_request(request_id) {
        handle_briefing_completion(state, &result)
    } else if let Some(run_id) = state.prompt_lab().ownership_for(request_id) {
        handle_prompt_lab_completion(state, request_id, run_id, &result, metadata)
    } else {
        Vec::new()
    }
}
```

Each sub-function is private within `llm_completed.rs`. They call into sibling modules:
- `handle_summary_completion` calls `super::briefing::dispatch_next_briefing_step`
- `handle_triage_completion` calls `super::triage::dispatch_next_triage_step`
- `handle_prompt_lab_completion` calls `super::prompt_lab::dispatch_next_compare_candidate`

And uses shared helpers from `mod.rs` (accessible via `super::*`):
- `build_summary_cache_key`
- `log_summary_cache_*`
- `short_hash`

- [ ] **Step 2: Wire up in `mod.rs`**

Replace the `Msg::LlmCompleted { request_id, result, metadata }` arm:

```rust
Msg::LlmCompleted { request_id, result, metadata } => {
    llm_completed::handle(&mut state, request_id, result, metadata)
}
```

- [ ] **Step 3: Build + clippy + test**

```bash
cargo build && cargo clippy --all-targets -- -D warnings && cargo test -p harvester_core
```

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_core/src/update/llm_completed.rs crates/harvester_core/src/update/mod.rs
git commit -m "refactor: extract LlmCompleted handler from update into sub-module"
```

---

## Task 8: Extract `import.rs` sub-module

**Files:**
- Create: `crates/harvester_core/src/update/import.rs`
- Modify: `crates/harvester_core/src/update/mod.rs`

- [x] **Step 1: Create `import.rs` with handler functions**

Move these match arm bodies:
- `Msg::ImportSavedWebpagesRequested` → `handle_import_requested`
- `Msg::ImportSavedWebpagesCompleted` → `handle_import_completed`
- `Msg::ImportSavedWebpagesFailed` → `handle_import_failed`
- `Msg::ImportedCorpusCleared` → `handle_corpus_cleared`

- [x] **Step 2: Wire up in `mod.rs`**

- [x] **Step 3: Build + clippy + test**

```bash
cargo build && cargo clippy --all-targets -- -D warnings && cargo test -p harvester_core
```

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_core/src/update/import.rs crates/harvester_core/src/update/mod.rs
git commit -m "refactor: extract import handlers from update into sub-module"
```

---

## Task 9: Remove suppressed clippy lints and final cleanup

**Files:**
- Modify: `crates/harvester_core/src/update/mod.rs`

- [ ] **Step 1: Remove the `#[allow(...)]` attributes**

The `update()` function currently has:
```rust
#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    clippy::excessive_nesting
)]
```

Remove all three `allow` attributes. After extraction, the function should be small enough to pass all three lints.

- [ ] **Step 2: Run clippy to confirm lints pass**

```bash
cargo clippy --all-targets -- -D warnings
```

If `too_many_lines` still fires (the remaining ~250 lines of match arms may exceed clippy's default 100-line threshold), consider either:
- Adjusting the threshold in `clippy.toml` / `Cargo.toml`
- Extracting a few more trivial arms into a `ui.rs` sub-module
- Keeping the allow for `too_many_lines` only (the other two should pass)

- [ ] **Step 3: Run full test suite**

```bash
cargo test -p harvester_core
```

- [ ] **Step 4: Update engineering diary**

Append a short entry to `docs/EngineeringDiary.md` summarizing the refactor: what was split, the module structure, and any decisions made during extraction (e.g. shared helper placement). Reference the plan document.

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_core/src/update/mod.rs docs/EngineeringDiary.md
git commit -m "refactor: remove clippy suppression from update() after module split"
```

---

## Notes

### Tests: fix references as you go, bulk-move deferred
The ~5,000 lines of tests remain in `update/mod.rs` under `#[cfg(test)] mod tests`. Most test the public `update()` function and don't need to move. However, **each extraction task must fix any tests that directly reference moved symbols** (structs, helper functions) — either by updating imports or relocating those specific tests into the sub-module. Bulk test splitting into per-domain files is deferred as independent work.

### Migration order matters
Tasks 2–8 can be done in any order after Task 1, but the recommended order is as written because:
1. Prompt Lab (Task 2) is the largest and most self-contained — good first extraction
2. Briefing (Task 3) and Triage (Task 4) export helpers needed by LLM Completed
3. LLM Completed (Task 7) depends on helpers from briefing, triage, and prompt_lab — do it after those

### Each task is independently shippable
After each task, the code compiles, passes clippy, and all tests pass. You can stop after any task and have a valid codebase.
