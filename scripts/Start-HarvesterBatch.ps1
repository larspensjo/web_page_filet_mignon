#Requires -Version 5.1
[CmdletBinding()]
param(
    # Leave empty (default) to invoke via 'cargo run -p harvester_batch --'.
    # Pass an explicit path (e.g. '.\target\release\harvester_batch.exe') to run that binary directly.
    [string]$HarvesterBatchCmd = '',
    [string]$ProjectRoot       = (Split-Path -Parent $PSScriptRoot),
    [int]$RefreshStaleSummariesLimit = 0,
    [int]$SignalCandidateThreshold = 0,
    [switch]$BatchApi,
    [switch]$VerboseProgress,
    [switch]$AsciiProgress
)

# Resolve invocation style: cargo run (default) vs direct binary
$script:useCargoRun = [string]::IsNullOrEmpty($HarvesterBatchCmd)
$script:harvesterDisplayCmd = if ($script:useCargoRun) { 'harvester_batch' } else { $HarvesterBatchCmd }
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($BatchApi -and $RefreshStaleSummariesLimit -gt 0) {
    throw '-BatchApi cannot be combined with -RefreshStaleSummariesLimit.'
}

$progressArgs = @()
if ($VerboseProgress) {
    $progressArgs += '--verbose-progress'
}
if ($AsciiProgress) {
    $progressArgs += '--ascii-progress'
}

# Optional direct batch mode for bounded summary refreshes without entering the TUI.
if ($RefreshStaleSummariesLimit -gt 0 -or $BatchApi) {
    $extra = @($progressArgs)
    if ($BatchApi) {
        $extra += '--batch-api'
    }
    if ($SignalCandidateThreshold -gt 0) {
        $extra += @('--signal-candidate-threshold', "$SignalCandidateThreshold")
    }
    if ($script:useCargoRun) {
        $refreshArgs = if ($RefreshStaleSummariesLimit -gt 0) { @('--refresh-stale-summaries-limit', "$RefreshStaleSummariesLimit") } else { @() }
        Write-Host "Running: cargo run -p harvester_batch -- $($refreshArgs + $extra -join ' ')"
        & cargo run -p harvester_batch -- @refreshArgs @extra
    } else {
        $refreshArgs = if ($RefreshStaleSummariesLimit -gt 0) { @('--refresh-stale-summaries-limit', "$RefreshStaleSummariesLimit") } else { @() }
        Write-Host "Running: $HarvesterBatchCmd $($refreshArgs + $extra -join ' ')"
        & $HarvesterBatchCmd @refreshArgs @extra
    }
    return
}

# Force UTF-8 so box-drawing characters render correctly on Windows consoles.
try { [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false) } catch { $null = $_ }

# ── Module imports ────────────────────────────────────────────────────────────
$launcherDir = Join-Path $PSScriptRoot 'harvester_launcher'
Import-Module (Join-Path $launcherDir 'Data.psm1')      -Force
Import-Module (Join-Path $launcherDir 'Input.psm1')     -Force
Import-Module (Join-Path $launcherDir 'Reducer.psm1')   -Force
Import-Module (Join-Path $launcherDir 'Effects.psm1')   -Force
Import-Module (Join-Path $launcherDir 'Render.psm1')    -Force

# ── Startup ───────────────────────────────────────────────────────────────────
$state = New-LauncherState -HarvesterCmd $script:harvesterDisplayCmd `
                            -UseCargoRun  $script:useCargoRun `
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
try { $savedCursorVisible = [Console]::CursorVisible } catch { $null = $_ }
try { [Console]::CursorVisible = $false } catch { $null = $_ }

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
    try { [Console]::CursorVisible = $savedCursorVisible } catch { $null = $_ }
    try { [Console]::ResetColor() } catch { $null = $_ }
    try { [Console]::Clear() } catch { $null = $_ }
}

# ── Post-exit launch ─────────────────────────────────────────────────────────
if ($null -ne $state.Pending.LaunchAfterExit) {
    $cmd = $state.Pending.LaunchAfterExit
    $launchArgs = @($cmd.Argv) + $progressArgs
    if ($script:useCargoRun) {
        Write-Host "Running: cargo run -p harvester_batch -- $($launchArgs -join ' ')"
        & cargo run -p harvester_batch -- @launchArgs
    } else {
        Write-Host "Running: $($cmd.FilePath) $($launchArgs -join ' ')"
        & $cmd.FilePath @launchArgs
    }
}
