# Design: Harvester Batch TUI Launcher

**Date:** 2026-02-21  
**Status:** Reviewed; changes incorporated, ready for implementation planning  
**References:** `docs/Discussion.BriefingAndArchive.md` (Ideas 3A-3C), `crates/harvester_batch/src/cli.rs`

---

## Draft Diary Entry

Context: `harvester_batch` currently exposes many flags and no interactive launcher, which increases operator error and slows daily operation. We also need checkpoint actions to degrade gracefully until Slice A CLI flags exist.

Change: Define a PowerShell TUI launcher architecture with reducer-owned state transitions, explicit effect handling, robust command execution (`argv`-based), checkpoint capability probing, and in-scope unit tests for reducer/render/effects.

---

## Baseline Checked Against Current Source

- `harvester_batch` currently supports: `--sources`, `--output-dir`, `--contexts-dir`, `--prompts-dir`, `--llm-concurrency`, `--force-unlock`, `--allow-unsupported-sources`, `--dry-run`, `--poll-interval` (`crates/harvester_batch/src/cli.rs`).
- Checkpoint flags (`--set-briefing-since*`, `--clear-briefing-since`) do not exist yet.
- Existing TUI reference (`ministry-of-future-plans/Browse-Ideas.ps1`) uses reducer + render + key-mapping modules and can be reused for input mapping patterns.

---

## Design Corrections From Review

1. ENTER behavior is now consistent with requirements:
   main run actions execute immediately (no confirm overlay).
2. Unidirectional flow is explicit:
   reducer emits effect requests; entrypoint executes effects and dispatches follow-up actions.
3. Command building/execution uses argument arrays, not concatenated shell strings:
   prevents quoting bugs for paths with spaces and avoids command injection risk.
4. Checkpoint graceful degradation uses capability probing (`--help`) rather than brittle stderr text matching.
5. Testing is in scope for this slice (Pester), not deferred.

---

## Context and Motivation

`harvester_batch` has 9 active flags and is error-prone to run manually. The launcher must:

1. show live config on startup (defaults file + hardcoded fallback),
2. run immediately on ENTER for `Run batch` and `Run dry-run`,
3. support fast parameter edits with keyboard navigation,
4. persist one default profile for next launch,
5. include checkpoint commands now, with safe degradation until Slice A flags exist.

---

## Decisions

| Question | Decision |
|----------|----------|
| TUI style | Full-screen two-pane console UI |
| Run action confirmation | No confirmation for run actions (`Enter` executes immediately) |
| Checkpoint actions | Present now; disabled with clear status if CLI capability probe says unsupported |
| Profiles | Single default profile at `scripts/harvester_launcher_defaults.json` |
| Submodule reuse | Reuse `ministry-of-future-plans/browser/Input.psm1`; implement launcher-specific reducer/render/effects modules |
| Command execution | Build `argv` array and invoke process with explicit argument list |
| Architecture | Reducer is pure and emits effects; IO is outside reducer |
| `--dry-run` | Chosen by left-pane action, not by parameter row |

---

## Architecture (UDF-Compliant)

Pipeline:
`KeyInput -> Action -> Reduce(State, Action) -> (State', Effects[]) -> RunEffects -> FollowUpActions -> Reduce(...) -> Render`

Rules:

- Reducer performs no IO and no direct process execution.
- All filesystem/process work happens in effect handlers.
- Every user-visible change is traceable by action boundaries.

### State Shape

```powershell
@{
  Data = @{
    Actions = @(...)      # action metadata
    Params  = @(...)      # parameter metadata
  }
  Ui = @{
    ActivePane = 'Left'   # Left|Right
    TooSmall   = $false
    Layout     = @{
      Width  = 0
      Height = 0
      Left   = @{ X=0; Y=0; W=0; H=0 }
      Right  = @{ X=0; Y=0; W=0; H=0 }
      Status = @{ X=0; Y=0; W=0; H=1 }
    }
  }
  Cursor = @{
    LeftIndex  = 0
    RightIndex = 0
    LeftScroll = 0
    RightScroll= 0
  }
  Values = @{
    LlmConcurrency=3; PollInterval=15; ForceUnlock=$false; AllowUnsupported=$false;
    Sources='sources.ron'; OutputDir='output'; ContextsDir='contexts'; PromptsDir='prompts'
  }
  Runtime = @{
    IsRunning = $true
    LastStatus = $null      # $null|'OK'|'Error'|'Warn'
    LastMessage = ''
    CheckpointDisplay = 'not set (all-time briefing)'
    CheckpointCliAvailable = $false
    HarvesterCmd = 'harvester_batch'
  }
  Pending = @{
    Effects = @()           # queued effect requests from reducer
    LaunchAfterExit = $null # @{ FilePath='...'; Argv=@(...) } for run/dry-run
  }
}
```

### Actions and Effect Requests

Actions:

- UI intent: `Resize`, `SwitchPane`, `MoveUp`, `MoveDown`, `PageUp`, `PageDown`, `MoveHome`, `MoveEnd`, `ValueIncrease`, `ValueDecrease`, `ValueToggle`, `Activate`, `SaveDefaults`, `Quit`.
- Effect results: `DefaultsLoaded`, `DefaultsLoadFailed`, `DefaultsSaved`, `DefaultsSaveFailed`, `CheckpointReadCompleted`, `CheckpointReadFailed`, `CheckpointCapabilityDetected`, `CheckpointCommandCompleted`, `CheckpointCommandFailed`.

Effect requests:

- `LoadDefaults`
- `SaveDefaults`
- `ProbeCheckpointCliSupport`
- `ReadCheckpointDisplay`
- `RunCheckpointCommand`
- `PrepareRunCommand` (compute `argv` and set `LaunchAfterExit`)

---

## Module Specifications

### `scripts/Start-HarvesterBatch.ps1`

Responsibilities:

- import launcher modules and submodule input mapper,
- initialize state then dispatch startup effects (`LoadDefaults`, `ProbeCheckpointCliSupport`, `ReadCheckpointDisplay`),
- run loop:
  1. detect resize -> dispatch `Resize`,
  2. render,
  3. read key -> map to action,
  4. reduce action,
  5. execute queued effects, dispatch follow-up actions, reduce again,
  6. exit when `Runtime.IsRunning = $false`,
- after loop: if `LaunchAfterExit` exists, run command with explicit argument array,
- always restore cursor and clear screen in `finally`.

### `scripts/harvester_launcher/Data.psm1`

- `Get-LauncherActionItems`
- `Get-LauncherParamDefs`
- `New-LauncherDefaults`
- `Get-DefaultsFilePath`

Defaults and parameter defs must be the single source of truth for both rendering and command build.

### `scripts/harvester_launcher/Input.psm1`

Wrap submodule `ConvertFrom-KeyInfoToAction` and override launcher-specific keys.

`Enter` always maps to `Activate` in browse mode. No confirm mode for run actions.

### `scripts/harvester_launcher/Reducer.psm1`

Exports:

- `New-LauncherState`
- `Invoke-LauncherReducer` returning:
  `@{ State = $nextState; Effects = @($effectRequests) }`
- `Build-CommandArgs` returning typed command spec:
  `@{ FilePath = $HarvesterCmd; Argv = @('--sources','...') }`

Reducer behavior highlights:

- run actions queue `PrepareRunCommand` and set `IsRunning = $false`,
- checkpoint actions queue `RunCheckpointCommand` only when `CheckpointCliAvailable = $true`,
- if checkpoint unsupported, no process call; status message explains Slice A dependency,
- value changes clamp by parameter metadata min/max.

### `scripts/harvester_launcher/Effects.psm1`

New module for side effects.

Exports:

- `Invoke-LauncherEffects -State -Effects`

Responsibilities:

- file IO (defaults read/write),
- probe support by executing `harvester_batch --help` and searching for required flags,
- checkpoint file read (`output/.briefing_checkpoint.ron`) for display,
- checkpoint command execution (`--set-briefing-since-now`, `--set-briefing-since`, `--clear-briefing-since`),
- emit follow-up actions only (no direct state mutation).

### `scripts/harvester_launcher/Render.psm1`

- frame-diff renderer similar to `Browse-Ideas`,
- dynamic layout sizing:
  left width computed from longest visible action label + padding, with min/max clamps based on current terminal width,
- too-small mode message remains, but threshold comes from computed minimum viable pane widths/heights, not magic layout literals,
- status bar shows active hints and last operation result.

---

## Command Construction and Execution

Use argument arrays for both preview and execution.

Example run command spec:

```powershell
@{
  FilePath = 'harvester_batch'
  Argv = @(
    '--sources', 'sources.ron',
    '--output-dir', 'output',
    '--contexts-dir', 'contexts',
    '--prompts-dir', 'prompts',
    '--llm-concurrency', '3',
    '--poll-interval', '15'
  )
}
```

Notes:

- include all explicit values for correctness and easier troubleshooting (not "omit defaults"),
- preview string is derived from `Argv` using a single shared escaping function,
- launch via invocation with explicit argument list, not a shell-concatenated string.

---

## Checkpoint Graceful Degradation

Capability strategy:

1. Startup effect runs `harvester_batch --help`.
2. If help output contains checkpoint flags, set `CheckpointCliAvailable = $true`.
3. If not available, keep actions visible but render them as disabled and show status:
   `Checkpoint commands unavailable (Slice A CLI flags not implemented yet)`.

Custom date flow:

1. On `Set checkpoint to custom date...`, temporarily suspend TUI and prompt for RFC3339 input.
2. Validate with `[DateTimeOffset]::TryParse(...)`.
3. On valid input, dispatch `RunCheckpointCommand` effect.
4. Resume TUI and refresh checkpoint display.

---

## Default Profile

Path: `scripts/harvester_launcher_defaults.json`

Format (with versioning):

```json
{
  "SchemaVersion": 1,
  "Sources": "sources.ron",
  "OutputDir": "output",
  "ContextsDir": "contexts",
  "PromptsDir": "prompts",
  "LlmConcurrency": 3,
  "PollInterval": 15,
  "ForceUnlock": false,
  "AllowUnsupported": false
}
```

Rules:

- unknown keys are ignored,
- missing keys fall back to hardcoded defaults,
- invalid types/ranges are clamped and reported in status message,
- save uses atomic replace pattern (temp file + move) to avoid partial writes.

---

## Blockers and Risks

1. Slice A CLI flags are not present yet.
   Mitigation: capability probe + disabled checkpoint actions.
2. PowerShell process invocation can break with quoted paths if implemented as one string.
   Mitigation: explicit `Argv` list only.
3. RON checkpoint parsing is brittle if regex is too narrow.
   Mitigation: tolerant parser helper and clear fallback text on parse failure.
4. Console resizing/artifacts can regress quickly.
   Mitigation: frame-diff tests and deterministic layout helpers.

---

## Verification and Testing Plan (In Scope)

Manual verification:

1. `pwsh scripts/Start-HarvesterBatch.ps1` renders correctly at normal size.
2. `Enter` on `Run dry-run` exits TUI and starts `harvester_batch --dry-run ...` immediately (no confirmation screen).
3. Value edit keys clamp correctly and preview updates instantly.
4. `S` persists defaults, relaunch reloads them.
5. Resize and too-small terminal behavior are stable and artifact-free.
6. If checkpoint flags absent, checkpoint actions are visibly disabled with clear status.

Automated tests (Pester):

1. Reducer tests:
   `Activate` on run action emits `PrepareRunCommand`, exits launcher, no IO.
2. Reducer tests:
   checkpoint action when unsupported emits no process effect and sets warning.
3. Command builder tests:
   paths containing spaces round-trip to argv and preview safely.
4. Defaults tests:
   load/save round-trip with schema version and clamping.
5. Render tests:
   too-small mode and pane width calculations.
6. Effects tests:
   capability probe parsing and checkpoint file parse fallback behavior.

---

## Future Extensions

- Multiple named profiles (`scripts/harvester_profiles.json`).
- `HARVESTER_PROFILE` environment variable override.
- Prompted path editing UI (inline text input) instead of defaults-file-only path changes.
- Optional confirm guard for destructive checkpoint actions (`Clear`) while keeping run actions immediate.
- Status/event log pane to improve operator traceability.
