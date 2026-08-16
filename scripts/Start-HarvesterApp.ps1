#Requires -Version 7.0
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Import-Module (Join-Path $PSScriptRoot 'lib\HarvesterLaunch.psm1') -Force
$spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $root
$code = 1
Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code)
exit $code
