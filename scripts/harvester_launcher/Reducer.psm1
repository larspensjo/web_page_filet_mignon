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

Export-ModuleMember -Function New-LauncherState, Copy-LauncherState, Get-LauncherLayout, Get-LauncherLayoutConstraints
