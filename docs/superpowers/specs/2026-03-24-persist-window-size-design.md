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

## Save Path

1. `Msg::WindowResized` gains a `window_height: i32` field (currently only `window_width` is forwarded).
2. `AppState` tracks both `window_width` and `window_height`.
3. The reducer emits `Effect::PersistWindowSize { width, height }` on every `Msg::WindowResized`.
4. The effect runner calls a new `persist_window_size` function that loads the current `PersistedState`, updates the two fields, and writes it back atomically.

### Save Trigger

The persist effect fires on every `Msg::WindowResized`. This is acceptable because:
- Windows only sends `WM_SIZE` at the end of a resize drag, not continuously during it.
- The state file is small and written atomically.
- No additional debouncing is needed.

## Restore Path

1. A new `load_window_size(state_path) -> Option<(i32, i32)>` function reads the state file and returns the persisted dimensions if present.
2. At startup in `app.rs`, the loaded dimensions replace the 960×720 defaults in `WindowConfig`.
3. If no persisted size exists (first run, missing file, or parse error), fall back to 960×720.

## Minimum Size Guard

If persisted values are below 960×720, use the defaults. This prevents a corrupted or hand-edited file from creating an unusable window.

## Changes by Layer

| Layer | File | Change |
|-------|------|--------|
| Core | `harvester_core/src/msg.rs` | Add `window_height: i32` to `Msg::WindowResized` |
| Core | `harvester_core/src/state.rs` | Track `window_height` alongside `window_width` |
| Core | `harvester_core/src/effect.rs` | Add `Effect::PersistWindowSize { width: i32, height: i32 }` |
| Core | `harvester_core/src/update.rs` | Emit `PersistWindowSize` on `WindowResized` |
| IO | `harvester_io/src/persistence.rs` | Add fields to `PersistedState`; add `load_window_size` and `persist_window_size` functions |
| IO | `harvester_io/src/effect_runner.rs` | Handle `Effect::PersistWindowSize` |
| App | `harvester_app/src/platform/app.rs` | Forward `height` in event translation; load persisted size at startup; pass to `WindowConfig` |

## Testing

- Unit test: `persist_window_size` round-trips through `load_window_size`.
- Unit test: missing fields in existing state file deserialize to `None` (backward compat).
- Unit test: minimum size guard clamps small values to defaults.
- Reducer test: `Msg::WindowResized` emits `Effect::PersistWindowSize` with correct dimensions.

## Out of Scope

- Window position persistence.
- Maximized/minimized state persistence.
- Multi-monitor awareness.
