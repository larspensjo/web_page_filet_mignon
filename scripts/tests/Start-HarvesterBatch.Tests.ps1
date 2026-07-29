#Requires -Version 5.1
Set-StrictMode -Version Latest

Describe 'Start-HarvesterBatch progress switches' {
    BeforeAll {
        $script:LauncherPath = Join-Path $PSScriptRoot '..\Start-HarvesterBatch.ps1'
    }

    function script:New-CaptureCommand {
        param([string]$TempDir, [string]$CapturePath)

        $commandPath = Join-Path $TempDir 'capture.cmd'
        @(
            '@echo off'
            ('echo %* > "{0}"' -f $CapturePath)
        ) | Set-Content -LiteralPath $commandPath -Encoding ascii
        $commandPath
    }

    function script:Invoke-DirectStartHarvesterBatchCapture {
        param([string[]]$Switches)

        $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("harvester-batch-launch-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $tempDir | Out-Null
        $capturePath = Join-Path $tempDir 'arguments.txt'
        try {
            $commandPath = New-CaptureCommand -TempDir $tempDir -CapturePath $capturePath
            $launchParams = @{ HarvesterBatchCmd = $commandPath }
            if ($Switches -contains '-Drain') {
                $launchParams.Drain = $true
            } else {
                $launchParams.BatchApi = $true
            }
            if ($Switches -contains '-VerboseProgress') { $launchParams.VerboseProgress = $true }
            if ($Switches -contains '-AsciiProgress') { $launchParams.AsciiProgress = $true }
            & $script:LauncherPath @launchParams
            Get-Content -LiteralPath $capturePath -Raw
        } finally {
            Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    function script:Invoke-RegularStartHarvesterBatchCapture {
        param([string[]]$Switches)

        $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("harvester-batch-launch-{0}" -f [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $tempDir | Out-Null
        $capturePath = Join-Path $tempDir 'arguments.txt'
        try {
            $commandPath = New-CaptureCommand -TempDir $tempDir -CapturePath $capturePath
            $launcherCopy = Join-Path $tempDir 'Start-HarvesterBatch.ps1'
            Copy-Item -LiteralPath $script:LauncherPath -Destination $launcherCopy
            $launcherSource = (Get-Content -LiteralPath $launcherCopy -Raw).
                Replace('([Console]::WindowWidth)', '100').
                Replace('([Console]::WindowHeight)', '30')
            Set-Content -LiteralPath $launcherCopy -Value $launcherSource -Encoding utf8
            $moduleDir = Join-Path $tempDir 'harvester_launcher'
            New-Item -ItemType Directory -Path $moduleDir | Out-Null

            @'
function New-LauncherState {
    param([string]$HarvesterCmd, [bool]$UseCargoRun, [int]$Width, [int]$Height)
    @{
        Runtime = @{ IsRunning = $false }
        Pending = @{ LaunchAfterExit = [pscustomobject]@{ FilePath = $HarvesterCmd; Argv = @('--poll-interval', '30') } }
    }
}
function Invoke-LauncherReducer { throw 'The reducer should not run in this test.' }
Export-ModuleMember -Function New-LauncherState, Invoke-LauncherReducer
'@ | Set-Content -LiteralPath (Join-Path $moduleDir 'Reducer.psm1') -Encoding utf8
            @'
function Invoke-LauncherEffects { @() }
Export-ModuleMember -Function Invoke-LauncherEffects
'@ | Set-Content -LiteralPath (Join-Path $moduleDir 'Effects.psm1') -Encoding utf8
            @'
function ConvertFrom-KeyInfoToLauncherAction { throw 'Input should not run in this test.' }
Export-ModuleMember -Function ConvertFrom-KeyInfoToLauncherAction
'@ | Set-Content -LiteralPath (Join-Path $moduleDir 'Input.psm1') -Encoding utf8
            @'
function Render-LauncherState { throw 'Rendering should not run in this test.' }
Export-ModuleMember -Function Render-LauncherState
'@ | Set-Content -LiteralPath (Join-Path $moduleDir 'Render.psm1') -Encoding utf8
            "'stub'" | Set-Content -LiteralPath (Join-Path $moduleDir 'Data.psm1') -Encoding utf8

            $launchParams = @{ HarvesterBatchCmd = $commandPath }
            if ($Switches -contains '-VerboseProgress') { $launchParams.VerboseProgress = $true }
            if ($Switches -contains '-AsciiProgress') { $launchParams.AsciiProgress = $true }
            & $launcherCopy @launchParams
            Get-Content -LiteralPath $capturePath -Raw
        } finally {
            Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'forwards -VerboseProgress as --verbose-progress on the direct path' {
        (Invoke-DirectStartHarvesterBatchCapture -Switches @('-VerboseProgress')) | Should -Match '--verbose-progress'
    }

    It 'does not forward --verbose-progress on the direct path when -VerboseProgress is absent' {
        (Invoke-DirectStartHarvesterBatchCapture -Switches @()) | Should -Not -Match '--verbose-progress'
    }

    It 'forwards -AsciiProgress as --ascii-progress on the direct path' {
        (Invoke-DirectStartHarvesterBatchCapture -Switches @('-AsciiProgress')) | Should -Match '--ascii-progress'
    }

    It 'does not forward --ascii-progress on the direct path when -AsciiProgress is absent' {
        (Invoke-DirectStartHarvesterBatchCapture -Switches @()) | Should -Not -Match '--ascii-progress'
    }

    It 'forwards -Drain as --drain without also passing --batch-api' {
        $arguments = Invoke-DirectStartHarvesterBatchCapture -Switches @('-Drain')
        $arguments | Should -Match '--drain'
        $arguments | Should -Not -Match '--batch-api'
    }

    It 'forwards -BatchApi as --batch-api without --drain' {
        $arguments = Invoke-DirectStartHarvesterBatchCapture -Switches @()
        $arguments | Should -Match '--batch-api'
        $arguments | Should -Not -Match '--drain'
    }

    It 'forwards progress switches alongside -Drain' {
        (Invoke-DirectStartHarvesterBatchCapture -Switches @('-Drain', '-AsciiProgress')) |
            Should -Match '--ascii-progress'
    }

    It 'rejects -Drain combined with -RefreshStaleSummariesLimit' {
        { & $script:LauncherPath -Drain -RefreshStaleSummariesLimit 5 } |
            Should -Throw -ExpectedMessage '*-Drain cannot be combined with -RefreshStaleSummariesLimit.*'
    }

    It 'forwards -VerboseProgress as --verbose-progress on the regular-run path' {
        (Invoke-RegularStartHarvesterBatchCapture -Switches @('-VerboseProgress')) | Should -Match '--verbose-progress'
    }

    It 'forwards -AsciiProgress as --ascii-progress on the regular-run path' {
        (Invoke-RegularStartHarvesterBatchCapture -Switches @('-AsciiProgress')) | Should -Match '--ascii-progress'
    }

    It 'does not forward progress flags on the regular-run path when switches are absent' {
        $arguments = Invoke-RegularStartHarvesterBatchCapture -Switches @()
        $arguments | Should -Not -Match '--verbose-progress'
        $arguments | Should -Not -Match '--ascii-progress'
    }
}
