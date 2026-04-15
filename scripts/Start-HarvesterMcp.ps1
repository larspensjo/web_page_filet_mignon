#Requires -Version 7.0
[CmdletBinding()]
param(
    # Leave empty to prefer target\debug\harvester_mcp.exe and fall back to cargo run.
    # Pass an explicit path to a built binary to avoid cargo invocation.
    [string]$HarvesterMcpCmd = '',

    [string]$ProjectRoot = (Split-Path -Parent $PSScriptRoot),

    [string]$OutputDir = (Join-Path (Split-Path -Parent $PSScriptRoot) 'output'),

    [string]$LogDir = '',

    [string]$AgentModel = '',

    [int]$ContextBudget = 0,

    [int]$ScoringCandidateCap = 0,

    [int]$TooBroadThreshold = 0,

    [int]$MinTriagePriority = 0,

    [int]$RetainLogRuns = 0
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$resolvedProjectRoot = (Resolve-Path -LiteralPath $ProjectRoot).ProviderPath
$resolvedOutputDir = if (Test-Path -LiteralPath $OutputDir) {
    (Resolve-Path -LiteralPath $OutputDir).ProviderPath
} else {
    [System.IO.Path]::GetFullPath($OutputDir, $resolvedProjectRoot)
}

$argList = @('--output-dir', $resolvedOutputDir)

if (-not [string]::IsNullOrWhiteSpace($LogDir)) {
    $resolvedLogDir = if (Test-Path -LiteralPath $LogDir) {
        (Resolve-Path -LiteralPath $LogDir).ProviderPath
    } else {
        [System.IO.Path]::GetFullPath($LogDir, $resolvedProjectRoot)
    }
    $argList += @('--log-dir', $resolvedLogDir)
}

if (-not [string]::IsNullOrWhiteSpace($AgentModel)) {
    $argList += @('--agent-model', $AgentModel)
}

if ($ContextBudget -gt 0) {
    $argList += @('--context-budget', $ContextBudget.ToString())
}

if ($ScoringCandidateCap -gt 0) {
    $argList += @('--scoring-candidate-cap', $ScoringCandidateCap.ToString())
}

if ($TooBroadThreshold -gt 0) {
    $argList += @('--too-broad-threshold', $TooBroadThreshold.ToString())
}

if ($MinTriagePriority -gt 0) {
    $argList += @('--min-triage-priority', $MinTriagePriority.ToString())
}

if ($RetainLogRuns -gt 0) {
    $argList += @('--retain-log-runs', $RetainLogRuns.ToString())
}

$useCargoRun = $false
$resolvedHarvesterMcpCmd = ''

if (-not [string]::IsNullOrWhiteSpace($HarvesterMcpCmd)) {
    $resolvedHarvesterMcpCmd = if (Test-Path -LiteralPath $HarvesterMcpCmd) {
        (Resolve-Path -LiteralPath $HarvesterMcpCmd).ProviderPath
    } else {
        throw "Harvester MCP binary not found: $HarvesterMcpCmd"
    }
} else {
    $defaultExe = Join-Path $resolvedProjectRoot 'target\debug\harvester_mcp.exe'
    if (Test-Path -LiteralPath $defaultExe) {
        $resolvedHarvesterMcpCmd = (Resolve-Path -LiteralPath $defaultExe).ProviderPath
    } else {
        $useCargoRun = $true
    }
}

Push-Location $resolvedProjectRoot
try {
    if ($useCargoRun) {
        & cargo run -q -p harvester_mcp -- @argList
    } else {
        & $resolvedHarvesterMcpCmd @argList
    }

    exit $LASTEXITCODE
} finally {
    Pop-Location
}
