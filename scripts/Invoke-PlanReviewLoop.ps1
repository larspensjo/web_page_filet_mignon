<#
Plan review + revision loop across 3 CLIs:
- codex  : review (gpt-5.3-codex, reasoning=medium)
- claude : update plan (default model, non-interactive via -p)
- gemini : review (gemini-3.1-pro-preview)

Usage:
  -Reviewers codex,claude               # two reviewers (one pass each)
  -Reviewers claude                     # single reviewer
  -Reviewers codex,claude,codex,claude  # repeated sequence

Design goals:
- Configurable: all knobs at top (models, reasoning, prompts, file naming).
- Robust: supports different CLIs/argument styles; good errors; safe writes; backups.
- Flexible: easy to add more reviewers, change prompt templates, change CLI flags.
#>

[CmdletBinding()]
param(
  # Existing plan file path (typically docs/plans/Plan.XXXX.md)
  [Parameter(Mandatory)]
  [string]$PlanPath,

  # Model that created the plan and will update it (claude|codex|gemini)
  [Parameter(Mandatory)]
  [ValidateSet('claude','codex','gemini')]
  [string]$PlanModel,

  # Ordered list of reviewer models (1 or more; may repeat for multi-pass)
  [Parameter(Mandatory)]
  [ValidateSet('claude','codex','gemini')]
  [ValidateCount(1, 99)]
  [string[]]$Reviewers,

  # Root directory for CLI execution (working dir / repo root)
  [string]$RepoRoot = (Get-Location).Path,

  # Where plan/review docs live (default: <RepoRoot>/docs/plans)
  [string]$PlansDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# =============================================================================
# CONFIGURATION (edit here first)
# =============================================================================

# ---- CLI behavior / models ----
$CliConfig = @{
  codex = @{
    # Non-interactive command:
    # - codex exec [PROMPT]
    # Model selection:
    # -m, --model <MODEL>
    # Reasoning level: not shown in -h output; assume supported via config override:
    #   -c reasoning.level="medium"  (string parsed as TOML)
    #
    # If your codex uses a different dotted key, change it below.
    ExecArgs = @('exec')
    ModelArgs = { param($modelName) @('--model', $modelName) }
    ReasoningArgs = { param($level) @('--config', "reasoning.level=`"$level`"") } # change key if needed
    DefaultModel = 'gpt-5.4'
    DefaultReasoning = 'heigh'
    # Optional safety defaults (you can tweak):
    ExtraArgs = @() # e.g. @('--sandbox','workspace-write','-a','on-request')
  }

  claude = @{
    # Non-interactive:
    # -p, --print
    # Model: --model <model> (we will NOT set by default; uses your configured default)
    PrintArgs = @('-p')
    ModelArgs = { param($modelName) @('--model', $modelName) }
    DefaultModel = $null  # keep default (e.g. Sonnet 4.6)
    ExtraArgs = @('--no-session-persistence') # keep runs isolated/deterministic in -p mode
  }

  gemini = @{
    # Non-interactive:
    # -p, --prompt "<prompt>"
    # Model:
    # -m, --model "<model>"
    PromptArgs = { param($prompt) @('-p', $prompt) }
    ModelArgs  = { param($modelName) @('-m', $modelName) }
    DefaultModel = 'gemini-3.1-pro-preview'
    ExtraArgs = @() # e.g. @('--approval-mode','plan')
  }
}

# ---- Review/Update prompt templates ----
# Keep these as functions so you can tweak structure easily.

function New-ReviewPrompt {
  param(
    [Parameter(Mandatory)][string]$ReviewerModel,
    [Parameter(Mandatory)][string]$PlanText
  )

@"
You have the role of a senior software engineer performing a review of an implementation plan.

Rules:
- Output Markdown only.
- Read-only review only. Do not edit files, do not claim to have edited files, and do not ask for write permissions.
- Be concrete and actionable.
- Focus on: correctness, missing requirements, sequencing, risks, test strategy, maintainability, and integration/rollout.
- Prefer prioritized bullets; include rationale briefly.
- If something is ambiguous, ask explicit questions.
- Check with source code to make sure the plan is correct.
- Elegant, robust and flexible solutions have a high priority.
- Are there opportunities for simplification or improvement without sacrificing correctness or robustness?
- See extra instructions in Agents.md.

Output headings:
# Review by $ReviewerModel
## Summary
## Strengths
## Risks / Gaps
## Suggested Improvements (prioritized)
## Questions / Assumptions
## Quick Checklist

--- BEGIN PLAN ---
$PlanText
--- END PLAN ---
"@
}

function New-UpdatePrompt {
  param(
    [Parameter(Mandatory)][string]$PlanModel,
    [Parameter(Mandatory)][string]$PlanPath,
    [Parameter(Mandatory)][string]$ReviewPath,
    [Parameter(Mandatory)][string]$PlanText,
    [Parameter(Mandatory)][string]$ReviewText
  )

@"
You are updating a software implementation plan based on a review.

Rules:
- Output the FULL UPDATED PLAN as Markdown only (no commentary).
- Return text only. Do not edit files, do not claim to have edited files, and do not ask for write permissions.
- Do not blindly accept review feedback. Independently validate each suggested change for correctness and relevance against the current plan and source code.
- Only apply suggestions that are correct and improve the plan. If a suggestion is incorrect, redundant, or out of scope, keep the plan behavior and add a brief rationale under a "Notes" section.
- Preserve useful structure; improve clarity and sequencing.
- Ensure the plan remains actionable: steps, milestones, acceptance criteria, test plan.
- Resolve issues raised in the review. If you intentionally do not apply a suggestion, incorporate a brief justification in the plan (e.g. under "Notes" or "Assumptions").

The caller will overwrite the file using your returned Markdown text.

Context:
Updater model: $PlanModel
Plan path: $PlanPath
Review path: $ReviewPath

--- BEGIN CURRENT PLAN ---
$PlanText
--- END CURRENT PLAN ---

--- BEGIN REVIEW ---
$ReviewText
--- END REVIEW ---
"@
}

# ---- File naming ----
# Plan is typically Plan.XXXX.md
# Reviews saved as Review.XXXX.MODEL.md
$FileNamePatterns = @{
  PlanIdRegex = '^(?i)Plan\.(?<id>[^.]+)\.md$'
  ReviewName  = { param($id,$model) "Review.$id.$model.md" }
  LogName     = { param($id) "ReviewLoop.$id.log" }
}

# ---- Logging ----
$LogTimestamps = $true

# =============================================================================
# Helpers
# =============================================================================

function Resolve-FullPath([string]$Path) {
  (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
}

function Set-Utf8ProcessEncoding {
  $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
  [Console]::InputEncoding = $utf8NoBom
  [Console]::OutputEncoding = $utf8NoBom
  $global:OutputEncoding = $utf8NoBom
}

[Diagnostics.CodeAnalysis.SuppressMessageAttribute]
function Ensure-Dir([string]$DirPath) {
  if (-not (Test-Path -LiteralPath $DirPath)) {
    New-Item -ItemType Directory -Path $DirPath | Out-Null
  }
}

function Get-PlanIdFromPath([string]$PlanPath) {
  $fileName = [IO.Path]::GetFileName($PlanPath)
  $m = [regex]::Match($fileName, $FileNamePatterns.PlanIdRegex)
  if ($m.Success) { return $m.Groups['id'].Value }
  return (Get-Date).ToString('yyyyMMdd-HHmmss')
}

function Write-AtomicUtf8([string]$Path, [string]$Content) {
  $dir = Split-Path -Parent $Path
  Ensure-Dir $dir

  $tmp = Join-Path $dir ("~tmp.{0}.{1}.tmp" -f ([IO.Path]::GetFileName($Path)), ([guid]::NewGuid().ToString('N').Substring(0,8)))
  [IO.File]::WriteAllText($tmp, $Content, (New-Object System.Text.UTF8Encoding($false)))
  Move-Item -Force -LiteralPath $tmp -Destination $Path
}

function Assert-CliExists([string]$CliName) {
  $cmd = Get-Command $CliName -ErrorAction SilentlyContinue
  if (-not $cmd) { throw "CLI '$CliName' not found in PATH." }
}

function Add-LogLine([string]$LogPath, [string]$Line) {
  if ($LogTimestamps) {
    $ts = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss')
    Add-Content -LiteralPath $LogPath -Value "[$ts] $Line" -Encoding utf8
  } else {
    Add-Content -LiteralPath $LogPath -Value $Line -Encoding utf8
  }
}

[Diagnostics.CodeAnalysis.SuppressMessageAttribute]
function Normalize-Text([object]$Output) {
  $s = ($Output | Out-String)
  # Trim only excessive leading/trailing whitespace; keep interior formatting.
  return $s.Trim()
}

function Invoke-Cli {
  <#
    Invokes one of: claude | codex | gemini
    Returns: stdout as string (trimmed). Throws on non-zero exit or empty output.
  #>
  param(
    [Parameter(Mandatory)][ValidateSet('claude','codex','gemini')][string]$Tool,
    [Parameter(Mandatory)][string]$Prompt,
    [Parameter(Mandatory)][string]$WorkingDir,

    # Optional overrides for model selection:
    [string]$Model = $null,

    # Optional for codex reasoning:
    [string]$Reasoning = $null
  )

  Assert-CliExists $Tool
  Push-Location $WorkingDir
  try {
    $cliArgs = @()

    switch ($Tool) {
      'codex' {
        $cfg = $CliConfig.codex
        $cliArgs += $cfg.ExecArgs
        $cliArgs += $cfg.ExtraArgs

        $modelToUse = if ($Model) { $Model } else { $cfg.DefaultModel }
        if ($modelToUse) { $cliArgs += & $cfg.ModelArgs $modelToUse }

        $reasonToUse = if ($Reasoning) { $Reasoning } else { $cfg.DefaultReasoning }
        if ($reasonToUse) { $cliArgs += & $cfg.ReasoningArgs $reasonToUse }

        # codex exec supports [PROMPT] positional
        $cliArgs += $Prompt
      }

      'claude' {
        $cfg = $CliConfig.claude
        $cliArgs += $cfg.PrintArgs
        $cliArgs += $cfg.ExtraArgs

        $modelToUse = if ($Model) { $Model } else { $cfg.DefaultModel }
        if ($modelToUse) { $cliArgs += & $cfg.ModelArgs $modelToUse }

        # prompt is positional
        $cliArgs += $Prompt
      }

      'gemini' {
        $cfg = $CliConfig.gemini
        $cliArgs += $cfg.ExtraArgs

        $modelToUse = if ($Model) { $Model } else { $cfg.DefaultModel }
        if ($modelToUse) { $cliArgs += & $cfg.ModelArgs $modelToUse }

        $cliArgs += & $cfg.PromptArgs $Prompt
      }
    }

    $argStr = ($cliArgs -join ' ')
    if ($argStr.Length -gt 120) { $argStr = $argStr.Substring(0, 117) + '...' }
    Write-Verbose "Invoking CLI '$Tool' with args: $argStr"

    # codex's npm wrapper (codex.ps1) sets StandardOutputEncoding on the node
    # process it spawns, which requires stdout to be truly redirected at the OS
    # level. PowerShell pipeline capture ($out = & tool) doesn't satisfy this,
    # causing "StandardOutputEncoding is only supported when standard output is
    # redirected". Workaround: redirect to a temp file (real OS file redirect).
    if ($Tool -eq 'codex') {
      $tmpOut = [IO.Path]::GetTempFileName()
      try {
        & $Tool @cliArgs > $tmpOut
        $exit = $LASTEXITCODE
        $out = Get-Content -LiteralPath $tmpOut -Raw -Encoding utf8
      } finally {
        Remove-Item -LiteralPath $tmpOut -ErrorAction SilentlyContinue
      }
    } else {
      $out = & $Tool @cliArgs
      $exit = $LASTEXITCODE
    }

    if ($exit -ne 0) {
      throw "CLI '$Tool' exited with code $exit. Args: $($cliArgs -join ' ')"
    }

    $text = Normalize-Text $out
    if ([string]::IsNullOrWhiteSpace($text)) {
      throw "CLI '$Tool' returned empty output. Args: $($cliArgs -join ' ')"
    }

    return $text
  }
  finally {
    Pop-Location
  }
}

# =============================================================================
# Main
# =============================================================================

$RepoRoot = Resolve-FullPath $RepoRoot
$PlanPath = Resolve-FullPath $PlanPath
Set-Utf8ProcessEncoding

if (-not $PlansDir) { $PlansDir = Join-Path $RepoRoot 'docs\plans' }
Ensure-Dir $PlansDir

$planId = Get-PlanIdFromPath $PlanPath
$logPath = Join-Path $PlansDir (& $FileNamePatterns.LogName $planId)

Write-AtomicUtf8 $logPath @"
Started: $(Get-Date)
PlanPath: $PlanPath
PlanId: $planId
PlanModel: $PlanModel
Reviewers: $($Reviewers -join ', ')
RepoRoot: $RepoRoot
PlansDir: $PlansDir

"@

$hasDuplicates = @($Reviewers | Sort-Object -Unique).Count -lt $Reviewers.Count
$round = 0

foreach ($reviewer in $Reviewers) {
  $round++

  Add-LogLine $logPath "=== Round ${round}: reviewer=$reviewer, updater=$PlanModel ==="

  $planText = Get-Content -LiteralPath $PlanPath -Raw -Encoding utf8

  # 1) Reviewer generates review
  $reviewPrompt = New-ReviewPrompt -ReviewerModel $reviewer -PlanText $planText
  Add-LogLine $logPath "Invoking reviewer '$reviewer'..."

  $reviewText = Invoke-Cli -Tool $reviewer -Prompt $reviewPrompt -WorkingDir $RepoRoot

  $reviewFileName = if ($hasDuplicates) {
    & $FileNamePatterns.ReviewName $planId "R$round.$reviewer"
  } else {
    & $FileNamePatterns.ReviewName $planId $reviewer
  }
  $reviewPath = Join-Path $PlansDir $reviewFileName
  Write-AtomicUtf8 -Path $reviewPath -Content $reviewText
  Add-LogLine $logPath "Saved review: $reviewPath"

  # 2) PlanModel updates the plan using the review
  $planText2 = Get-Content -LiteralPath $PlanPath -Raw -Encoding utf8
  $updatePrompt = New-UpdatePrompt -PlanModel $PlanModel -PlanPath $PlanPath -ReviewPath $reviewPath -PlanText $planText2 -ReviewText $reviewText

  Add-LogLine $logPath "Invoking updater '$PlanModel'..."
  $updatedPlan = Invoke-Cli -Tool $PlanModel -Prompt $updatePrompt -WorkingDir $RepoRoot

  Write-AtomicUtf8 -Path $PlanPath -Content $updatedPlan
  Add-LogLine $logPath "Updated plan written: $PlanPath"
  Add-LogLine $logPath ""
}

Add-LogLine $logPath "Completed: $(Get-Date)"
Write-Output "Done. Plan updated: $PlanPath. Reviews: $PlansDir\Review.$planId.*.md. Log: $logPath"

<#
NOTES / TWEAK POINTS

1) Codex reasoning level:
   The codex -h output you pasted does not show a direct flag for “reasoning”.
   This script uses:  codex exec --config reasoning.level="medium"
   If your codex expects a different dotted config key, change it here:
     $CliConfig.codex.ReasoningArgs

2) Change models:
   - codex:   $CliConfig.codex.DefaultModel
   - gemini:  $CliConfig.gemini.DefaultModel
   - claude:  leave null to use your default (Sonnet 4.6 as you said)

3) Safety:
   If you want “read-only” tool usage for reviewers, add:
     $CliConfig.codex.ExtraArgs = @('--sandbox','read-only','-a','never')
     $CliConfig.gemini.ExtraArgs = @('--approval-mode','plan')
     $CliConfig.claude.ExtraArgs = @('--permission-mode','plan')
#>
