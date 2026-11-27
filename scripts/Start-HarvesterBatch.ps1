#Requires -Version 7.0
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Import-Module (Join-Path $PSScriptRoot 'lib\HarvesterLaunch.psm1') -Force
$code = 1
if ($args.Count -gt 0) {
    Invoke-HarvesterMorningLaunch -RepositoryRoot $root -ArgumentList $args -ExitCode ([ref]$code)
} else {
    $spec = Get-HarvesterLaunchSpec -Name Batch -RepositoryRoot $root
    Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code)
}
exit $code
