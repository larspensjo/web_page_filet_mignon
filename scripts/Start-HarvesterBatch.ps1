#Requires -Version 5.1
[CmdletBinding()]
param(
    # Leave empty (default) to invoke via 'cargo run -p harvester_batch --'.
    # Pass an explicit path (e.g. '.\target\release\harvester_batch.exe') to run that binary directly.
    [string]$HarvesterBatchCmd = '',
    [string]$ProjectRoot       = (Split-Path -Parent $PSScriptRoot)
)

# Resolve invocation style: cargo run (default) vs direct binary
$script:useCargoRun = [string]::IsNullOrEmpty($HarvesterBatchCmd)
$script:harvesterDisplayCmd = if ($script:useCargoRun) { 'harvester_batch' } else { $HarvesterBatchCmd }
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Force UTF-8 so box-drawing characters render correctly on Windows consoles.
try { [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false) } catch { }

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
$state = New-LauncherState -HarvesterCmd $script:harvesterDisplayCmd `
                            -Width  ([Console]::WindowWidth) `
                            -Height ([Console]::WindowHeight)

function Invoke-EffectLoop {
    param([hashtable]$StateIn, [object[]]$Effects)
    $s = $StateIn
    $followUps = Invoke-LauncherEffects -State $s -Effects $Effects
    foreach ($a in $followUps) {
        $r = Invoke-LauncherReducer -State $s -Action $a
        $s = $r.State
        if ($r.Effects.Count -gt 0) {
            $s = Invoke-EffectLoop -StateIn $s -Effects $r.Effects
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
$savedCursorVisible = $true
try { $savedCursorVisible = [Console]::CursorVisible } catch { }
try { [Console]::CursorVisible = $false } catch { }

try {
    while ($state.Runtime.IsRunning) {

        # Resize detection
        $w = [Console]::WindowWidth; $h = [Console]::WindowHeight
        if ($w -ne $state.Ui.Layout.Width -or $h -ne $state.Ui.Layout.Height) {
            $r         = Invoke-LauncherReducer -State $state -Action @{ Type='Resize'; Width=$w; Height=$h }
            $state     = $r.State
            $prevFrame = @()   # force full repaint after resize
        }

        # Render
        $prevFrame = Render-LauncherState -State $state -PreviousFrame $prevFrame

        # Input
        $key    = [Console]::ReadKey($true)
        $action = ConvertFrom-KeyInfoToLauncherAction -KeyInfo $key
        if ($null -eq $action) { continue }

        # Reduce
        $result = Invoke-LauncherReducer -State $state -Action $action
        $state  = $result.State

        # Effects
        if ($result.Effects.Count -gt 0) {
            $state = Invoke-EffectLoop -StateIn $state -Effects $result.Effects
        }
    }
} finally {
    try { [Console]::CursorVisible = $savedCursorVisible } catch { }
    try { [Console]::ResetColor() } catch { }
    try { [Console]::Clear() } catch { }
}

# ── Post-exit launch ─────────────────────────────────────────────────────────
if ($null -ne $state.Pending.LaunchAfterExit) {
    $cmd = $state.Pending.LaunchAfterExit
    if ($script:useCargoRun) {
        Write-Host "Running: cargo run -p harvester_batch -- $($cmd.Argv -join ' ')"
        & cargo run -p harvester_batch -- @($cmd.Argv)
    } else {
        Write-Host "Running: $($cmd.FilePath) $($cmd.Argv -join ' ')"
        & $cmd.FilePath @($cmd.Argv)
    }
}
