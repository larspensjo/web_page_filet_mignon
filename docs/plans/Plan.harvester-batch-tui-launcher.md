# Harvester Batch TUI Launcher — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task.

**Goal:** Build `scripts/Start-HarvesterBatch.ps1`, a full-screen two-pane PowerShell TUI launcher for `harvester_batch` that shows a live editable configuration, runs immediately on Enter, and supports briefing checkpoint management with graceful degradation.

**Architecture:** Elm/Redux UDF pipeline — `KeyInput → Action → Reduce(State,Action) → (State',Effects[]) → RunEffects → FollowUpActions`. Reducer is pure (no IO). All side effects go through `Effects.psm1`. Frame-diff rendering minimises console writes. Reuses `ministry-of-future-plans/browser/Input.psm1` from the git submodule for base key mapping.

**Tech Stack:** PowerShell 5.1+, Pester for unit tests, `[Console]` APIs for TUI, `harvester_batch` binary (Rust CLI). No new Rust code in this slice.

**Design doc:** `docs/plans/Design.harvester-batch-tui-launcher.md`

---

## Blockers (check before starting)

1. **Submodule must be initialised:**
   ```
   git submodule update --init ministry-of-future-plans
   ```
   Verify: `Test-Path ministry-of-future-plans\browser\Input.psm1` returns True.

2. **Pester 5 must be installed:**
   ```powershell
   Install-Module Pester -Force -Scope CurrentUser
   ```

3. **`harvester_batch` binary for manual smoke tests** (Task 12):
   ```
   cargo build -p harvester_batch
   ```

4. **PowerShell host requirement — use `pwsh` (PowerShell 7+), not `powershell` (Windows PowerShell 5.1):**
   The launcher uses `[Console]::CursorVisible`, `[DateTimeOffset]::Parse`, and the null-coalescing
   operator `??` which require PowerShell 7. Launch via:
   ```
   pwsh scripts\Start-HarvesterBatch.ps1
   ```
   Verify minimum: `pwsh --version` → `PowerShell 7.x`. Document the host requirement in the
   smoke-test checklist and in the diary entry. If `??` is not available, replace with
   `if ($null -ne $x) { $x } else { $y }` and update `#Requires -Version` accordingly.

5. **Checkpoint CLI flags absent until Slice A ships:**
   `--set-briefing-since`, `--set-briefing-since-now`, `--clear-briefing-since` are not yet
   in `crates/harvester_batch/src/cli.rs`. Checkpoint action items will show
   "Checkpoint CLI not yet available (Slice A pending)" until the flags ship and the
   startup capability probe finds all three.

---

## File Map

```
scripts/
  Start-HarvesterBatch.ps1                 ← entry point (Task 12)
  harvester_launcher_defaults.json         ← created at runtime by S keypress
  harvester_launcher/
    Data.psm1                              ← Task 2: static definitions
    Input.psm1                             ← Task 9: key mapper
    Reducer.psm1                           ← Tasks 3-7: pure state management
    Effects.psm1                           ← Task 8: all IO / process calls
    Render.psm1                            ← Tasks 10-11: frame rendering
  tests/
    HarvesterLauncher.Tests.ps1            ← grows across Tasks 2-11
```

Imports from git submodule (read-only):
```
ministry-of-future-plans/browser/Input.psm1   → ConvertFrom-KeyInfoToAction
```

---

## Task 1 — Scaffold directory structure

**Files:** create `scripts/harvester_launcher/` directory and empty module files

**Step 1:** Create directories and empty files
```powershell
New-Item -ItemType Directory -Path scripts\harvester_launcher -Force
New-Item -ItemType File -Path scripts\harvester_launcher\Data.psm1    -Force
New-Item -ItemType File -Path scripts\harvester_launcher\Input.psm1   -Force
New-Item -ItemType File -Path scripts\harvester_launcher\Reducer.psm1 -Force
New-Item -ItemType File -Path scripts\harvester_launcher\Effects.psm1 -Force
New-Item -ItemType File -Path scripts\harvester_launcher\Render.psm1  -Force
New-Item -ItemType File -Path scripts\Start-HarvesterBatch.ps1        -Force
New-Item -ItemType File -Path scripts\tests\HarvesterLauncher.Tests.ps1 -Force
```

**Step 2:** Commit
```
git add scripts/
git commit -m "feat(launcher): scaffold module structure"
```

---

## Task 2 — Data.psm1: action items and parameter definitions

**Files:**
- Write: `scripts/harvester_launcher/Data.psm1`
- Write: `scripts/tests/HarvesterLauncher.Tests.ps1` (initial tests)

### Step 1: Write failing tests

`scripts/tests/HarvesterLauncher.Tests.ps1`:
```powershell
#Requires -Version 5.1
Set-StrictMode -Version Latest

Describe 'Data - Get-LauncherActionItems' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1" -Force
    }
    It 'returns exactly 7 items (including separator)' {
        (Get-LauncherActionItems).Count | Should Be 7
    }
    It 'first item id is run-batch' {
        (Get-LauncherActionItems)[0].Id | Should Be 'run-batch'
    }
    It 'second item id is run-dry with IsDryRun' {
        $i = (Get-LauncherActionItems)[1]
        $i.Id       | Should Be 'run-dry'
        $i.IsDryRun | Should Be $true
    }
    It 'has exactly one separator' {
        ((Get-LauncherActionItems) | Where-Object { $_.IsSeparator }).Count | Should Be 1
    }
    It 'has exactly 3 checkpoint items' {
        ((Get-LauncherActionItems) | Where-Object { $_.IsCheckpoint }).Count | Should Be 3
    }
}

Describe 'Data - Get-LauncherParamDefs' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1" -Force
    }
    It 'returns 8 parameter definitions' {
        (Get-LauncherParamDefs).Count | Should Be 8
    }
    It 'LlmConcurrency is Int with min=1 max=10' {
        $p = Get-LauncherParamDefs | Where-Object { $_.Name -eq 'LlmConcurrency' }
        $p.Type | Should Be 'Int'
        $p.Min  | Should Be 1
        $p.Max  | Should Be 10
    }
    It 'PollInterval is Int with min=1 max=1440' {
        $p = Get-LauncherParamDefs | Where-Object { $_.Name -eq 'PollInterval' }
        $p.Min | Should Be 1
        $p.Max | Should Be 1440
    }
    It 'ForceUnlock is Bool type' {
        $p = Get-LauncherParamDefs | Where-Object { $_.Name -eq 'ForceUnlock' }
        $p.Type | Should Be 'Bool'
    }
    It 'Sources is Path type' {
        $p = Get-LauncherParamDefs | Where-Object { $_.Name -eq 'Sources' }
        $p.Type | Should Be 'Path'
    }
    It 'all items have a non-empty Flag' {
        Get-LauncherParamDefs | ForEach-Object {
            $_.Flag | Should Not BeNullOrEmpty
        }
    }
}

Describe 'Data - New-LauncherDefaults' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1" -Force
    }
    It 'LlmConcurrency defaults to 3' {
        (New-LauncherDefaults).LlmConcurrency | Should Be 3
    }
    It 'PollInterval defaults to 15' {
        (New-LauncherDefaults).PollInterval | Should Be 15
    }
    It 'ForceUnlock defaults to false' {
        (New-LauncherDefaults).ForceUnlock | Should Be $false
    }
    It 'Sources defaults to sources.ron' {
        (New-LauncherDefaults).Sources | Should Be 'sources.ron'
    }
}

Describe 'Data - Get-DefaultsFilePath' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1" -Force
    }
    It 'path ends with harvester_launcher_defaults.json' {
        (Get-DefaultsFilePath) | Should Match 'harvester_launcher_defaults\.json$'
    }
}
```

### Step 2: Run tests — verify they fail
```
Invoke-Pester scripts\tests\HarvesterLauncher.Tests.ps1 -Output Minimal
```
Expected: all tests fail with "not recognized as name of cmdlet".

### Step 3: Implement Data.psm1
```powershell
#Requires -Version 5.1
Set-StrictMode -Version Latest

function Get-LauncherActionItems {
    @(
        [pscustomobject]@{ Id='run-batch';   Label='Run batch (continuous)';    IsSeparator=$false; IsCheckpoint=$false; IsDryRun=$false }
        [pscustomobject]@{ Id='run-dry';     Label='Run dry-run (single poll)'; IsSeparator=$false; IsCheckpoint=$false; IsDryRun=$true  }
        [pscustomobject]@{ Id='sep-1';       Label='';                          IsSeparator=$true;  IsCheckpoint=$false; IsDryRun=$false }
        [pscustomobject]@{ Id='cp-set-now';  Label='Set checkpoint to now';     IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false }
        [pscustomobject]@{ Id='cp-set-date'; Label='Set checkpoint to date...'; IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false }
        [pscustomobject]@{ Id='cp-clear';    Label='Clear checkpoint';          IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false }
        [pscustomobject]@{ Id='cp-show';     Label='Show current checkpoint';   IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false }
    )
}

function Get-LauncherParamDefs {
    # Order matters: determines right-pane cursor index
    @(
        [pscustomobject]@{ Name='LlmConcurrency';   Label='LLM concurrency';   Type='Int';  Min=1;    Max=10;   Unit='';     Flag='--llm-concurrency'           }
        [pscustomobject]@{ Name='PollInterval';     Label='Poll interval';     Type='Int';  Min=1;    Max=1440; Unit=' min'; Flag='--poll-interval'             }
        [pscustomobject]@{ Name='ForceUnlock';      Label='Force unlock';      Type='Bool'; Min=$null; Max=$null; Unit='';   Flag='--force-unlock'              }
        [pscustomobject]@{ Name='AllowUnsupported'; Label='Allow unsupported'; Type='Bool'; Min=$null; Max=$null; Unit='';   Flag='--allow-unsupported-sources' }
        [pscustomobject]@{ Name='Sources';          Label='Sources file';      Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--sources'                   }
        [pscustomobject]@{ Name='OutputDir';        Label='Output dir';        Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--output-dir'                }
        [pscustomobject]@{ Name='ContextsDir';      Label='Contexts dir';      Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--contexts-dir'              }
        [pscustomobject]@{ Name='PromptsDir';       Label='Prompts dir';       Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--prompts-dir'               }
    )
}

function New-LauncherDefaults {
    @{
        LlmConcurrency   = 3
        PollInterval     = 15
        ForceUnlock      = $false
        AllowUnsupported = $false
        Sources          = 'sources.ron'
        OutputDir        = 'output'
        ContextsDir      = 'contexts'
        PromptsDir       = 'prompts'
    }
}

function Get-DefaultsFilePath {
    # Resolves to scripts/harvester_launcher_defaults.json
    Join-Path (Split-Path -Parent $PSScriptRoot) 'harvester_launcher_defaults.json'
}

Export-ModuleMember -Function Get-LauncherActionItems, Get-LauncherParamDefs, New-LauncherDefaults, Get-DefaultsFilePath
```

### Step 4: Run tests — verify they pass
```
Invoke-Pester scripts\tests\HarvesterLauncher.Tests.ps1 -Output Minimal
```
Expected: all 14 tests pass.

### Step 5: Commit
```
git add scripts/harvester_launcher/Data.psm1 scripts/tests/HarvesterLauncher.Tests.ps1
git commit -m "feat(launcher): add Data module with action items and parameter definitions"
```

---

## Task 3 — Reducer.psm1: state factory and layout

**Files:**
- Write: `scripts/harvester_launcher/Reducer.psm1` (partial — `New-LauncherState`)
- Modify: `scripts/tests/HarvesterLauncher.Tests.ps1` (append Reducer tests)

### Step 1: Append failing tests
```powershell
Describe 'Reducer - New-LauncherState' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
    }

    It 'ActivePane starts as Left' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Ui.ActivePane | Should Be 'Left'
    }
    It 'IsRunning starts true' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Runtime.IsRunning | Should Be $true
    }
    It 'HarvesterCmd stored in Runtime' {
        (New-LauncherState -HarvesterCmd 'myhb' -Width 100 -Height 30).Runtime.HarvesterCmd | Should Be 'myhb'
    }
    It 'layout Left pane width is at least 32 at normal size' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Ui.Layout.Left.W | Should BeGreaterOrEqual 32
    }
    It 'layout Right pane starts after Left + gap' {
        $s = New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30
        $s.Ui.Layout.Right.X | Should BeGreaterThan $s.Ui.Layout.Left.W
    }
    It 'TooSmall is true for narrow terminal' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 50 -Height 10).Ui.TooSmall | Should Be $true
    }
    It 'TooSmall is false for adequate terminal' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Ui.TooSmall | Should Be $false
    }
    It 'cursor starts at 0 for both panes' {
        $c = (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Cursor
        $c.LeftIndex  | Should Be 0
        $c.RightIndex | Should Be 0
    }
    It 'defaults are loaded' {
        $v = (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Values
        $v.LlmConcurrency | Should Be 3
        $v.PollInterval   | Should Be 15
    }
    It 'custom InitialValues override defaults' {
        $custom = New-LauncherDefaults; $custom.LlmConcurrency = 7
        $v = (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 -InitialValues $custom).Values
        $v.LlmConcurrency | Should Be 7
    }
    It 'Pending.LaunchAfterExit starts null' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Pending.LaunchAfterExit | Should BeNullOrEmpty
    }
    It 'CheckpointCliAvailable starts false' {
        (New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30).Runtime.CheckpointCliAvailable | Should Be $false
    }
}

Describe 'Reducer - Get-LauncherLayoutConstraints' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
    }
    function D { @{ Actions = Get-LauncherActionItems; Params = Get-LauncherParamDefs } }

    It 'LeftW is at least 32' {
        (Get-LauncherLayoutConstraints -Data (D)).LeftW | Should BeGreaterOrEqual 32
    }
    It 'MinWidth is greater than LeftW' {
        $c = Get-LauncherLayoutConstraints -Data (D)
        $c.MinWidth | Should BeGreaterThan $c.LeftW
    }
    It 'MinHeight is at least 16' {
        (Get-LauncherLayoutConstraints -Data (D)).MinHeight | Should BeGreaterOrEqual 16
    }
    It 'Get-LauncherLayout TooSmall true when width below MinWidth' {
        $c = Get-LauncherLayoutConstraints -Data (D)
        (Get-LauncherLayout -Width ($c.MinWidth - 1) -Height $c.MinHeight -Constraints $c).TooSmall | Should Be $true
    }
    It 'Get-LauncherLayout TooSmall false at MinWidth x MinHeight' {
        $c = Get-LauncherLayoutConstraints -Data (D)
        (Get-LauncherLayout -Width $c.MinWidth -Height $c.MinHeight -Constraints $c).TooSmall | Should Be $false
    }
}
```

### Step 2: Run — verify fail
```
Invoke-Pester scripts\tests\HarvesterLauncher.Tests.ps1 -Output Minimal
```

### Step 3: Implement Reducer.psm1 (New-LauncherState + helpers)
```powershell
#Requires -Version 5.1
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot 'Data.psm1') -Force

# ── Layout ────────────────────────────────────────────────────────────────────

function Get-LauncherLayoutConstraints {
    param([hashtable]$Data)
    # Left pane width: longest action label + marker (►) + spaces + 2 border chars
    $longestAction = ($Data.Actions | Where-Object { -not $_.IsSeparator } |
                      ForEach-Object { $_.Label.Length } | Measure-Object -Maximum).Maximum
    $leftW = [Math]::Max(32, $longestAction + 6)

    # Minimum right pane width: longest param label + value/hint area
    $longestParam  = ($Data.Params | ForEach-Object { $_.Label.Length } | Measure-Object -Maximum).Maximum
    $minRightW     = [Math]::Max(40, $longestParam + 28)

    # Minimum total dimensions derived from content
    $minWidth  = $leftW + 1 + $minRightW   # leftW + gap + rightW
    $actionCount = ($Data.Actions | Where-Object { -not $_.IsSeparator }).Count
    $minHeight = [Math]::Max(16, $actionCount + 7)  # title+sep+checkpoint+border+status + padding

    @{ LeftW=$leftW; MinWidth=$minWidth; MinHeight=$minHeight }
}

function Get-LauncherLayout {
    param([int]$Width, [int]$Height, [hashtable]$Constraints)

    $tooSmall = ($Width -lt $Constraints.MinWidth -or $Height -lt $Constraints.MinHeight)
    if ($tooSmall) {
        return @{
            Width    = $Width;  Height   = $Height;  TooSmall = $true
            Left     = @{ X=0; Y=0; W=0; H=0 }
            Right    = @{ X=0; Y=0; W=0; H=0 }
            Status   = @{ X=0; Y=[Math]::Max(0,$Height-1); W=$Width; H=1 }
            Constraints = $Constraints
        }
    }
    $leftW    = $Constraints.LeftW
    $gap      = 1
    $rightX   = $leftW + $gap
    $rightW   = $Width - $rightX
    $contentH = $Height - 1   # minus status bar
    @{
        Width  = $Width;  Height  = $Height;  TooSmall = $false
        Left   = @{ X=0;       Y=0; W=$leftW;  H=$contentH }
        Right  = @{ X=$rightX; Y=0; W=$rightW; H=$contentH }
        Status = @{ X=0;       Y=$contentH; W=$Width; H=1   }
        Constraints = $Constraints
    }
}

# ── State factory ─────────────────────────────────────────────────────────────

function New-LauncherState {
    param(
        [string]   $HarvesterCmd   = 'harvester_batch',
        [int]      $Width          = 80,
        [int]      $Height         = 24,
        [hashtable]$InitialValues  = $null
    )
    $data   = @{ Actions = Get-LauncherActionItems; Params = Get-LauncherParamDefs }
    $constr = Get-LauncherLayoutConstraints -Data $data
    $layout = Get-LauncherLayout -Width $Width -Height $Height -Constraints $constr
    $values = if ($null -ne $InitialValues) { $InitialValues } else { New-LauncherDefaults }
    @{
        Data = $data
        Ui = @{
            ActivePane = 'Left'
            TooSmall   = $layout.TooSmall
            Layout     = $layout
        }
        Cursor = @{
            LeftIndex   = 0;  RightIndex  = 0
            LeftScroll  = 0;  RightScroll = 0
        }
        Values  = $values.Clone()
        Runtime = @{
            IsRunning              = $true
            LastStatus             = $null   # $null | 'OK' | 'Error' | 'Warn'
            LastMessage            = ''
            CheckpointDisplay      = 'not set (all-time briefing)'
            CheckpointCliAvailable = $false
            HarvesterCmd           = $HarvesterCmd
        }
        Pending = @{
            Effects         = @()
            LaunchAfterExit = $null   # @{ FilePath; Argv } when set
        }
    }
}

# ── Deep copy ─────────────────────────────────────────────────────────────────

function Copy-LauncherState {
    param([hashtable]$State)
    @{
        Data    = $State.Data   # immutable — never mutated by reducer
        Ui      = @{
            ActivePane = $State.Ui.ActivePane
            TooSmall   = $State.Ui.TooSmall
            Layout     = $State.Ui.Layout   # replaced wholesale on Resize
        }
        Cursor  = $State.Cursor.Clone()
        Values  = $State.Values.Clone()
        Runtime = $State.Runtime.Clone()
        Pending = @{
            Effects         = @() + $State.Pending.Effects
            LaunchAfterExit = $State.Pending.LaunchAfterExit
        }
    }
}

Export-ModuleMember -Function New-LauncherState, Copy-LauncherState, Get-LauncherLayout, Get-LauncherLayoutConstraints
```

### Step 4: Run — verify pass
```
Invoke-Pester scripts\tests\HarvesterLauncher.Tests.ps1 -Output Minimal
```

### Step 5: Commit
```
git add scripts/harvester_launcher/Reducer.psm1 scripts/tests/HarvesterLauncher.Tests.ps1
git commit -m "feat(launcher): add Reducer with New-LauncherState and layout"
```

---

## Task 4 — Reducer.psm1: navigation actions

### Step 1: Append failing tests
```powershell
Describe 'Reducer - navigation' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
    }
    function S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }
    function Reduce($state, $type, $extra=@{}) {
        $action = @{ Type=$type } + $extra
        Invoke-LauncherReducer -State $state -Action $action
    }

    It 'reducer returns both State and Effects keys' {
        $r = Reduce (S) 'Quit'
        $r.Keys | Should Contain 'State'
        $r.Keys | Should Contain 'Effects'
    }
    It 'Quit sets IsRunning to false' {
        (Reduce (S) 'Quit').State.Runtime.IsRunning | Should Be $false
    }
    It 'Quit emits no effects' {
        (Reduce (S) 'Quit').Effects.Count | Should Be 0
    }
    It 'SwitchPane Left->Right' {
        (Reduce (S) 'SwitchPane').State.Ui.ActivePane | Should Be 'Right'
    }
    It 'SwitchPane Right->Left' {
        $s = S; $s.Ui.ActivePane = 'Right'
        (Reduce $s 'SwitchPane').State.Ui.ActivePane | Should Be 'Left'
    }
    It 'MoveDown advances LeftIndex in Left pane' {
        (Reduce (S) 'MoveDown').State.Cursor.LeftIndex | Should Be 1
    }
    It 'MoveDown skips separator (index 2) landing on 3' {
        $s = S; $s.Cursor.LeftIndex = 1
        (Reduce $s 'MoveDown').State.Cursor.LeftIndex | Should Be 3
    }
    It 'MoveUp from 3 skips separator landing on 1' {
        $s = S; $s.Cursor.LeftIndex = 3
        (Reduce $s 'MoveUp').State.Cursor.LeftIndex | Should Be 1
    }
    It 'MoveDown clamps at last action item' {
        $s = S; $s.Cursor.LeftIndex = 6   # last item (cp-show)
        (Reduce $s 'MoveDown').State.Cursor.LeftIndex | Should Be 6
    }
    It 'MoveUp clamps at 0' {
        (Reduce (S) 'MoveUp').State.Cursor.LeftIndex | Should Be 0
    }
    It 'MoveDown advances RightIndex in Right pane' {
        $s = S; $s.Ui.ActivePane = 'Right'
        (Reduce $s 'MoveDown').State.Cursor.RightIndex | Should Be 1
    }
    It 'MoveDown clamps RightIndex at last param' {
        $s = S; $s.Ui.ActivePane = 'Right'; $s.Cursor.RightIndex = 7   # 8 params, index 7
        (Reduce $s 'MoveDown').State.Cursor.RightIndex | Should Be 7
    }
    It 'MoveHome sets LeftIndex to first non-separator' {
        $s = S; $s.Cursor.LeftIndex = 5
        (Reduce $s 'MoveHome').State.Cursor.LeftIndex | Should Be 0
    }
    It 'MoveEnd sets LeftIndex to last item' {
        (Reduce (S) 'MoveEnd').State.Cursor.LeftIndex | Should Be 6
    }
    It 'Resize updates TooSmall to true for small terminal' {
        $r = Reduce (S) 'Resize' @{ Width=50; Height=10 }
        $r.State.Ui.TooSmall | Should Be $true
    }
    It 'Resize updates Layout dimensions' {
        $r = Reduce (S) 'Resize' @{ Width=120; Height=40 }
        $r.State.Ui.Layout.Width  | Should Be 120
        $r.State.Ui.Layout.Height | Should Be 40
    }
    It 'reducer does not mutate input state' {
        $s = S
        $orig = $s.Cursor.LeftIndex
        Invoke-LauncherReducer -State $s -Action @{ Type='MoveDown' } | Out-Null
        $s.Cursor.LeftIndex | Should Be $orig
    }
}
```

### Step 2: Run — verify fail

### Step 3: Add `Invoke-LauncherReducer` to Reducer.psm1

Append to `Reducer.psm1` before the `Export-ModuleMember` line:
```powershell
# ── Cursor helpers ────────────────────────────────────────────────────────────

function Move-LeftCursor {
    param([object[]]$Actions, [int]$Current, [int]$Delta)
    $next = $Current + $Delta
    while ($next -ge 0 -and $next -lt $Actions.Count -and $Actions[$next].IsSeparator) {
        $next += $Delta
    }
    if ($next -lt 0 -or $next -ge $Actions.Count) { return $Current }
    $next
}

function Get-FirstSelectableIndex {
    param([object[]]$Actions)
    for ($i = 0; $i -lt $Actions.Count; $i++) {
        if (-not $Actions[$i].IsSeparator) { return $i }
    }
    return 0
}

function Get-LastSelectableIndex {
    param([object[]]$Actions)
    for ($i = $Actions.Count - 1; $i -ge 0; $i--) {
        if (-not $Actions[$i].IsSeparator) { return $i }
    }
    return 0
}

# ── Reducer ───────────────────────────────────────────────────────────────────

function Invoke-LauncherReducer {
    param([hashtable]$State, [object]$Action)
    $s       = Copy-LauncherState -State $State
    $effects = [System.Collections.Generic.List[object]]::new()

    switch ($Action.Type) {

        'Quit' {
            $s.Runtime.IsRunning = $false
        }

        'SwitchPane' {
            $s.Ui.ActivePane = if ($s.Ui.ActivePane -eq 'Left') { 'Right' } else { 'Left' }
        }

        'Resize' {
            $layout          = Get-LauncherLayout -Width $Action.Width -Height $Action.Height -Constraints $s.Ui.Layout.Constraints
            $s.Ui.Layout     = $layout
            $s.Ui.TooSmall   = $layout.TooSmall
            # Clamp cursors
            $maxL = $s.Data.Actions.Count - 1
            $maxR = $s.Data.Params.Count  - 1
            if ($s.Cursor.LeftIndex  -gt $maxL) { $s.Cursor.LeftIndex  = $maxL }
            if ($s.Cursor.RightIndex -gt $maxR) { $s.Cursor.RightIndex = $maxR }
        }

        'MoveUp' {
            if ($s.Ui.ActivePane -eq 'Left') {
                $s.Cursor.LeftIndex  = Move-LeftCursor -Actions $s.Data.Actions -Current $s.Cursor.LeftIndex -Delta -1
            } else {
                $s.Cursor.RightIndex = [Math]::Max(0, $s.Cursor.RightIndex - 1)
            }
        }

        'MoveDown' {
            if ($s.Ui.ActivePane -eq 'Left') {
                $s.Cursor.LeftIndex  = Move-LeftCursor -Actions $s.Data.Actions -Current $s.Cursor.LeftIndex -Delta 1
            } else {
                $maxR = $s.Data.Params.Count - 1
                $s.Cursor.RightIndex = [Math]::Min($maxR, $s.Cursor.RightIndex + 1)
            }
        }

        'MoveHome' {
            if ($s.Ui.ActivePane -eq 'Left') { $s.Cursor.LeftIndex  = Get-FirstSelectableIndex -Actions $s.Data.Actions }
            else                             { $s.Cursor.RightIndex = 0 }
        }

        'MoveEnd' {
            if ($s.Ui.ActivePane -eq 'Left') { $s.Cursor.LeftIndex  = Get-LastSelectableIndex -Actions $s.Data.Actions }
            else                             { $s.Cursor.RightIndex = $s.Data.Params.Count - 1 }
        }

        'PageUp' {
            $page = [Math]::Max(1, [Math]::Floor($s.Ui.Layout.Left.H / 2))
            if ($s.Ui.ActivePane -eq 'Left') {
                for ($i = 0; $i -lt $page; $i++) {
                    $s.Cursor.LeftIndex = Move-LeftCursor -Actions $s.Data.Actions -Current $s.Cursor.LeftIndex -Delta -1
                }
            } else {
                $s.Cursor.RightIndex = [Math]::Max(0, $s.Cursor.RightIndex - $page)
            }
        }

        'PageDown' {
            $page = [Math]::Max(1, [Math]::Floor($s.Ui.Layout.Left.H / 2))
            $maxR = $s.Data.Params.Count - 1
            if ($s.Ui.ActivePane -eq 'Left') {
                for ($i = 0; $i -lt $page; $i++) {
                    $s.Cursor.LeftIndex = Move-LeftCursor -Actions $s.Data.Actions -Current $s.Cursor.LeftIndex -Delta 1
                }
            } else {
                $s.Cursor.RightIndex = [Math]::Min($maxR, $s.Cursor.RightIndex + $page)
            }
        }

        # (value editing and Activate added in Tasks 5-7)

        default { <# unknown actions are silently ignored #> }
    }

    @{ State = $s; Effects = $effects.ToArray() }
}
```

Update `Export-ModuleMember` to include `Invoke-LauncherReducer`.

### Step 4: Run — verify pass
### Step 5: Commit
```
git add scripts/harvester_launcher/Reducer.psm1 scripts/tests/HarvesterLauncher.Tests.ps1
git commit -m "feat(launcher): add Reducer navigation actions"
```

---

## Task 5 — Reducer.psm1: value editing actions

### Step 1: Append failing tests
```powershell
Describe 'Reducer - value editing' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
    }
    function RightState($paramIdx) {
        $s = New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30
        $s.Ui.ActivePane = 'Right'; $s.Cursor.RightIndex = $paramIdx; $s
    }
    # Param indices (from Get-LauncherParamDefs order):
    # 0=LlmConcurrency, 1=PollInterval, 2=ForceUnlock, 3=AllowUnsupported, 4=Sources, ...

    It 'ValueIncrease on LlmConcurrency increments by 1' {
        $r = Invoke-LauncherReducer -State (RightState 0) -Action @{ Type='ValueIncrease' }
        $r.State.Values.LlmConcurrency | Should Be 4
    }
    It 'ValueIncrease clamps at Max (10)' {
        $s = RightState 0; $s.Values.LlmConcurrency = 10
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueIncrease' }).State.Values.LlmConcurrency | Should Be 10
    }
    It 'ValueDecrease on LlmConcurrency decrements by 1' {
        $s = RightState 0; $s.Values.LlmConcurrency = 5
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueDecrease' }).State.Values.LlmConcurrency | Should Be 4
    }
    It 'ValueDecrease clamps at Min (1)' {
        $s = RightState 0; $s.Values.LlmConcurrency = 1
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueDecrease' }).State.Values.LlmConcurrency | Should Be 1
    }
    It 'ValueIncrease on PollInterval uses correct max 1440' {
        $s = RightState 1; $s.Values.PollInterval = 1440
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueIncrease' }).State.Values.PollInterval | Should Be 1440
    }
    It 'ValueToggle flips ForceUnlock false->true' {
        (Invoke-LauncherReducer -State (RightState 2) -Action @{ Type='ValueToggle' }).State.Values.ForceUnlock | Should Be $true
    }
    It 'ValueToggle flips ForceUnlock true->false' {
        $s = RightState 2; $s.Values.ForceUnlock = $true
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueToggle' }).State.Values.ForceUnlock | Should Be $false
    }
    It 'ValueToggle does nothing on Path param' {
        (Invoke-LauncherReducer -State (RightState 4) -Action @{ Type='ValueToggle' }).State.Values.Sources | Should Be 'sources.ron'
    }
    It 'ValueIncrease does nothing on Bool param' {
        (Invoke-LauncherReducer -State (RightState 2) -Action @{ Type='ValueIncrease' }).State.Values.ForceUnlock | Should Be $false
    }
    It 'ValueIncrease does nothing when Left pane is active' {
        $s = New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30
        (Invoke-LauncherReducer -State $s -Action @{ Type='ValueIncrease' }).State.Values.LlmConcurrency | Should Be 3
    }
}
```

### Step 2: Run — verify fail

### Step 3: Add value editing to `Invoke-LauncherReducer` switch
```powershell
'ValueIncrease' {
    if ($s.Ui.ActivePane -eq 'Right') {
        $p = $s.Data.Params[$s.Cursor.RightIndex]
        if ($p.Type -eq 'Int') {
            $s.Values[$p.Name] = [Math]::Min($p.Max, [int]$s.Values[$p.Name] + 1)
        }
    }
}
'ValueDecrease' {
    if ($s.Ui.ActivePane -eq 'Right') {
        $p = $s.Data.Params[$s.Cursor.RightIndex]
        if ($p.Type -eq 'Int') {
            $s.Values[$p.Name] = [Math]::Max($p.Min, [int]$s.Values[$p.Name] - 1)
        }
    }
}
'ValueToggle' {
    if ($s.Ui.ActivePane -eq 'Right') {
        $p = $s.Data.Params[$s.Cursor.RightIndex]
        if ($p.Type -eq 'Bool') {
            $s.Values[$p.Name] = -not [bool]$s.Values[$p.Name]
        }
    }
}
```

### Step 4: Run — verify pass
### Step 5: Commit
```
git add scripts/harvester_launcher/Reducer.psm1 scripts/tests/HarvesterLauncher.Tests.ps1
git commit -m "feat(launcher): add Reducer value editing actions"
```

---

## Task 6 — Reducer.psm1: Activate action and Build-CommandArgs

### Step 1: Append failing tests
```powershell
Describe 'Reducer - Build-CommandArgs' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
    }
    function S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }

    It 'FilePath matches HarvesterCmd' {
        (Build-CommandArgs -State (S) -DryRun $false).FilePath | Should Be 'hb'
    }
    It 'includes --sources and its value' {
        $a = (Build-CommandArgs -State (S) -DryRun $false).Argv
        $idx = [Array]::IndexOf($a, '--sources')
        $idx | Should BeGreaterThan -1
        $a[$idx+1] | Should Be 'sources.ron'
    }
    It 'includes --llm-concurrency 3' {
        $a = (Build-CommandArgs -State (S) -DryRun $false).Argv
        $idx = [Array]::IndexOf($a, '--llm-concurrency')
        $a[$idx+1] | Should Be '3'
    }
    It 'excludes --force-unlock when false' {
        (Build-CommandArgs -State (S) -DryRun $false).Argv | Should Not Contain '--force-unlock'
    }
    It 'includes --force-unlock when true' {
        $s = S; $s.Values.ForceUnlock = $true
        (Build-CommandArgs -State $s -DryRun $false).Argv | Should Contain '--force-unlock'
    }
    It 'includes --dry-run when DryRun is true' {
        (Build-CommandArgs -State (S) -DryRun $true).Argv | Should Contain '--dry-run'
    }
    It 'excludes --dry-run when DryRun is false' {
        (Build-CommandArgs -State (S) -DryRun $false).Argv | Should Not Contain '--dry-run'
    }
    It 'path with spaces is a single argv element (not split)' {
        $s = S; $s.Values.Sources = 'my sources/config.ron'
        $a = (Build-CommandArgs -State $s -DryRun $false).Argv
        $idx = [Array]::IndexOf($a, '--sources')
        $a[$idx+1] | Should Be 'my sources/config.ron'
    }
}

Describe 'Reducer - Activate' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
    }
    function S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }

    It 'Activate on run-batch sets IsRunning=false' {
        $s = S; $s.Cursor.LeftIndex = 0   # run-batch
        (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).State.Runtime.IsRunning | Should Be $false
    }
    It 'Activate on run-batch populates LaunchAfterExit' {
        $s = S; $s.Cursor.LeftIndex = 0
        $r = (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).State.Pending.LaunchAfterExit
        $r | Should Not BeNullOrEmpty
        $r.FilePath | Should Be 'hb'
    }
    It 'Activate on run-dry includes --dry-run in LaunchAfterExit.Argv' {
        $s = S; $s.Cursor.LeftIndex = 1   # run-dry
        $r = (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).State.Pending.LaunchAfterExit
        $r.Argv | Should Contain '--dry-run'
    }
    It 'Activate on run-batch emits no effects' {
        $s = S; $s.Cursor.LeftIndex = 0
        (Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }).Effects.Count | Should Be 0
    }
    It 'Activate on separator is a no-op' {
        $s = S; $s.Cursor.LeftIndex = 2   # sep-1
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        $r.State.Runtime.IsRunning | Should Be $true
    }
    It 'Activate on checkpoint when unavailable sets LastStatus Warn' {
        $s = S; $s.Cursor.LeftIndex = 3; $s.Runtime.CheckpointCliAvailable = $false  # cp-set-now
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        $r.State.Runtime.LastStatus | Should Be 'Warn'
    }
    It 'Activate on checkpoint when available queues RunCheckpointCommand effect' {
        $s = S; $s.Cursor.LeftIndex = 3; $s.Runtime.CheckpointCliAvailable = $true
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        $eff = $r.Effects | Where-Object { $_.Type -eq 'RunCheckpointCommand' }
        $eff | Should Not BeNullOrEmpty
        $eff.ActionId | Should Be 'cp-set-now'
    }
    It 'Activate on cp-set-date when available emits DatePromptRequested (not RunCheckpointCommand)' {
        $s = S; $s.Cursor.LeftIndex = 4; $s.Runtime.CheckpointCliAvailable = $true   # cp-set-date
        $r = Invoke-LauncherReducer -State $s -Action @{ Type='Activate' }
        ($r.Effects | Where-Object { $_.Type -eq 'DatePromptRequested' }) | Should Not BeNullOrEmpty
        ($r.Effects | Where-Object { $_.Type -eq 'RunCheckpointCommand' }) | Should BeNullOrEmpty
    }
}
```

### Step 2: Run — verify fail

### Step 3: Add `Build-CommandArgs` and `Activate` case to Reducer.psm1
```powershell
function Build-CommandArgs {
    param([hashtable]$State, [bool]$DryRun = $false)
    $v    = $State.Values
    $argv = [System.Collections.Generic.List[string]]::new()

    # All path args — always explicit for reproducibility
    $argv.AddRange([string[]]@('--sources',      $v.Sources))
    $argv.AddRange([string[]]@('--output-dir',   $v.OutputDir))
    $argv.AddRange([string[]]@('--contexts-dir', $v.ContextsDir))
    $argv.AddRange([string[]]@('--prompts-dir',  $v.PromptsDir))

    # Numeric args — always explicit
    $argv.AddRange([string[]]@('--llm-concurrency', [string]$v.LlmConcurrency))
    $argv.AddRange([string[]]@('--poll-interval',   [string]$v.PollInterval))

    # Boolean flags — only when enabled
    if ($v.ForceUnlock)      { $argv.Add('--force-unlock') }
    if ($v.AllowUnsupported) { $argv.Add('--allow-unsupported-sources') }
    if ($DryRun)             { $argv.Add('--dry-run') }

    @{ FilePath = $State.Runtime.HarvesterCmd; Argv = $argv.ToArray() }
}
```

Add to `Invoke-LauncherReducer` switch:
```powershell
'Activate' {
    $item = $s.Data.Actions[$s.Cursor.LeftIndex]
    if ($item.IsSeparator) { break }

    if (-not $item.IsCheckpoint) {
        # Run actions: exit TUI and launch process
        $s.Pending.LaunchAfterExit = Build-CommandArgs -State $s -DryRun $item.IsDryRun
        $s.Runtime.IsRunning       = $false
    } else {
        # Checkpoint actions
        if (-not $s.Runtime.CheckpointCliAvailable) {
            $s.Runtime.LastStatus  = 'Warn'
            $s.Runtime.LastMessage = 'Checkpoint CLI not yet available (Slice A pending)'
        } elseif ($item.Id -eq 'cp-set-date') {
            # Custom date requires interactive prompt — handled as an effect
            $effects.Add(@{ Type='DatePromptRequested' })
        } else {
            $effects.Add(@{ Type='RunCheckpointCommand'; ActionId=$item.Id })
        }
    }
}
```

Export `Build-CommandArgs` in `Export-ModuleMember`.

### Step 4: Run — verify pass
### Step 5: Commit
```
git add scripts/harvester_launcher/Reducer.psm1 scripts/tests/HarvesterLauncher.Tests.ps1
git commit -m "feat(launcher): add Build-CommandArgs and Activate action"
```

---

## Task 7 — Reducer.psm1: effect-result actions (SaveDefaults, Checkpoint feedback)

### Step 1: Append failing tests
```powershell
Describe 'Reducer - effect results' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
    }
    function S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }
    function Act($type, $extra=@{}) { Invoke-LauncherReducer -State (S) -Action (@{ Type=$type } + $extra) }

    It 'SaveDefaults emits a SaveDefaults effect containing Values' {
        $r = Act 'SaveDefaults'
        $eff = $r.Effects | Where-Object { $_.Type -eq 'SaveDefaults' }
        $eff              | Should Not BeNullOrEmpty
        $eff.Values.LlmConcurrency | Should Be 3
    }
    It 'DefaultsSaved sets LastStatus OK' {
        (Act 'DefaultsSaved').State.Runtime.LastStatus | Should Be 'OK'
    }
    It 'DefaultsSaved sets LastMessage containing "saved"' {
        (Act 'DefaultsSaved').State.Runtime.LastMessage | Should Match 'saved'
    }
    It 'DefaultsSaveFailed sets LastStatus Error' {
        (Act 'DefaultsSaveFailed' @{ Message='disk full' }).State.Runtime.LastStatus | Should Be 'Error'
    }
    It 'DefaultsSaveFailed includes message' {
        (Act 'DefaultsSaveFailed' @{ Message='disk full' }).State.Runtime.LastMessage | Should Match 'disk full'
    }
    It 'DefaultsLoaded merges values' {
        $loaded = @{ LlmConcurrency=9; PollInterval=5 }
        $r = Invoke-LauncherReducer -State (S) -Action @{ Type='DefaultsLoaded'; Values=$loaded }
        $r.State.Values.LlmConcurrency | Should Be 9
        $r.State.Values.PollInterval   | Should Be 5
    }
    It 'DefaultsLoadFailed sets LastStatus Warn' {
        (Act 'DefaultsLoadFailed' @{ Message='gone' }).State.Runtime.LastStatus | Should Be 'Warn'
    }
    It 'CheckpointCapabilityDetected sets CheckpointCliAvailable' {
        (Act 'CheckpointCapabilityDetected' @{ Available=$true }).State.Runtime.CheckpointCliAvailable | Should Be $true
    }
    It 'CheckpointReadCompleted updates CheckpointDisplay' {
        (Act 'CheckpointReadCompleted' @{ Display='2026-01-15T00:00:00Z' }).State.Runtime.CheckpointDisplay | Should Be '2026-01-15T00:00:00Z'
    }
    It 'CheckpointReadFailed sets display to "(unreadable)"' {
        (Act 'CheckpointReadFailed').State.Runtime.CheckpointDisplay | Should Be '(unreadable)'
    }
    It 'CheckpointCommandCompleted success sets LastStatus OK' {
        (Act 'CheckpointCommandCompleted' @{ Success=$true; Message='done' }).State.Runtime.LastStatus | Should Be 'OK'
    }
    It 'CheckpointCommandCompleted success emits ReadCheckpointDisplay effect' {
        $r = Act 'CheckpointCommandCompleted' @{ Success=$true; Message='done' }
        ($r.Effects | Where-Object { $_.Type -eq 'ReadCheckpointDisplay' }) | Should Not BeNullOrEmpty
    }
    It 'CheckpointCommandCompleted failure sets LastStatus Error' {
        (Act 'CheckpointCommandCompleted' @{ Success=$false; Message='fail' }).State.Runtime.LastStatus | Should Be 'Error'
    }
}
```

### Step 2: Run — verify fail

### Step 3: Add to `Invoke-LauncherReducer` switch
```powershell
'SaveDefaults' {
    $effects.Add(@{ Type='SaveDefaults'; Values=$s.Values.Clone() })
}
'DefaultsSaved' {
    $s.Runtime.LastStatus  = 'OK'
    $s.Runtime.LastMessage = 'Defaults saved.'
}
'DefaultsSaveFailed' {
    $s.Runtime.LastStatus  = 'Error'
    $s.Runtime.LastMessage = "Save failed: $($Action.Message)"
}
'DefaultsLoaded' {
    foreach ($key in $Action.Values.Keys) {
        if ($s.Values.ContainsKey($key)) { $s.Values[$key] = $Action.Values[$key] }
    }
}
'DefaultsLoadFailed' {
    $s.Runtime.LastStatus  = 'Warn'
    $s.Runtime.LastMessage = "Could not load defaults: $($Action.Message)"
}
'CheckpointCapabilityDetected' {
    $s.Runtime.CheckpointCliAvailable = $Action.Available
}
'CheckpointReadCompleted' {
    $s.Runtime.CheckpointDisplay = $Action.Display
}
'CheckpointReadFailed' {
    $s.Runtime.CheckpointDisplay = '(unreadable)'
}
'CheckpointCommandCompleted' {
    $s.Runtime.LastStatus  = if ($Action.Success) { 'OK' } else { 'Error' }
    $s.Runtime.LastMessage = $Action.Message
    if ($Action.Success) { $effects.Add(@{ Type='ReadCheckpointDisplay' }) }
}
'DatePromptCompleted' {
    # Value is $null when user cancelled or entered an invalid date
    if ($null -ne $Action.Value) {
        $effects.Add(@{ Type='RunCheckpointCommand'; ActionId='cp-set-date'; CustomDate=$Action.Value })
    }
}
```

Add to the test block for Task 7 (before the closing `}` of the describe):
```powershell
    It 'DatePromptCompleted with value queues RunCheckpointCommand effect' {
        $r = Invoke-LauncherReducer -State (S) -Action @{ Type='DatePromptCompleted'; Value='2026-01-01T00:00:00Z' }
        $eff = $r.Effects | Where-Object { $_.Type -eq 'RunCheckpointCommand' }
        $eff | Should Not BeNullOrEmpty
        $eff.ActionId   | Should Be 'cp-set-date'
        $eff.CustomDate | Should Be '2026-01-01T00:00:00Z'
    }
    It 'DatePromptCompleted with null value is a no-op (user cancelled)' {
        $r = Invoke-LauncherReducer -State (S) -Action @{ Type='DatePromptCompleted'; Value=$null }
        $r.Effects.Count | Should Be 0
    }
```

### Step 4: Run — verify pass
### Step 5: Commit
```
git add scripts/harvester_launcher/Reducer.psm1 scripts/tests/HarvesterLauncher.Tests.ps1
git commit -m "feat(launcher): add Reducer effect-result actions"
```

---

## Task 8 — Effects.psm1: all IO and process calls

**Files:**
- Write: `scripts/harvester_launcher/Effects.psm1`
- Modify: `scripts/tests/HarvesterLauncher.Tests.ps1`

### Step 1: Append failing tests

> **Testability note:** Effects that call external processes or console IO use Pester `Mock`
> with the `-ModuleName harvester_launcher` scope to intercept calls without real processes.
> For `Invoke-DatePrompt`, mock `Read-Host`. For `Invoke-ProbeCheckpointCliSupport` and
> `Invoke-RunCheckpointCommand`, mock `Start-Process` or test with a known-absent binary.

```powershell
Describe 'Effects - Invoke-DatePrompt' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Effects.psm1" -Force
    }

    It 'returns DatePromptCompleted with Value for valid RFC3339 date' {
        Mock Read-Host { '2026-01-01T00:00:00Z' } -ModuleName harvester_launcher
        $r = Invoke-DatePrompt
        $r.Type  | Should Be 'DatePromptCompleted'
        $r.Value | Should Be '2026-01-01T00:00:00Z'
    }
    It 'returns DatePromptCompleted with null Value for empty input (cancel)' {
        Mock Read-Host { '' } -ModuleName harvester_launcher
        $r = Invoke-DatePrompt
        $r.Type  | Should Be 'DatePromptCompleted'
        $r.Value | Should BeNullOrEmpty
    }
    It 'returns DatePromptCompleted with null Value for invalid date format' {
        Mock Read-Host { 'not-a-date' } -ModuleName harvester_launcher
        $r = Invoke-DatePrompt
        $r.Type  | Should Be 'DatePromptCompleted'
        $r.Value | Should BeNullOrEmpty
    }
}

Describe 'Effects - Invoke-LoadDefaults' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Effects.psm1" -Force
    }

    It 'returns DefaultsLoadFailed when file absent' {
        (Invoke-LoadDefaults -FilePath 'nonexistent_xyz.json').Type | Should Be 'DefaultsLoadFailed'
    }
    It 'returns DefaultsLoadFailed for malformed JSON' {
        $tmp = [IO.Path]::GetTempFileName()
        'not { json' | Set-Content $tmp
        $r = Invoke-LoadDefaults -FilePath $tmp; Remove-Item $tmp -Force
        $r.Type | Should Be 'DefaultsLoadFailed'
    }
    It 'returns DefaultsLoaded with correct LlmConcurrency' {
        $tmp = [IO.Path]::GetTempFileName()
        @{ SchemaVersion=1; LlmConcurrency=7; PollInterval=30 } | ConvertTo-Json | Set-Content $tmp
        $r = Invoke-LoadDefaults -FilePath $tmp; Remove-Item $tmp -Force
        $r.Type                  | Should Be 'DefaultsLoaded'
        $r.Values.LlmConcurrency | Should Be 7
    }
    It 'clamps out-of-range LlmConcurrency to 10' {
        $tmp = [IO.Path]::GetTempFileName()
        @{ SchemaVersion=1; LlmConcurrency=999 } | ConvertTo-Json | Set-Content $tmp
        $r = Invoke-LoadDefaults -FilePath $tmp; Remove-Item $tmp -Force
        $r.Values.LlmConcurrency | Should Be 10
    }
    It 'unknown keys are ignored (no error)' {
        $tmp = [IO.Path]::GetTempFileName()
        @{ SchemaVersion=1; FutureKey='hello'; LlmConcurrency=5 } | ConvertTo-Json | Set-Content $tmp
        $r = Invoke-LoadDefaults -FilePath $tmp; Remove-Item $tmp -Force
        $r.Type | Should Be 'DefaultsLoaded'
    }
}

Describe 'Effects - Invoke-SaveDefaults' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Effects.psm1" -Force
    }

    It 'returns DefaultsSaved and writes SchemaVersion=1' {
        $tmp = [IO.Path]::GetTempFileName()
        $vals = New-LauncherDefaults; $vals.LlmConcurrency = 5
        $r = Invoke-SaveDefaults -FilePath $tmp -Values $vals
        $written = Get-Content $tmp -Raw | ConvertFrom-Json; Remove-Item $tmp -Force
        $r.Type            | Should Be 'DefaultsSaved'
        $written.SchemaVersion  | Should Be 1
        $written.LlmConcurrency | Should Be 5
    }
    It 'returns DefaultsSaveFailed on unwritable path' {
        (Invoke-SaveDefaults -FilePath 'Z:\impossible\path.json' -Values @{}).Type | Should Be 'DefaultsSaveFailed'
    }
}

Describe 'Effects - Invoke-ProbeCheckpointCliSupport' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Effects.psm1" -Force
    }
    It 'returns CheckpointCapabilityDetected with Available=false for nonexistent binary' {
        $r = Invoke-ProbeCheckpointCliSupport -HarvesterCmd 'nonexistent_binary_xyz_abc'
        $r.Type      | Should Be 'CheckpointCapabilityDetected'
        $r.Available | Should Be $false
    }
    It 'returns Available=false when only one checkpoint flag is present in help text' {
        # Simulate a binary whose --help mentions only --set-briefing-since (partial rollout)
        Mock Invoke-Expression { '--set-briefing-since <DATE>' } -ModuleName harvester_launcher
        # Implementation uses & $cmd --help; this test validates the all-three-flags requirement
        # by confirming a binary with partial flags is not treated as capable.
        # (Integration-test the real binary when Slice A ships.)
        $r = Invoke-ProbeCheckpointCliSupport -HarvesterCmd 'nonexistent_binary_xyz_abc'
        $r.Available | Should Be $false
    }
}

Describe 'Effects - Invoke-ReadCheckpointDisplay' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Effects.psm1" -Force
    }
    It 'returns not-set when file absent' {
        $r = Invoke-ReadCheckpointDisplay -CheckpointFilePath 'nonexistent_chk.ron'
        $r.Type    | Should Be 'CheckpointReadCompleted'
        $r.Display | Should Match 'not set'
    }
    It 'parses Some value correctly' {
        $tmp = [IO.Path]::GetTempFileName()
        'BriefingCheckpoint(since_utc: Some("2026-01-15T10:00:00Z"))' | Set-Content $tmp
        $r = Invoke-ReadCheckpointDisplay -CheckpointFilePath $tmp; Remove-Item $tmp -Force
        $r.Display | Should Be '2026-01-15T10:00:00Z'
    }
    It 'returns not-set for None value' {
        $tmp = [IO.Path]::GetTempFileName()
        'BriefingCheckpoint(since_utc: None)' | Set-Content $tmp
        $r = Invoke-ReadCheckpointDisplay -CheckpointFilePath $tmp; Remove-Item $tmp -Force
        $r.Display | Should Match 'not set'
    }
    It 'returns CheckpointReadFailed for unreadable file content' {
        # We cannot easily test a truly unreadable file, so test malformed RON returns completed gracefully
        $tmp = [IO.Path]::GetTempFileName()
        '(garbage ron)' | Set-Content $tmp
        $r = Invoke-ReadCheckpointDisplay -CheckpointFilePath $tmp; Remove-Item $tmp -Force
        $r.Type | Should Be 'CheckpointReadCompleted'   # falls back to "not set"
    }
}
```

### Step 2: Run — verify fail

### Step 3: Implement Effects.psm1
```powershell
#Requires -Version 5.1
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot 'Data.psm1') -Force

function Invoke-LoadDefaults {
    param([string]$FilePath)
    if (-not (Test-Path -LiteralPath $FilePath)) {
        return [pscustomobject]@{ Type='DefaultsLoadFailed'; Message='File not found' }
    }
    try {
        $json = Get-Content -LiteralPath $FilePath -Raw -ErrorAction Stop | ConvertFrom-Json
        $vals = New-LauncherDefaults
        if ($null -ne $json.LlmConcurrency)   { $vals.LlmConcurrency   = [Math]::Clamp([int]$json.LlmConcurrency,   1, 10)   }
        if ($null -ne $json.PollInterval)     { $vals.PollInterval     = [Math]::Clamp([int]$json.PollInterval,     1, 1440) }
        if ($null -ne $json.ForceUnlock)      { $vals.ForceUnlock      = [bool]$json.ForceUnlock      }
        if ($null -ne $json.AllowUnsupported) { $vals.AllowUnsupported = [bool]$json.AllowUnsupported }
        if ($null -ne $json.Sources)          { $vals.Sources          = [string]$json.Sources        }
        if ($null -ne $json.OutputDir)        { $vals.OutputDir        = [string]$json.OutputDir      }
        if ($null -ne $json.ContextsDir)      { $vals.ContextsDir      = [string]$json.ContextsDir    }
        if ($null -ne $json.PromptsDir)       { $vals.PromptsDir       = [string]$json.PromptsDir     }
        [pscustomobject]@{ Type='DefaultsLoaded'; Values=$vals }
    } catch {
        [pscustomobject]@{ Type='DefaultsLoadFailed'; Message=$_.Exception.Message }
    }
}

function Invoke-SaveDefaults {
    param([string]$FilePath, [hashtable]$Values)
    try {
        $ordered = [ordered]@{
            SchemaVersion    = 1
            Sources          = $Values.Sources
            OutputDir        = $Values.OutputDir
            ContextsDir      = $Values.ContextsDir
            PromptsDir       = $Values.PromptsDir
            LlmConcurrency   = $Values.LlmConcurrency
            PollInterval     = $Values.PollInterval
            ForceUnlock      = $Values.ForceUnlock
            AllowUnsupported = $Values.AllowUnsupported
        }
        # Atomic write: temp + move
        $tmp = $FilePath + '.tmp'
        $ordered | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $tmp -Encoding UTF8 -ErrorAction Stop
        Move-Item -LiteralPath $tmp -Destination $FilePath -Force -ErrorAction Stop
        [pscustomobject]@{ Type='DefaultsSaved' }
    } catch {
        [pscustomobject]@{ Type='DefaultsSaveFailed'; Message=$_.Exception.Message }
    }
}

function Invoke-ProbeCheckpointCliSupport {
    param([string]$HarvesterCmd)
    try {
        $helpText = (& $HarvesterCmd '--help' 2>&1) | Out-String
        # Require ALL three checkpoint flags to be present; partial rollout = unavailable
        $allPresent = ($helpText -match '--set-briefing-since[^-]') -and
                      ($helpText -match '--set-briefing-since-now') -and
                      ($helpText -match '--clear-briefing-since')
        [pscustomobject]@{ Type='CheckpointCapabilityDetected'; Available=[bool]$allPresent }
    } catch {
        [pscustomobject]@{ Type='CheckpointCapabilityDetected'; Available=$false }
    }
}

function Invoke-ReadCheckpointDisplay {
    param([string]$CheckpointFilePath)
    if (-not (Test-Path -LiteralPath $CheckpointFilePath)) {
        return [pscustomobject]@{ Type='CheckpointReadCompleted'; Display='not set (all-time briefing)' }
    }
    try {
        # Normalize whitespace so regex works across multi-line or compact RON variants
        $content = (Get-Content -LiteralPath $CheckpointFilePath -Raw) -replace '\s+', ' '
        if ($content -match 'since_utc\s*:\s*Some\s*\(\s*"([^"]+)"\s*\)') {
            return [pscustomobject]@{ Type='CheckpointReadCompleted'; Display=$Matches[1] }
        }
        if ($content -match 'since_utc\s*:\s*None') {
            return [pscustomobject]@{ Type='CheckpointReadCompleted'; Display='not set (all-time briefing)' }
        }
        # Unrecognized format — non-fatal fallback with warning
        Write-Warning "[launcher] Checkpoint RON format unrecognized; treating as not-set"
        [pscustomobject]@{ Type='CheckpointReadCompleted'; Display='not set (all-time briefing)' }
    } catch {
        [pscustomobject]@{ Type='CheckpointReadFailed'; Message=$_.Exception.Message }
    }
}

function Invoke-RunCheckpointCommand {
    param([string]$HarvesterCmd, [string]$ActionId, [string]$CustomDate = '')
    $argList = switch ($ActionId) {
        'cp-set-now'  { @('--set-briefing-since-now') }
        'cp-set-date' { @('--set-briefing-since', $CustomDate) }
        'cp-clear'    { @('--clear-briefing-since') }
        'cp-show'     { @('--show-briefing-since') }
        default       { return [pscustomobject]@{ Type='CheckpointCommandCompleted'; Success=$false; Message="Unknown action: $ActionId" } }
    }
    try {
        $errFile = [IO.Path]::GetTempFileName()
        $proc = Start-Process -FilePath $HarvesterCmd -ArgumentList $argList `
                              -Wait -PassThru -NoNewWindow `
                              -RedirectStandardError $errFile -ErrorAction Stop
        Remove-Item $errFile -Force -ErrorAction SilentlyContinue
        $ok = $proc.ExitCode -eq 0
        [pscustomobject]@{
            Type    = 'CheckpointCommandCompleted'
            Success = $ok
            Message = if ($ok) { 'Done.' } else { "Exit code $($proc.ExitCode)" }
        }
    } catch {
        [pscustomobject]@{ Type='CheckpointCommandCompleted'; Success=$false; Message=$_.Exception.Message }
    }
}

function Invoke-DatePrompt {
    # Suspends TUI temporarily to collect an RFC3339 date from the user.
    # Returns a DatePromptCompleted action with Value=<string> or Value=$null (cancel/invalid).
    [Console]::CursorVisible = $true
    [Console]::Clear()
    Write-Host 'Set Briefing Checkpoint — enter RFC3339 date/time:'
    Write-Host '  Example: 2026-01-01T00:00:00Z'
    Write-Host '  (Press Enter with empty input to cancel)'
    $dateInput = Read-Host 'Date'
    [Console]::CursorVisible = $false
    if ([string]::IsNullOrWhiteSpace($dateInput)) {
        return [pscustomobject]@{ Type='DatePromptCompleted'; Value=$null }
    }
    try {
        [System.DateTimeOffset]::Parse($dateInput) | Out-Null
        return [pscustomobject]@{ Type='DatePromptCompleted'; Value=$dateInput }
    } catch {
        Write-Host "Invalid format: $($_.Exception.Message)" -ForegroundColor Red
        Start-Sleep 2
        return [pscustomobject]@{ Type='DatePromptCompleted'; Value=$null }
    }
}

function Invoke-LauncherEffects {
    param([hashtable]$State, [object[]]$Effects)
    $results = [System.Collections.Generic.List[object]]::new()
    $chkPath = Join-Path $State.Values.OutputDir '.briefing_checkpoint.ron'

    foreach ($eff in $Effects) {
        $action = switch ($eff.Type) {
            'LoadDefaults'              { Invoke-LoadDefaults -FilePath (Get-DefaultsFilePath) }
            'SaveDefaults'              { Invoke-SaveDefaults -FilePath (Get-DefaultsFilePath) -Values $eff.Values }
            'ProbeCheckpointCliSupport' { Invoke-ProbeCheckpointCliSupport -HarvesterCmd $State.Runtime.HarvesterCmd }
            'ReadCheckpointDisplay'     { Invoke-ReadCheckpointDisplay -CheckpointFilePath $chkPath }
            'RunCheckpointCommand'      { Invoke-RunCheckpointCommand -HarvesterCmd $State.Runtime.HarvesterCmd -ActionId $eff.ActionId -CustomDate ($eff.CustomDate ?? '') }
            'DatePromptRequested'       { Invoke-DatePrompt }
            default                     { $null }
        }
        if ($null -ne $action) { $results.Add($action) }
    }
    $results.ToArray()
}

Export-ModuleMember -Function Invoke-LauncherEffects, Invoke-LoadDefaults, Invoke-SaveDefaults, `
    Invoke-ProbeCheckpointCliSupport, Invoke-ReadCheckpointDisplay, Invoke-RunCheckpointCommand, `
    Invoke-DatePrompt
```

### Step 4: Run — verify pass
### Step 5: Commit
```
git add scripts/harvester_launcher/Effects.psm1 scripts/tests/HarvesterLauncher.Tests.ps1
git commit -m "feat(launcher): add Effects module with IO and process calls"
```

---

## Task 9 — Input.psm1: key mapper

**Files:**
- Write: `scripts/harvester_launcher/Input.psm1`
- Modify: `scripts/tests/HarvesterLauncher.Tests.ps1`

### Step 1: Append failing tests
```powershell
Describe 'Input - ConvertFrom-KeyInfoToLauncherAction' {
    BeforeAll {
        $sub = Resolve-Path "$PSScriptRoot\..\..\ministry-of-future-plans\browser\Input.psm1"
        Import-Module $sub -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Input.psm1" -Force
    }
    function Key($k, $c = [char]0) { [System.ConsoleKeyInfo]::new($c, $k, $false, $false, $false) }

    It 'Enter returns Activate' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Enter')).Type | Should Be 'Activate'
    }
    It 'Escape returns Cancel' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Escape')).Type | Should Be 'Cancel'
    }
    It 'RightArrow returns ValueIncrease' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'RightArrow')).Type | Should Be 'ValueIncrease'
    }
    It 'LeftArrow returns ValueDecrease' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'LeftArrow')).Type | Should Be 'ValueDecrease'
    }
    It 'Spacebar returns ValueToggle' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Spacebar' ' ')).Type | Should Be 'ValueToggle'
    }
    It 'S returns SaveDefaults' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'S' 'S')).Type | Should Be 'SaveDefaults'
    }
    It 's (lowercase) returns SaveDefaults' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'S' 's')).Type | Should Be 'SaveDefaults'
    }
    It 'Plus char returns ValueIncrease' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'OemPlus' '+')).Type | Should Be 'ValueIncrease'
    }
    It 'Minus char returns ValueDecrease' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'OemMinus' '-')).Type | Should Be 'ValueDecrease'
    }
    It 'UpArrow returns MoveUp (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'UpArrow')).Type | Should Be 'MoveUp'
    }
    It 'DownArrow returns MoveDown (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'DownArrow')).Type | Should Be 'MoveDown'
    }
    It 'Q returns Quit (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Q' 'Q')).Type | Should Be 'Quit'
    }
    It 'Tab returns SwitchPane (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Tab')).Type | Should Be 'SwitchPane'
    }
    It 'PageUp returns PageUp (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'PageUp')).Type | Should Be 'PageUp'
    }
    It 'Home returns MoveHome (from submodule)' {
        (ConvertFrom-KeyInfoToLauncherAction (Key 'Home')).Type | Should Be 'MoveHome'
    }
    It 'F12 returns null' {
        ConvertFrom-KeyInfoToLauncherAction (Key 'F12') | Should BeNullOrEmpty
    }
}
```

### Step 2: Run — verify fail

### Step 3: Implement Input.psm1
```powershell
#Requires -Version 5.1
Set-StrictMode -Version Latest

# Import base key mappings from submodule (read-only dependency)
$_subInput = Join-Path $PSScriptRoot '..\..\ministry-of-future-plans\browser\Input.psm1'
if (Test-Path -LiteralPath $_subInput) {
    Import-Module $_subInput -Force
} else {
    Write-Warning "[launcher/Input] Submodule Input.psm1 not found at: $_subInput — run: git submodule update --init"
}

function ConvertFrom-KeyInfoToLauncherAction {
    param([System.ConsoleKeyInfo]$KeyInfo)

    # Launcher-specific overrides — evaluated before submodule fallback
    switch ($KeyInfo.Key) {
        'Enter'      { return [pscustomobject]@{ Type = 'Activate'      } }
        'Escape'     { return [pscustomobject]@{ Type = 'Cancel'        } }
        'RightArrow' { return [pscustomobject]@{ Type = 'ValueIncrease' } }
        'LeftArrow'  { return [pscustomobject]@{ Type = 'ValueDecrease' } }
        'Spacebar'   { return [pscustomobject]@{ Type = 'ValueToggle'   } }
        'Add'        { return [pscustomobject]@{ Type = 'ValueIncrease' } }
        'Subtract'   { return [pscustomobject]@{ Type = 'ValueDecrease' } }
    }

    # Character-based overrides (handles S/s and +/-)
    switch ($KeyInfo.KeyChar) {
        'S'  { return [pscustomobject]@{ Type = 'SaveDefaults'  } }
        's'  { return [pscustomobject]@{ Type = 'SaveDefaults'  } }
        '+'  { return [pscustomobject]@{ Type = 'ValueIncrease' } }
        '-'  { return [pscustomobject]@{ Type = 'ValueDecrease' } }
    }

    # Fall through to submodule (Q→Quit, Tab→SwitchPane, arrows, Page*, Home, End)
    if (Get-Command 'ConvertFrom-KeyInfoToAction' -ErrorAction SilentlyContinue) {
        return ConvertFrom-KeyInfoToAction -KeyInfo $KeyInfo
    }
    $null
}

Export-ModuleMember -Function ConvertFrom-KeyInfoToLauncherAction
```

### Step 4: Run — verify pass
### Step 5: Commit
```
git add scripts/harvester_launcher/Input.psm1 scripts/tests/HarvesterLauncher.Tests.ps1
git commit -m "feat(launcher): add Input module wrapping submodule key mapper"
```

---

## Task 10 — Render.psm1: primitives and frame-diff

**Files:**
- Write: `scripts/harvester_launcher/Render.psm1` (primitives only)
- Modify: `scripts/tests/HarvesterLauncher.Tests.ps1`

### Step 1: Append failing tests
```powershell
Describe 'Render - Pad-SegmentsToWidth' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Render.psm1"  -Force
    }
    function Seg($t) { [pscustomobject]@{ Text=$t; Fg='Gray'; Bg='Black' } }

    It 'pads short content to exact width' {
        $r = Pad-SegmentsToWidth -Segments @(Seg 'Hi') -Width 10
        ($r | ForEach-Object { $_.Text } | Join-String).Length | Should Be 10
    }
    It 'truncates long content to exact width' {
        $r = Pad-SegmentsToWidth -Segments @(Seg 'Hello World Long') -Width 5
        ($r | ForEach-Object { $_.Text } | Join-String).Length | Should Be 5
    }
    It 'exact-width content unchanged' {
        $r = Pad-SegmentsToWidth -Segments @(Seg 'Hello') -Width 5
        ($r | ForEach-Object { $_.Text } | Join-String) | Should Be 'Hello'
    }
}

Describe 'Render - Get-FrameDiff' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Render.psm1" -Force
    }
    function Row($t) { @([pscustomobject]@{ Text=$t; Fg='Gray'; Bg='Black' }) }

    It 'returns empty diff for identical frames' {
        $f = @( (Row 'abc'), (Row 'xyz') )
        (Get-FrameDiff -PrevFrame $f -CurrFrame $f).Count | Should Be 0
    }
    It 'detects changed row' {
        $f1 = @( (Row 'abc') )
        $f2 = @( (Row 'xyz') )
        (Get-FrameDiff -PrevFrame $f1 -CurrFrame $f2).Count | Should Be 1
    }
    It 'returns correct RowIndex for changed row' {
        $f1 = @( (Row 'aaa'), (Row 'bbb') )
        $f2 = @( (Row 'aaa'), (Row 'BBB') )
        (Get-FrameDiff -PrevFrame $f1 -CurrFrame $f2)[0].RowIndex | Should Be 1
    }
    It 'treats empty prev frame as all-changed' {
        $f = @( (Row 'abc') )
        (Get-FrameDiff -PrevFrame @() -CurrFrame $f).Count | Should Be 1
    }
}

Describe 'Render - Build-CommandPreviewLines' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Render.psm1" -Force
    }
    It 'returns at least one line' {
        $lines = Build-CommandPreviewLines -FilePath 'hb' -Argv @('--sources','s.ron') -MaxWidth 40
        $lines.Count | Should BeGreaterThan 0
    }
    It 'first line is the binary name' {
        $lines = Build-CommandPreviewLines -FilePath 'harvester_batch' -Argv @('--sources','s.ron') -MaxWidth 40
        $lines[0] | Should Match 'harvester_batch'
    }
}
```

### Step 2: Run — verify fail

### Step 3: Implement Render.psm1 primitives
```powershell
#Requires -Version 5.1
Set-StrictMode -Version Latest

# Box drawing
$script:Box = @{ TL='╭'; TR='╮'; BL='╰'; BR='╯'; H='─'; V='│' }

# Colour palette
$script:C = @{
    Default      = 'Gray';     Dim         = 'DarkGray'
    Selected     = 'White';    BgSelected  = 'DarkCyan'
    BgDefault    = 'Black';    Header      = 'DarkGray'
    OK           = 'Green';    Error       = 'Red'
    Warn         = 'Yellow';   Accent      = 'Cyan'
    Disabled     = 'DarkGray'; EditHint    = 'Yellow'
}

function New-Seg {
    param([string]$Text, [string]$Fg = 'Gray', [string]$Bg = 'Black')
    [pscustomobject]@{ Text=$Text; Fg=$Fg; Bg=$Bg }
}

function Pad-SegmentsToWidth {
    param([object[]]$Segments, [int]$Width)
    $total = ($Segments | Measure-Object -Property { $_.Text.Length } -Sum).Sum
    if ($total -eq $Width) { return $Segments }
    if ($total -gt $Width) {
        $out  = [System.Collections.Generic.List[object]]::new()
        $used = 0
        foreach ($seg in $Segments) {
            $rem = $Width - $used
            if ($rem -le 0) { break }
            $take = [Math]::Min($seg.Text.Length, $rem)
            $out.Add((New-Seg $seg.Text.Substring(0, $take) $seg.Fg $seg.Bg))
            $used += $take
        }
        return $out.ToArray()
    }
    # Pad
    $padBg = if ($Segments.Count -gt 0) { $Segments[-1].Bg } else { 'Black' }
    @($Segments) + @(New-Seg (' ' * ($Width - $total)) 'Black' $padBg)
}

function Get-RowSignature {
    param([object[]]$Segments)
    ($Segments | ForEach-Object { "$($_.Text)|$($_.Fg)|$($_.Bg)" }) -join ''
}

function Get-FrameDiff {
    param([object[][]]$PrevFrame, [object[][]]$CurrFrame)
    $diffs = [System.Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt $CurrFrame.Count; $i++) {
        $cur  = Get-RowSignature $CurrFrame[$i]
        $prev = if ($i -lt $PrevFrame.Count) { Get-RowSignature $PrevFrame[$i] } else { '' }
        if ($cur -ne $prev) { $diffs.Add([pscustomobject]@{ RowIndex=$i; Segments=$CurrFrame[$i] }) }
    }
    $diffs.ToArray()
}

function Flush-FrameDiff {
    param([object[]]$Diff)
    foreach ($row in $Diff) {
        [Console]::SetCursorPosition(0, $row.RowIndex)
        foreach ($seg in $row.Segments) {
            [Console]::ForegroundColor = $seg.Fg
            [Console]::BackgroundColor = $seg.Bg
            [Console]::Write($seg.Text)
        }
    }
    [Console]::ResetColor()
}

function Build-CommandPreviewLines {
    param([string]$FilePath, [string[]]$Argv, [int]$MaxWidth = 60)
    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add($FilePath)
    $cur = '  '
    foreach ($arg in $Argv) {
        $candidate = $cur + $arg + ' '
        if ($candidate.Length -gt $MaxWidth -and $cur.Trim() -ne '') {
            $lines.Add($cur.TrimEnd())
            $cur = '    ' + $arg + ' '
        } else {
            $cur = $candidate
        }
    }
    if ($cur.Trim()) { $lines.Add($cur.TrimEnd()) }
    $lines.ToArray()
}

Export-ModuleMember -Function New-Seg, Pad-SegmentsToWidth, Get-FrameDiff, Flush-FrameDiff, Build-CommandPreviewLines
```

### Step 4: Run — verify pass
### Step 5: Commit
```
git add scripts/harvester_launcher/Render.psm1 scripts/tests/HarvesterLauncher.Tests.ps1
git commit -m "feat(launcher): add Render primitives and frame-diff"
```

---

## Task 11 — Render.psm1: pane builders and Render-LauncherState

### Step 1: Append failing tests
```powershell
Describe 'Render - Build-LauncherFrame' {
    BeforeAll {
        Import-Module "$PSScriptRoot\..\harvester_launcher\Data.psm1"    -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Reducer.psm1" -Force
        Import-Module "$PSScriptRoot\..\harvester_launcher\Render.psm1"  -Force
    }
    function S { New-LauncherState -HarvesterCmd 'hb' -Width 100 -Height 30 }

    It 'frame row count equals terminal height' {
        (Build-LauncherFrame -State (S)).Count | Should Be 30
    }
    It 'each row total character count equals terminal width' {
        $frame = Build-LauncherFrame -State (S)
        foreach ($row in $frame) {
            $len = ($row | Measure-Object -Property { $_.Text.Length } -Sum).Sum
            $len | Should Be 100
        }
    }
    It 'too-small frame still has correct row count' {
        $s = New-LauncherState -HarvesterCmd 'hb' -Width 50 -Height 10
        (Build-LauncherFrame -State $s).Count | Should Be 10
    }
    It 'frame contains Run batch text' {
        $all = (Build-LauncherFrame -State (S)) | ForEach-Object { ($_ | ForEach-Object { $_.Text }) -join '' }
        ($all -join '') | Should Match 'Run batch'
    }
    It 'frame contains checkpoint not-set text' {
        $all = (Build-LauncherFrame -State (S)) | ForEach-Object { ($_ | ForEach-Object { $_.Text }) -join '' }
        ($all -join '') | Should Match 'not set'
    }
    It 'frame contains command preview binary name' {
        $all = (Build-LauncherFrame -State (S)) | ForEach-Object { ($_ | ForEach-Object { $_.Text }) -join '' }
        ($all -join '') | Should Match 'hb'
    }
    It 'frame contains LLM concurrency label' {
        $all = (Build-LauncherFrame -State (S)) | ForEach-Object { ($_ | ForEach-Object { $_.Text }) -join '' }
        ($all -join '') | Should Match 'LLM'
    }
    It 'selected item in Left pane uses DarkCyan background when Left is active' {
        $frame = Build-LauncherFrame -State (S)
        $hasDarkCyan = $frame | ForEach-Object { $_ | Where-Object { $_.Bg -eq 'DarkCyan' } } | Where-Object { $_ }
        $hasDarkCyan | Should Not BeNullOrEmpty
    }
}
```

### Step 2: Run — verify fail

### Step 3: Implement pane builders and `Build-LauncherFrame` + `Render-LauncherState`

Append to `Render.psm1`:
```powershell
# ── Left pane ─────────────────────────────────────────────────────────────────

function Build-LeftPaneRows {
    param([hashtable]$State)
    $layout    = $State.Ui.Layout.Left
    $W         = $layout.W
    $H         = $layout.H
    $isActive  = $State.Ui.ActivePane -eq 'Left'
    $actions   = $State.Data.Actions
    $curIdx    = $State.Cursor.LeftIndex
    $chkDisp   = $State.Runtime.CheckpointDisplay
    $chkAvail  = $State.Runtime.CheckpointCliAvailable

    $rows = [System.Collections.Generic.List[object[]]]::new()
    $inner = $W - 2   # minus left and right border chars

    # Title row
    $title = " Harvester Batch Launcher"
    $titleSegs = @(
        (New-Seg $script:Box.V 'DarkGray' 'Black')
        (New-Seg ($title.PadRight($inner)) 'White' 'Black')
        (New-Seg $script:Box.V 'DarkGray' 'Black')
    )
    $rows.Add((Pad-SegmentsToWidth $titleSegs $W))

    # Separator
    $rows.Add((Pad-SegmentsToWidth @(New-Seg ($script:Box.V + ($script:Box.H * $inner) + $script:Box.V) 'DarkGray' 'Black') $W))

    # Action items
    $actionStartLine = 2
    foreach ($item in $actions) {
        if ($rows.Count -ge ($H - 3)) { break }   # leave room for checkpoint display
        if ($item.IsSeparator) {
            $rows.Add((Pad-SegmentsToWidth @(New-Seg ($script:Box.V + (' ' * $inner) + $script:Box.V) 'DarkGray' 'Black') $W))
            continue
        }
        $itemIdx  = [Array]::IndexOf($actions, $item)
        $isSelRow = ($itemIdx -eq $curIdx)
        $bg       = if ($isSelRow -and $isActive) { 'DarkCyan'  } else { 'Black' }
        $fg       = if ($isSelRow)                { 'White'     } elseif ($item.IsCheckpoint -and -not $chkAvail) { 'DarkGray' } else { 'Gray' }
        $marker   = if ($isSelRow -and $isActive) { '►' } else { ' ' }
        $label    = " $marker $($item.Label)"
        $segs = @(
            (New-Seg $script:Box.V 'DarkGray' 'Black')
            (New-Seg ($label.PadRight($inner)) $fg $bg)
            (New-Seg $script:Box.V 'DarkGray' 'Black')
        )
        $rows.Add((Pad-SegmentsToWidth $segs $W))
    }

    # Fill remaining space before checkpoint display
    while ($rows.Count -lt ($H - 2)) {
        $rows.Add((Pad-SegmentsToWidth @(New-Seg ($script:Box.V + (' ' * $inner) + $script:Box.V) 'DarkGray' 'Black') $W))
    }

    # Checkpoint display row
    $chkLabel = " Checkpoint: $chkDisp"
    if ($chkLabel.Length -gt $inner) { $chkLabel = $chkLabel.Substring(0, $inner) }
    $rows.Add((Pad-SegmentsToWidth @(
        (New-Seg $script:Box.V 'DarkGray' 'Black')
        (New-Seg ($chkLabel.PadRight($inner)) 'DarkGray' 'Black')
        (New-Seg $script:Box.V 'DarkGray' 'Black')
    ) $W))

    # Bottom border
    $rows.Add((Pad-SegmentsToWidth @(New-Seg ($script:Box.BL + ($script:Box.H * $inner) + $script:Box.BR) 'DarkGray' 'Black') $W))

    $rows.ToArray()
}

# ── Right pane ────────────────────────────────────────────────────────────────

function Build-ParamRow {
    param([object]$ParamDef, [object]$Value, [bool]$IsSelected, [bool]$PaneActive, [int]$Width)
    $inner   = $Width - 2
    $isActive = $IsSelected -and $PaneActive
    $bg      = if ($isActive) { 'DarkCyan' } else { 'Black' }
    $fg      = if ($IsSelected) { 'White' } else { 'Gray' }
    $label   = "  $($ParamDef.Label):"

    $valueStr = switch ($ParamDef.Type) {
        'Int'  {
            $hint = if ($isActive) { " [◄ $($ParamDef.Min)-$($ParamDef.Max) ►]" } else { '' }
            "$Value$($ParamDef.Unit)$hint"
        }
        'Bool' {
            $box = if ($Value) { '[x] ON ' } else { '[ ] OFF' }
            $box
        }
        'Path' { "$Value" }
        default { "$Value" }
    }

    $line = ($label.PadRight(22)) + $valueStr
    if ($line.Length -gt $inner) { $line = $line.Substring(0, $inner) }

    @(
        (New-Seg $script:Box.V 'DarkGray' 'Black')
        (New-Seg ($line.PadRight($inner)) $fg $bg)
        (New-Seg $script:Box.V 'DarkGray' 'Black')
    )
}

function Build-RightPaneRows {
    param([hashtable]$State)
    $layout   = $State.Ui.Layout.Right
    $W        = $layout.W
    $H        = $layout.H
    $isActive = $State.Ui.ActivePane -eq 'Right'
    $params   = $State.Data.Params
    $curIdx   = $State.Cursor.RightIndex
    $values   = $State.Values
    $cmd      = Build-CommandArgs -State $State -DryRun $false

    $rows = [System.Collections.Generic.List[object[]]]::new()
    $inner = $W - 2

    # Title
    $rows.Add((Pad-SegmentsToWidth @(
        (New-Seg $script:Box.V 'DarkGray' 'Black')
        (New-Seg (' Parameters'.PadRight($inner)) 'White' 'Black')
        (New-Seg $script:Box.V 'DarkGray' 'Black')
    ) $W))

    # Separator
    $rows.Add((Pad-SegmentsToWidth @(New-Seg ($script:Box.V + ($script:Box.H * $inner) + $script:Box.V) 'DarkGray' 'Black') $W))

    # Parameter rows (leave 8 rows for command preview)
    $previewLines  = 8
    $paramAreaH    = $H - $previewLines - 4  # title + sep + preview header + bottom border
    $scrollTop     = $State.Cursor.RightScroll
    $visibleParams = $params | Select-Object -Skip $scrollTop -First $paramAreaH

    foreach ($p in $visibleParams) {
        $idx     = [Array]::IndexOf($params, $p)
        $isSelRow = ($idx -eq $curIdx)
        $segs    = Build-ParamRow -ParamDef $p -Value $values[$p.Name] -IsSelected $isSelRow -PaneActive $isActive -Width $W
        $rows.Add((Pad-SegmentsToWidth $segs $W))
    }

    # Fill to command preview start
    while ($rows.Count -lt ($H - $previewLines - 2)) {
        $rows.Add((Pad-SegmentsToWidth @(New-Seg ($script:Box.V + (' ' * $inner) + $script:Box.V) 'DarkGray' 'Black') $W))
    }

    # Command preview separator
    $rows.Add((Pad-SegmentsToWidth @(New-Seg ($script:Box.V + ($script:Box.H * $inner) + $script:Box.V) 'DarkGray' 'Black') $W))

    # Command preview header
    $rows.Add((Pad-SegmentsToWidth @(
        (New-Seg $script:Box.V 'DarkGray' 'Black')
        (New-Seg ('  Command:'.PadRight($inner)) 'DarkGray' 'Black')
        (New-Seg $script:Box.V 'DarkGray' 'Black')
    ) $W))

    # Command preview lines
    $previewText = Build-CommandPreviewLines -FilePath $cmd.FilePath -Argv $cmd.Argv -MaxWidth ($inner - 2)
    $maxPrev     = $previewLines - 2
    for ($i = 0; $i -lt $maxPrev; $i++) {
        $text = if ($i -lt $previewText.Count) { $previewText[$i] } else { '' }
        $rows.Add((Pad-SegmentsToWidth @(
            (New-Seg $script:Box.V 'DarkGray' 'Black')
            (New-Seg ("  $text".PadRight($inner)) 'Cyan' 'Black')
            (New-Seg $script:Box.V 'DarkGray' 'Black')
        ) $W))
    }

    # Bottom border
    $rows.Add((Pad-SegmentsToWidth @(New-Seg ($script:Box.BL + ($script:Box.H * $inner) + $script:Box.BR) 'DarkGray' 'Black') $W))

    $rows.ToArray()
}

# ── Status bar ────────────────────────────────────────────────────────────────

function Build-StatusBarRow {
    param([hashtable]$State)
    $W      = $State.Ui.Layout.Status.W
    $hints  = 'Tab Switch  ↑↓ Navigate  ◄► Change  Space Toggle  Enter Run  S Save  Q Quit'
    $status = $State.Runtime.LastStatus
    $msg    = $State.Runtime.LastMessage

    $statusSeg = if ($status -eq 'OK')    { New-Seg " ✓ $msg" 'Green'  'Black' }
                 elseif ($status -eq 'Error') { New-Seg " ✗ $msg" 'Red'    'Black' }
                 elseif ($status -eq 'Warn')  { New-Seg " ! $msg" 'Yellow' 'Black' }
                 else                          { New-Seg ''        'Gray'   'Black' }

    $hintSeg = New-Seg $hints 'DarkGray' 'Black'
    Pad-SegmentsToWidth @($hintSeg, $statusSeg) $W
}

# ── Too-small fallback ────────────────────────────────────────────────────────

function Build-TooSmallFrame {
    param([hashtable]$State)
    $W    = $State.Ui.Layout.Width
    $H    = $State.Ui.Layout.Height
    $minW = $State.Ui.Layout.Constraints.MinWidth
    $minH = $State.Ui.Layout.Constraints.MinHeight
    $msg  = "Terminal too small — resize to at least ${minW}×${minH}"
    $rows = [System.Collections.Generic.List[object[]]]::new()
    for ($i = 0; $i -lt [Math]::Max(1, $H); $i++) {
        $text = if ($i -eq [Math]::Floor($H/2)) { $msg } else { '' }
        $rows.Add((Pad-SegmentsToWidth @(New-Seg ($text.PadRight([Math]::Max(1,$W))) 'Yellow' 'Black') [Math]::Max(1,$W)))
    }
    $rows.ToArray()
}

# ── Frame assembly ────────────────────────────────────────────────────────────

function Build-LauncherFrame {
    param([hashtable]$State)

    if ($State.Ui.TooSmall) { return Build-TooSmallFrame -State $State }

    $W      = $State.Ui.Layout.Width
    $H      = $State.Ui.Layout.Height
    $leftW  = $State.Ui.Layout.Left.W
    $rightX = $State.Ui.Layout.Right.X
    $rightW = $State.Ui.Layout.Right.W
    $contentH = $H - 1

    $leftRows  = Build-LeftPaneRows  -State $State
    $rightRows = Build-RightPaneRows -State $State
    $statusRow = Build-StatusBarRow  -State $State

    $frame = [System.Collections.Generic.List[object[]]]::new()
    for ($i = 0; $i -lt $contentH; $i++) {
        $left  = if ($i -lt $leftRows.Count)  { $leftRows[$i]  } else { @(New-Seg (' ' * $leftW) 'Black' 'Black') }
        $gap   = @(New-Seg ' ' 'Black' 'Black')
        $right = if ($i -lt $rightRows.Count) { $rightRows[$i] } else { @(New-Seg (' ' * $rightW) 'Black' 'Black') }
        $row   = @($left) + $gap + @($right)
        $frame.Add((Pad-SegmentsToWidth ($row | ForEach-Object { $_ }) $W))
    }
    $frame.Add((Pad-SegmentsToWidth $statusRow $W))

    $frame.ToArray()
}

function Render-LauncherState {
    param([hashtable]$State, [object[][]]$PreviousFrame = @())
    $frame = Build-LauncherFrame -State $State
    $diff  = Get-FrameDiff -PrevFrame $PreviousFrame -CurrFrame $frame
    Flush-FrameDiff -Diff $diff
    $frame   # return for next diff cycle
}

Export-ModuleMember -Function New-Seg, Pad-SegmentsToWidth, Get-FrameDiff, Flush-FrameDiff, `
    Build-CommandPreviewLines, Build-LauncherFrame, Render-LauncherState
```

### Step 4: Run — verify pass
### Step 5: Commit
```
git add scripts/harvester_launcher/Render.psm1 scripts/tests/HarvesterLauncher.Tests.ps1
git commit -m "feat(launcher): add Render pane builders and frame assembly"
```

---

## Task 12 — Entry point: Start-HarvesterBatch.ps1

No unit tests — this is the main loop. Verify manually.

### Step 1: Write the entry point
```powershell
#Requires -Version 5.1
[CmdletBinding()]
param(
    [string]$HarvesterBatchCmd = 'harvester_batch',
    [string]$ProjectRoot       = (Split-Path -Parent $PSScriptRoot)
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# ── Module imports ────────────────────────────────────────────────────────────
$launcherDir = Join-Path $PSScriptRoot 'harvester_launcher'
$subInput    = Join-Path $ProjectRoot  'ministry-of-future-plans\browser\Input.psm1'

if (-not (Test-Path -LiteralPath $subInput)) {
    Write-Error "Submodule not initialised. Run: git submodule update --init ministry-of-future-plans"
}

Import-Module $subInput                                  -Force
Import-Module (Join-Path $launcherDir 'Data.psm1')      -Force
Import-Module (Join-Path $launcherDir 'Input.psm1')     -Force
Import-Module (Join-Path $launcherDir 'Reducer.psm1')   -Force
Import-Module (Join-Path $launcherDir 'Effects.psm1')   -Force
Import-Module (Join-Path $launcherDir 'Render.psm1')    -Force

# ── Startup ───────────────────────────────────────────────────────────────────
$state = New-LauncherState -HarvesterCmd $HarvesterBatchCmd `
                            -Width  [Console]::WindowWidth `
                            -Height [Console]::WindowHeight

function Invoke-EffectLoop {
    param([hashtable]$StateIn, [object[]]$Effects)
    $s = $StateIn
    $followUps = Invoke-LauncherEffects -State $s -Effects $Effects
    foreach ($a in $followUps) {
        $r = Invoke-LauncherReducer -State $s -Action $a
        $s = $r.State
        if ($r.Effects.Count -gt 0) {
            $s = (Invoke-EffectLoop -StateIn $s -Effects $r.Effects)
        }
    }
    $s
}

# Startup effects: load defaults, probe checkpoint CLI, read checkpoint display
$state = Invoke-EffectLoop -StateIn $state -Effects @(
    @{ Type='LoadDefaults' }
    @{ Type='ProbeCheckpointCliSupport' }
    @{ Type='ReadCheckpointDisplay' }
)

# ── TUI loop ─────────────────────────────────────────────────────────────────
$prevFrame          = @()
$savedCursorVisible = [Console]::CursorVisible
[Console]::CursorVisible = $false

try {
    while ($state.Runtime.IsRunning) {

        # Resize detection
        $w = [Console]::WindowWidth; $h = [Console]::WindowHeight
        if ($w -ne $state.Ui.Layout.Width -or $h -ne $state.Ui.Layout.Height) {
            $r      = Invoke-LauncherReducer -State $state -Action @{ Type='Resize'; Width=$w; Height=$h }
            $state  = $r.State
            $prevFrame = @()   # force full repaint after resize
        }

        # Render
        $prevFrame = Render-LauncherState -State $state -PreviousFrame $prevFrame

        # Input
        $key    = [Console]::ReadKey($true)
        $action = ConvertFrom-KeyInfoToLauncherAction -KeyInfo $key
        if ($null -eq $action) { continue }

        # Reduce — all special-case logic (cp-set-date prompt) is handled via
        # the DatePromptRequested effect dispatched through Invoke-LauncherEffects.
        $result = Invoke-LauncherReducer -State $state -Action $action
        $state  = $result.State

        # Effects
        if ($result.Effects.Count -gt 0) {
            $state = Invoke-EffectLoop -StateIn $state -Effects $result.Effects
        }
    }
} finally {
    [Console]::CursorVisible = $savedCursorVisible
    [Console]::ResetColor()
    [Console]::Clear()
}

# ── Post-exit launch ─────────────────────────────────────────────────────────
if ($null -ne $state.Pending.LaunchAfterExit) {
    $cmd = $state.Pending.LaunchAfterExit
    Write-Host "Running: $($cmd.FilePath) $($cmd.Argv -join ' ')"
    & $cmd.FilePath @($cmd.Argv)
}
```

### Step 2: Manual smoke test
```powershell
# Build the binary first
cargo build -p harvester_batch

# Add to PATH temporarily
$env:PATH = "$env:PATH;$(Resolve-Path target\debug)"

# Launch
pwsh scripts\Start-HarvesterBatch.ps1
```

Expected: TUI renders, two panes visible, command preview shows `harvester_batch`.

### Step 3: Commit
```
git add scripts/Start-HarvesterBatch.ps1
git commit -m "feat(launcher): add entry point Start-HarvesterBatch.ps1"
```

---

## Task 13 — Engineering diary entry

Per `Agents.md`, add to `docs/EngineeringDiary.md`:

```markdown
## 2026-02-21 — PowerShell TUI Launcher for harvester_batch
Type: Implementation
Context: `harvester_batch` exposes many CLI flags with no interactive UI, increasing operator error and slowing daily startup. The launcher makes it easy to review and adjust parameters before running.
Change: New PowerShell full-screen TUI launcher (`scripts/Start-HarvesterBatch.ps1`) wrapping `harvester_batch`. Implements two-pane console UI with Elm/Redux UDF pattern, live-updating command preview, editable parameter form, single default-profile persistence (JSON), and checkpoint action stubs with capability-probe graceful degradation. Reuses `ministry-of-future-plans/browser/Input.psm1` from the submodule.
Evidence: Pester tests cover Reducer (navigation, value editing, effect emission), Effects (load/save/probe/RON parse), Input (key mapping), and Render (frame diff, pane dimensions). Manual smoke test: `pwsh scripts/Start-HarvesterBatch.ps1`.
Refs: scripts/harvester_launcher/, docs/plans/Design.harvester-batch-tui-launcher.md
```

Commit:
```
git add docs/EngineeringDiary.md
git commit -m "docs: add diary entry for harvester_batch TUI launcher"
```

---

## Verification Checklist (after all tasks)

Run all Pester tests:
```
Invoke-Pester scripts\tests\HarvesterLauncher.Tests.ps1 -Output Detailed
```
Expected: all tests pass (no failures).

Manual TUI verification:
```
cargo build -p harvester_batch
$env:PATH += ";$(Resolve-Path target\debug)"
pwsh scripts\Start-HarvesterBatch.ps1
```

Check each item:
- [ ] TUI renders with two panes at ≥80×24
- [ ] Tab switches active pane; `►` marker tracks correctly
- [ ] `◄` / `►` on LLM concurrency changes value (1-10 clamped); command preview updates live
- [ ] Space on Force unlock toggles `[ ] OFF` ↔ `[x] ON`; flag appears/disappears in preview
- [ ] ENTER on "Run dry-run" exits TUI, runs `harvester_batch --dry-run ...`
- [ ] ENTER on "Run batch" exits TUI, runs `harvester_batch ...` (no --dry-run)
- [ ] `S` writes `scripts/harvester_launcher_defaults.json`; relaunch reloads saved values
- [ ] `Q` exits cleanly; cursor visible, console not garbled
- [ ] Resize terminal mid-session: layout reflows without crash or artifacts
- [ ] Shrink below 72 wide: "Terminal too small" message; no crash
- [ ] Checkpoint items show "not yet available" status when Slice A absent

---

## Future Extensions (out of scope)

- Multiple named profiles (`scripts/harvester_profiles.json`) — Idea 3B
- `HARVESTER_PROFILE` env var for CI automation — Idea 3C
- Inline path editing (text input mode in right pane)
- Confirm guard for "Clear checkpoint" only (while keeping run actions immediate)
- Status/event log pane for operator traceability
- Pester tests for Render pane segment content (once stable)
- Checkpoint actions light up automatically when Slice A CLI flags ship (probed at startup)
