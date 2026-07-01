#Requires -Version 7.0

<#
Iteratively shrinks one Rust source file by extracting cohesive modules, verifying
each extraction with cargo fmt + clippy, staging (never committing) verified results.
See docs/plans/Design.RustFileShrink.md.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$FilePath,
    [string]$RepoRoot = (Get-Location).Path,
    [string]$PromptsDir,
    [string]$ArtifactsDir,
    [ValidateRange(1, 100)][int]$MaxIterations = 10,
    [ValidateRange(1, 1000000)][int]$MinLines = 300,
    [string]$RecommendModel = 'opus',
    [string]$ExtractModel = 'sonnet',
    [switch]$RunTests,
    [switch]$PreflightOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot 'lib\AgentCli.psm1') -Force

function Get-FileLineCount {
    param([Parameter(Mandatory)][string]$Path)
    $text = Read-TextFile $Path
    if ([string]::IsNullOrEmpty($text)) { return 0 }
    return @($text -split "`r?`n").Count
}

function Test-ShrinkRecommendation {
    param([Parameter(Mandatory)][object]$Recommendation)

    $allowed = @('decision', 'reason', 'next_step_summary', 'candidate', 'confidence')
    foreach ($p in $Recommendation.PSObject.Properties.Name) {
        if ($p -notin $allowed) { throw "Unknown recommendation field: $p" }
    }

    $decision = [string](Get-ObjectProperty -Object $Recommendation -Name 'decision' -Default '')
    if ($decision -notin @('extract', 'stop')) { throw "Invalid decision: '$decision'" }

    $confidence = [string](Get-ObjectProperty -Object $Recommendation -Name 'confidence' -Default '')
    if ($confidence -notin @('', 'low', 'medium', 'high')) { Write-Warning "Unexpected confidence value: '$confidence'" }
    elseif ($confidence -eq '') { Write-Warning "Recommendation missing optional 'confidence' field." }

    foreach ($req in @('reason', 'next_step_summary')) {
        if ([string]::IsNullOrWhiteSpace([string](Get-ObjectProperty -Object $Recommendation -Name $req -Default ''))) {
            throw "Missing required field: $req"
        }
    }

    $candidate = Get-ObjectProperty -Object $Recommendation -Name 'candidate' -Default $null
    if ($decision -eq 'extract') {
        if ($null -eq $candidate) { throw "decision 'extract' requires a candidate." }
        foreach ($cf in @('name', 'description', 'suggested_destination', 'estimated_lines')) {
            if ($null -eq (Get-ObjectProperty -Object $candidate -Name $cf -Default $null)) {
                throw "candidate is missing required field: $cf"
            }
        }
    } elseif ($null -ne $candidate) {
        throw "decision 'stop' must not include a candidate."
    }
}

function Get-ShrinkSourceRoot {
    param([Parameter(Mandatory)][string]$RelPath)

    $norm = ($RelPath -replace '\\', '/')
    $parts = @($norm -split '/' | Where-Object { $_ -ne '' })
    $idx = [array]::LastIndexOf($parts, 'src')
    if ($idx -ge 0) {
        return (($parts[0..$idx]) -join '/')
    }
    $parent = (Split-Path -Parent $norm) -replace '\\', '/'
    return $parent
}

function Test-ShrinkDestination {
    param(
        [Parameter(Mandatory)][string]$Destination,
        [Parameter(Mandatory)][string]$TargetRelPath
    )

    $d = ($Destination -replace '\\', '/').Trim()
    if ([string]::IsNullOrWhiteSpace($d)) { throw "suggested_destination is empty." }
    if ([System.IO.Path]::IsPathRooted($d) -or $d -match '^[A-Za-z]:') {
        throw "suggested_destination must be repo-relative, not rooted: $d"
    }
    if (@($d -split '/') -contains '..') {
        throw "suggested_destination must not contain '..': $d"
    }
    if (-not $d.EndsWith('.rs', [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "suggested_destination must end in .rs: $d"
    }

    $srcRoot = Get-ShrinkSourceRoot -RelPath ($TargetRelPath -replace '\\', '/')
    if (-not [string]::IsNullOrEmpty($srcRoot) -and
        -not $d.StartsWith("$srcRoot/", [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "suggested_destination must be under the target's source directory '$srcRoot': $d"
    }
}

function Test-ShrinkPathAllowed {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$TargetRelPath,
        [Parameter(Mandatory)][string]$DestinationRelPath
    )

    $norm = { param($p) ($p -replace '\\', '/').TrimStart('./') }
    $key = & $norm $Path
    $target = & $norm $TargetRelPath
    $dest = & $norm $DestinationRelPath

    if ($key -eq $target -or $key -eq $dest) { return $true }

    $wiringNames = @('mod.rs', 'lib.rs', 'main.rs')
    $leaf = Split-Path -Leaf $key
    if ($leaf -in $wiringNames) {
        $keyDir = (Split-Path -Parent $key) -replace '\\', '/'
        foreach ($anchor in @($target, $dest)) {
            $anchorDir = (Split-Path -Parent $anchor) -replace '\\', '/'
            # ancestor (or same dir) of the target/destination
            if ($anchorDir -eq $keyDir -or $anchorDir.StartsWith("$keyDir/", [System.StringComparison]::Ordinal)) {
                return $true
            }
        }
    }
    return $false
}

function New-ShrinkCommitMessage {
    param(
        [Parameter(Mandatory)][string]$TargetRelPath,
        [Parameter(Mandatory)][object[]]$Extractions
    )

    $count = $Extractions.Count
    $subject = "Shrink ${TargetRelPath}: extract $count module(s)"
    $bullets = foreach ($e in $Extractions) {
        $name = Get-ObjectProperty -Object $e -Name 'name' -Default '?'
        $dest = Get-ObjectProperty -Object $e -Name 'destination' -Default '?'
        $lines = Get-ObjectProperty -Object $e -Name 'estimated_lines' -Default 0
        "- $name -> $dest (~$lines lines)"
    }
    return ($subject + "`n`n" + ($bullets -join "`n"))
}

function Get-ShrinkChangedPaths {
    param([Parameter(Mandatory)][string]$RepoRoot)

    # Current worktree delta vs the index ONLY — not vs HEAD. Already-staged
    # checkpoint paths (which equal the index) must not reappear, or iteration N
    # would evaluate iteration N-1's destination as an "unexpected" path.
    $unstaged = @((Invoke-Git -RepoRoot $RepoRoot -Arguments @('diff', '--name-only', '--')).Text -split "`r?`n" |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $untracked = @((Invoke-Git -RepoRoot $RepoRoot -Arguments @('ls-files', '--others', '--exclude-standard')).Text -split "`r?`n" |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) })

    $paths = @()
    foreach ($p in (@($unstaged) + @($untracked))) {
        $paths += (ConvertTo-GitStatusPathKey -RepoRoot $RepoRoot -Path $p)
    }
    return @($paths | Sort-Object -Unique)
}

function Save-ShrinkCheckpoint {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string[]]$Paths
    )
    if ($Paths.Count -eq 0) { return }
    Invoke-Git -RepoRoot $RepoRoot -Arguments (@('add', '--') + $Paths) | Out-Null
}

function Restore-ShrinkCheckpoint {
    param(
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string]$ArtifactGlob
    )
    # Revert tracked worktree files to the index (the last verified checkpoint).
    Invoke-Git -RepoRoot $RepoRoot -Arguments @('restore', '--worktree', '--', '.') | Out-Null
    # Remove untracked files created during the failed iteration, but keep artifacts.
    Invoke-Git -RepoRoot $RepoRoot -Arguments @('clean', '-fd', '-e', $ArtifactGlob, '--', '.') | Out-Null
}

function Get-ShrinkArtifactGlob {
    param([Parameter(Mandatory)][string]$Slug)
    return "Shrink.$Slug.*"
}

function Resolve-ShrinkCargoRoot {
    param([Parameter(Mandatory)][string]$FileFullPath)

    $dir = Split-Path -Parent $FileFullPath
    Push-Location $dir
    try {
        $manifest = (& cargo locate-project --message-format plain 2>$null | Out-String).Trim()
        $code = $LASTEXITCODE
    } finally { Pop-Location }
    if ($code -ne 0 -or [string]::IsNullOrWhiteSpace($manifest)) {
        throw "Could not locate a Cargo.toml governing '$FileFullPath' (cargo locate-project failed)."
    }
    return (Split-Path -Parent $manifest)
}

function Resolve-ShrinkContext {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(Mandatory)][string]$RepoRoot,
        [Parameter(Mandatory)][string]$PromptsDir,
        [Parameter(Mandatory)][string]$ArtifactsDir
    )

    $repoFull = (& git -C $RepoRoot rev-parse --show-toplevel 2>$null | Out-String).Trim()
    if ([string]::IsNullOrWhiteSpace($repoFull)) { $repoFull = [System.IO.Path]::GetFullPath($RepoRoot) }

    $fileFull = Resolve-FullPath -Path $FilePath -BasePath $repoFull -MustExist
    Assert-PathUnderRepo -RepoRoot $repoFull -Path $fileFull
    if ([System.IO.Path]::GetExtension($fileFull) -ne '.rs') {
        throw "FilePath must be a .rs file: $fileFull"
    }

    $relPath = Get-GitPath -RepoRoot $repoFull -Path $fileFull
    $slug = New-SafeFileSegment -Text ([System.IO.Path]::GetFileNameWithoutExtension($fileFull))
    $cargoRoot = Resolve-ShrinkCargoRoot -FileFullPath $fileFull

    $recSchemaPath = Join-Path $PromptsDir 'shrink-recommendation.schema.json'
    $stepSchemaPath = Join-Path $PromptsDir 'phase-cycle-step-result.schema.json'
    foreach ($p in @($recSchemaPath, $stepSchemaPath,
                     (Join-Path $PromptsDir 'shrink-recommend.md'),
                     (Join-Path $PromptsDir 'shrink-extract.md'))) {
        if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { throw "Required prompt/schema not found: $p" }
    }

    $artifactsFull = Resolve-FullPath -Path $ArtifactsDir -BasePath $repoFull
    Ensure-Dir $artifactsFull

    [pscustomobject]@{
        RepoRoot         = $repoFull
        CargoRoot        = $cargoRoot
        FilePath         = $fileFull
        FileRelPath      = $relPath
        Slug             = $slug
        PromptsDir       = $PromptsDir
        ArtifactsDir     = $artifactsFull
        ArtifactGlob     = Get-ShrinkArtifactGlob -Slug $slug
        LogPath          = Join-Path $artifactsFull "Shrink.$slug.log"
        RecSchemaPath    = $recSchemaPath
        RecSchemaText    = Read-TextFile $recSchemaPath
        StepSchemaPath   = $stepSchemaPath
        StepSchemaText   = Read-TextFile $stepSchemaPath
    }
}

function Invoke-ShrinkGate {
    param(
        [Parameter(Mandatory)][string]$CargoRoot,
        [bool]$RunTests = $false
    )

    $commands = @(
        @{ Name = 'cargo fmt'; Args = @('fmt') },
        @{ Name = 'cargo clippy'; Args = @('clippy', '--all-targets', '--', '-D', 'warnings') }
    )
    if ($RunTests) { $commands += @{ Name = 'cargo test'; Args = @('test') } }

    foreach ($c in $commands) {
        Push-Location $CargoRoot
        try {
            $tmpOut = [System.IO.Path]::GetTempFileName()
            & cargo @($c.Args) > $tmpOut 2>&1
            $code = $LASTEXITCODE
            $out = Read-TextFile $tmpOut
            Remove-Item -LiteralPath $tmpOut -ErrorAction SilentlyContinue
        } finally { Pop-Location }
        if ($code -ne 0) { throw "Gate failed: $($c.Name) exited $code.`n$out" }
    }
}

function Invoke-ShrinkCleanArtifacts {
    param([Parameter(Mandatory)][object]$Ctx)
    # Overwrite/clean this command's own artifacts at run start (design §4.4).
    Get-ChildItem -LiteralPath $Ctx.ArtifactsDir -Filter $Ctx.ArtifactGlob -File -ErrorAction SilentlyContinue |
        Remove-Item -Force -ErrorAction SilentlyContinue
}

function Invoke-RustFileShrinkMain {
    Set-Utf8ProcessEncoding

    $promptsDir = if ([string]::IsNullOrWhiteSpace($PromptsDir)) { Join-Path $PSScriptRoot 'prompts' } else { $PromptsDir }
    $artifactsDir = if ([string]::IsNullOrWhiteSpace($ArtifactsDir)) { 'docs/plans' } else { $ArtifactsDir }

    $ctx = Resolve-ShrinkContext -FilePath $FilePath -RepoRoot $RepoRoot -PromptsDir $promptsDir -ArtifactsDir $artifactsDir

    Assert-CliExists 'claude'
    Assert-HelpContains -Tool 'claude' -Arguments @('--help') -ExpectedFlags @(
        '-p', '--no-session-persistence', '--input-format', '--model', '--permission-mode', '--allowedTools'
    )

    # Clean-worktree guard, tolerating this command's own artifacts (design §4.4).
    $artifactRelPaths = @(Get-ChildItem -LiteralPath $ctx.ArtifactsDir -Filter $ctx.ArtifactGlob -File -ErrorAction SilentlyContinue |
        ForEach-Object { Get-GitPath -RepoRoot $ctx.RepoRoot -Path $_.FullName })
    $unexpected = Get-WorktreeStatusText -RepoRoot $ctx.RepoRoot -ExcludedPaths $artifactRelPaths
    if (-not [string]::IsNullOrWhiteSpace($unexpected)) {
        throw "Worktree must be clean (aside from this command's own Shrink.$($ctx.Slug).* artifacts).`n$unexpected"
    }

    if ($PreflightOnly) {
        Write-Host 'Preflight OK. No artifacts written, no AI steps invoked.'
        Write-Host "  Repo:      $($ctx.RepoRoot)"
        Write-Host "  CargoRoot: $($ctx.CargoRoot)"
        Write-Host "  File:      $($ctx.FileRelPath)"
        Write-Host "  Slug:      $($ctx.Slug)"
        Write-Host "  Artifacts: $($ctx.ArtifactsDir) (glob $($ctx.ArtifactGlob))"
        if ($artifactRelPaths.Count -gt 0) {
            Write-Host "  NOTE: pre-existing artifacts that a real run would overwrite:"
            $artifactRelPaths | ForEach-Object { Write-Host "    - $_" }
        }
        return
    }

    Invoke-ShrinkCleanArtifacts -Ctx $ctx
    Write-AtomicUtf8 -Path $ctx.LogPath -Content "Started: $(Get-Date)`nFile: $($ctx.FileRelPath)`nMinLines: $MinLines`nMaxIterations: $MaxIterations`n"

    $extractions = @()
    $checkpoints = 0
    $haltReason = $null
    $failedCandidate = $null

    for ($n = 1; $n -le $MaxIterations; $n++) {
        $lineCount = Get-FileLineCount -Path $ctx.FilePath
        Add-LogLine -LogPath $ctx.LogPath -Line "Iteration ${n}: file is $lineCount lines."
        if ($lineCount -le $MinLines) { $haltReason = 'min_lines_reached'; break }

        # --- Recommend (Opus, read-only) ---
        $recPrompt = Expand-PromptTemplate -PromptsDir $ctx.PromptsDir -Name 'shrink-recommend.md' -Variables @{
            RECOMMENDATION_SCHEMA = $ctx.RecSchemaText
            FILE_PATH             = $ctx.FileRelPath
            LINE_COUNT            = $lineCount
            MIN_LINES             = $MinLines
            FILE_TEXT             = (Read-TextFile $ctx.FilePath)
        }
        $recOut = Invoke-Cli -Tool 'claude' -Prompt $recPrompt -WorkingDir $ctx.RepoRoot -Model $RecommendModel -PermissionMode 'plan'
        $recPath = Join-Path $ctx.ArtifactsDir ("Shrink.$($ctx.Slug).iter{0:D2}.recommend.json" -f $n)
        try {
            $recommendation = ConvertFrom-AgentJson -Text $recOut
            Test-ShrinkRecommendation -Recommendation $recommendation
        } catch {
            Write-AtomicUtf8 -Path ([System.IO.Path]::ChangeExtension($recPath, '.raw.txt')) -Content $recOut
            throw "Recommendation invalid (iter $n): $($_.Exception.Message)"
        }
        Write-AtomicUtf8 -Path $recPath -Content (ConvertTo-PrettyJson -Value $recommendation)

        if ((Get-ObjectProperty -Object $recommendation -Name 'decision' -Default '') -eq 'stop') {
            $haltReason = "stop: $(Get-ObjectProperty -Object $recommendation -Name 'reason' -Default '')"
            break
        }

        $candidate = Get-ObjectProperty -Object $recommendation -Name 'candidate' -Default $null
        $destRel = [string](Get-ObjectProperty -Object $candidate -Name 'suggested_destination' -Default '')

        # Validate the destination BEFORE granting edit permission (review finding #4).
        # No edits exist yet, so an invalid destination is a clean halt (no restore needed).
        try {
            Test-ShrinkDestination -Destination $destRel -TargetRelPath $ctx.FileRelPath
        } catch {
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "invalid destination (iter $n): $($_.Exception.Message)"
            break
        }

        Write-Host (Get-ObjectProperty -Object $recommendation -Name 'next_step_summary' -Default '')

        # --- Extract (Sonnet, edits) ---
        $extractPrompt = Expand-PromptTemplate -PromptsDir $ctx.PromptsDir -Name 'shrink-extract.md' -Variables @{
            STEP_RESULT_SCHEMA  = $ctx.StepSchemaText
            RECOMMENDATION_JSON = (ConvertTo-PrettyJson -Value $recommendation)
            FILE_PATH           = $ctx.FileRelPath
        }
        $extractRawPath = Join-Path $ctx.ArtifactsDir ("Shrink.$($ctx.Slug).iter{0:D2}.extract.raw.txt" -f $n)
        $extractPath = Join-Path $ctx.ArtifactsDir ("Shrink.$($ctx.Slug).iter{0:D2}.extract.json" -f $n)

        # The extract step has edit permission, so ANY failure here — a CLI error or
        # invalid/unparseable JSON — can leave half-applied Rust edits. Restore the
        # checkpoint on every failure path before exiting (review finding #2 / §4.3).
        try {
            $extractOut = Invoke-Cli -Tool 'claude' -Prompt $extractPrompt -WorkingDir $ctx.RepoRoot -Model $ExtractModel `
                -PermissionMode 'acceptEdits' -OutputLastMessagePath $extractRawPath
            $stepResult = ConvertFrom-AgentJson -Text $extractOut
            Write-AtomicUtf8 -Path $extractPath -Content (ConvertTo-PrettyJson -Value $stepResult)
        } catch {
            $rawForError = Get-Variable -Name 'extractOut' -ValueOnly -ErrorAction SilentlyContinue
            if (-not [string]::IsNullOrEmpty([string]$rawForError)) {
                Write-AtomicUtf8 -Path ([System.IO.Path]::ChangeExtension($extractPath, '.error.raw.txt')) -Content ([string]$rawForError)
            }
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "extract step failed (iter $n): $($_.Exception.Message)"
            Restore-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -ArtifactGlob $ctx.ArtifactGlob
            break
        }

        # Halt on any non-success status (incl. manual_feedback_required).
        $status = [string](Get-ObjectProperty -Object $stepResult -Name 'status' -Default '')
        if ($status -ne 'success') {
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "extract status '$status' (iter $n)"
            Restore-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -ArtifactGlob $ctx.ArtifactGlob
            break
        }

        # --- Script gate: fmt, then clippy (+ test) ---
        try {
            Invoke-ShrinkGate -CargoRoot $ctx.CargoRoot -RunTests:$RunTests.IsPresent
        } catch {
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "gate failure (iter $n): $($_.Exception.Message)"
            Restore-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -ArtifactGlob $ctx.ArtifactGlob
            break
        }

        # --- Size sanity guard (post-fmt) ---
        $newCount = Get-FileLineCount -Path $ctx.FilePath
        if ($newCount -ge $lineCount) {
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "no size reduction (iter $n): $lineCount -> $newCount"
            Restore-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -ArtifactGlob $ctx.ArtifactGlob
            break
        }

        # --- Path-gated staging ---
        # Drop this command's own artifacts (they live under ArtifactsDir and match the
        # slug glob) before applying the allowlist; only real code changes are staged.
        $artifactsRel = Get-GitPath -RepoRoot $ctx.RepoRoot -Path $ctx.ArtifactsDir
        $artifactPrefix = "$artifactsRel/Shrink.$($ctx.Slug)."
        $codeChanged = @(Get-ShrinkChangedPaths -RepoRoot $ctx.RepoRoot | Where-Object {
            -not $_.StartsWith($artifactPrefix, [System.StringComparison]::OrdinalIgnoreCase)
        })
        # Files staged by prior iterations are allowed to be modified (e.g., adding
        # re-exports to a previously extracted module). These are safe to permit because
        # the script requires a clean worktree on entry, so all staged files are ours.
        $priorStagedPaths = @(
            (Invoke-Git -RepoRoot $ctx.RepoRoot -Arguments @('diff', '--cached', '--name-only', '--')).Text `
            -split "`r?`n" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
            ForEach-Object { ConvertTo-GitStatusPathKey -RepoRoot $ctx.RepoRoot -Path $_ } |
            Where-Object { -not $_.StartsWith($artifactPrefix, [System.StringComparison]::OrdinalIgnoreCase) }
        )
        $violations = @($codeChanged | Where-Object {
            $p = $_
            -not (Test-ShrinkPathAllowed -Path $p -TargetRelPath $ctx.FileRelPath -DestinationRelPath $destRel) -and
            $p -notin $priorStagedPaths
        })
        if ($violations.Count -gt 0) {
            $failedCandidate = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            $haltReason = "unexpected changed paths (iter $n): $($violations -join ', ')"
            Restore-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -ArtifactGlob $ctx.ArtifactGlob
            break
        }
        Save-ShrinkCheckpoint -RepoRoot $ctx.RepoRoot -Paths $codeChanged

        # Secondary guard: artifacts/log stay unstaged.
        Unstage-PathsIfNeeded -RepoRoot $ctx.RepoRoot -Paths $artifactRelPaths

        # Invariant: the new checkpoint must be a consistent staged state — no file
        # may have both index- and worktree-side changes (review finding #5 / §4.3).
        Assert-NoPartiallyStagedFiles -RepoRoot $ctx.RepoRoot -ExcludedPaths $artifactRelPaths

        $extractions += [pscustomobject]@{
            name            = (Get-ObjectProperty -Object $candidate -Name 'name' -Default '?')
            destination     = $destRel
            estimated_lines = (Get-ObjectProperty -Object $candidate -Name 'estimated_lines' -Default 0)
            summary         = (Get-ObjectProperty -Object $stepResult -Name 'summary' -Default '')
        }
        $checkpoints++
        Add-LogLine -LogPath $ctx.LogPath -Line "Checkpoint $checkpoints staged: $($destRel) ($lineCount -> $newCount lines)."
    }

    if ($null -eq $haltReason) { $haltReason = 'max_iterations_reached' }
    Add-LogLine -LogPath $ctx.LogPath -Line "Halt: $haltReason"

    # --- Exit ---
    Write-Host ''
    if ($checkpoints -gt 0) {
        # Re-assert the staged index maps to a consistent diff before suggesting a
        # commit message (review finding #5 / §4.3).
        Assert-NoPartiallyStagedFiles -RepoRoot $ctx.RepoRoot -ExcludedPaths $artifactRelPaths
        $commitMessage = New-ShrinkCommitMessage -TargetRelPath $ctx.FileRelPath -Extractions $extractions
        Write-Host "Shrink complete: $checkpoints extraction(s) staged. Halt reason: $haltReason"
        if ($failedCandidate) { Write-Warning "Halted after a failed candidate: $failedCandidate (its changes were restored)." }
        Write-Host ''
        Write-Host 'Suggested git commit message (nothing was committed):'
        Write-Host $commitMessage
        Add-LogLine -LogPath $ctx.LogPath -Line "Suggested commit subject: $(($commitMessage -split "`n")[0])"
    } else {
        Write-Host "No staged extraction produced. Halt reason: $haltReason"
    }
    Add-LogLine -LogPath $ctx.LogPath -Line "Completed: $(Get-Date)"
}

# Run only when invoked directly, not when dot-sourced for tests.
if ($MyInvocation.InvocationName -ne '.') {
    Invoke-RustFileShrinkMain
}
