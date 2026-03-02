# Design: Reviewers List Parameter for Invoke-PlanReviewLoop.ps1

## Goal

Replace the fixed `-ReviewModel1` / `-ReviewModel2` parameters with a
flexible `-Reviewers` array, supporting any number of reviewers including
a single reviewer or repeated sequences like `codex,claude,codex,claude`.

## Parameter Changes

Remove `-ReviewModel1` and `-ReviewModel2`. Add:

```powershell
[Parameter(Mandatory)]
[ValidateSet('claude','codex','gemini')]
[ValidateCount(1, 99)]
[string[]]$Reviewers
```

### Usage examples

```powershell
# Single reviewer
.\Invoke-PlanReviewLoop.ps1 -PlanPath ... -PlanModel claude -Reviewers claude

# Two different reviewers (equivalent to old default)
.\Invoke-PlanReviewLoop.ps1 -PlanPath ... -PlanModel claude -Reviewers codex,claude

# Repeated sequence
.\Invoke-PlanReviewLoop.ps1 -PlanPath ... -PlanModel claude -Reviewers codex,claude,codex,claude
```

## Loop

The loop initialisation changes from:

```powershell
$reviewers = @($ReviewModel1, $ReviewModel2)
```

to using `$Reviewers` directly (already a `string[]`). Everything else in
the loop body is unchanged.

## File Naming

Detect duplicates once before the loop:

```powershell
$hasDuplicates = ($Reviewers | Sort-Object -Unique).Count -lt $Reviewers.Count
```

Inside the loop, compute the review file name:

```powershell
$reviewFileName = if ($hasDuplicates) {
    & $FileNamePatterns.ReviewName $planId "R$round.$reviewer"
} else {
    & $FileNamePatterns.ReviewName $planId $reviewer
}
```

The existing `ReviewName` scriptblock (`"Review.$id.$model.md"`) is
unchanged; it receives a different `$model` argument.

### Examples

No duplicates — current naming preserved:

```
Review.MyPlan.codex.md
Review.MyPlan.claude.md
```

With duplicates (`codex,claude,codex,claude`):

```
Review.MyPlan.R1.codex.md
Review.MyPlan.R2.claude.md
Review.MyPlan.R3.codex.md
Review.MyPlan.R4.claude.md
```

## Log Header

Change:

```
Reviewers: $ReviewModel1, $ReviewModel2
```

to:

```
Reviewers: $($Reviewers -join ', ')
```
