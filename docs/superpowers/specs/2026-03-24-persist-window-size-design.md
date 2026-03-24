# Persist Window Size

## Problem

The main Harvester window opens at a hardcoded 960×720 every launch. Users who prefer a larger window must resize it every time.

## Decision

Persist the window's width and height in the existing `.harvester_state.ron` file. On next launch, restore those dimensions instead of using the hardcoded defaults.

Position is NOT persisted — Windows continues to choose placement via `CW_USEDEFAULT`.

## Approach

Extend the existing `PersistedState` struct rather than introducing a new file. Two integers do not justify separate persistence infrastructure.

## Data Model

Add two optional fields to `PersistedState` in `harvester_io/src/persistence.rs`:

```rust
#[serde(default)]
window_width: Option<i32>,
#[serde(default)]
window_height: Option<i32>,
```

`Option` with `#[serde(default)]` ensures backward compatibility — existing state files without these fields deserialize cleanly.

## Dimension Semantics

The persisted values are **outer window dimensions** (including title bar and borders), matching what `CreateWindowExW` expects. This avoids needing `AdjustWindowRect` conversions on restore.

To obtain outer dimensions at save time, CommanDuctUI calls `GetWindowRect` on `WM_EXITSIZEMOVE` and reports `(rect.right - rect.left, rect.bottom - rect.top)` in the event.

## Save Path

1. CommanDuctUI emits a new `AppEvent::WindowResizeCompleted { window_id, outer_width, outer_height }` from the existing `WM_EXITSIZEMOVE` handler, using `GetWindowRect` to obtain outer dimensions.
2. The app translates this into a new `Msg::WindowResizeCompleted { outer_width, outer_height }`.
3. `AppState` tracks `last_outer_width` and `last_outer_height`.
4. The reducer emits `Effect::PersistWindowSize { width, height }` on `Msg::WindowResizeCompleted`.
5. The effect runner calls a new `persist_window_size` function that loads the current `PersistedState`, updates the two fields, and writes it back atomically.

Note: The existing `Msg::WindowResized { window_width }` (client-area, every `WM_SIZE`) is unchanged — it continues to drive layout recalculation. The new message is separate and only fires once per completed resize drag.

### Save Trigger

The persist effect fires only on `WM_EXITSIZEMOVE`, which Windows sends once at the end of a resize or move drag. This means:
- No disk writes during the drag itself (unlike `WM_SIZE` which fires continuously).
- At most one write per user interaction.
- No debouncing needed.

## Restore Path

1. A new `load_window_size(state_path) -> Option<(i32, i32)>` function reads the state file and returns the persisted dimensions if present.
2. At startup in `app.rs`, the loaded dimensions replace the 960×720 defaults in `WindowConfig`.
3. If no persisted size exists (first run, missing file, or parse error), fall back to 960×720.

## Minimum Size Guard

If persisted outer dimensions are below 960×720, use the defaults. Since both the persisted values and `WindowConfig` use outer dimensions, the comparison is apples-to-apples. This prevents a corrupted or hand-edited file from creating an unusable window.

## Changes by Layer

| Layer | File | Change |
|-------|------|--------|
| CommanDuctUI | `CommanDuctUI/src/types.rs` | Add `AppEvent::WindowResizeCompleted { window_id, outer_width, outer_height }` |
| CommanDuctUI | `CommanDuctUI/src/window_common.rs` | Emit `WindowResizeCompleted` from `WM_EXITSIZEMOVE` handler using `GetWindowRect` |
| Core | `harvester_core/src/msg.rs` | Add `Msg::WindowResizeCompleted { outer_width: i32, outer_height: i32 }` |
| Core | `harvester_core/src/state.rs` | Track `last_outer_width` and `last_outer_height` |
| Core | `harvester_core/src/effect.rs` | Add `Effect::PersistWindowSize { width: i32, height: i32 }` |
| Core | `harvester_core/src/update.rs` | Emit `PersistWindowSize` on `WindowResizeCompleted` |
| IO | `harvester_io/src/persistence.rs` | Add fields to `PersistedState`; add `load_window_size` and `persist_window_size` functions |
| IO | `harvester_io/src/effect_runner.rs` | Handle `Effect::PersistWindowSize` |
| App | `harvester_app/src/platform/app.rs` | Translate `AppEvent::WindowResizeCompleted` to `Msg::WindowResizeCompleted`; load persisted size at startup; pass to `WindowConfig` |

## Testing

- Unit test: `persist_window_size` round-trips through `load_window_size`.
- Unit test: `persist_window_size` preserves existing completed jobs and pre-triage overrides in the file.
- Unit test: missing fields in existing state file deserialize to `None` (backward compat).
- Unit test: minimum size guard clamps small values to defaults.
- Reducer test: `Msg::WindowResizeCompleted` emits `Effect::PersistWindowSize` with correct dimensions.

## Out of Scope

- Window position persistence.
- Maximized/minimized state persistence.
- Multi-monitor awareness.
