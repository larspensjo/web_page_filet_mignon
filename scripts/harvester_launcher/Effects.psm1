#Requires -Version 5.1
Set-StrictMode -Version Latest

Import-Module (Join-Path $PSScriptRoot 'Data.psm1') -Force -Global

function Invoke-LoadDefaults {
    param([string]$FilePath)
    if (-not (Test-Path -LiteralPath $FilePath)) {
        return [pscustomobject]@{ Type='DefaultsLoadFailed'; Message='File not found' }
    }
    try {
        $json = Get-Content -LiteralPath $FilePath -Raw -ErrorAction Stop | ConvertFrom-Json
        $vals = New-LauncherDefaults
        $props = $json.PSObject.Properties
        if ($props['LlmConcurrency'])   { $vals.LlmConcurrency   = [Math]::Max(1,    [Math]::Min(12,   [int]$props['LlmConcurrency'].Value))   }
        if ($props['PollInterval'])     { $vals.PollInterval     = [Math]::Max(1,    [Math]::Min(1440, [int]$props['PollInterval'].Value))     }
        if ($props['ForceUnlock'])      { $vals.ForceUnlock      = [bool]$props['ForceUnlock'].Value      }
        if ($props['AllowUnsupported']) { $vals.AllowUnsupported = [bool]$props['AllowUnsupported'].Value }
        if ($props['Sources'])          { $vals.Sources          = [string]$props['Sources'].Value        }
        if ($props['OutputDir'])        { $vals.OutputDir        = [string]$props['OutputDir'].Value      }
        if ($props['ContextsDir'])      { $vals.ContextsDir      = [string]$props['ContextsDir'].Value    }
        if ($props['PromptsDir'])       { $vals.PromptsDir       = [string]$props['PromptsDir'].Value     }
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
    param([string]$HarvesterCmd, [bool]$UseCargoRun = $false)
    if ($UseCargoRun) {
        # In cargo-run (dev) mode the binary is always built from current source,
        # so checkpoint flags are guaranteed present — skip the slow probe.
        return [pscustomobject]@{ Type='CheckpointCapabilityDetected'; Available=$true }
    }
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
    param([string]$HarvesterCmd, [bool]$UseCargoRun = $false, [string]$OutputDir = 'output', [string]$ActionId, [string]$CustomDate = '')
    $flagArgs = switch ($ActionId) {
        'cp-set-now'  { @('--set-briefing-since-now') }
        'cp-set-date' { @('--set-briefing-since', $CustomDate) }
        'cp-clear'    { @('--clear-briefing-since') }
        'cp-show'     { @('--show-briefing-since') }
        default       { return [pscustomobject]@{ Type='CheckpointCommandCompleted'; Success=$false; Message="Unknown action: $ActionId" } }
    }
    # Always forward --output-dir so the binary writes to the correct location
    $argList = @('--output-dir', $OutputDir) + $flagArgs
    try {
        if ($UseCargoRun) {
            & cargo run -q -p harvester_batch -- @argList 2>&1 | Out-Null
            $ok = ($LASTEXITCODE -eq 0)
            [pscustomobject]@{ Type='CheckpointCommandCompleted'; Success=$ok; Message=if ($ok) { 'Done.' } else { "Exit code $LASTEXITCODE" } }
        } else {
            $errFile = [IO.Path]::GetTempFileName()
            $proc = Start-Process -FilePath $HarvesterCmd -ArgumentList $argList `
                                  -Wait -PassThru -NoNewWindow `
                                  -RedirectStandardError $errFile -ErrorAction Stop
            Remove-Item $errFile -Force -ErrorAction SilentlyContinue
            $ok = $proc.ExitCode -eq 0
            [pscustomobject]@{ Type='CheckpointCommandCompleted'; Success=$ok; Message=if ($ok) { 'Done.' } else { "Exit code $($proc.ExitCode)" } }
        }
    } catch {
        [pscustomobject]@{ Type='CheckpointCommandCompleted'; Success=$false; Message=$_.Exception.Message }
    }
}

function Invoke-DatePrompt {
    # Suspends TUI temporarily to collect an RFC3339 date from the user.
    # Returns a DatePromptCompleted action with Value=<string> or Value=$null (cancel/invalid).
    try { [Console]::CursorVisible = $true } catch { $null = $_ }
    try { [Console]::Clear() } catch { $null = $_ }
    Write-Host 'Set Briefing Checkpoint — enter RFC3339 date/time:'
    Write-Host '  Example: 2026-01-01T00:00:00Z'
    Write-Host '  (Press Enter with empty input to cancel)'
    $dateInput = Read-Host 'Date'
    try { [Console]::CursorVisible = $false } catch { $null = $_ }
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

function Invoke-ImportFolderPrompt {
    param([string]$CurrentValue = '')

    try { [Console]::CursorVisible = $true } catch { $null = $_ }
    try { [Console]::Clear() } catch { $null = $_ }

    Write-Host 'Import Saved Webpages - enter folder containing saved .htm/.html files:'
    if ([string]::IsNullOrWhiteSpace($CurrentValue)) {
        Write-Host '  Press Enter with empty input to cancel.'
    } else {
        Write-Host "  Press Enter to keep current: $CurrentValue"
    }

    $folderInput = Read-Host 'Folder'
    try { [Console]::CursorVisible = $false } catch { $null = $_ }

    $selected = if ([string]::IsNullOrWhiteSpace($folderInput)) { $CurrentValue } else { $folderInput.Trim() }
    if ([string]::IsNullOrWhiteSpace($selected)) {
        return [pscustomobject]@{
            Type    = 'ImportFolderPromptCompleted'
            Value   = $null
            Message = 'Import cancelled: no input folder selected.'
        }
    }

    if (-not (Test-Path -LiteralPath $selected -PathType Container)) {
        Write-Host "Folder not found: $selected" -ForegroundColor Red
        Start-Sleep 2
        return [pscustomobject]@{
            Type    = 'ImportFolderPromptCompleted'
            Value   = $null
            Message = "Import cancelled: folder not found: $selected"
        }
    }

    try {
        $resolved = (Resolve-Path -LiteralPath $selected -ErrorAction Stop | Select-Object -First 1 -ExpandProperty Path)
        return [pscustomobject]@{
            Type  = 'ImportFolderPromptCompleted'
            Value = $resolved
        }
    } catch {
        return [pscustomobject]@{
            Type    = 'ImportFolderPromptCompleted'
            Value   = $null
            Message = $_.Exception.Message
        }
    }
}

function Invoke-LauncherEffects {
    param([hashtable]$State, [object[]]$Effects)
    $results = [System.Collections.Generic.List[object]]::new()
    $chkPath = Join-Path $State.Values.OutputDir '.briefing_checkpoint.ron'

    foreach ($eff in $Effects) {
        $customDate = ''
        if ($eff -is [hashtable]) {
            if ($eff.ContainsKey('CustomDate') -and $null -ne $eff['CustomDate']) {
                $customDate = [string]$eff['CustomDate']
            }
        } elseif ($null -ne $eff) {
            $effProps = $eff.PSObject.Properties
            if ($effProps['CustomDate'] -and $null -ne $effProps['CustomDate'].Value) {
                $customDate = [string]$effProps['CustomDate'].Value
            }
        }

        $action = switch ($eff.Type) {
            'LoadDefaults'              { Invoke-LoadDefaults -FilePath (Get-DefaultsFilePath) }
            'SaveDefaults'              { Invoke-SaveDefaults -FilePath (Get-DefaultsFilePath) -Values $eff.Values }
            'ProbeCheckpointCliSupport' { Invoke-ProbeCheckpointCliSupport -HarvesterCmd $State.Runtime.HarvesterCmd -UseCargoRun $State.Runtime.UseCargoRun }
            'ReadCheckpointDisplay'     { Invoke-ReadCheckpointDisplay -CheckpointFilePath $chkPath }
            'RunCheckpointCommand'      { Invoke-RunCheckpointCommand -HarvesterCmd $State.Runtime.HarvesterCmd -UseCargoRun $State.Runtime.UseCargoRun -OutputDir $State.Values.OutputDir -ActionId $eff.ActionId -CustomDate $customDate }
            'DatePromptRequested'       { Invoke-DatePrompt }
            'ImportFolderPromptRequested' {
                Invoke-ImportFolderPrompt -CurrentValue ([string]$eff.CurrentValue)
            }
            default                     { $null }
        }
        if ($null -ne $action) { $results.Add($action) }
    }
    $results.ToArray()
}

Export-ModuleMember -Function Invoke-LauncherEffects, Invoke-LoadDefaults, Invoke-SaveDefaults, `
    Invoke-ProbeCheckpointCliSupport, Invoke-ReadCheckpointDisplay, Invoke-RunCheckpointCommand, `
    Invoke-DatePrompt, Invoke-ImportFolderPrompt
