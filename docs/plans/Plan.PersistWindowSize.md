# Persist Window Size Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist the main window's outer dimensions so the app restores at the user's preferred size on next launch.

**Architecture:** Extend the existing `PersistedState` in `.harvester_state.ron` with two optional fields. CommanDuctUI emits a new `AppEvent::WindowResizeCompleted` on `WM_EXITSIZEMOVE` with outer dimensions from `GetWindowRect`. The reducer emits `Effect::PersistWindowSize` which the effect runner writes to disk.

**Tech Stack:** Rust, Win32 API (GetWindowRect), RON serialization, CommanDuctUI framework

**Spec:** `docs/superpowers/specs/2026-03-24-persist-window-size-design.md`

---

### Task 1: Add `window_width`/`window_height` fields to `PersistedState` and persistence functions

**Files:**
- Modify: `crates/harvester_io/src/persistence.rs:27-31` (PersistedState struct)
- Modify: `crates/harvester_io/src/persistence.rs:177-237` (persist_runtime_state)

- [ ] **Step 1: Add the two optional fields to `PersistedState`**

In `crates/harvester_io/src/persistence.rs`, add to the `PersistedState` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedState {
    completed: Vec<PersistedJob>,
    #[serde(default)]
    pre_triage_overrides: Vec<PersistedPreTriageOverride>,
    #[serde(default)]
    window_width: Option<i32>,
    #[serde(default)]
    window_height: Option<i32>,
}
```

- [ ] **Step 1b: Fix `persist_runtime_state` to preserve window size fields**

**Critical:** The existing `persist_runtime_state` builds a fresh `PersistedState` from its arguments, which would write `window_width: None` and `window_height: None`, clobbering any persisted window size. Fix by loading existing window size fields before writing:

In `persist_runtime_state` (line 177), after the function signature and `ensure_output_dir` check, load the existing state to carry forward window size:

```rust
pub fn persist_runtime_state(
    state_path: &Path,
    completed: &[CompletedJobSnapshot],
    pre_triage_overrides: &std::collections::HashMap<ArticleFilterKey, ManualDecision>,
) {
    let output_dir = state_path.parent().unwrap_or_else(|| Path::new("."));
    if let Err(err) = ensure_output_dir(output_dir) {
        engine_error!("Failed to ensure output dir {:?}: {}", output_dir, err);
        return;
    }

    // Carry forward window size from existing state to avoid clobbering.
    let existing: PersistedState = fs::read_to_string(state_path)
        .ok()
        .and_then(|text| ron::from_str(&text).ok())
        .unwrap_or_default();

    let state = PersistedState {
        completed: completed
            .iter()
            .map(|job| PersistedJob { /* ... existing mapping unchanged ... */ })
            .collect(),
        pre_triage_overrides: pre_triage_overrides
            .iter()
            .map(|(key, decision)| PersistedPreTriageOverride { /* ... existing mapping unchanged ... */ })
            .collect(),
        window_width: existing.window_width,
        window_height: existing.window_height,
    };
    // ... rest of serialization unchanged ...
```

The key change is: load `existing` state, then set `window_width: existing.window_width` and `window_height: existing.window_height` in the new `PersistedState`.

- [ ] **Step 2: Write the backward compatibility verification test**

Add to the `tests` module in `crates/harvester_io/src/persistence.rs`:

```rust
#[test]
fn load_state_without_window_size_deserializes_to_none() {
    let temp = tempdir().expect("tempdir");
    let content = r#"
(
  completed: [
    (
      url: "https://example.com",
      tokens: Some(42u32),
      bytes: Some(1024u64),
    ),
  ],
)
"#;
    write_state(temp.path(), content);
    let path = state_path(temp.path());
    let text = fs::read_to_string(&path).unwrap();
    let state: super::PersistedState = ron::from_str(&text).unwrap();
    assert_eq!(state.window_width, None);
    assert_eq!(state.window_height, None);
}
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo nextest run -p harvester_io load_state_without_window_size`
Expected: PASS (the `#[serde(default)]` on the new fields makes this work immediately)

- [ ] **Step 4: Add `load_window_size` function**

Add after `load_pre_triage_overrides` in `crates/harvester_io/src/persistence.rs`:

```rust
pub fn load_window_size(state_path: &Path) -> Option<(i32, i32)> {
    let content = match fs::read_to_string(state_path) {
        Ok(text) => text,
        Err(_) => return None,
    };
    let state: PersistedState = match ron::from_str(&content) {
        Ok(s) => s,
        Err(_) => return None,
    };
    match (state.window_width, state.window_height) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => None,
    }
}
```

- [ ] **Step 5: Add `persist_window_size` function**

Add after `load_window_size` in `crates/harvester_io/src/persistence.rs`:

```rust
pub fn persist_window_size(state_path: &Path, width: i32, height: i32) {
    let content = fs::read_to_string(state_path).unwrap_or_default();
    let mut state: PersistedState = ron::from_str(&content).unwrap_or_default();
    state.window_width = Some(width);
    state.window_height = Some(height);

    let output_dir = state_path.parent().unwrap_or_else(|| Path::new("."));
    if let Err(err) = ensure_output_dir(output_dir) {
        engine_error!("Failed to ensure output dir {:?}: {}", output_dir, err);
        return;
    }

    let pretty = ron::ser::PrettyConfig::new();
    let serialized = match ron::ser::to_string_pretty(&state, pretty) {
        Ok(text) => text,
        Err(err) => {
            engine_error!("Failed to serialize window size: {}", err);
            return;
        }
    };

    let writer = AtomicFileWriter::new(PathBuf::from(output_dir));
    let filename = state_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".harvester_state.ron");
    if let Err(err) = writer.write(filename, &serialized) {
        engine_error!("Failed to write window size to {:?}: {}", state_path, err);
    }
}
```

- [ ] **Step 6: Export the new functions from the crate**

Find the `pub use persistence::` line in `crates/harvester_io/src/lib.rs` and add `load_window_size` and `persist_window_size` to the re-exports.

- [ ] **Step 7: Write the round-trip test**

Add to the `tests` module in `crates/harvester_io/src/persistence.rs`:

```rust
#[test]
fn persist_and_load_window_size_roundtrips() {
    let temp = tempdir().expect("tempdir");
    let path = state_path(temp.path());
    persist_window_size(&path, 1200, 900);
    let loaded = load_window_size(&path);
    assert_eq!(loaded, Some((1200, 900)));
}
```

- [ ] **Step 8: Write the test that window size preserves existing data**

```rust
#[test]
fn persist_window_size_preserves_existing_jobs() {
    let temp = tempdir().expect("tempdir");
    let path = state_path(temp.path());
    let snapshot = vec![CompletedJobSnapshot {
        url: "https://example.com".to_string(),
        tokens: Some(10),
        bytes: Some(512),
        links: vec![],
        fetched_utc: None,
    }];
    persist_completed_jobs(&path, &snapshot);

    persist_window_size(&path, 1200, 900);

    let loaded_jobs = load_completed_jobs(&path);
    assert_eq!(loaded_jobs.len(), 1);
    assert_eq!(loaded_jobs[0].url, "https://example.com");
    let loaded_size = load_window_size(&path);
    assert_eq!(loaded_size, Some((1200, 900)));
}
```

- [ ] **Step 8b: Write the reverse-direction clobbering test**

This verifies that `persist_runtime_state` (called when saving jobs/overrides) does NOT erase window size:

```rust
#[test]
fn persist_runtime_state_preserves_window_size() {
    let temp = tempdir().expect("tempdir");
    let path = state_path(temp.path());

    // First, persist a window size
    persist_window_size(&path, 1200, 900);

    // Now persist completed jobs via persist_runtime_state
    let jobs = vec![CompletedJobSnapshot {
        url: "https://example.com".to_string(),
        tokens: Some(10),
        bytes: Some(512),
        links: vec![],
        fetched_utc: None,
    }];
    persist_completed_jobs(&path, &jobs);

    // Window size must survive
    let loaded_size = load_window_size(&path);
    assert_eq!(loaded_size, Some((1200, 900)));
}
```

- [ ] **Step 9: Run all persistence tests**

Run: `cargo nextest run -p harvester_io`
Expected: All PASS

- [ ] **Step 10: Commit**

```bash
git add crates/harvester_io/src/persistence.rs crates/harvester_io/src/lib.rs
git commit -m "feat: add window size fields to PersistedState with load/save"
```

---

### Task 2: Add `AppEvent::WindowResizeCompleted` to CommanDuctUI

**Files:**
- Modify: `src/CommanDuctUI/src/types.rs:203-212` (AppEvent enum)
- Modify: `src/CommanDuctUI/src/window_common.rs:1849-1856` (WM_EXITSIZEMOVE handler)

- [ ] **Step 1: Add `WindowResizeCompleted` variant to `AppEvent`**

In `src/CommanDuctUI/src/types.rs`, add after the `WindowResized` variant:

```rust
    /// Signals that a window resize drag has completed.
    /// Dimensions are outer window dimensions (including frame/title bar).
    WindowResizeCompleted {
        window_id: WindowId,
        outer_width: i32,
        outer_height: i32,
    },
```

- [ ] **Step 2: Emit the event from `WM_EXITSIZEMOVE` handler**

In `src/CommanDuctUI/src/window_common.rs`, replace the `WM_EXITSIZEMOVE` arm (around line 1849):

```rust
            WM_EXITSIZEMOVE => {
                let _ = self.with_window_data_write(window_id, |window_data| {
                    window_data.end_live_drag_interaction();
                    Ok(())
                });
                self.trigger_layout_recalculation(window_id);
                log::info!("[UiMsg] exitsizemove: window_id={window_id:?} hwnd={hwnd:?}");
                let mut rect = RECT::default();
                if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
                    let outer_width = rect.right - rect.left;
                    let outer_height = rect.bottom - rect.top;
                    event_to_send = Some(AppEvent::WindowResizeCompleted {
                        window_id,
                        outer_width,
                        outer_height,
                    });
                }
            }
```

Note: `GetWindowRect` is already available via the `UI::WindowsAndMessaging::*` wildcard import. `RECT` is imported from `Win32::Foundation`.

- [ ] **Step 3: Build to verify no compile errors**

Run: `cargo build`
Expected: Compiles with possible warnings about unused variant (that's fine, Task 3 will use it)

- [ ] **Step 4: Update CommanDuctUI version and changelog**

Per project rules (`Agents.md`): if CommanDuctUI changes, update its version and changelog and preserve dark-theme support. Find the CommanDuctUI changelog/version file and add an entry for this change. Dark-theme support is unaffected (this change only adds an event, no visual changes).

- [ ] **Step 5: Commit**

```bash
git add src/CommanDuctUI/
git commit -m "feat(CommanDuctUI): emit WindowResizeCompleted on WM_EXITSIZEMOVE"
```

---

### Task 3: Add `Msg::WindowResizeCompleted` and `Effect::PersistWindowSize`

**Files:**
- Modify: `crates/harvester_core/src/msg.rs:120-123` (Msg enum)
- Modify: `crates/harvester_core/src/effect.rs` (Effect enum)
- Modify: `crates/harvester_core/src/update.rs:286-299` (reducer)

- [ ] **Step 1: Write the failing reducer test**

Add a test in the appropriate test file for update.rs (check where existing `WindowResized` tests live, or add inline). The test should verify that `Msg::WindowResizeCompleted` emits `Effect::PersistWindowSize`:

```rust
#[test]
fn window_resize_completed_emits_persist_effect() {
    let mut state = AppState::default();
    let effects = update(
        &mut state,
        Msg::WindowResizeCompleted {
            outer_width: 1200,
            outer_height: 900,
        },
    );
    assert_eq!(
        effects,
        vec![Effect::PersistWindowSize {
            width: 1200,
            height: 900,
        }]
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p harvester_core window_resize_completed`
Expected: FAIL — `WindowResizeCompleted` variant doesn't exist yet

- [ ] **Step 3: Add `Msg::WindowResizeCompleted` variant**

In `crates/harvester_core/src/msg.rs`, add after the `WindowResized` variant:

```rust
    /// Window resize drag completed. Carries outer (frame) dimensions for persistence.
    WindowResizeCompleted {
        outer_width: i32,
        outer_height: i32,
    },
```

- [ ] **Step 4: Add `Effect::PersistWindowSize` variant**

In `crates/harvester_core/src/effect.rs`, add before the closing `}` of the enum:

```rust
    /// Persist the window's outer dimensions to disk.
    PersistWindowSize {
        width: i32,
        height: i32,
    },
```

- [ ] **Step 5: Add reducer arm for `WindowResizeCompleted`**

In `crates/harvester_core/src/update.rs`, add after the `Msg::WindowResized` arm (around line 299):

```rust
        Msg::WindowResizeCompleted {
            outer_width,
            outer_height,
        } => {
            vec![Effect::PersistWindowSize {
                width: outer_width,
                height: outer_height,
            }]
        }
```

- [ ] **Step 6: Update `is_geometry_only_message` in `app.rs`**

In `crates/harvester_app/src/platform/app.rs`, update the function at line 349:

```rust
fn is_geometry_only_message(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::SplitterMoved { .. } | Msg::WindowResized { .. } | Msg::WindowResizeCompleted { .. }
    )
}
```

Also update `GeometryBatchStats::record` (around line 330) to handle the new variant — or just add it to the `_ => {}` catch-all which already exists. Since this message fires once per drag (not frequently), no special batching stats are needed.

- [ ] **Step 7: Run the reducer test**

Run: `cargo nextest run -p harvester_core window_resize_completed`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add crates/harvester_core/src/msg.rs crates/harvester_core/src/effect.rs crates/harvester_core/src/update.rs crates/harvester_app/src/platform/app.rs
git commit -m "feat: add WindowResizeCompleted msg and PersistWindowSize effect"
```

---

### Task 4: Wire up the effect runner and app event translation

**Files:**
- Modify: `crates/harvester_io/src/effect_runner.rs` (handle PersistWindowSize)
- Modify: `crates/harvester_app/src/platform/app.rs:1159-1165` (event translation)

- [ ] **Step 1: Handle `Effect::PersistWindowSize` in the effect runner**

In `crates/harvester_io/src/effect_runner.rs`, add a new arm in `execute_effect` (after the `SaveBriefingCheckpoint` arm, around line 1045):

```rust
            Effect::PersistWindowSize { width, height } => {
                let path = self.paths.state_path.clone();
                thread::spawn(move || {
                    crate::persist_window_size(&path, width, height);
                    engine_info!(
                        "[window-size] Persisted {}x{} to {:?}",
                        width,
                        height,
                        path
                    );
                });
            }
```

- [ ] **Step 2: Translate `AppEvent::WindowResizeCompleted` to `Msg`**

In `crates/harvester_app/src/platform/app.rs`, in the `handle_event` method, add a new arm **between** the `AppEvent::WindowResized` arm (line ~1159-1165) and the `_ => {}` catch-all (line ~1166). The new arm must come before the catch-all or it will be silently swallowed:

```rust
            AppEvent::WindowResizeCompleted {
                window_id,
                outer_width,
                outer_height,
            } if window_id == self.window_id => {
                let _ = self.msg_tx.send(Msg::WindowResizeCompleted {
                    outer_width,
                    outer_height,
                });
            }
```

- [ ] **Step 3: Build and verify**

Run: `cargo build`
Expected: Compiles cleanly

- [ ] **Step 4: Run all tests**

Run: `cargo nextest run`
Expected: All PASS

- [ ] **Step 5: Commit**

```bash
git add crates/harvester_io/src/effect_runner.rs crates/harvester_app/src/platform/app.rs
git commit -m "feat: wire PersistWindowSize effect and WindowResizeCompleted event"
```

---

### Task 5: Restore persisted window size at startup

**Files:**
- Modify: `crates/harvester_app/src/platform/app.rs:53-62` (run_app startup)

- [ ] **Step 1: Write the failing test for the minimum size guard**

Add to the `tests` module in `crates/harvester_io/src/persistence.rs`:

```rust
#[test]
fn load_window_size_returns_none_when_below_minimum_not_enforced_here() {
    // load_window_size returns raw values; the caller enforces the minimum.
    let temp = tempdir().expect("tempdir");
    let path = state_path(temp.path());
    persist_window_size(&path, 100, 100);
    let loaded = load_window_size(&path);
    assert_eq!(loaded, Some((100, 100)));
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo nextest run -p harvester_io load_window_size_returns_none`
Expected: PASS (confirms load_window_size is a pure data accessor)

- [ ] **Step 3: Load persisted size and apply minimum guard at startup**

In `crates/harvester_app/src/platform/app.rs`, modify the `run_app` function. Replace the hardcoded `WindowConfig`:

The `output_dir` and `paths` construction currently lives at line 80-86, after window creation at line 57-62. Move it up to before `PlatformInterface::new`. The `paths` variable is reused later in the function so no other changes are needed — just reorder. Then use the loaded size for `WindowConfig`:

```rust
    const DEFAULT_WINDOW_WIDTH: i32 = 960;
    const DEFAULT_WINDOW_HEIGHT: i32 = 720;

    let output_dir = effects::default_output_dir();
    let paths = RuntimePaths::new(
        output_dir.clone(),
        effects::default_source_config_path(),
        effects::contexts_directory(),
        effects::prompts_directory(),
    );

    // Restore persisted window size, falling back to defaults.
    // Both dimensions must meet the minimum; otherwise use defaults for both.
    let (initial_width, initial_height) =
        harvester_io::load_window_size(&paths.state_path)
            .filter(|&(w, h)| w >= DEFAULT_WINDOW_WIDTH && h >= DEFAULT_WINDOW_HEIGHT)
            .unwrap_or((DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT));

    let platform = PlatformInterface::new("harvester_app".to_string())?;
    let window_id = platform.create_window(WindowConfig {
        title: "Harvester",
        width: initial_width,
        height: initial_height,
    })?;
```

Remove the now-duplicate `output_dir` and `paths` lines from their old location (formerly lines 80-86). Those variables are already in scope from the moved block.

- [ ] **Step 4: Add the `load_window_size` import**

Ensure `load_window_size` is in the import from `harvester_io` at the top of `app.rs` (around line 33).

- [ ] **Step 5: Build and verify**

Run: `cargo build`
Expected: Compiles cleanly

- [ ] **Step 6: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: No warnings

- [ ] **Step 7: Commit**

```bash
git add crates/harvester_app/src/platform/app.rs
git commit -m "feat: restore persisted window size at startup with minimum guard"
```

---

### Task 6: Manual smoke test and final clippy

- [ ] **Step 1: Run the full test suite**

Run: `cargo nextest run`
Expected: All PASS

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Manual smoke test**

1. Delete `.harvester_state.ron` (or use a fresh output dir) to start clean.
2. Launch the app — window should open at 960x720.
3. Resize the window to something larger (e.g., drag to ~1200x900).
4. Close the app.
5. Open `.harvester_state.ron` — verify `window_width: Some(...)` and `window_height: Some(...)` are present.
6. Relaunch the app — window should open at the larger size.
7. Verify that existing data (completed jobs, triage overrides) is still intact in the file.

- [ ] **Step 4: Add diary entry**

Add a brief entry to `docs/EngineeringDiary.md` noting the persist-window-size feature, the `WM_EXITSIZEMOVE` approach, and the `persist_runtime_state` clobbering fix.

- [ ] **Step 5: Commit any final fixes if needed**
