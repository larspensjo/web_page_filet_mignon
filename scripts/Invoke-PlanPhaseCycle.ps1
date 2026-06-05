#Requires -Version 7.0

<#
Runs one AI-assisted implementation phase cycle and stops before commit.

Usage:
  pwsh .\scripts\Invoke-PlanPhaseCycle.ps1 `
    -PlanPath docs\plans\Plan.Example.md `
    -Phase "Phase 1"

Cycle:
  1. Claude elaborates the selected phase in the plan.
  2. Codex reviews the selected phase plan as structured JSON.
  3. Claude applies relevant plan-review findings to the plan.
  4. Codex implements the phase and stages implementation changes.
  5. Claude reviews staged changes as structured JSON.
  6. Codex applies relevant staged-review findings and stages fixes.
  7. Claude replaces the phase with a completion summary and proposes a commit message.

The script requires a clean worktree at the start, never commits, does not run a
fixed test command itself, stages the final plan with the implementation, and
keeps generated review/log artifacts unstaged.

Recovery:
  If a cycle is interrupted after a plan rewrite, inspect the dirty worktree and
  either keep, stage, or restore the generated plan/review/log changes before
  rerunning. The script intentionally refuses to start from a dirty worktree.

CLI assumptions:
  The script preflights the required advertised flags in `claude --help` and
  `codex exec --help`. The Codex reasoning config key is still an external
  convention: `reasoning.level`.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string]$PlanPath,

    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string]$Phase,

    [string]$RepoRoot = (Get-Location).Path,
    [string]$PlansDir,
    [string]$PromptsDir,

    [string]$ClaudeModel = 'opus',
    [string]$CodexPlanReviewModel = 'gpt-5.5',
    [string]$CodexImplementationModel = 'gpt-5.4-mini',
    [string]$CodexStagedReviewFixModel = 'gpt-5.5',

    [string]$CodexPlanReviewReasoning = 'high',
    [string]$CodexImplementationReasoning = 'medium',
    [string]$CodexStagedReviewFixReasoning = 'high',

    [ValidateRange(10000, 2000000)]
    [int]$MaxInlineDiffChars = 250000
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Set-Utf8ProcessEncoding {
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    [Console]::InputEncoding = $utf8NoBom
    [Console]::OutputEncoding = $utf8NoBom
    $global:OutputEncoding = $utf8NoBom
}

function Resolve-FullPath {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$BasePath,
        [switch]$MustExist
    )

    $candidate = if ([System.IO.Path]::IsPathRooted($Path)) {
        $Path
    } else {
        Join-Path $BasePath $Path
    }

    $fullPath = [System.IO.Path]::GetFullPath($candidate)
    if ($MustExist -and -not (Test-Path -LiteralPath $fullPath)) {
        throw "Path not found: $fullPath"
    }

    return $fullPath
}

function Ensure-Dir {
    param([Parameter(Mandatory)][string]$DirPath)

    if (-not (Test-Path -LiteralPath $DirPath)) {
        New-Item -ItemType Directory -Path $DirPath | Out-Null
    }
}

function Write-AtomicUtf8 {
    param(
        [Parameter(Mandatory)][string]$Path,
        [AllowNull()][string]$Content
    )

    $dir = Split-Path -Parent $Path
    Ensure-Dir $dir

    $tmp = Join-Path $dir ("~tmp.{0}.{1}.tmp" -f ([System.IO.Path]::GetFileName($Path)), ([guid]::NewGuid().ToString('N').Substring(0, 8)))
    [System.IO.File]::WriteAllText($tmp, $Content, [System.Text.UTF8Encoding]::new($false))
    Move-Item -Force -LiteralPath $tmp -Destination $Path
}

function Add-LogLine {
    param(
        [Parameter(Mandatory)][string]$LogPath,
        [Parameter(Mandatory)][string]$Line
    )

    $timestamp = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss')
    Add-Content -LiteralPath $LogPath -Value "[$timestamp] $Line" -Encoding utf8
}

function Normalize-Text {
    param([AllowNull()][object]$Output)

    if ($null -eq $Output) {
        return ''
    }

    return (($Output | Out-String).Trim())
}

function Invoke-Git {
    param(
        [Parameter(Mandatory)][string[]]$Arguments,
        [switch]$AllowNonZero
    )

    $gitArgs = @('-c', 'core.quotepath=false') + $Arguments
    $tmpOut = [System.IO.Path]::GetTempFileName()
    $tmpErr = [System.IO.Path]::GetTempFileName()
    Push-Location $script:RepoRoot
    try {
        & git @gitArgs > $tmpOut 2> $tmpErr
        $exitCode = $LASTEXITCODE
    } finally {
        Pop-Location
    }

    try {
        $stdout = if (Test-Path -LiteralPath $tmpOut) { Read-TextFile $tmpOut } else { '' }
        $stderr = if (Test-Path -LiteralPath $tmpErr) { Read-TextFile $tmpErr } else { '' }
        $text = Normalize-Text $stdout
        $errorText = Normalize-Text $stderr

        if (-not $AllowNonZero -and $exitCode -ne 0) {
            throw "git $($Arguments -join ' ') failed with exit code $exitCode.`nSTDERR:`n$errorText`nSTDOUT:`n$text"
        }

        [pscustomobject]@{
            ExitCode = $exitCode
            Text     = $text
            Stderr   = $errorText
        }
    } finally {
        Remove-Item -LiteralPath $tmpOut -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $tmpErr -ErrorAction SilentlyContinue
    }
}

function Assert-CliExists {
    param([Parameter(Mandatory)][string]$CliName)

    $cmd = Get-Command $CliName -ErrorAction SilentlyContinue
    if (-not $cmd) {
        throw "CLI '$CliName' not found in PATH."
    }
}

function Assert-CleanWorktree {
    $status = (Invoke-Git -Arguments @('status', '--porcelain=v1')).Text
    if (-not [string]::IsNullOrWhiteSpace($status)) {
        throw "Workspace must be clean before starting a phase cycle. If this follows an interrupted cycle, inspect the dirty files and either keep, stage, or restore them before rerunning.`n$status"
    }
}

function Get-CliHelpText {
    param(
        [Parameter(Mandatory)][string]$Tool,
        [Parameter(Mandatory)][string[]]$Arguments
    )

    $output = & $Tool @Arguments 2>&1
    $exitCode = $LASTEXITCODE
    $text = Normalize-Text $output
    if ($exitCode -ne 0) {
        throw "Could not inspect '$Tool $($Arguments -join ' ')' help output. Exit code $exitCode.`n$text"
    }

    return $text
}

function Assert-HelpContains {
    param(
        [Parameter(Mandatory)][string]$Tool,
        [Parameter(Mandatory)][string[]]$Arguments,
        [Parameter(Mandatory)][string[]]$ExpectedFlags
    )

    $helpText = Get-CliHelpText -Tool $Tool -Arguments $Arguments
    foreach ($flag in $ExpectedFlags) {
        if ($helpText.IndexOf($flag, [System.StringComparison]::Ordinal) -lt 0) {
            throw "CLI '$Tool' help output does not advertise required flag '$flag'. Inspect or update scripts/Invoke-PlanPhaseCycle.ps1 before running the cycle."
        }
    }
}

function Assert-CliFlagSupport {
    Assert-HelpContains -Tool 'claude' -Arguments @('--help') -ExpectedFlags @(
        '-p',
        '--no-session-persistence',
        '--input-format',
        '--model',
        '--permission-mode'
    )

    Assert-HelpContains -Tool 'codex' -Arguments @('exec', '--help') -ExpectedFlags @(
        '--cd',
        '--color',
        '--sandbox',
        '--model',
        '--config',
        '--output-schema',
        '--output-last-message'
    )
}

function Assert-PhaseExists {
    param(
        [Parameter(Mandatory)][string]$PlanText,
        [Parameter(Mandatory)][string]$PhaseText
    )

    if ($PlanText.IndexOf($PhaseText, [System.StringComparison]::OrdinalIgnoreCase) -lt 0) {
        Write-Warning "Phase string '$PhaseText' was not found in the plan. Stopping before invoking any AI CLI."
        throw "Phase not found in plan: $PhaseText"
    }
}

function Assert-PathUnderRepo {
    param([Parameter(Mandatory)][string]$Path)

    $relative = [System.IO.Path]::GetRelativePath($script:RepoRoot, $Path)
    if ($relative.StartsWith('..') -or [System.IO.Path]::IsPathRooted($relative)) {
        throw "Path must be inside repo root '$script:RepoRoot': $Path"
    }
}

function Get-GitPath {
    param([Parameter(Mandatory)][string]$Path)

    $relative = [System.IO.Path]::GetRelativePath($script:RepoRoot, $Path)
    return ($relative -replace '\\', '/')
}

function Get-PlanIdFromPath {
    param([Parameter(Mandatory)][string]$Path)

    $fileName = [System.IO.Path]::GetFileName($Path)
    $match = [regex]::Match($fileName, '^(?i)Plan\.(?<id>.+?)\.md$')
    if ($match.Success) {
        return $match.Groups['id'].Value
    }

    return [System.IO.Path]::GetFileNameWithoutExtension($fileName)
}

function New-SafeFileSegment {
    param([Parameter(Mandatory)][string]$Text)

    $segment = $Text.Trim()
    foreach ($char in [System.IO.Path]::GetInvalidFileNameChars()) {
        $segment = $segment.Replace([string]$char, '-')
    }

    $segment = [regex]::Replace($segment, '\s+', '')
    $segment = [regex]::Replace($segment, '[^A-Za-z0-9_.-]', '-')
    $segment = $segment.Trim('.-')
    if ([string]::IsNullOrWhiteSpace($segment)) {
        return 'Phase'
    }

    return $segment
}

function Read-TextFile {
    param([Parameter(Mandatory)][string]$Path)

    Get-Content -LiteralPath $Path -Raw -Encoding utf8
}

function Read-PromptTemplate {
    param([Parameter(Mandatory)][string]$Name)

    $path = Join-Path $script:PromptsDir $Name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Prompt template not found: $path"
    }

    Read-TextFile $path
}

function Expand-PromptTemplate {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][hashtable]$Variables
    )

    $text = Read-PromptTemplate $Name
    foreach ($key in $Variables.Keys) {
        $placeholder = '{{' + $key + '}}'
        $text = $text.Replace($placeholder, [string]$Variables[$key])
    }

    return $text
}

function Extract-MarkedSection {
    param(
        [Parameter(Mandatory)][string]$Text,
        [Parameter(Mandatory)][string]$SectionName
    )

    $escapedName = [regex]::Escape($SectionName)
    $pattern = "(?s)--- BEGIN $escapedName ---\s*(.*?)\s*--- END $escapedName ---"
    $match = [regex]::Match($Text, $pattern)
    if (-not $match.Success) {
        throw "Expected marked section not found: $SectionName"
    }

    return $match.Groups[1].Value.Trim()
}

function Get-ObjectProperty {
    param(
        [AllowNull()][object]$Object,
        [Parameter(Mandatory)][string]$Name,
        [AllowNull()][object]$Default = $null
    )

    if ($null -eq $Object) {
        return $Default
    }

    $property = $Object.PSObject.Properties | Where-Object { $_.Name -eq $Name } | Select-Object -First 1
    if ($null -eq $property) {
        return $Default
    }

    return $property.Value
}

function ConvertFrom-AgentJson {
    param([Parameter(Mandatory)][string]$Text)

    $jsonText = $Text.Trim()
    $fenced = [regex]::Match($jsonText, '(?s)^\s*```(?:json)?\s*(.*?)\s*```\s*$')
    if ($fenced.Success) {
        $jsonText = $fenced.Groups[1].Value.Trim()
    }

    try {
        return ($jsonText | ConvertFrom-Json -Depth 50)
    } catch {
        $start = $jsonText.IndexOf('{')
        $end = $jsonText.LastIndexOf('}')
        if ($start -ge 0 -and $end -gt $start) {
            $candidate = $jsonText.Substring($start, $end - $start + 1)
            return ($candidate | ConvertFrom-Json -Depth 50)
        }

        throw
    }
}

function ConvertTo-PrettyJson {
    param([Parameter(Mandatory)][object]$Value)

    $Value | ConvertTo-Json -Depth 50
}

function Write-StepResultSummary {
    param(
        [Parameter(Mandatory)][object]$StepResult,
        [Parameter(Mandatory)][string]$ArtifactPath
    )

    $step = Get-ObjectProperty -Object $StepResult -Name 'step' -Default 'unknown'
    $status = Get-ObjectProperty -Object $StepResult -Name 'status' -Default 'unknown'
    $summary = Get-ObjectProperty -Object $StepResult -Name 'summary' -Default ''
    $changedFiles = @(Get-ObjectProperty -Object $StepResult -Name 'changed_files' -Default @())
    $verification = @(Get-ObjectProperty -Object $StepResult -Name 'verification' -Default @())

    Write-Host "  Step result: $step => $status"
    if (-not [string]::IsNullOrWhiteSpace($summary)) {
        Write-Host "  Summary: $summary"
    }
    Write-Host "  Result:  $ArtifactPath"
    if ($changedFiles.Count -gt 0) {
        Write-Host "  Changed files reported: $($changedFiles.Count)"
    }
    if ($verification.Count -gt 0) {
        Write-Host "  Verification items reported: $($verification.Count)"
    }
}

function Write-StepResultArtifact {
    param(
        [Parameter(Mandatory)][string]$Output,
        [Parameter(Mandatory)][string]$ArtifactPath,
        [Parameter(Mandatory)][string]$LogPath
    )

    $rawPath = [System.IO.Path]::ChangeExtension($ArtifactPath, '.raw.txt')
    try {
        $stepResult = ConvertFrom-AgentJson -Text $Output
    } catch {
        Write-AtomicUtf8 -Path $rawPath -Content $Output
        Add-LogLine -LogPath $LogPath -Line "Saved unparsable step result output: $rawPath"
        throw "Step result output was not valid JSON. Raw output saved to $rawPath"
    }

    Write-AtomicUtf8 -Path $ArtifactPath -Content (ConvertTo-PrettyJson -Value $stepResult)
    Remove-Item -LiteralPath $rawPath -ErrorAction SilentlyContinue
    Add-LogLine -LogPath $LogPath -Line "Saved step result JSON: $ArtifactPath"
    Write-StepResultSummary -StepResult $stepResult -ArtifactPath $ArtifactPath

    return $stepResult
}

function Assert-StepResultSuccess {
    param(
        [Parameter(Mandatory)][object]$StepResult,
        [Parameter(Mandatory)][string]$ArtifactPath
    )

    $status = [string](Get-ObjectProperty -Object $StepResult -Name 'status' -Default '')
    $manualFeedback = Get-ObjectProperty -Object $StepResult -Name 'manual_feedback_required' -Default $false
    if ($status -eq 'success' -and $manualFeedback -ne $true) {
        return
    }

    $reason = Get-ObjectProperty -Object $StepResult -Name 'manual_feedback_reason' -Default ''
    if (-not [string]::IsNullOrWhiteSpace($reason)) {
        Write-Warning $reason
    }

    throw "Step did not report success. See $ArtifactPath"
}

function Get-FindingsForSeverity {
    param(
        [Parameter(Mandatory)][object]$Review,
        [Parameter(Mandatory)][string]$Severity
    )

    $bySeverity = Get-ObjectProperty -Object $Review -Name 'findings_by_severity' -Default $null
    if ($null -ne $bySeverity) {
        $findings = Get-ObjectProperty -Object $bySeverity -Name $Severity -Default @()
        return @($findings)
    }

    $flatFindings = @(Get-ObjectProperty -Object $Review -Name 'findings' -Default @())
    return @($flatFindings | Where-Object {
        ([string](Get-ObjectProperty -Object $_ -Name 'severity' -Default '')).ToLowerInvariant() -eq $Severity
    })
}

function Get-AllReviewFindings {
    param([Parameter(Mandatory)][object]$Review)

    $all = @()
    foreach ($severity in @('blocker', 'high', 'medium', 'low', 'nit')) {
        $all += @(Get-FindingsForSeverity -Review $Review -Severity $severity)
    }

    if ($all.Count -gt 0) {
        return $all
    }

    return @(Get-ObjectProperty -Object $Review -Name 'findings' -Default @())
}

function Test-ManualFeedbackRequired {
    param([Parameter(Mandatory)][object]$Review)

    $decision = [string](Get-ObjectProperty -Object $Review -Name 'decision' -Default '')
    if ($decision -eq 'stop_for_manual_feedback') {
        return $true
    }

    $manualFeedback = Get-ObjectProperty -Object $Review -Name 'manual_feedback_required' -Default $false
    if ($manualFeedback -eq $true) {
        return $true
    }

    foreach ($finding in @(Get-AllReviewFindings -Review $Review)) {
        if ((Get-ObjectProperty -Object $finding -Name 'requires_manual_feedback' -Default $false) -eq $true) {
            return $true
        }
    }

    foreach ($question in @(Get-ObjectProperty -Object $Review -Name 'questions' -Default @())) {
        if ((Get-ObjectProperty -Object $question -Name 'blocking' -Default $false) -eq $true) {
            return $true
        }
    }

    return $false
}

function Write-ReviewSummary {
    param(
        [Parameter(Mandatory)][object]$Review,
        [Parameter(Mandatory)][string]$ArtifactPath
    )

    $decision = Get-ObjectProperty -Object $Review -Name 'decision' -Default 'unknown'
    $summary = Get-ObjectProperty -Object $Review -Name 'summary' -Default ''
    $counts = @{}
    foreach ($severity in @('blocker', 'high', 'medium', 'low', 'nit')) {
        $counts[$severity] = @(Get-FindingsForSeverity -Review $Review -Severity $severity).Count
    }

    Write-Host ("  Decision: {0}; findings: blocker={1}, high={2}, medium={3}, low={4}, nit={5}" -f `
            $decision, $counts['blocker'], $counts['high'], $counts['medium'], $counts['low'], $counts['nit'])
    if (-not [string]::IsNullOrWhiteSpace($summary)) {
        Write-Host "  Summary: $summary"
    }
    Write-Host "  Review:  $ArtifactPath"

    $important = @()
    $important += @(Get-FindingsForSeverity -Review $Review -Severity 'blocker')
    $important += @(Get-FindingsForSeverity -Review $Review -Severity 'high')
    foreach ($finding in @($important | Select-Object -First 5)) {
        $title = Get-ObjectProperty -Object $finding -Name 'title' -Default ''
        if (-not [string]::IsNullOrWhiteSpace($title)) {
            Write-Host "  - $title"
        }
    }
}

function Invoke-Cli {
    param(
        [Parameter(Mandatory)]
        [ValidateSet('claude', 'codex')]
        [string]$Tool,

        [Parameter(Mandatory)]
        [string]$Prompt,

        [Parameter(Mandatory)]
        [string]$WorkingDir,

        [AllowNull()][string]$Model,
        [AllowNull()][string]$PermissionMode,
        [AllowNull()][string]$Sandbox,
        [AllowNull()][string]$Reasoning,
        [AllowNull()][string]$OutputLastMessagePath,
        [AllowNull()][string]$OutputSchemaPath
    )

    Assert-CliExists $Tool

    $cliArgs = @()
    switch ($Tool) {
        'codex' {
            $cliArgs += @('exec', '--cd', $WorkingDir, '--color', 'never')
            if (-not [string]::IsNullOrWhiteSpace($Sandbox)) {
                $cliArgs += @('--sandbox', $Sandbox)
            }
            if (-not [string]::IsNullOrWhiteSpace($Model)) {
                $cliArgs += @('--model', $Model)
            }
            if (-not [string]::IsNullOrWhiteSpace($Reasoning)) {
                $cliArgs += @('-c', "reasoning.level=`"$Reasoning`"")
            }
            if (-not [string]::IsNullOrWhiteSpace($OutputSchemaPath)) {
                $cliArgs += @('--output-schema', $OutputSchemaPath)
            }
            if (-not [string]::IsNullOrWhiteSpace($OutputLastMessagePath)) {
                Ensure-Dir (Split-Path -Parent $OutputLastMessagePath)
                Remove-Item -LiteralPath $OutputLastMessagePath -ErrorAction SilentlyContinue
                $cliArgs += @('--output-last-message', $OutputLastMessagePath)
            }
            $cliArgs += '-'
        }
        'claude' {
            $cliArgs += @('-p', '--no-session-persistence', '--input-format', 'text')
            if (-not [string]::IsNullOrWhiteSpace($Model)) {
                $cliArgs += @('--model', $Model)
            }
            if (-not [string]::IsNullOrWhiteSpace($PermissionMode)) {
                $cliArgs += @('--permission-mode', $PermissionMode)
            }
        }
    }

    $tmpOut = [System.IO.Path]::GetTempFileName()
    $tmpErr = [System.IO.Path]::GetTempFileName()
    Push-Location $WorkingDir
    try {
        $Prompt | & $Tool @cliArgs > $tmpOut 2> $tmpErr
        $exitCode = $LASTEXITCODE
    } finally {
        Pop-Location
    }

    try {
        $stdout = if (Test-Path -LiteralPath $tmpOut) { Read-TextFile $tmpOut } else { '' }
        $stderr = if (Test-Path -LiteralPath $tmpErr) { Read-TextFile $tmpErr } else { '' }

        if ($exitCode -ne 0) {
            throw "CLI '$Tool' exited with code $exitCode.`nSTDERR:`n$stderr`nSTDOUT:`n$stdout"
        }

        $lastMessage = ''
        if (-not [string]::IsNullOrWhiteSpace($OutputLastMessagePath) -and (Test-Path -LiteralPath $OutputLastMessagePath)) {
            $lastMessage = Read-TextFile $OutputLastMessagePath
        }

        $result = if (-not [string]::IsNullOrWhiteSpace($lastMessage)) {
            $lastMessage
        } else {
            $stdout
        }

        $result = $result.Trim()
        if ([string]::IsNullOrWhiteSpace($result)) {
            throw "CLI '$Tool' returned empty output.`nSTDERR:`n$stderr"
        }

        return $result
    } finally {
        Remove-Item -LiteralPath $tmpOut -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $tmpErr -ErrorAction SilentlyContinue
    }
}

function Invoke-ClaudePlanRewrite {
    param(
        [Parameter(Mandatory)][string]$PromptName,
        [Parameter(Mandatory)][hashtable]$Variables,
        [Parameter(Mandatory)][string]$LogPath,
        [Parameter(Mandatory)][string]$StepName
    )

    $prompt = Expand-PromptTemplate -Name $PromptName -Variables $Variables
    $output = Invoke-Cli -Tool 'claude' -Prompt $prompt -WorkingDir $script:RepoRoot -Model $script:ClaudeModel -PermissionMode 'plan' `
        -Sandbox $null -Reasoning $null -OutputLastMessagePath $null -OutputSchemaPath $null

    try {
        $updatedPlan = Extract-MarkedSection -Text $output -SectionName 'UPDATED PLAN'
    } catch {
        $rawPath = Join-Path $script:PlansDir ("Cycle.$script:PlanId.$script:PhaseSlug.$StepName.raw.txt")
        Write-AtomicUtf8 -Path $rawPath -Content $output
        Add-LogLine -LogPath $LogPath -Line "Saved unparsable plan rewrite output: $rawPath"
        throw
    }

    Assert-PhaseExists -PlanText $updatedPlan -PhaseText $script:Phase
    Write-AtomicUtf8 -Path $script:PlanPath -Content $updatedPlan
    Add-LogLine -LogPath $LogPath -Line "Updated plan written by $StepName"

    return $output
}

function Invoke-ReviewJsonStep {
    param(
        [Parameter(Mandatory)]
        [ValidateSet('claude', 'codex')]
        [string]$Tool,

        [Parameter(Mandatory)][string]$PromptName,
        [Parameter(Mandatory)][hashtable]$Variables,
        [Parameter(Mandatory)][string]$ArtifactPath,
        [Parameter(Mandatory)][string]$LogPath,
        [AllowNull()][string]$Model,
        [AllowNull()][string]$Reasoning,
        [AllowNull()][string]$Sandbox,
        [AllowNull()][string]$OutputSchemaPath
    )

    $prompt = Expand-PromptTemplate -Name $PromptName -Variables $Variables
    $rawPath = [System.IO.Path]::ChangeExtension($ArtifactPath, '.raw.txt')
    $outputLastMessage = if ($Tool -eq 'codex') { $rawPath } else { $null }

    $output = Invoke-Cli -Tool $Tool -Prompt $prompt -WorkingDir $script:RepoRoot -Model $Model -PermissionMode 'plan' `
        -Sandbox $Sandbox -Reasoning $Reasoning -OutputLastMessagePath $outputLastMessage -OutputSchemaPath $OutputSchemaPath

    try {
        $review = ConvertFrom-AgentJson -Text $output
    } catch {
        Write-AtomicUtf8 -Path $rawPath -Content $output
        Add-LogLine -LogPath $LogPath -Line "Saved unparsable review output: $rawPath"
        throw "Review output was not valid JSON. Raw output saved to $rawPath"
    }

    Write-AtomicUtf8 -Path $ArtifactPath -Content (ConvertTo-PrettyJson -Value $review)
    Remove-Item -LiteralPath $rawPath -ErrorAction SilentlyContinue
    Add-LogLine -LogPath $LogPath -Line "Saved review JSON: $ArtifactPath"

    Write-ReviewSummary -Review $review -ArtifactPath $ArtifactPath
    if (Test-ManualFeedbackRequired -Review $review) {
        $reason = Get-ObjectProperty -Object $review -Name 'manual_feedback_reason' -Default ''
        if (-not [string]::IsNullOrWhiteSpace($reason)) {
            Write-Warning $reason
        }
        throw "Manual feedback required by review: $ArtifactPath"
    }

    return $review
}

function Get-StagedDiffContext {
    $status = (Invoke-Git -Arguments @('status', '--short')).Text
    $nameStatus = (Invoke-Git -Arguments @('diff', '--cached', '--name-status', '--')).Text
    $stat = (Invoke-Git -Arguments @('diff', '--cached', '--stat', '--')).Text
    $diff = (Invoke-Git -Arguments @('diff', '--cached', '--no-ext-diff', '--')).Text

    $diffNote = 'Full staged diff is included.'
    if ($diff.Length -gt $script:MaxInlineDiffChars) {
        $diff = $diff.Substring(0, $script:MaxInlineDiffChars)
        $diffNote = "Staged diff was truncated to $script:MaxInlineDiffChars characters. Inspect the local staged diff directly if needed."
    }

@"
--- BEGIN GIT STATUS ---
$status
--- END GIT STATUS ---

--- BEGIN STAGED NAME STATUS ---
$nameStatus
--- END STAGED NAME STATUS ---

--- BEGIN STAGED STAT ---
$stat
--- END STAGED STAT ---

--- BEGIN STAGED DIFF NOTE ---
$diffNote
--- END STAGED DIFF NOTE ---

--- BEGIN STAGED DIFF ---
$diff
--- END STAGED DIFF ---
"@
}

function Unstage-PathsIfNeeded {
    param([Parameter(Mandatory)][string[]]$Paths)

    $staged = @((Invoke-Git -Arguments @('diff', '--cached', '--name-only', '--')).Text -split "`r?`n" | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_)
    })
    if ($staged.Count -eq 0) {
        return
    }

    $stagedSet = @{}
    foreach ($path in $staged) {
        $stagedSet[$path.Replace('\', '/')] = $true
    }

    $toUnstage = @()
    foreach ($path in $Paths) {
        if ([string]::IsNullOrWhiteSpace($path)) {
            continue
        }

        $gitPath = Get-GitPath $path
        if ($stagedSet.ContainsKey($gitPath)) {
            $toUnstage += $gitPath
        }
    }

    if ($toUnstage.Count -gt 0) {
        Invoke-Git -Arguments (@('restore', '--staged', '--') + $toUnstage) | Out-Null
    }
}

function Assert-StagedChangesExist {
    param([Parameter(Mandatory)][string]$Context)

    $result = Invoke-Git -Arguments @('diff', '--cached', '--quiet', '--') -AllowNonZero
    if ($result.ExitCode -eq 0) {
        throw "No staged implementation changes found after $Context."
    }
    if ($result.ExitCode -ne 1) {
        throw "git diff --cached --quiet failed with exit code $($result.ExitCode).`n$($result.Text)"
    }
}

function Stage-Plan {
    $planGitPath = Get-GitPath $script:PlanPath
    Invoke-Git -Arguments @('add', '--', $planGitPath) | Out-Null
}

function Write-Step {
    param(
        [Parameter(Mandatory)][int]$Number,
        [Parameter(Mandatory)][string]$Message,
        [Parameter(Mandatory)][string]$LogPath
    )

    $line = "[$Number/7] $Message"
    Write-Host ''
    Write-Host $line
    Add-LogLine -LogPath $LogPath -Line $line
}

Set-Utf8ProcessEncoding

$initialRoot = Resolve-FullPath -Path $RepoRoot -BasePath (Get-Location).Path -MustExist
$PlanPath = Resolve-FullPath -Path $PlanPath -BasePath $initialRoot -MustExist

Push-Location $initialRoot
try {
    $gitRootText = (& git -c core.quotepath=false rev-parse --show-toplevel 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($gitRootText)) {
        throw "Could not resolve git repository root from $initialRoot.`n$gitRootText"
    }
} finally {
    Pop-Location
}

$script:RepoRoot = [System.IO.Path]::GetFullPath($gitRootText)
$script:PlanPath = $PlanPath
Assert-PathUnderRepo $script:PlanPath

if ([string]::IsNullOrWhiteSpace($PlansDir)) {
    $PlansDir = Join-Path $script:RepoRoot 'docs\plans'
} else {
    $PlansDir = Resolve-FullPath -Path $PlansDir -BasePath $script:RepoRoot
}
$script:PlansDir = $PlansDir

if ([string]::IsNullOrWhiteSpace($PromptsDir)) {
    $PromptsDir = Join-Path $script:RepoRoot 'scripts\prompts'
} else {
    $PromptsDir = Resolve-FullPath -Path $PromptsDir -BasePath $script:RepoRoot
}
$script:PromptsDir = $PromptsDir

$script:ClaudeModel = $ClaudeModel
$script:Phase = $Phase
$script:MaxInlineDiffChars = $MaxInlineDiffChars
$script:PlanId = Get-PlanIdFromPath $script:PlanPath
$script:PhaseSlug = New-SafeFileSegment $Phase

$reviewSchemaPath = Join-Path $script:PromptsDir 'phase-cycle-review.schema.json'
if (-not (Test-Path -LiteralPath $reviewSchemaPath -PathType Leaf)) {
    throw "Review schema not found: $reviewSchemaPath"
}
$reviewSchemaText = Read-TextFile $reviewSchemaPath
$stepResultSchemaPath = Join-Path $script:PromptsDir 'phase-cycle-step-result.schema.json'
if (-not (Test-Path -LiteralPath $stepResultSchemaPath -PathType Leaf)) {
    throw "Step result schema not found: $stepResultSchemaPath"
}
$stepResultSchemaText = Read-TextFile $stepResultSchemaPath

Assert-CliExists 'claude'
Assert-CliExists 'codex'
Assert-CliFlagSupport

$initialPlanText = Read-TextFile $script:PlanPath
Assert-PhaseExists -PlanText $initialPlanText -PhaseText $Phase
Assert-CleanWorktree

Ensure-Dir $script:PlansDir
$logPath = Join-Path $script:PlansDir ("Cycle.$script:PlanId.$script:PhaseSlug.log")
$planReviewPath = Join-Path $script:PlansDir ("Review.$script:PlanId.$script:PhaseSlug.Plan.codex.json")
$stagedReviewPath = Join-Path $script:PlansDir ("Review.$script:PlanId.$script:PhaseSlug.Staged.claude.json")
$implementationResultPath = Join-Path $script:PlansDir ("Cycle.$script:PlanId.$script:PhaseSlug.Step4.Implementation.codex.json")
$stagedReviewFixResultPath = Join-Path $script:PlansDir ("Cycle.$script:PlanId.$script:PhaseSlug.Step6.StagedReviewFix.codex.json")
$completionResultPath = Join-Path $script:PlansDir ("Cycle.$script:PlanId.$script:PhaseSlug.Step7.Completion.claude.json")
$generatedArtifacts = @(
    $logPath,
    $planReviewPath,
    $stagedReviewPath,
    $implementationResultPath,
    $stagedReviewFixResultPath,
    $completionResultPath
)

Write-AtomicUtf8 -Path $logPath -Content @"
Started: $(Get-Date)
RepoRoot: $script:RepoRoot
PlanPath: $script:PlanPath
Phase: $Phase
ClaudeModel: $ClaudeModel
CodexPlanReviewModel: $CodexPlanReviewModel
CodexImplementationModel: $CodexImplementationModel
CodexStagedReviewFixModel: $CodexStagedReviewFixModel

"@

Write-Host "Plan phase cycle"
Write-Host "  Plan:  $script:PlanPath"
Write-Host "  Phase: $Phase"
Write-Host "  Log:   $logPath"

Write-Step -Number 1 -Message 'Claude elaborates the selected phase in the plan.' -LogPath $logPath
$planText = Read-TextFile $script:PlanPath
Invoke-ClaudePlanRewrite -PromptName 'phase-cycle-elaborate-plan.md' -Variables @{
    PLAN_PATH = Get-GitPath $script:PlanPath
    PLAN_TEXT = $planText
    PHASE = $Phase
} -LogPath $logPath -StepName 'Step1.ElaboratePlan' | Out-Null

Write-Step -Number 2 -Message 'Codex reviews the selected phase plan.' -LogPath $logPath
$planText = Read-TextFile $script:PlanPath
Invoke-ReviewJsonStep -Tool 'codex' -PromptName 'phase-cycle-review-plan.md' -Variables @{
    PLAN_PATH = Get-GitPath $script:PlanPath
    PLAN_TEXT = $planText
    PHASE = $Phase
    REVIEW_SCHEMA = $reviewSchemaText
} -ArtifactPath $planReviewPath -LogPath $logPath -Model $CodexPlanReviewModel -Reasoning $CodexPlanReviewReasoning `
    -Sandbox 'read-only' -OutputSchemaPath $reviewSchemaPath | Out-Null

Write-Step -Number 3 -Message 'Claude applies relevant plan-review findings.' -LogPath $logPath
$planText = Read-TextFile $script:PlanPath
$planReviewJson = Read-TextFile $planReviewPath
Invoke-ClaudePlanRewrite -PromptName 'phase-cycle-apply-plan-review.md' -Variables @{
    PLAN_PATH = Get-GitPath $script:PlanPath
    PLAN_TEXT = $planText
    PHASE = $Phase
    REVIEW_JSON = $planReviewJson
} -LogPath $logPath -StepName 'Step3.ApplyPlanReview' | Out-Null

Write-Step -Number 4 -Message 'Codex implements the selected phase and stages implementation changes.' -LogPath $logPath
$planText = Read-TextFile $script:PlanPath
$implementationPrompt = Expand-PromptTemplate -Name 'phase-cycle-implement-phase.md' -Variables @{
    PLAN_PATH = Get-GitPath $script:PlanPath
    PLAN_TEXT = $planText
    PHASE = $Phase
    PLAN_REVIEW_PATH = Get-GitPath $planReviewPath
    PLAN_REVIEW_JSON = $planReviewJson
    STEP_RESULT_SCHEMA = $stepResultSchemaText
}
$implementationRawPath = [System.IO.Path]::ChangeExtension($implementationResultPath, '.raw.txt')
$implementationOutput = Invoke-Cli -Tool 'codex' -Prompt $implementationPrompt -WorkingDir $script:RepoRoot -Model $CodexImplementationModel `
    -PermissionMode $null -Sandbox 'workspace-write' -Reasoning $CodexImplementationReasoning `
    -OutputLastMessagePath $implementationRawPath -OutputSchemaPath $stepResultSchemaPath
$implementationResult = Write-StepResultArtifact -Output $implementationOutput -ArtifactPath $implementationResultPath -LogPath $logPath
Assert-StepResultSuccess -StepResult $implementationResult -ArtifactPath $implementationResultPath
Unstage-PathsIfNeeded -Paths (@($script:PlanPath) + $generatedArtifacts)
Assert-StagedChangesExist -Context 'implementation'
Add-LogLine -LogPath $logPath -Line 'Implementation step completed with staged changes.'

Write-Step -Number 5 -Message 'Claude reviews the staged changes.' -LogPath $logPath
$planText = Read-TextFile $script:PlanPath
$stagedDiffContext = Get-StagedDiffContext
Invoke-ReviewJsonStep -Tool 'claude' -PromptName 'phase-cycle-review-staged.md' -Variables @{
    PLAN_PATH = Get-GitPath $script:PlanPath
    PLAN_TEXT = $planText
    PHASE = $Phase
    STAGED_DIFF_CONTEXT = $stagedDiffContext
    REVIEW_SCHEMA = $reviewSchemaText
} -ArtifactPath $stagedReviewPath -LogPath $logPath -Model $ClaudeModel -Reasoning $null `
    -Sandbox $null -OutputSchemaPath $null | Out-Null

Write-Step -Number 6 -Message 'Codex applies relevant staged-review findings.' -LogPath $logPath
$stagedReviewJson = Read-TextFile $stagedReviewPath
$stagedDiffContext = Get-StagedDiffContext
$fixPrompt = Expand-PromptTemplate -Name 'phase-cycle-apply-staged-review.md' -Variables @{
    PLAN_PATH = Get-GitPath $script:PlanPath
    PLAN_TEXT = Read-TextFile $script:PlanPath
    PHASE = $Phase
    STAGED_REVIEW_PATH = Get-GitPath $stagedReviewPath
    STAGED_REVIEW_JSON = $stagedReviewJson
    STAGED_DIFF_CONTEXT = $stagedDiffContext
    STEP_RESULT_SCHEMA = $stepResultSchemaText
}
$stagedReviewFixRawPath = [System.IO.Path]::ChangeExtension($stagedReviewFixResultPath, '.raw.txt')
$stagedReviewFixOutput = Invoke-Cli -Tool 'codex' -Prompt $fixPrompt -WorkingDir $script:RepoRoot -Model $CodexStagedReviewFixModel `
    -PermissionMode $null -Sandbox 'workspace-write' -Reasoning $CodexStagedReviewFixReasoning `
    -OutputLastMessagePath $stagedReviewFixRawPath -OutputSchemaPath $stepResultSchemaPath
$stagedReviewFixResult = Write-StepResultArtifact -Output $stagedReviewFixOutput -ArtifactPath $stagedReviewFixResultPath -LogPath $logPath
Assert-StepResultSuccess -StepResult $stagedReviewFixResult -ArtifactPath $stagedReviewFixResultPath
Unstage-PathsIfNeeded -Paths (@($script:PlanPath) + $generatedArtifacts)
Assert-StagedChangesExist -Context 'staged-review fix'
Add-LogLine -LogPath $logPath -Line 'Staged-review fix step completed.'

Write-Step -Number 7 -Message 'Claude marks the phase complete in the plan and proposes a commit message.' -LogPath $logPath
$stagedDiffContext = Get-StagedDiffContext
$completionPrompt = Expand-PromptTemplate -Name 'phase-cycle-complete-plan.md' -Variables @{
    PLAN_PATH = Get-GitPath $script:PlanPath
    PLAN_TEXT = Read-TextFile $script:PlanPath
    PHASE = $Phase
    STAGED_DIFF_CONTEXT = $stagedDiffContext
    STEP_RESULT_SCHEMA = $stepResultSchemaText
}
$completionOutput = Invoke-Cli -Tool 'claude' -Prompt $completionPrompt -WorkingDir $script:RepoRoot -Model $ClaudeModel -PermissionMode 'plan' `
    -Sandbox $null -Reasoning $null -OutputLastMessagePath $null -OutputSchemaPath $null
try {
    $completedPlan = Extract-MarkedSection -Text $completionOutput -SectionName 'UPDATED PLAN'
    $commitMessage = Extract-MarkedSection -Text $completionOutput -SectionName 'SUGGESTED COMMIT MESSAGE'
    $completionResultJson = Extract-MarkedSection -Text $completionOutput -SectionName 'STEP RESULT JSON'
} catch {
    $rawPath = Join-Path $script:PlansDir ("Cycle.$script:PlanId.$script:PhaseSlug.Step7.CompletePlan.raw.txt")
    Write-AtomicUtf8 -Path $rawPath -Content $completionOutput
    Add-LogLine -LogPath $logPath -Line "Saved unparsable completion output: $rawPath"
    throw
}

$completionResult = Write-StepResultArtifact -Output $completionResultJson -ArtifactPath $completionResultPath -LogPath $logPath
Assert-StepResultSuccess -StepResult $completionResult -ArtifactPath $completionResultPath
Assert-PhaseExists -PlanText $completedPlan -PhaseText $Phase
Write-AtomicUtf8 -Path $script:PlanPath -Content $completedPlan
Stage-Plan
Unstage-PathsIfNeeded -Paths $generatedArtifacts
Add-LogLine -LogPath $logPath -Line 'Final plan update staged.'
Add-LogLine -LogPath $logPath -Line "Suggested commit message: $commitMessage"
Add-LogLine -LogPath $logPath -Line "Completed: $(Get-Date)"

$stagedStatus = (Invoke-Git -Arguments @('diff', '--cached', '--name-status', '--')).Text
$untrackedArtifacts = (Invoke-Git -Arguments @('status', '--short', '--', (Get-GitPath $script:PlansDir))).Text

Write-Host ''
Write-Host 'Cycle complete. Review/log artifacts were left unstaged.'
Write-Host "Plan review:       $planReviewPath"
Write-Host "Implementation:    $implementationResultPath"
Write-Host "Staged review:     $stagedReviewPath"
Write-Host "Review fix:        $stagedReviewFixResultPath"
Write-Host "Plan completion:   $completionResultPath"
Write-Host "Log:               $logPath"
Write-Host ''
Write-Host 'Staged changes:'
Write-Host $stagedStatus
if (-not [string]::IsNullOrWhiteSpace($untrackedArtifacts)) {
    Write-Host ''
    Write-Host 'Plan-directory status, including unstaged artifacts:'
    Write-Host $untrackedArtifacts
}
Write-Host ''
Write-Host 'Suggested git commit message:'
Write-Host $commitMessage
