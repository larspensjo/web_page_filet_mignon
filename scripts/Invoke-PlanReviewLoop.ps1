<#
Plan review + revision loop across 3 CLIs:
- codex  : review (gpt-5.4, reasoning=high)
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

Import-Module (Join-Path $PSScriptRoot 'lib\AgentCli.psm1') -Force -DisableNameChecking

# =============================================================================
# CONFIGURATION (edit here first)
# =============================================================================

function Get-ReviewerModel {
    param([Parameter(Mandatory)][string]$Tool)
    switch ($Tool) {
        'codex'  { 'gpt-5.5' }
        'gemini' { 'gemini-3.1-pro-preview' }
        'claude' { $null }   # keep configured default
    }
}

function Get-ReviewerReasoning {
    param([Parameter(Mandatory)][string]$Tool)
    if ($Tool -eq 'codex') { 'high' } else { $null }
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
  ReviewName  = { param($id,$model) "Review.$id.$model.md" }
  LogName     = { param($id) "ReviewLoop.$id.log" }
}

# =============================================================================
# Main
# =============================================================================

$RepoRoot = Resolve-FullPath -Path $RepoRoot -BasePath (Get-Location).Path -MustExist
$PlanPath = Resolve-FullPath -Path $PlanPath -BasePath (Get-Location).Path -MustExist
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

  $reviewText = Invoke-Cli -Tool $reviewer -Prompt $reviewPrompt -WorkingDir $RepoRoot `
      -Model (Get-ReviewerModel $reviewer) -Reasoning (Get-ReviewerReasoning $reviewer)

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
  $updatedPlan = Invoke-Cli -Tool $PlanModel -Prompt $updatePrompt -WorkingDir $RepoRoot `
      -Model (Get-ReviewerModel $PlanModel) -Reasoning (Get-ReviewerReasoning $PlanModel)

  Write-AtomicUtf8 -Path $PlanPath -Content $updatedPlan
  Add-LogLine $logPath "Updated plan written: $PlanPath"
  Add-LogLine $logPath ""
}

Add-LogLine $logPath "Completed: $(Get-Date)"
Write-Output "Done. Plan updated: $PlanPath. Reviews: $PlansDir\Review.$planId.*.md. Log: $logPath"

<#
NOTES / TWEAK POINTS

1) Codex reasoning level:
   This script uses:  codex exec -c reasoning.level="high"
   If your codex expects a different dotted config key, change Get-ReviewerReasoning.

2) Change models:
   - codex:   edit Get-ReviewerModel to return a different string for 'codex'
   - gemini:  edit Get-ReviewerModel to return a different string for 'gemini'
   - claude:  return $null to use your configured default

3) Safety:
   If you want "read-only" tool usage for reviewers, pass extra args via the
   Invoke-Cli -ExtraArgs parameter, e.g.:
     Invoke-Cli ... -ExtraArgs @('--permission-mode','plan')   # claude
     Invoke-Cli ... -ExtraArgs @('--approval-mode','plan')     # gemini
#>
