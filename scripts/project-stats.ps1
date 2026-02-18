#Requires -Version 5.1

# ============================================================================
# Project Statistics Report Generator
# ============================================================================
#
# Usage: .\project-stats.ps1
#
# Generates a comprehensive statistics report for the web_page_filet_mignon
# project, including:
# - Rust source code lines by crate
# - PowerShell script lines
# - Documentation lines (plans, requirements, other)
# - External dependency count
# ============================================================================

# --- Configuration ---
$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot

# --- Helper Functions ---

function Count-Lines {
    param(
        [Parameter(Mandatory=$true)]
        [System.IO.FileInfo[]]$Files
    )

    if ($Files.Count -eq 0) {
        return 0
    }

    $totalLines = 0
    foreach ($file in $Files) {
        try {
            $totalLines += (Get-Content $file.FullName -ErrorAction SilentlyContinue).Count
        } catch {
            Write-Warning "Could not read file: $($file.FullName)"
        }
    }

    return $totalLines
}

function Count-RustTests {
    param(
        [Parameter(Mandatory=$true)]
        [System.IO.FileInfo[]]$Files
    )

    if ($Files.Count -eq 0) {
        return 0
    }

    $totalTests = 0
    foreach ($file in $Files) {
        try {
            $content = Get-Content $file.FullName -Raw -ErrorAction SilentlyContinue
            if ($content) {
                # Match test attributes: #[test], #[tokio::test], #[rstest], etc.
                $testMatches = [regex]::Matches($content, '#\[(?:[\w:]+::)?test[^\]]*\]')
                $totalTests += $testMatches.Count
            }
        } catch {
            Write-Warning "Could not read file: $($file.FullName)"
        }
    }

    return $totalTests
}

function Get-WorkspaceCrates {
    $workspaceCargoToml = Join-Path $projectRoot "Cargo.toml"
    if (-not (Test-Path $workspaceCargoToml)) {
        return @()
    }

    $workspaceContent = Get-Content $workspaceCargoToml -Raw -ErrorAction Stop
    if ($workspaceContent -notmatch '(?s)\[workspace\](.*?)(\r?\n\[|$)') {
        return @()
    }

    $workspaceSection = $matches[1]
    if ($workspaceSection -notmatch '(?s)members\s*=\s*\[(.*?)\]') {
        return @()
    }

    $membersSection = $matches[1]
    $memberMatches = [regex]::Matches($membersSection, '"([^"]+)"')
    $crates = @()

    foreach ($memberMatch in $memberMatches) {
        $relativeMemberPath = $memberMatch.Groups[1].Value.Replace('/', '\')
        $cratePath = Join-Path $projectRoot $relativeMemberPath
        if (-not (Test-Path $cratePath)) {
            continue
        }

        $crateCargoToml = Join-Path $cratePath "Cargo.toml"
        $crateName = Split-Path $cratePath -Leaf
        if (Test-Path $crateCargoToml) {
            $crateCargoContent = Get-Content $crateCargoToml -Raw -ErrorAction SilentlyContinue
            if ($crateCargoContent -match '(?s)\[package\](.*?)(\r?\n\[|$)') {
                $packageSection = $matches[1]
                if ($packageSection -match '(?m)^\s*name\s*=\s*"([^"]+)"') {
                    $crateName = $matches[1]
                }
            }
        }

        $crates += @{
            Name = $crateName
            CratePath = $cratePath
        }
    }

    return $crates
}

function Get-RustStats {
    $modules = Get-WorkspaceCrates

    $stats = @{}

    foreach ($module in $modules) {
        $modulePath = Join-Path $module.CratePath "src"
        $testsPath = Join-Path $module.CratePath "tests"
        $allFiles = @()

        # Collect source files
        if (Test-Path $modulePath) {
            $allFiles += Get-ChildItem -Path $modulePath -Filter "*.rs" -Recurse -File
        }

        # Also check for integration tests directory at crate level.
        if (Test-Path $testsPath) {
            $allFiles += Get-ChildItem -Path $testsPath -Filter "*.rs" -Recurse -File
        }

        if ($allFiles.Count -gt 0) {
            $lineCount = Count-Lines -Files $allFiles
            $testCount = Count-RustTests -Files $allFiles
            $stats[$module.Name] = @{
                Lines = $lineCount
                Files = $allFiles.Count
                Tests = $testCount
            }
        } else {
            $stats[$module.Name] = @{
                Lines = 0
                Files = 0
                Tests = 0
            }
        }
    }

    return $stats
}

function Get-SubmoduleStats {
    $submodulePath = Join-Path $projectRoot "src\CommanDuctUI\src"
    $allFiles = @()

    # Collect source files from submodule
    if (Test-Path $submodulePath) {
        $allFiles += Get-ChildItem -Path $submodulePath -Filter "*.rs" -Recurse -File
    }

    if ($allFiles.Count -gt 0) {
        $lineCount = Count-Lines -Files $allFiles
        $testCount = Count-RustTests -Files $allFiles
        return @{
            Lines = $lineCount
            Files = $allFiles.Count
            Tests = $testCount
        }
    } else {
        return @{
            Lines = 0
            Files = 0
            Tests = 0
        }
    }
}

function Get-DocumentationStats {
    $stats = @{
        Planning = @{Lines = 0; Files = 0}
        Other = @{Lines = 0; Files = 0}
    }

    $docsPath = Join-Path $projectRoot "docs"
    if (Test-Path $docsPath) {
        # Planning documents (Plan.*.md)
        $planFiles = Get-ChildItem -Path $docsPath -Filter "Plan.*.md" -File
        $stats.Planning.Lines = Count-Lines -Files $planFiles
        $stats.Planning.Files = $planFiles.Count

        # Other documentation (everything else in docs/)
        $allDocs = Get-ChildItem -Path $docsPath -Filter "*.md" -Recurse -File
        $otherDocs = $allDocs | Where-Object {
            $_.Name -notmatch "^Plan\."
        }
        $stats.Other.Lines = Count-Lines -Files $otherDocs
        $stats.Other.Files = $otherDocs.Count
    }

    # Include README.md and other top-level docs
    $readmePath = Join-Path $projectRoot "README.md"
    if (Test-Path $readmePath) {
        $stats.Other.Lines += (Get-Content $readmePath -ErrorAction SilentlyContinue).Count
        $stats.Other.Files += 1
    }

    return $stats
}

function Get-PowerShellStats {
    $allPsFiles = @()

    # Scripts directory
    $scriptsPath = Join-Path $projectRoot "scripts"
    if (Test-Path $scriptsPath) {
        $allPsFiles += Get-ChildItem -Path $scriptsPath -Filter "*.ps1" -File
    }

    # Root directory scripts
    $rootPsFiles = Get-ChildItem -Path $projectRoot -Filter "*.ps1" -File -ErrorAction SilentlyContinue
    if ($rootPsFiles) {
        $allPsFiles += $rootPsFiles
    }

    return @{
        Lines = Count-Lines -Files $allPsFiles
        Files = $allPsFiles.Count
    }
}

function Get-DependencyCount {
    $cargoTomlPath = Join-Path $projectRoot "Cargo.toml"

    if (-not (Test-Path $cargoTomlPath)) {
        return 0
    }

    $content = Get-Content $cargoTomlPath -Raw

    # Extract [workspace.dependencies] section (for workspace root)
    if ($content -match '(?s)\[workspace\.dependencies\](.*?)(\r?\n\[|$)') {
        $depsSection = $matches[1]

        # Count lines that look like dependencies (exclude comments and empty lines)
        $deps = $depsSection -split "`n" | Where-Object {
            $_ -match '^\s*[a-zA-Z0-9_-]+\s*='
        }

        return $deps.Count
    }

    # Fallback to [dependencies] section for non-workspace crates
    if ($content -match '(?s)\[dependencies\](.*?)(\[|$)') {
        $depsSection = $matches[1]

        # Count lines that look like dependencies (exclude workspace members and comments)
        $deps = $depsSection -split "`n" | Where-Object {
            $_ -match '^\s*[a-zA-Z0-9_-]+\s*=' -and
            $_ -notmatch 'workspace\s*=\s*true'
        }

        return $deps.Count
    }

    return 0
}

function Show-StatisticsReport {
    param(
        [hashtable]$RustStats,
        [hashtable]$SubmoduleStats,
        [hashtable]$PowerShellStats,
        [hashtable]$DocStats,
        [int]$DependencyCount
    )

    # Calculate totals
    $totalRustLines = ($RustStats.Values | ForEach-Object { $_.Lines } | Measure-Object -Sum).Sum
    $totalRustTests = ($RustStats.Values | ForEach-Object { $_.Tests } | Measure-Object -Sum).Sum
    $totalDocLines = ($DocStats.Values | ForEach-Object { $_.Lines } | Measure-Object -Sum).Sum
    $totalProjectLines = $totalRustLines + $PowerShellStats.Lines + $totalDocLines

    # Header
    Write-Host ""
    Write-Host "================================================================" -ForegroundColor Cyan
    Write-Host "          PROJECT STATISTICS REPORT                           " -ForegroundColor Cyan
    Write-Host "          web_page_filet_mignon                               " -ForegroundColor Cyan
    Write-Host "================================================================" -ForegroundColor Cyan
    Write-Host ""

    # Rust Source Code Section
    Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host "  RUST SOURCE CODE (by crate)" -ForegroundColor Cyan
    Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host ""

    # Sort by line count descending
    $sortedModules = $RustStats.GetEnumerator() | Sort-Object {$_.Value.Lines} -Descending

    foreach ($module in $sortedModules) {
        if ($module.Value.Lines -gt 0) {
            $dots = "." * (30 - $module.Key.Length)
            Write-Host "  $($module.Key) $dots" -NoNewline
            Write-Host $("{0,10:N0}" -f $module.Value.Lines) -ForegroundColor Green -NoNewline
            Write-Host " lines" -NoNewline
            Write-Host $(" {0,5:N0} unit tests" -f $module.Value.Tests) -ForegroundColor DarkGreen
        }
    }

    Write-Host "  -----------------------------------------" -ForegroundColor DarkGray
    Write-Host "  TOTAL RUST " -NoNewline
    Write-Host $("{0,28:N0}" -f $totalRustLines) -ForegroundColor Yellow -NoNewline
    Write-Host " lines" -ForegroundColor Yellow -NoNewline
    Write-Host $(" {0,5:N0} unit tests" -f $totalRustTests) -ForegroundColor Yellow
    Write-Host ""

    # Submodule Section
    if ($SubmoduleStats.Lines -gt 0) {
        Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
        Write-Host "  GIT SUBMODULES" -ForegroundColor Cyan
        Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
        Write-Host ""

        Write-Host "  CommanDuctUI (submodule) ....." -NoNewline
        Write-Host $("{0,10:N0}" -f $SubmoduleStats.Lines) -ForegroundColor Green -NoNewline
        Write-Host " lines" -NoNewline
        Write-Host $(" {0,5:N0} unit tests" -f $SubmoduleStats.Tests) -ForegroundColor DarkGreen
        Write-Host ""
    }

    # Scripts Section
    Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host "  SCRIPTS" -ForegroundColor Cyan
    Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host ""

    Write-Host "  PowerShell Scripts ..........." -NoNewline
    Write-Host $("{0,10:N0}" -f $PowerShellStats.Lines) -ForegroundColor Green -NoNewline
    Write-Host " lines ($($PowerShellStats.Files) files)"
    Write-Host ""

    # Documentation Section
    Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host "  DOCUMENTATION" -ForegroundColor Cyan
    Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host ""

    Write-Host "  Planning Documents ..........." -NoNewline
    Write-Host $("{0,10:N0}" -f $DocStats.Planning.Lines) -ForegroundColor Green -NoNewline
    Write-Host " lines ($($DocStats.Planning.Files) files)"

    Write-Host "  Other Documentation .........." -NoNewline
    Write-Host $("{0,10:N0}" -f $DocStats.Other.Lines) -ForegroundColor Green -NoNewline
    Write-Host " lines ($($DocStats.Other.Files) files)"

    Write-Host "  -----------------------------------------" -ForegroundColor DarkGray
    Write-Host "  TOTAL DOCUMENTATION " -NoNewline
    Write-Host $("{0,19:N0}" -f $totalDocLines) -ForegroundColor Yellow -NoNewline
    Write-Host " lines" -ForegroundColor Yellow
    Write-Host ""

    # Dependencies Section
    Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host "  DEPENDENCIES" -ForegroundColor Cyan
    Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host ""

    Write-Host "  External Crates .............." -NoNewline
    Write-Host $("{0,10:N0}" -f $DependencyCount) -ForegroundColor Green -NoNewline
    Write-Host " dependencies"
    Write-Host ""

    # Summary Section
    Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host "  SUMMARY" -ForegroundColor Cyan
    Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host ""

    Write-Host "  Total Project Lines .........." -NoNewline
    Write-Host $("{0,10:N0}" -f $totalProjectLines) -ForegroundColor Magenta -NoNewline
    Write-Host " lines" -ForegroundColor Magenta

    $rustPercent = [math]::Round(($totalRustLines / $totalProjectLines) * 100)
    Write-Host $("  Primary Language ............. Rust ({0}% of codebase)" -f $rustPercent) -ForegroundColor Magenta

    $docPercent = [math]::Round(($totalDocLines / ($totalRustLines + $totalDocLines)) * 100)
    Write-Host $("  Documentation Ratio .......... {0}% (docs vs code)" -f $docPercent) -ForegroundColor Magenta
    Write-Host ""

    Write-Host "---------------------------------------------------------------" -ForegroundColor Cyan
    Write-Host ""

    $timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    Write-Host "Generated: $timestamp" -ForegroundColor DarkGray
    Write-Host ""
}

function Invoke-ProjectStatsReport {
    try {
        if (-not (Test-Path $projectRoot)) {
            throw "Project root not found: $projectRoot"
        }

        # Collect all statistics
        Write-Host "Collecting project statistics..." -ForegroundColor Yellow

        $rustStats = Get-RustStats
        $submoduleStats = Get-SubmoduleStats
        $powerShellStats = Get-PowerShellStats
        $docStats = Get-DocumentationStats
        $dependencyCount = Get-DependencyCount

        # Display formatted report
        Show-StatisticsReport `
            -RustStats $rustStats `
            -SubmoduleStats $submoduleStats `
            -PowerShellStats $powerShellStats `
            -DocStats $docStats `
            -DependencyCount $dependencyCount

    } catch {
        Write-Host $("ERROR: {0}" -f $_) -ForegroundColor Red
        Write-Host $_.ScriptStackTrace -ForegroundColor Red
        exit 1
    }
}

# Run report when invoked directly, but not when dot-sourced for tests.
if ($MyInvocation.InvocationName -ne '.') {
    Invoke-ProjectStatsReport
}
