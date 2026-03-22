# Archive Untriaged Warning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show a yellow warning in the archive dialog when the export uses old `TriageComplete` data while pre-triage-ready articles are waiting and will be excluded.

**Architecture:** Add `pending_pre_triage_count: usize` to the existing effect/message chain (`OpenArchiveDialog` → `ArchiveDialogReady` → `ShowArchiveDialog`). Compute the count in the `ArchiveClicked` reducer handler using the `CurrentWorkingCorpus` selector already in place. Render a `FormRow::Note` warning in `build_archive_form_descriptor` when the count is non-zero.

**Tech Stack:** Rust, `harvester_core` (reducer/effects), `harvester_io` (effect runner), `harvester_app` (UI). Builds on the `feature/working-corpus-selector` branch — `CurrentWorkingCorpus` and `CurrentWorkingCorpusSource` must already exist.

**Spec:** `docs/superpowers/specs/2026-03-22-archive-untriaged-warning-design.md`

---

## File Map

| File | Change |
|---|---|
| `crates/harvester_core/src/effect.rs` | Add `pending_pre_triage_count: usize` to `OpenArchiveDialog` and `ShowArchiveDialog` |
| `crates/harvester_core/src/msg.rs` | Add `pending_pre_triage_count: usize` to `ArchiveDialogReady` |
| `crates/harvester_core/src/update.rs` | Compute and pass count in `ArchiveClicked`; pass through in `ArchiveDialogReady` |
| `crates/harvester_io/src/effect_runner.rs` | Pass `pending_pre_triage_count` through in `OpenArchiveDialog` handler |
| `crates/harvester_app/src/platform/app.rs` | Add parameter to `build_archive_form_descriptor`; add `FormRow::Note` when > 0 |

---

## Task 1: Thread `pending_pre_triage_count` through effects and messages

This task adds the field to all the types and data-flow plumbing. No behavior changes yet — just compiling plumbing. All existing tests must still pass at the end.

**Files:**
- Modify: `crates/harvester_core/src/effect.rs`
- Modify: `crates/harvester_core/src/msg.rs`
- Modify: `crates/harvester_core/src/update.rs` (two sites: `ArchiveClicked` and `ArchiveDialogReady`)
- Modify: `crates/harvester_io/src/effect_runner.rs`
- Modify: `crates/harvester_app/src/platform/app.rs`

**Background — existing chain:**

```
ArchiveClicked (update.rs)
  → Effect::OpenArchiveDialog { request_id, article_count, since_utc, default_basename }
    → effect_runner.rs spawns thread, checks file existence
      → Msg::ArchiveDialogReady { request_id, article_count, since_utc, default_basename,
                                   default_file_exists, export_dir }
        → update.rs ArchiveDialogReady handler
          → Effect::ShowArchiveDialog { same fields }
            → app.rs ShowArchiveDialog handler
              → build_archive_form_descriptor(...)
```

- [ ] **Step 1: Add field to `OpenArchiveDialog` and `ShowArchiveDialog` in `effect.rs`**

  In `crates/harvester_core/src/effect.rs`, add `pending_pre_triage_count: usize` to both variants:

  ```rust
  OpenArchiveDialog {
      request_id: u64,
      article_count: usize,
      since_utc: Option<chrono::DateTime<chrono::Utc>>,
      default_basename: String,
      pending_pre_triage_count: usize,   // NEW
  },
  ShowArchiveDialog {
      request_id: u64,
      article_count: usize,
      since_utc: Option<chrono::DateTime<chrono::Utc>>,
      default_basename: String,
      default_file_exists: bool,
      export_dir: PathBuf,
      pending_pre_triage_count: usize,   // NEW
  },
  ```

- [ ] **Step 2: Add field to `ArchiveDialogReady` in `msg.rs`**

  In `crates/harvester_core/src/msg.rs`, add `pending_pre_triage_count: usize` to `ArchiveDialogReady`:

  ```rust
  ArchiveDialogReady {
      request_id: u64,
      article_count: usize,
      since_utc: Option<DateTime<Utc>>,
      default_basename: String,
      default_file_exists: bool,
      export_dir: PathBuf,
      pending_pre_triage_count: usize,   // NEW
  },
  ```

- [ ] **Step 3: Pass field through in `update.rs` — `ArchiveClicked` handler**

  In the `ArchiveClicked` arm (around line 97), pass `pending_pre_triage_count: 0` for now (real value comes in Task 2):

  ```rust
  vec![Effect::OpenArchiveDialog {
      request_id,
      article_count,
      since_utc,
      default_basename: "archive.md".to_string(),
      pending_pre_triage_count: 0,   // will be computed in Task 2
  }]
  ```

- [ ] **Step 4: Pass field through in `update.rs` — `ArchiveDialogReady` handler**

  In the `ArchiveDialogReady` arm (around line 901), destructure and forward the new field:

  ```rust
  Msg::ArchiveDialogReady {
      request_id,
      article_count,
      since_utc,
      default_basename,
      default_file_exists,
      export_dir,
      pending_pre_triage_count,   // NEW
  } => {
      if request_id != state.archive_request_id() {
          return (state, Vec::new());
      }
      vec![Effect::ShowArchiveDialog {
          request_id,
          article_count,
          since_utc,
          default_basename,
          default_file_exists,
          export_dir,
          pending_pre_triage_count,   // NEW
      }]
  }
  ```

- [ ] **Step 5: Pass field through in `effect_runner.rs` — `OpenArchiveDialog` handler**

  In `crates/harvester_io/src/effect_runner.rs`, the `OpenArchiveDialog` handler (around line 219) spawns a thread. Capture `pending_pre_triage_count` and pass it to `Msg::ArchiveDialogReady`:

  ```rust
  Effect::OpenArchiveDialog {
      request_id,
      article_count,
      since_utc,
      default_basename,
      pending_pre_triage_count,   // NEW — destructure
  } => {
      let msg_tx = self.msg_tx.clone();
      let output_dir = self.paths.output_dir.clone();
      thread::spawn(move || {
          let default_file_exists = output_dir.join(&default_basename).exists();
          // ... existing log line (update it to include pending_pre_triage_count if desired) ...
          let _ = msg_tx.send(Msg::ArchiveDialogReady {
              request_id,
              article_count,
              since_utc,
              default_basename,
              default_file_exists,
              export_dir: output_dir,
              pending_pre_triage_count,   // NEW
          });
      });
  }
  ```

- [ ] **Step 6: Update `app.rs` — add parameter to `build_archive_form_descriptor` and thread it through**

  In `crates/harvester_app/src/platform/app.rs`:

  Add `pending_pre_triage_count: usize` parameter to `build_archive_form_descriptor` (keep the warning row as a `todo!()` or simply don't add it yet — just get it to compile):

  ```rust
  fn build_archive_form_descriptor(
      request_id: u64,
      article_count: usize,
      since_utc: Option<chrono::DateTime<Utc>>,
      default_basename: String,
      default_file_exists: bool,
      export_dir: PathBuf,
      pending_pre_triage_count: usize,   // NEW
  ) -> FormDialogDescriptor {
  ```

  In the `ShowArchiveDialog` match arm (~line 635), destructure and pass the new field:

  ```rust
  Effect::ShowArchiveDialog {
      request_id,
      article_count,
      since_utc,
      default_basename,
      default_file_exists,
      export_dir,
      pending_pre_triage_count,   // NEW
  } => {
      let form = build_archive_form_descriptor(
          request_id,
          article_count,
          since_utc,
          default_basename,
          default_file_exists,
          export_dir,
          pending_pre_triage_count,   // NEW
      );
  ```

- [ ] **Step 7: Build to verify the plumbing compiles**

  Run: `cargo build -p harvester_core -p harvester_io -p harvester_app`

  Expected: compiles without errors (warnings about unused `pending_pre_triage_count` parameter are fine).

- [ ] **Step 8: Fix all existing test compile errors and run tests**

  Adding the new field will break existing tests that use named struct destructuring or construction for `Effect::OpenArchiveDialog`, `Effect::ShowArchiveDialog`, and `Msg::ArchiveDialogReady`. Search for all occurrences in `update.rs` tests:

  ```
  grep -n "OpenArchiveDialog\|ShowArchiveDialog\|ArchiveDialogReady" crates/harvester_core/src/update.rs
  ```

  For each named destructuring pattern (e.g. `Effect::OpenArchiveDialog { request_id, article_count, .. }`), either:
  - Add `pending_pre_triage_count: _,` to the pattern, or
  - Add `..` if not already present

  For each named construction site (e.g. building `Msg::ArchiveDialogReady { ... }` in test helpers), add `pending_pre_triage_count: 0`.

  Run: `cargo nextest run`

  Expected: all existing tests pass.

- [ ] **Step 9: Commit**

  ```
  git add crates/harvester_core/src/effect.rs \
          crates/harvester_core/src/msg.rs \
          crates/harvester_core/src/update.rs \
          crates/harvester_io/src/effect_runner.rs \
          crates/harvester_app/src/platform/app.rs
  git commit -m "Thread pending_pre_triage_count through archive dialog effect chain"
  ```

---

## Task 2: Compute the count and add the warning

**Files:**
- Modify: `crates/harvester_core/src/update.rs` (compute in `ArchiveClicked`)
- Modify: `crates/harvester_app/src/platform/app.rs` (add `FormRow::Note`)
- Test: `crates/harvester_core/src/update.rs` (new tests in the existing test module)

**Background — imports already present in `update.rs`:**

`CurrentWorkingCorpusSource` is re-exported from `harvester_core::working_corpus`. The `ArchiveClicked` handler already calls `state.current_working_corpus()` and binds `source`. `state.pre_triage()` returns `&PreTriageSession`, which has `resolved_included_urls() -> Vec<String>`.

- [ ] **Step 1: Write the failing tests**

  Add these four tests inside the existing `#[cfg(test)]` block in `update.rs`. Look at the existing tests `parity_a_pre_triage_ready_corpus_count_dialog_count_urls_match` and `parity_b_triage_complete_corpus_count_dialog_count_urls_match` for the helper pattern (`ready_pre_triage_state`, `complete_triage_state_for_test`).

  Helper reference:
  - `ready_pre_triage_state(urls: &[&str])` — builds an `AppState` in `PreTriageReady` phase (pre-triage has ready articles, triage is idle)
  - `complete_triage_state_for_test(n: usize)` — builds an `AppState` with `TriageComplete` corpus and idle pre-triage

  For the first test (TriageComplete + ready pre-triage), you need a combined state. Start from `complete_triage_state_for_test(1)` to get a `TriageComplete` state, then drive a pre-triage refresh on top of it using messages until pre-triage enters `ReadyToTriage`. Look at how existing tests like `refresh_between_open_and_submit_uses_pinned_snapshot` drive a pre-triage refresh (they use `add_completed_job_for_test`, `apply_pending_pre_triage_refresh_evaluation`, etc.). If building the combined state requires more than ~10 lines, ask for context before guessing.

  ```rust
  #[test]
  fn archive_clicked_with_triage_complete_and_pre_triage_ready_sets_pending_count() {
      engine_logging::initialize_for_tests();
      // Build a state where triage is complete AND pre-triage has ready articles.
      // Start from complete_triage_state_for_test, then drive a pre-triage refresh.
      let state = ...;  // see note above
      let (_, effects) = update(state, Msg::ArchiveClicked);
      let count = effects.iter().find_map(|e| {
          if let Effect::OpenArchiveDialog { pending_pre_triage_count, .. } = e {
              Some(*pending_pre_triage_count)
          } else { None }
      }).expect("expected OpenArchiveDialog effect");
      assert!(count > 0, "expected pending_pre_triage_count > 0 when pre-triage has ready articles, got {}", count);
  }

  #[test]
  fn archive_clicked_with_pre_triage_ready_corpus_has_zero_pending_count() {
      engine_logging::initialize_for_tests();
      // PreTriageReady: the export uses pre-triage articles directly, so no "pending" articles.
      let state = ready_pre_triage_state(&["https://example.com/a", "https://example.com/b"]);
      let (_, effects) = update(state, Msg::ArchiveClicked);
      let count = effects.iter().find_map(|e| {
          if let Effect::OpenArchiveDialog { pending_pre_triage_count, .. } = e {
              Some(*pending_pre_triage_count)
          } else { None }
      }).expect("expected OpenArchiveDialog effect");
      assert_eq!(count, 0, "PreTriageReady corpus should have zero pending count");
  }

  #[test]
  fn archive_clicked_with_triage_complete_and_empty_pre_triage_has_zero_pending_count() {
      engine_logging::initialize_for_tests();
      let state = complete_triage_state_for_test(2);
      // pre-triage is idle/empty — no ready articles to warn about
      let (_, effects) = update(state, Msg::ArchiveClicked);
      let count = effects.iter().find_map(|e| {
          if let Effect::OpenArchiveDialog { pending_pre_triage_count, .. } = e {
              Some(*pending_pre_triage_count)
          } else { None }
      }).expect("expected OpenArchiveDialog effect");
      assert_eq!(count, 0, "TriageComplete with empty pre-triage should have zero pending count");
  }

  #[test]
  fn archive_clicked_with_pre_triage_reviewing_has_zero_pending_count() {
      engine_logging::initialize_for_tests();
      // Reviewing phase: resolved_included_urls() returns empty (articles are not yet resolved),
      // so pending count must be 0 even though articles are being reviewed.
      // Build a reviewing state by driving manual decisions through the reducer.
      // Look at working_corpus tests (test pre_triage_reviewing_takes_precedence_over_complete_triage)
      // for how to construct a Reviewing state. If the state cannot be built, ask for context.
      let state = ...;
      let (_, effects) = update(state, Msg::ArchiveClicked);
      let count = effects.iter().find_map(|e| {
          if let Effect::OpenArchiveDialog { pending_pre_triage_count, .. } = e {
              Some(*pending_pre_triage_count)
          } else { None }
      }).expect("expected OpenArchiveDialog effect");
      assert_eq!(count, 0, "PreTriageReviewing corpus should have zero pending count");
  }
  ```

- [ ] **Step 2: Run tests to confirm they fail**

  Run: `cargo nextest run -p harvester_core archive_clicked_with_triage`

  Expected: tests fail (the count is always 0 since we haven't implemented the logic yet).

- [ ] **Step 3: Compute `pending_pre_triage_count` in `ArchiveClicked`**

  In `update.rs`, in the `ArchiveClicked` arm, after computing `corpus`, replace `pending_pre_triage_count: 0` with:

  ```rust
  let pending_pre_triage_count =
      if corpus.source() == CurrentWorkingCorpusSource::TriageComplete {
          state.pre_triage().resolved_included_urls().len()
      } else {
          0
      };
  ```

  Then pass it through:

  ```rust
  vec![Effect::OpenArchiveDialog {
      request_id,
      article_count,
      since_utc,
      default_basename: "archive.md".to_string(),
      pending_pre_triage_count,
  }]
  ```

  `CurrentWorkingCorpusSource` is available via `use crate::working_corpus::CurrentWorkingCorpusSource;` — check if it is already imported at the top of the file; if not, add it.

- [ ] **Step 4: Run tests to confirm they pass**

  Run: `cargo nextest run -p harvester_core archive_clicked_with`

  Expected: all four new tests pass.

- [ ] **Step 5: Add warning row in `build_archive_form_descriptor`**

  In `crates/harvester_app/src/platform/app.rs`, in `build_archive_form_descriptor`, add the warning row after the existing `article_count == 0` / `default_file_exists` checks:

  ```rust
  if article_count == 0 {
      rows.push(FormRow::Note {
          text: "No articles match the current filter.".to_string(),
          severity: MessageSeverity::Warning,
      });
  } else if default_file_exists {
      rows.push(FormRow::Note {
          text: "file already exists - will be overwritten".to_string(),
          severity: MessageSeverity::Warning,
      });
  }
  // NEW:
  if pending_pre_triage_count > 0 {
      rows.push(FormRow::Note {
          text: format!(
              "{} article{} await triage and are not included in this export.",
              pending_pre_triage_count,
              if pending_pre_triage_count == 1 { "" } else { "s" }
          ),
          severity: MessageSeverity::Warning,
      });
  }
  ```

- [ ] **Step 6: Run full test suite**

  Run: `cargo nextest run`

  Expected: all tests pass.

- [ ] **Step 7: Commit**

  ```
  git add crates/harvester_core/src/update.rs \
          crates/harvester_app/src/platform/app.rs
  git commit -m "Show untriaged articles warning in archive dialog"
  ```

---

## Done

Run `cargo clippy --all-targets -- -D warnings` to verify no warnings.

Update `docs/EngineeringDiary.md` with a brief entry, then delete this plan file in the same commit.
