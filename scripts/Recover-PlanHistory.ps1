#Requires -Version 5.1

# ============================================================================
# Recover Plan History from Git
# ============================================================================
#
# Recovers deleted Plan.*.md files from git history and extracts their goals
# for seeding the Engineering Diary retroactively.
#
# Usage:
#   .\Recover-PlanHistory.ps1 -Phase Find      # Step 1: find deleted plans
#   .\Recover-PlanHistory.ps1 -Phase Date       # Step 2: assign dates & sort
#   .\Recover-PlanHistory.ps1 -Phase Extract    # Step 3: extract goal text
#   .\Recover-PlanHistory.ps1 -Phase Validate   # Cross-check with FutureIdeas
#
# Each phase reads its input from the previous phase's output file and writes
# its own output file, so you can inspect/edit intermediary results.
#
# Output files (all in docs/):
#   phase1-deleted-plans.json    - raw list of deleted plan files + commits
#   phase2-dated-plans.json      - plans sorted by estimated start date
#   phase3-plan-goals.md         - extracted goal descriptions, diary-ready
# ============================================================================

param(
    [Parameter(Mandatory=$true)]
    [ValidateSet('Find', 'Date', 'Extract', 'Validate')]
    [string]$Phase
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$outputDir = Join-Path $projectRoot "docs"

# --- Output file paths ---
$phase1Output = Join-Path $outputDir "phase1-deleted-plans.json"
$phase2Output = Join-Path $outputDir "phase2-dated-plans.json"
$phase3Output = Join-Path $outputDir "phase3-plan-goals.md"

# --- Helper Functions ---

function Write-StepHeader {
    param([string]$Title)
    Write-Host ""
    Write-Host "== $Title ==" -ForegroundColor Cyan
    Write-Host ""
}

function Write-ItemInfo {
    param([string]$Label, [string]$Value)
    Write-Host "  $Label" -ForegroundColor Yellow -NoNewline
    Write-Host ": $Value"
}

function Assert-FileExists {
    param(
        [Parameter(Mandatory=$true)]
        [string]$Path,
        [Parameter(Mandatory=$true)]
        [string]$PhaseName
    )
    if (-not (Test-Path $Path)) {
        Write-Host "ERROR: Input file not found: $Path" -ForegroundColor Red
        Write-Host "Run phase '$PhaseName' first." -ForegroundColor Red
        exit 1
    }
}

function Get-PlanSearchStems {
    <#
    .SYNOPSIS
    Extracts search stems from a plan filename for git log --grep matching.
    Returns stems from most specific to broadest.
    Example: "Plan.Phase7.HeadlessBatchRunner.md" -> @("HeadlessBatchRunner", "Phase7")
    #>
    param([Parameter(Mandatory=$true)] [string]$PlanName)

    $baseName = [System.IO.Path]::GetFileNameWithoutExtension($PlanName)
    $segments = $baseName -split '\.'

    # Remove "Plan" prefix and generic segments like "Rough"
    $meaningful = $segments | Where-Object { $_ -notin @('Plan', 'Rough') }

    # Return most specific first (last segment), then progressively broader
    [array]::Reverse($meaningful)
    return $meaningful
}

function Extract-GoalFromContent {
    <#
    .SYNOPSIS
    Extracts the first meaningful paragraph from a plan document.
    Skips title headings, metadata lines, section headings like "## Goal",
    revision dates, and blank lines. Returns the goal text as a single string.
    #>
    param([Parameter(Mandatory=$true)] [string]$Content)

    $lines = $Content -split "`n" | ForEach-Object { $_.TrimEnd("`r") }
    $goalLines = @()
    $collecting = $false

    foreach ($line in $lines) {
        # Always skip blank lines before we start collecting
        if (-not $collecting -and [string]::IsNullOrWhiteSpace($line)) {
            continue
        }

        # Skip all headings (# Title, ## Goal, ## Context, etc.) before collecting
        if (-not $collecting -and $line -match '^#+\s') {
            continue
        }

        # Skip horizontal rules
        if ($line -match '^\s*---\s*$') {
            if ($collecting) { break }
            continue
        }

        # Skip metadata-style lines: **Key**: value, *Key*: value, Key: value
        if (-not $collecting -and $line -match '^\s*\*{0,2}(Status|Phase|Type|Date|Blockers|Version|Priority|Created|Revised|Scope|Step|Parent)\b') {
            continue
        }

        # Skip bare date/revision lines like "Revised: 2026-02-08 (post-review)"
        if (-not $collecting -and $line -match '^\s*(Revised|Created|Updated|Date)\s*:') {
            continue
        }

        # Skip lines that are ONLY a heading keyword with no substance
        # e.g., a line that says just "## Goal" or "## Context" with nothing after
        if (-not $collecting -and $line -match '^\s*$') {
            continue
        }

        # If we get here, it's a substantive content line
        if (-not [string]::IsNullOrWhiteSpace($line)) {
            $collecting = $true
        }

        if ($collecting) {
            # Stop at the next heading
            if ($goalLines.Count -gt 0 -and $line -match '^#+\s') {
                break
            }
            # Stop after encountering a blank line (end of first paragraph)
            if ($goalLines.Count -gt 0 -and [string]::IsNullOrWhiteSpace($line)) {
                break
            }
            # Skip bullet-point metadata inside the first paragraph
            if ($line -match '^\s*[-*]\s*\*{0,2}(Status|Priority|Effort|Risk|Blockers|Created|Revised|Scope|Phase|Step|Parent)\b') {
                continue
            }

            $goalLines += $line
        }
    }

    if ($goalLines.Count -eq 0) {
        return "(No goal text found)"
    }

    # Clean up markdown formatting for readability
    $result = ($goalLines -join " ").Trim()
    # Strip leading bold markers from metadata-like prefixes that slipped through
    $result = $result -replace '^\*\*\w+\*\*:\s*\d{4}-\d{2}-\d{2}\s*', ''
    # Strip bold: **text** -> text
    $result = $result -replace '\*\*(.+?)\*\*', '$1'
    # Strip italic: *text* -> text  (but not list bullets)
    $result = $result -replace '(?<!\s)\*(.+?)\*', '$1'
    # Strip inline code: `text` -> text
    $result = $result -replace '`([^`]+)`', '$1'
    # Strip markdown links: [text](url) -> text
    $result = $result -replace '\[([^\]]+)\]\([^\)]+\)', '$1'
    # Strip em-dash mojibake from UTF-8 encoding issues
    $result = $result -replace 'ΓÇö', ' — '
    $result = $result -replace 'ΓÇ£', '"'
    $result = $result -replace 'ΓÇ¥', '"'
    $result = $result -replace 'ΓÇô', ' — '
    # Normalize real em/en-dashes to spaced dashes for plain-text readability
    $result = $result -replace '\s*—\s*', ' — '
    $result = $result -replace '\s*–\s*', ' — '
    # Strip numbered list prefixes: "1. text" -> "text"
    $result = $result -replace '(\s)\d+\.\s+', '$1'
    $result = $result -replace '^\d+\.\s+', ''
    # Collapse multiple spaces
    $result = $result -replace '\s{2,}', ' '
    return $result.Trim()
}

# ============================================================================
# Phase 1: Find all deleted Plan.*.md files in git history
# ============================================================================
function Invoke-PhaseFind {
    Write-StepHeader "Phase 1: Finding deleted Plan.*.md files in git history"

    Push-Location $projectRoot
    try {
        # Find all commits that deleted files matching docs/Plan.*.md or Plan.*.md
        # --diff-filter=D shows only deletions
        # Output format: hash<TAB>date<TAB>subject, followed by file names
        $gitOutput = git log --all --diff-filter=D --name-only `
            --pretty=format:"COMMIT:%H|%aI|%s" `
            -- "docs/Plan.*.md" "Plan.*.md" "ministry-of-future-plans/docs/Plan.*.md" 2>&1

        if ($LASTEXITCODE -ne 0) {
            Write-Host "ERROR: git log failed: $gitOutput" -ForegroundColor Red
            exit 1
        }

        $plans = @{}
        $currentCommit = $null
        $currentDate = $null
        $currentSubject = $null

        foreach ($line in $gitOutput) {
            $line = $line.Trim()
            if ([string]::IsNullOrWhiteSpace($line)) { continue }

            if ($line -match '^COMMIT:([^|]+)\|([^|]+)\|(.+)$') {
                $currentCommit = $Matches[1]
                $currentDate = $Matches[2]
                $currentSubject = $Matches[3]
            }
            elseif ($null -ne $currentCommit -and $line -match 'Plan\..*\.md$') {
                $planFile = $line
                $planName = Split-Path -Leaf $planFile

                # Keep the LAST deletion (most recent) for each plan name
                if (-not $plans.ContainsKey($planName) -or $currentDate -gt $plans[$planName].DeleteDate) {
                    $plans[$planName] = [ordered]@{
                        PlanName     = $planName
                        FilePath     = $planFile
                        DeleteCommit = $currentCommit
                        DeleteDate   = $currentDate
                        DeleteSubject = $currentSubject
                        # Parent commit where the file was still alive
                        AliveCommit  = "$($currentCommit)~1"
                    }
                }
            }
        }

        $result = @($plans.Values | Sort-Object { $_.DeleteDate })

        Write-Host "Found $($result.Count) deleted plan file(s):" -ForegroundColor Green
        foreach ($plan in $result) {
            Write-ItemInfo $plan.PlanName "deleted $($plan.DeleteDate.Substring(0,10))"
        }

        $result | ConvertTo-Json -Depth 5 | Set-Content -Path $phase1Output -Encoding UTF8
        Write-Host ""
        Write-Host "Output written to: $phase1Output" -ForegroundColor Green
    }
    finally {
        Pop-Location
    }
}

# ============================================================================
# Phase 2: Assign approximate start dates using commit-message heuristics
# ============================================================================
function Invoke-PhaseDate {
    Write-StepHeader "Phase 2: Dating plans via commit-message heuristics"

    Assert-FileExists -Path $phase1Output -PhaseName "Find"

    $plans = Get-Content $phase1Output -Raw | ConvertFrom-Json

    # Ensure we always work with an array, even if JSON contains a single object
    if ($plans -isnot [System.Array]) {
        $plans = @($plans)
    }

    Push-Location $projectRoot
    try {
        $datedPlans = @()

        foreach ($plan in $plans) {
            $stems = Get-PlanSearchStems -PlanName $plan.PlanName
            $firstCommitHash = $null
            $firstCommitDate = $null
            $firstCommitSubject = $null
            $matchedStem = $null

            # Try each stem, most specific first
            foreach ($stem in $stems) {
                if ($stem.Length -lt 3) { continue } # Skip very short stems

                $grepOutput = git log --all --oneline --format="%H|%aI|%s" `
                    --grep="$stem" -i 2>&1

                if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($grepOutput)) {
                    continue
                }

                $commits = @($grepOutput | ForEach-Object {
                    $parts = $_ -split '\|', 3
                    if ($parts.Count -eq 3) {
                        [PSCustomObject]@{
                            Hash    = $parts[0]
                            Date    = $parts[1]
                            Subject = $parts[2]
                        }
                    }
                })

                if ($commits.Count -gt 0) {
                    # Take the earliest commit as the start date
                    $earliest = $commits | Sort-Object Date | Select-Object -First 1
                    $firstCommitHash = $earliest.Hash
                    $firstCommitDate = $earliest.Date
                    $firstCommitSubject = $earliest.Subject
                    $matchedStem = $stem
                    break
                }
            }

            # Also try matching the full plan filename in commit messages
            if (-not $firstCommitHash) {
                $planBaseName = [System.IO.Path]::GetFileNameWithoutExtension($plan.PlanName)
                $grepOutput = git log --all --oneline --format="%H|%aI|%s" `
                    --grep="$planBaseName" -i 2>&1

                if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($grepOutput)) {
                    $commits = @($grepOutput | ForEach-Object {
                        $parts = $_ -split '\|', 3
                        if ($parts.Count -eq 3) {
                            [PSCustomObject]@{
                                Hash    = $parts[0]
                                Date    = $parts[1]
                                Subject = $parts[2]
                            }
                        }
                    })

                    if ($commits.Count -gt 0) {
                        $earliest = $commits | Sort-Object Date | Select-Object -First 1
                        $firstCommitHash = $earliest.Hash
                        $firstCommitDate = $earliest.Date
                        $firstCommitSubject = $earliest.Subject
                        $matchedStem = $planBaseName
                    }
                }
            }

            # Fallback: find when the file was first ADDED to git (--diff-filter=A)
            if (-not $firstCommitHash) {
                $filePath = $plan.FilePath
                $addOutput = git log --all --diff-filter=A --format="%H|%aI|%s" `
                    -- "$filePath" 2>&1

                if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($addOutput)) {
                    # Take the earliest addition (last line, since git log is newest-first)
                    $addLines = @($addOutput | Where-Object { $_ -match '\|' })
                    if ($addLines.Count -gt 0) {
                        $parts = ($addLines[-1]) -split '\|', 3
                        if ($parts.Count -eq 3) {
                            $firstCommitHash = $parts[0]
                            $firstCommitDate = $parts[1]
                            $firstCommitSubject = $parts[2]
                            $matchedStem = "(file-added)"
                        }
                    }
                }
            }

            $datedPlans += [ordered]@{
                PlanName          = $plan.PlanName
                FilePath          = $plan.FilePath
                DeleteCommit      = $plan.DeleteCommit
                DeleteDate        = $plan.DeleteDate
                AliveCommit       = $plan.AliveCommit
                EstimatedStart    = if ($firstCommitDate) { $firstCommitDate } else { "(unknown)" }
                StartCommit       = if ($firstCommitHash) { $firstCommitHash } else { "(none)" }
                StartSubject      = if ($firstCommitSubject) { $firstCommitSubject } else { "(no matching commits)" }
                MatchedStem       = if ($matchedStem) { $matchedStem } else { "(no match)" }
            }
        }

        # Sort by estimated start date (unknowns at the end)
        $sorted = $datedPlans | Sort-Object {
            if ($_.EstimatedStart -eq "(unknown)") { "9999-99-99" }
            else { $_.EstimatedStart }
        }

        Write-Host "Dated $($sorted.Count) plan(s):" -ForegroundColor Green
        Write-Host ""
        Write-Host ("{0,-50} {1,-12} {2,-12} {3}" -f "Plan", "Start", "Deleted", "Matched Stem") -ForegroundColor DarkGray
        Write-Host ("{0,-50} {1,-12} {2,-12} {3}" -f "----", "-----", "-------", "------------") -ForegroundColor DarkGray

        foreach ($p in $sorted) {
            $start = if ($p.EstimatedStart -ne "(unknown)") { $p.EstimatedStart.Substring(0,10) } else { "???" }
            $deleted = $p.DeleteDate.Substring(0,10)
            $color = if ($p.EstimatedStart -eq "(unknown)") { "DarkYellow" } else { "White" }
            Write-Host ("{0,-50} {1,-12} {2,-12} {3}" -f $p.PlanName, $start, $deleted, $p.MatchedStem) -ForegroundColor $color
        }

        $sorted | ConvertTo-Json -Depth 5 | Set-Content -Path $phase2Output -Encoding UTF8
        Write-Host ""
        Write-Host "Output written to: $phase2Output" -ForegroundColor Green
        Write-Host ""
        Write-Host "TIP: Review and manually adjust dates in $phase2Output before running -Phase Extract" -ForegroundColor DarkCyan
    }
    finally {
        Pop-Location
    }
}

# ============================================================================
# Phase 3: Extract goal descriptions from plan files via git show
# ============================================================================
function Invoke-PhaseExtract {
    Write-StepHeader "Phase 3: Extracting goal descriptions from deleted plans"

    Assert-FileExists -Path $phase2Output -PhaseName "Date"

    $plans = Get-Content $phase2Output -Raw | ConvertFrom-Json

    if ($plans -isnot [System.Array]) {
        $plans = @($plans)
    }

    Push-Location $projectRoot
    try {
        # Ensure git outputs UTF-8 so em-dashes and other Unicode survive
        $previousEncoding = [Console]::OutputEncoding
        [Console]::OutputEncoding = [System.Text.Encoding]::UTF8

        $output = @()
        $output += "# Recovered Plan Goals"
        $output += ""
        $output += "Auto-generated by ``Recover-PlanHistory.ps1 -Phase Extract``"
        $output += "Review and condense these into Engineering Diary entries."

        $successCount = 0
        $failCount = 0

        foreach ($plan in $plans) {
            $output += ""

            $startDate = if ($plan.EstimatedStart -ne "(unknown)") {
                $plan.EstimatedStart.Substring(0, 10)
            } else { "YYYY-MM-DD" }

            $deletedDate = $plan.DeleteDate.Substring(0, 10)

            $output += "## $startDate - $($plan.PlanName)"
            $output += ""
            $output += "Type: Implementation"
            $output += "Period: $startDate to $deletedDate"
            if ($plan.StartCommit -ne "(none)") {
                $output += "StartCommit: ``$($plan.StartCommit.Substring(0,8))``"
            }

            # Try to retrieve the file content from git
            $aliveCommit = $plan.AliveCommit
            $filePath = $plan.FilePath

            $content = git show "${aliveCommit}:${filePath}" 2>&1

            if ($LASTEXITCODE -ne 0) {
                # Try without the docs/ prefix in case path changed
                $altPath = "docs/$($plan.PlanName)"
                $content = git show "${aliveCommit}:${altPath}" 2>&1

                if ($LASTEXITCODE -ne 0) {
                    # Try just the filename at root
                    $content = git show "${aliveCommit}:$($plan.PlanName)" 2>&1
                }
            }

            if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($content)) {
                $rawContent = $content -join "`n"
                $goal = Extract-GoalFromContent -Content $rawContent
                $output += "Context: $goal"
                $output += "Change: (TODO: summarize what was implemented)"
                $successCount++
                Write-Host "  OK: $($plan.PlanName)" -ForegroundColor Green
            }
            else {
                $output += "Context: (Could not retrieve file content from git)"
                $output += "Change: (TODO: fill in manually)"
                $failCount++
                Write-Host "  FAIL: $($plan.PlanName) - could not retrieve from git" -ForegroundColor Red
            }

            $output += "Evidence: Plan completed and deleted."
            $output += "Refs: $($plan.FilePath)"
        }

        [Console]::OutputEncoding = $previousEncoding

        $output | Set-Content -Path $phase3Output -Encoding UTF8

        Write-Host ""
        Write-Host "Extracted goals: $successCount OK, $failCount failed" -ForegroundColor Green
        Write-Host "Output written to: $phase3Output" -ForegroundColor Green
        Write-Host ""
        Write-Host "NEXT STEPS:" -ForegroundColor Cyan
        Write-Host "  1. Review $phase3Output" -ForegroundColor White
        Write-Host "  2. Condense each 'Context:' into a single-line summary" -ForegroundColor White
        Write-Host "  3. Fill in 'Change:' with what was actually implemented" -ForegroundColor White
        Write-Host "  4. Copy polished entries into docs/EngineeringDiary.md" -ForegroundColor White
    }
    finally {
        Pop-Location
    }
}

# ============================================================================
# Phase Validate: Cross-check with FutureIdeas SourceDoc references
# ============================================================================
function Invoke-PhaseValidate {
    Write-StepHeader "Validate: Cross-checking with FutureIdeas SourceDoc references"

    Assert-FileExists -Path $phase1Output -PhaseName "Find"

    $plans = Get-Content $phase1Output -Raw | ConvertFrom-Json
    if ($plans -isnot [System.Array]) {
        $plans = @($plans)
    }
    $foundNames = $plans | ForEach-Object { $_.PlanName }

    # Parse SourceDoc references from FutureIdeas.md
    $futureIdeasPath = Join-Path $outputDir "FutureIdeas.md"
    if (-not (Test-Path $futureIdeasPath)) {
        Write-Host "WARNING: FutureIdeas.md not found at $futureIdeasPath" -ForegroundColor Yellow
        return
    }

    $sourceDocNames = @()
    Get-Content $futureIdeasPath | ForEach-Object {
        if ($_ -match '^\s*-\s*SourceDoc:\s*(.+\.md)\s*$') {
            $name = $Matches[1].Trim()
            if ($name -match '^Plan\.' -and $name -notin $sourceDocNames) {
                $sourceDocNames += $name
            }
        }
    }

    Write-Host "Plan files found in git (deleted): $($foundNames.Count)" -ForegroundColor White
    Write-Host "Plan files referenced in FutureIdeas.md: $($sourceDocNames.Count)" -ForegroundColor White
    Write-Host ""

    # Plans in FutureIdeas but NOT found in git
    $missingFromGit = $sourceDocNames | Where-Object { $_ -notin $foundNames }
    if ($missingFromGit.Count -gt 0) {
        Write-Host "Referenced in FutureIdeas but NOT found as deleted in git:" -ForegroundColor Yellow
        foreach ($name in $missingFromGit) {
            Write-Host "  - $name" -ForegroundColor Yellow
        }
    }
    else {
        Write-Host "All FutureIdeas SourceDoc references found in git history." -ForegroundColor Green
    }

    Write-Host ""

    # Plans in git but NOT referenced in FutureIdeas
    $extraInGit = $foundNames | Where-Object { $_ -notin $sourceDocNames }
    if ($extraInGit.Count -gt 0) {
        Write-Host "Found in git but NOT referenced in FutureIdeas:" -ForegroundColor Cyan
        foreach ($name in $extraInGit) {
            Write-Host "  - $name (may not have had future ideas harvested)" -ForegroundColor Cyan
        }
    }
    else {
        Write-Host "All deleted plans are referenced in FutureIdeas." -ForegroundColor Green
    }

    # Check for plans still alive on disk
    Write-Host ""
    $stillAlive = Get-ChildItem -Path $outputDir -Filter "Plan.*.md" -ErrorAction SilentlyContinue
    if ($stillAlive.Count -gt 0) {
        Write-Host "Plan files still on disk (not yet deleted/completed):" -ForegroundColor Magenta
        foreach ($f in $stillAlive) {
            Write-Host "  - $($f.Name)" -ForegroundColor Magenta
        }
    }
    else {
        Write-Host "No Plan.*.md files currently on disk." -ForegroundColor DarkGray
    }
}

# ============================================================================
# Main dispatch
# ============================================================================

try {
    switch ($Phase) {
        'Find'     { Invoke-PhaseFind }
        'Date'     { Invoke-PhaseDate }
        'Extract'  { Invoke-PhaseExtract }
        'Validate' { Invoke-PhaseValidate }
    }
}
catch {
    Write-Host ""
    Write-Host "ERROR: $_" -ForegroundColor Red
    Write-Host $_.ScriptStackTrace -ForegroundColor DarkRed
    exit 1
}
