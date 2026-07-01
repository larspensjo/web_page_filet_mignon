#Requires -Version 7.0
Set-StrictMode -Version Latest

# AgentCli — generic toolkit shared by the plan-automation and file-shrink
# scripts. Keep this module domain-agnostic: no Harvester/plan/shrink terms.

$script:AgentCliVersion = '1.0.0'

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
        [Parameter(Mandatory)][AllowEmptyString()][string]$Line
    )

    $timestamp = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss')
    Add-Content -LiteralPath $LogPath -Value "[$timestamp] $Line" -Encoding utf8
}

function Normalize-Text {
    param([AllowNull()][object]$Output)

    if ($null -eq $Output) {
        return ''
    }

    return (($Output | Out-String).TrimEnd())
}

function Read-TextFile {
    param([Parameter(Mandatory)][string]$Path)

    Get-Content -LiteralPath $Path -Raw -Encoding utf8
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
        $parseError = $_
        for ($start = 0; $start -lt $jsonText.Length; $start++) {
            if ($jsonText[$start] -ne '{') {
                continue
            }

            $depth = 0
            $inString = $false
            $escaped = $false
            for ($end = $start; $end -lt $jsonText.Length; $end++) {
                $char = $jsonText[$end]

                if ($inString) {
                    if ($escaped) {
                        $escaped = $false
                    } elseif ($char -eq '\') {
                        $escaped = $true
                    } elseif ($char -eq '"') {
                        $inString = $false
                    }
                    continue
                }

                if ($char -eq '"') {
                    $inString = $true
                } elseif ($char -eq '{') {
                    $depth++
                } elseif ($char -eq '}') {
                    $depth--
                    if ($depth -eq 0) {
                        $candidate = $jsonText.Substring($start, $end - $start + 1)
                        try {
                            return ($candidate | ConvertFrom-Json -Depth 50)
                        } catch {
                            break
                        }
                    }
                }
            }
        }

        throw $parseError
    }
}

function ConvertTo-PrettyJson {
    param([Parameter(Mandatory)][object]$Value)

    $Value | ConvertTo-Json -Depth 50
}

function Invoke-Git {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string[]]$Arguments,
        [switch]$AllowNonZero
    )

    $gitArgs = @('-c', 'core.quotepath=false') + $Arguments
    $tmpOut = [System.IO.Path]::GetTempFileName()
    $tmpErr = [System.IO.Path]::GetTempFileName()
    Push-Location $RepoRoot
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

        [pscustomobject]@{ ExitCode = $exitCode; Text = $text; Stderr = $errorText }
    } finally {
        Remove-Item -LiteralPath $tmpOut -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $tmpErr -ErrorAction SilentlyContinue
    }
}

function Get-GitPath {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string]$Path
    )

    $relative = [System.IO.Path]::GetRelativePath($RepoRoot, $Path)
    return ($relative -replace '\\', '/')
}

function ConvertTo-GitStatusPathKey {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string]$Path
    )

    $gitPath = if ([System.IO.Path]::IsPathRooted($Path)) {
        Get-GitPath -RepoRoot $RepoRoot -Path $Path
    } else {
        $Path
    }

    $gitPath = $gitPath.Trim().Replace('\', '/')
    while ($gitPath.StartsWith('./', [System.StringComparison]::Ordinal)) {
        $gitPath = $gitPath.Substring(2)
    }

    return $gitPath
}

function Get-StatusPaths {
    param([Parameter(Mandatory)][string]$StatusLine)

    if ($StatusLine.Length -le 3) {
        return @()
    }

    $pathText = $StatusLine.Substring(3)
    if ($pathText.Contains(' -> ')) {
        return @($pathText -split ' -> ')
    }

    return @($pathText)
}

function Get-WorktreeStatusText {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [string[]]$ExcludedPaths = @()
    )

    $excludedSet = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($path in $ExcludedPaths) {
        if ([string]::IsNullOrWhiteSpace($path)) {
            continue
        }
        [void]$excludedSet.Add((ConvertTo-GitStatusPathKey -RepoRoot $RepoRoot -Path $path))
    }

    $statusLines = @((Invoke-Git -RepoRoot $RepoRoot -Arguments @('status', '--porcelain=v1')).Text -split "`r?`n" | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_)
    })

    $keptLines = @()
    foreach ($line in $statusLines) {
        $paths = @(Get-StatusPaths -StatusLine $line)
        $isExcluded = $paths.Count -gt 0
        foreach ($path in $paths) {
            $key = ConvertTo-GitStatusPathKey -RepoRoot $RepoRoot -Path $path
            if (-not $excludedSet.Contains($key)) {
                $isExcluded = $false
                break
            }
        }

        if (-not $isExcluded) {
            $keptLines += $line
        }
    }

    return ($keptLines -join "`n")
}

function Assert-CleanWorktree {
    param([Parameter(Mandatory)][string]$RepoRoot)

    $status = (Invoke-Git -RepoRoot $RepoRoot -Arguments @('status', '--porcelain=v1')).Text
    if (-not [string]::IsNullOrWhiteSpace($status)) {
        throw "Workspace must be clean. Resolve the dirty files before continuing.`n$status"
    }
}

function Unstage-PathsIfNeeded {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        # AllowEmptyCollection: callers legitimately pass an empty set (e.g. a fresh
        # run with no pre-existing artifacts). A Mandatory array otherwise rejects @()
        # before the body runs, even though "nothing to unstage" is a valid no-op.
        [Parameter(Mandatory)][AllowEmptyCollection()][string[]]$Paths
    )

    $staged = @((Invoke-Git -RepoRoot $RepoRoot -Arguments @('diff', '--cached', '--name-only', '--')).Text -split "`r?`n" | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_)
    })
    if ($staged.Count -eq 0) {
        return
    }

    $stagedSet = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($path in $staged) {
        [void]$stagedSet.Add((ConvertTo-GitStatusPathKey -RepoRoot $RepoRoot -Path $path))
    }

    $toUnstage = @()
    foreach ($path in $Paths) {
        if ([string]::IsNullOrWhiteSpace($path)) {
            continue
        }

        $key = ConvertTo-GitStatusPathKey -RepoRoot $RepoRoot -Path $path
        if ($stagedSet.Contains($key)) {
            $toUnstage += $key
        }
    }

    if ($toUnstage.Count -gt 0) {
        Invoke-Git -RepoRoot $RepoRoot -Arguments (@('restore', '--staged', '--') + $toUnstage) | Out-Null
    }
}

function Assert-StagedChangesExist {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string]$Context
    )

    $result = Invoke-Git -RepoRoot $RepoRoot -Arguments @('diff', '--cached', '--quiet', '--') -AllowNonZero
    if ($result.ExitCode -eq 0) {
        throw "No staged implementation changes found after $Context."
    }
    if ($result.ExitCode -ne 1) {
        throw "git diff --cached --quiet failed with exit code $($result.ExitCode).`n$($result.Text)"
    }
}

function Assert-PathUnderRepo {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string]$Path
    )

    $relative = [System.IO.Path]::GetRelativePath($RepoRoot, $Path)
    if ($relative.StartsWith('..') -or [System.IO.Path]::IsPathRooted($relative)) {
        throw "Path must be inside repo root '$RepoRoot': $Path"
    }
}

function Assert-NoPartiallyStagedFiles {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [string[]]$ExcludedPaths = @()
    )

    $excluded = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    foreach ($p in $ExcludedPaths) {
        if (-not [string]::IsNullOrWhiteSpace($p)) {
            [void]$excluded.Add((ConvertTo-GitStatusPathKey -RepoRoot $RepoRoot -Path $p))
        }
    }

    $lines = @((Invoke-Git -RepoRoot $RepoRoot -Arguments @('status', '--porcelain=v1')).Text -split "`r?`n" |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })

    $partial = @()
    foreach ($line in $lines) {
        if ($line.Length -lt 3) { continue }
        $indexCol = $line[0]
        $worktreeCol = $line[1]
        if ($indexCol -eq '?' -or $indexCol -eq '!') { continue }   # untracked / ignored
        if ($indexCol -eq ' ' -or $worktreeCol -eq ' ') { continue } # fully staged or unstaged-only

        $paths = @(Get-StatusPaths -StatusLine $line)
        $allExcluded = $paths.Count -gt 0
        foreach ($pp in $paths) {
            if (-not $excluded.Contains((ConvertTo-GitStatusPathKey -RepoRoot $RepoRoot -Path $pp))) {
                $allExcluded = $false; break
            }
        }
        if (-not $allExcluded) { $partial += $line }
    }

    if ($partial.Count -gt 0) {
        throw "Partially-staged files detected (index and worktree both differ):`n$($partial -join "`n")"
    }
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

function Assert-CliExists {
    param([Parameter(Mandatory)][string]$CliName)

    $cmd = Get-Command $CliName -ErrorAction SilentlyContinue
    if (-not $cmd) {
        throw "CLI '$CliName' not found in PATH."
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
            throw "CLI '$Tool' help output does not advertise required flag '$flag'. Inspect or update the calling script before running."
        }
    }
}

function Read-PromptTemplate {
    param(
        [Parameter(Mandatory)][string]$PromptsDir,
        [Parameter(Mandatory)][string]$Name
    )

    $path = Join-Path $PromptsDir $Name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Prompt template not found: $path"
    }

    Read-TextFile $path
}

function Expand-PromptTemplate {
    param(
        [Parameter(Mandatory)][string]$PromptsDir,
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][hashtable]$Variables
    )

    $text = Read-PromptTemplate -PromptsDir $PromptsDir -Name $Name
    foreach ($key in $Variables.Keys) {
        $placeholder = '{{' + $key + '}}'
        $text = $text.Replace($placeholder, [string]$Variables[$key])
    }

    return $text
}

function Get-CliArgs {
    param(
        [Parameter(Mandatory)][ValidateSet('claude', 'codex', 'gemini')][string]$Tool,
        [Parameter(Mandatory)][string]$WorkingDir,
        [string]$Prompt,
        [AllowNull()][string]$Model,
        [AllowNull()][string]$PermissionMode,
        [AllowNull()][string]$Sandbox,
        [AllowNull()][string]$Reasoning,
        [AllowNull()][string]$OutputSchemaPath,
        [AllowNull()][string]$OutputLastMessagePath,
        [string[]]$AllowedTools = @(),
        [string[]]$ExtraArgs = @()
    )

    $cliArgs = @()
    switch ($Tool) {
        'codex' {
            $cliArgs += @('exec', '--cd', $WorkingDir, '--color', 'never')
            if (-not [string]::IsNullOrWhiteSpace($Sandbox))  { $cliArgs += @('--sandbox', $Sandbox) }
            if (-not [string]::IsNullOrWhiteSpace($Model))    { $cliArgs += @('--model', $Model) }
            if (-not [string]::IsNullOrWhiteSpace($Reasoning)) { $cliArgs += @('-c', "reasoning.level=`"$Reasoning`"") }
            if (-not [string]::IsNullOrWhiteSpace($OutputSchemaPath)) { $cliArgs += @('--output-schema', $OutputSchemaPath) }
            if (-not [string]::IsNullOrWhiteSpace($OutputLastMessagePath)) { $cliArgs += @('--output-last-message', $OutputLastMessagePath) }
            $cliArgs += $ExtraArgs
            $cliArgs += '-'    # read prompt from stdin
        }
        'claude' {
            $cliArgs += @('-p', '--no-session-persistence', '--input-format', 'text')
            if (-not [string]::IsNullOrWhiteSpace($Model))          { $cliArgs += @('--model', $Model) }
            if (-not [string]::IsNullOrWhiteSpace($PermissionMode)) { $cliArgs += @('--permission-mode', $PermissionMode) }
            if ($AllowedTools.Count -gt 0)                          { $cliArgs += @('--allowedTools') + $AllowedTools }
            $cliArgs += $ExtraArgs
        }
        'gemini' {
            if (-not [string]::IsNullOrWhiteSpace($Model)) { $cliArgs += @('-m', $Model) }
            $cliArgs += $ExtraArgs
            $cliArgs += @('-p', $Prompt)
        }
    }
    return $cliArgs
}

function Invoke-Cli {
    param(
        [Parameter(Mandatory)][ValidateSet('claude', 'codex', 'gemini')][string]$Tool,
        [Parameter(Mandatory)][string]$Prompt,
        [Parameter(Mandatory)][string]$WorkingDir,
        [AllowNull()][string]$Model,
        [AllowNull()][string]$PermissionMode,
        [AllowNull()][string]$Sandbox,
        [AllowNull()][string]$Reasoning,
        [AllowNull()][string]$OutputSchemaPath,
        [AllowNull()][string]$OutputLastMessagePath,
        [string[]]$AllowedTools = @(),
        [string[]]$ExtraArgs = @()
    )

    Assert-CliExists $Tool

    if (-not [string]::IsNullOrWhiteSpace($OutputLastMessagePath)) {
        Ensure-Dir (Split-Path -Parent $OutputLastMessagePath)
        Remove-Item -LiteralPath $OutputLastMessagePath -ErrorAction SilentlyContinue
    }

    $cliArgs = Get-CliArgs -Tool $Tool -WorkingDir $WorkingDir -Prompt $Prompt -Model $Model `
        -PermissionMode $PermissionMode -Sandbox $Sandbox -Reasoning $Reasoning `
        -OutputSchemaPath $OutputSchemaPath -OutputLastMessagePath $OutputLastMessagePath `
        -AllowedTools $AllowedTools -ExtraArgs $ExtraArgs

    $usesStdin = ($Tool -eq 'claude' -or $Tool -eq 'codex')
    $tmpOut = [System.IO.Path]::GetTempFileName()
    $tmpErr = [System.IO.Path]::GetTempFileName()
    Push-Location $WorkingDir
    try {
        if ($usesStdin) {
            $Prompt | & $Tool @cliArgs > $tmpOut 2> $tmpErr
        } else {
            & $Tool @cliArgs > $tmpOut 2> $tmpErr
        }
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

        # codex writes --output-last-message natively; claude/gemini do not, so
        # persist stdout to satisfy the OutputLastMessagePath artifact contract
        # for every tool.
        if (-not [string]::IsNullOrWhiteSpace($OutputLastMessagePath) -and -not (Test-Path -LiteralPath $OutputLastMessagePath)) {
            Write-AtomicUtf8 -Path $OutputLastMessagePath -Content $stdout
        }

        $lastMessage = ''
        if (-not [string]::IsNullOrWhiteSpace($OutputLastMessagePath) -and (Test-Path -LiteralPath $OutputLastMessagePath)) {
            $lastMessage = Read-TextFile $OutputLastMessagePath
        }

        $result = if (-not [string]::IsNullOrWhiteSpace($lastMessage)) { $lastMessage } else { $stdout }
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

Export-ModuleMember -Variable AgentCliVersion -Function `
    Set-Utf8ProcessEncoding, Resolve-FullPath, Ensure-Dir, Write-AtomicUtf8, `
    Add-LogLine, Normalize-Text, Read-TextFile, ConvertFrom-AgentJson, `
    ConvertTo-PrettyJson, Get-ObjectProperty, `
    Extract-MarkedSection, Get-PlanIdFromPath, New-SafeFileSegment, `
    Assert-CliExists, Get-CliHelpText, Assert-HelpContains, `
    Read-PromptTemplate, Expand-PromptTemplate, `
    Invoke-Git, Get-GitPath, ConvertTo-GitStatusPathKey, Get-StatusPaths, `
    Get-WorktreeStatusText, Assert-CleanWorktree, Unstage-PathsIfNeeded, `
    Assert-StagedChangesExist, Assert-PathUnderRepo, Assert-NoPartiallyStagedFiles, `
    Get-CliArgs, Invoke-Cli
