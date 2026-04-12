#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$Query,

    [int]$MaxResults = 3,

    [string]$OutputDir = 'output',

    # Leave empty (default) to invoke via 'cargo run -q -p harvester_mcp --'.
    # Pass an explicit path (e.g. '.\target\release\harvester_mcp.exe') to run that binary directly.
    [string]$HarvesterMcpCmd = '',

    [string]$ProjectRoot = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($ProjectRoot)) {
    $scriptRoot = if ([string]::IsNullOrWhiteSpace($PSScriptRoot)) {
        Split-Path -Parent $PSCommandPath
    }
    else {
        $PSScriptRoot
    }
    $ProjectRoot = Split-Path -Parent $scriptRoot
}

function New-HarvesterMcpStartInfo {
    param(
        [string]$ProjectRootPath,
        [string]$OutputDirPath,
        [string]$CommandOverride
    )

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $psi.WorkingDirectory = $ProjectRootPath

    if ([string]::IsNullOrWhiteSpace($CommandOverride)) {
        $psi.FileName = 'cargo'
        $null = $psi.ArgumentList.Add('run')
        $null = $psi.ArgumentList.Add('-q')
        $null = $psi.ArgumentList.Add('-p')
        $null = $psi.ArgumentList.Add('harvester_mcp')
        $null = $psi.ArgumentList.Add('--')
        $null = $psi.ArgumentList.Add('--output-dir')
        $null = $psi.ArgumentList.Add($OutputDirPath)
    }
    else {
        $resolved = Resolve-Path -LiteralPath (Join-Path $ProjectRootPath $CommandOverride) -ErrorAction SilentlyContinue
        $psi.FileName = if ($resolved) { $resolved.Path } else { $CommandOverride }
        $null = $psi.ArgumentList.Add('--output-dir')
        $null = $psi.ArgumentList.Add($OutputDirPath)
    }

    return $psi
}

function Send-JsonLine {
    param(
        [System.Diagnostics.Process]$Process,
        [hashtable]$Payload
    )

    $json = $Payload | ConvertTo-Json -Compress -Depth 20
    $Process.StandardInput.WriteLine($json)
    $Process.StandardInput.Flush()
}

function Read-JsonLine {
    param(
        [System.Diagnostics.Process]$Process,
        [int]$TimeoutMs = 30000
    )

    $task = $Process.StandardOutput.ReadLineAsync()
    if (-not $task.Wait($TimeoutMs)) {
        throw "Timed out waiting for MCP response after $TimeoutMs ms."
    }
    return $task.Result
}

function Write-JsonObject {
    param(
        [Parameter(Mandatory)]
        [object]$Value
    )

    $Value | ConvertTo-Json -Depth 30
}

$process = New-Object System.Diagnostics.Process
$process.StartInfo = New-HarvesterMcpStartInfo -ProjectRootPath $ProjectRoot -OutputDirPath $OutputDir -CommandOverride $HarvesterMcpCmd

try {
    if (-not $process.Start()) {
        throw 'Failed to start harvester_mcp.'
    }

    Send-JsonLine -Process $process -Payload @{
        jsonrpc = '2.0'
        id = 1
        method = 'initialize'
        params = @{
            protocolVersion = '2025-11-25'
            capabilities = @{}
            clientInfo = @{
                name = 'smoke'
                version = '1.0'
            }
        }
    }

    $initializeLine = Read-JsonLine -Process $process
    if ([string]::IsNullOrWhiteSpace($initializeLine)) {
        throw 'Did not receive initialize response from harvester_mcp.'
    }

    $initialize = $initializeLine | ConvertFrom-Json
    Write-Output 'Initialize response:'
    $initialize | ConvertTo-Json -Depth 20 -Compress

    Send-JsonLine -Process $process -Payload @{
        jsonrpc = '2.0'
        method = 'notifications/initialized'
    }

    Send-JsonLine -Process $process -Payload @{
        jsonrpc = '2.0'
        id = 2
        method = 'tools/call'
        params = @{
            name = 'query_knowledge_base'
            arguments = @{
                question = $Query
                max_results = $MaxResults
            }
        }
    }

    while ($true) {
        $line = Read-JsonLine -Process $process -TimeoutMs 120000
        if ($null -eq $line) {
            throw 'harvester_mcp exited before returning the tool result.'
        }

        $message = $line | ConvertFrom-Json
        if ($message.id -eq 2) {
            Write-Output ''
            Write-Output 'query_knowledge_base result:'

            $toolText = $message.result.content[0].text
            if ([string]::IsNullOrWhiteSpace($toolText)) {
                Write-JsonObject -Value $message
            }
            else {
                try {
                    $toolPayload = $toolText | ConvertFrom-Json
                    Write-JsonObject -Value $toolPayload
                }
                catch {
                    Write-JsonObject -Value $message
                }
            }
            break
        }
    }
}
finally {
    try {
        if (-not $process.HasExited) {
            $process.Kill()
        }
    } catch {
        $null = $_
    }
    $process.Dispose()
}
