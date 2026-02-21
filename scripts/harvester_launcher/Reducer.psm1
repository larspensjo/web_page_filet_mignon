#Requires -Version 5.1
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot 'Data.psm1') -Force -Global

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

        # (Activate added in Task 6)

        default { <# unknown actions are silently ignored #> }
    }

    @{ State = $s; Effects = $effects.ToArray() }
}

Export-ModuleMember -Function New-LauncherState, Copy-LauncherState, Get-LauncherLayout, Get-LauncherLayoutConstraints, Invoke-LauncherReducer
