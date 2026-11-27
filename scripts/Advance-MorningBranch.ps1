#Requires -Version 7.0
param(
    [Parameter(Mandatory)]
    [string]$Commit
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Import-Module (Join-Path $PSScriptRoot 'lib\HarvesterLaunch.psm1') -Force
Invoke-HarvesterMorningBranchAdvance -RepositoryRoot $root -Commit $Commit
