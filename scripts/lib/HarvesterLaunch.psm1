Set-StrictMode -Version Latest

$script:HarvesterLaunchPolicies = [ordered]@{
    Batch = [pscustomobject]@{
        Package              = 'harvester_batch'
        BinaryName           = 'harvester_batch.exe'
        RuntimeArguments     = [string[]]@('--use-stored-keys')
        FrontendDirectory    = $null
        FrontendBuildCommand = $null
    }
    Ui = [pscustomobject]@{
        Package              = 'harvester_ui'
        BinaryName           = 'harvester_ui.exe'
        RuntimeArguments     = [string[]]@('--use-stored-keys')
        FrontendDirectory    = 'frontend'
        FrontendBuildCommand = [string[]]@('npm', 'run', 'build')
    }
}

function Get-HarvesterLaunchSpec {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [ValidateSet('Batch', 'Ui')]
        [string]$Name,

        [Parameter(Mandatory)]
        [string]$RepositoryRoot
    )

    $root = [System.IO.Path]::GetFullPath($RepositoryRoot)
    $policy = $script:HarvesterLaunchPolicies[$Name]
    [pscustomobject]@{
        Name             = $Name
        RepositoryRoot   = $root
        Package          = $policy.Package
        BinaryName       = $policy.BinaryName
        ExecutablePath   = Join-Path $root (Join-Path 'target\debug' $policy.BinaryName)
        RuntimeArguments = [string[]]$policy.RuntimeArguments.Clone()
        FrontendDirectory = $policy.FrontendDirectory
        FrontendBuildCommand = if ($null -eq $policy.FrontendBuildCommand) { $null } else { [string[]]$policy.FrontendBuildCommand.Clone() }
    }
}

function Invoke-DefaultHarvesterBuild {
    param(
        [Parameter(Mandatory)]
        [psobject]$Spec,

        [Parameter()]
        [scriptblock]$ProcessRunner
    )

    $runner = if ($null -eq $ProcessRunner) {
        {
            param([string]$Command, [string[]]$Arguments, [string]$WorkingDirectory)
            Push-Location -LiteralPath $WorkingDirectory
            try {
                & $Command @Arguments | Out-Host
                $LASTEXITCODE
            }
            finally {
                Pop-Location
            }
        }
    } else { $ProcessRunner }

    if ($null -ne $Spec.FrontendDirectory) {
        $frontendDirectory = Join-Path $Spec.RepositoryRoot $Spec.FrontendDirectory
        $frontendCommand = [string[]]$Spec.FrontendBuildCommand
        $frontendArguments = [string[]]@($frontendCommand | Select-Object -Skip 1)
        $frontendExitCode = & $runner $frontendCommand[0] $frontendArguments $frontendDirectory
        if ($frontendExitCode -ne 0) { throw "frontend build failed with exit code $frontendExitCode." }
    }
    $cargoExitCode = & $runner 'cargo' @('build', '-p', $Spec.Package) $Spec.RepositoryRoot
    if ($cargoExitCode -ne 0) { throw "cargo build -p $($Spec.Package) failed with exit code $cargoExitCode." }
}

function Invoke-DefaultHarvesterChild {
    param(
        [Parameter(Mandatory)]
        [string]$Executable,

        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [string[]]$ArgumentList,

        [Parameter(Mandatory)]
        [ref]$ExitCode
    )

    $PSNativeCommandUseErrorActionPreference = $false
    & $Executable @ArgumentList
    $ExitCode.Value = $LASTEXITCODE
}

function Set-HarvesterLaunchUtf8ProcessEncoding {
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    [Console]::InputEncoding = $utf8NoBom
    [Console]::OutputEncoding = $utf8NoBom
    $global:OutputEncoding = $utf8NoBom
}

function Initialize-HarvesterLaunch {
    param([scriptblock]$EnvironmentVariableProbe)

    try {
        Set-HarvesterLaunchUtf8ProcessEncoding
    }
    catch {
        Write-Verbose "Unable to set UTF-8 console encoding: $($_.Exception.Message)"
    }

    $environmentProbe = if ($null -eq $EnvironmentVariableProbe) {
        {
            param(
                [string]$Name,
                [System.EnvironmentVariableTarget]$Target
            )
            [System.Environment]::GetEnvironmentVariable($Name, $Target)
        }
    }
    else {
        $EnvironmentVariableProbe
    }

    # Warn once per variable, including persistent scopes that the current process did not inherit.
    foreach ($name in @('OPENAI_API_KEY', 'BRAVE_SEARCH_API_KEY')) {
        foreach ($target in [System.Enum]::GetValues([System.EnvironmentVariableTarget])) {
            if (-not [string]::IsNullOrWhiteSpace((& $environmentProbe $name $target))) {
                Write-Warning "$name is set and Harvester ignores it. Remove it if no other application needs it."
                break
            }
        }
    }
}

function Invoke-HarvesterLaunch {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [psobject]$Spec,

        [Parameter(Mandatory)]
        [ref]$ExitCode,

        [Parameter()]
        [scriptblock]$BuildInvoker,

        [Parameter()]
        [scriptblock]$EnvironmentVariableProbe
    )

    $ExitCode.Value = 1
    Initialize-HarvesterLaunch $EnvironmentVariableProbe

    $build = if ($null -eq $BuildInvoker) {
        { param($launchSpec) Invoke-DefaultHarvesterBuild -Spec $launchSpec }
    }
    else {
        $BuildInvoker
    }
    Push-Location -LiteralPath $Spec.RepositoryRoot
    try {
        & $build $Spec

        if (-not (Test-Path -LiteralPath $Spec.ExecutablePath -PathType Leaf)) {
            throw "Built binary was not found at '$($Spec.ExecutablePath)'."
        }

        Invoke-DefaultHarvesterChild $Spec.ExecutablePath $Spec.RuntimeArguments $ExitCode
    }
    finally {
        Pop-Location
    }
}

function Write-HarvesterMorningFailure {
    param([string]$Status, [string]$Reason, $Commit)

    $result = [ordered]@{
        v                   = 1
        status              = $Status
        articles            = 0
        path                = $null
        previous_kept       = $null
        previous_superseded = $false
        cost_usd            = $null
        cost_basis          = 'openai_price_table'
        archive_schema      = 2
        since_utc           = $null
        until_utc           = $null
        failed_sources      = 0
        backlog_articles    = 0
        excused_articles    = 0
        lock_holder         = $null
        reason              = $Reason
        commit              = $Commit
    }
    'HARVESTER_RESULT ' + (ConvertTo-Json -InputObject $result -Compress -EscapeHandling EscapeNonAscii)
}

function Invoke-DefaultHarvesterMorningProcess {
    param(
        [string]$Command,
        [string[]]$ArgumentList,
        [string]$WorkingDirectory,
        [ref]$ExitCode,
        [hashtable]$Environment
    )

    $PSNativeCommandUseErrorActionPreference = $false
    $savedEnvironment = @{}
    Push-Location -LiteralPath $WorkingDirectory
    try {
        if ($null -ne $Environment) {
            foreach ($name in $Environment.Keys) {
                $savedEnvironment[$name] = [System.Environment]::GetEnvironmentVariable($name)
                [System.Environment]::SetEnvironmentVariable($name, $Environment[$name])
            }
        }

        & $Command @ArgumentList
        $ExitCode.Value = $LASTEXITCODE
    }
    finally {
        foreach ($name in $savedEnvironment.Keys) {
            [System.Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name])
        }
        Pop-Location
    }
}

function Get-HarvesterMorningProcessRunner {
    param([scriptblock]$ProcessRunner)

    if ($null -ne $ProcessRunner) { return $ProcessRunner }
    {
        param($Command, $ArgumentList, $WorkingDirectory, $ExitCode, $Environment)
        Invoke-DefaultHarvesterMorningProcess $Command $ArgumentList $WorkingDirectory $ExitCode $Environment
    }
}

function Invoke-HarvesterMorningGit {
    param(
        [scriptblock]$ProcessRunner,
        [string[]]$ArgumentList,
        [string]$RepositoryRoot,
        [hashtable]$Environment
    )

    $code = -1
    $output = @(& $ProcessRunner 'git' $ArgumentList $RepositoryRoot ([ref]$code) $Environment)
    [pscustomobject]@{ Code = $code; Output = $output }
}

function Get-HarvesterGitCommitId {
    param($Result)

    if ($Result.Code -eq 0 -and $Result.Output.Count -eq 1 -and $Result.Output[0] -cmatch '\A[0-9a-fA-F]{40}\z') {
        $Result.Output[0]
    }
}

function Test-HarvesterCheckoutPath {
    param([string[]]$Output, [string]$ExpectedPath)

    if ($Output.Count -ne 1 -or -not [System.IO.Path]::IsPathFullyQualified($Output[0])) {
        return $false
    }
    $actual = [System.IO.Path]::TrimEndingDirectorySeparator([System.IO.Path]::GetFullPath($Output[0]))
    [string]::Equals($actual, $ExpectedPath, [StringComparison]::OrdinalIgnoreCase)
}

function Get-HarvesterMorningCheckoutFailure {
    param([hashtable]$Git, [string]$MorningCheckout, [ref]$CheckoutCommit)

    $inCheckout = @('-C', $MorningCheckout)
    $topLevel = Invoke-HarvesterMorningGit @Git -ArgumentList ($inCheckout + @('rev-parse', '--show-toplevel'))
    if ($topLevel.Code -ne 0 -or -not (Test-HarvesterCheckoutPath $topLevel.Output $MorningCheckout)) {
        return 'checkout_foreign'
    }
    $head = Invoke-HarvesterMorningGit @Git -ArgumentList ($inCheckout + @('rev-parse', '--verify', 'HEAD'))
    $CheckoutCommit.Value = Get-HarvesterGitCommitId $head
    if (-not $CheckoutCommit.Value) { return 'checkout_foreign' }

    $origin = Invoke-HarvesterMorningGit @Git -ArgumentList ($inCheckout + @('remote', 'get-url', 'origin'))
    if ($origin.Code -ne 0 -or -not (Test-HarvesterCheckoutPath $origin.Output $Git.RepositoryRoot)) {
        return 'checkout_foreign'
    }
    $branch = Invoke-HarvesterMorningGit @Git -ArgumentList ($inCheckout + @('symbolic-ref', '--quiet', 'HEAD'))
    if ($branch.Code -ne 0 -or $branch.Output.Count -ne 1 -or $branch.Output[0] -cne 'refs/heads/morning') {
        return 'checkout_foreign'
    }

    # Keep the preflight read-only and include untracked files even if Git config hides them.
    $statusEnvironment = @{ GIT_OPTIONAL_LOCKS = '0' }
    $statusArgs = $inCheckout + @('status', '--porcelain', '--untracked-files=normal')
    $status = Invoke-HarvesterMorningGit @Git -ArgumentList $statusArgs -Environment $statusEnvironment
    if ($status.Code -ne 0) { return 'checkout_foreign' }
    if ($status.Output.Count -ne 0) { return 'checkout_dirty' }
}

function Get-HarvesterMorningUpdateFailure {
    param([string]$RepositoryRoot, [string]$MorningCheckout, [scriptblock]$ProcessRunner, [ref]$CheckoutCommit)

    $git = @{ RepositoryRoot = $RepositoryRoot; ProcessRunner = $ProcessRunner }
    $inCheckout = @('-C', $MorningCheckout)
    $failureReason = 'checkout_foreign'
    try {
        $exists = Test-Path -LiteralPath $MorningCheckout
        if ($exists) {
            $checkoutFailure = Get-HarvesterMorningCheckoutFailure $git $MorningCheckout $CheckoutCommit
            if ($checkoutFailure) { return $checkoutFailure }
        }

        $failureReason = 'fast_forward_failed'
        $release = Invoke-HarvesterMorningGit @git -ArgumentList @('rev-parse', '--verify', '--quiet', 'refs/heads/morning')
        if ($release.Code -eq 1) { return 'no_morning_branch' }
        $releaseCommit = Get-HarvesterGitCommitId $release
        if (-not $releaseCommit) { return $failureReason }
        $onMain = Invoke-HarvesterMorningGit @git -ArgumentList @('merge-base', '--is-ancestor', $releaseCommit, 'refs/heads/main')
        if ($onMain.Code -eq 1) { return 'morning_not_on_main' }
        if ($onMain.Code -ne 0) { return $failureReason }

        if (-not $exists) {
            $failureReason = 'clone_failed'
            $clone = Invoke-HarvesterMorningGit @git -ArgumentList @(
                'clone', '--branch', 'morning', '--single-branch', $RepositoryRoot, $MorningCheckout
            )
            if ($clone.Code -ne 0) { return $failureReason }
            $failureReason = 'checkout_foreign'
            $checkoutFailure = Get-HarvesterMorningCheckoutFailure $git $MorningCheckout $CheckoutCommit
            if ($checkoutFailure) { return $checkoutFailure }
        }

        $failureReason = 'fast_forward_failed'
        $fetch = Invoke-HarvesterMorningGit @git -ArgumentList ($inCheckout + @('fetch', 'origin', 'morning'))
        if ($fetch.Code -ne 0) { return $failureReason }
        $fetched = Invoke-HarvesterMorningGit @git -ArgumentList ($inCheckout + @('rev-parse', 'FETCH_HEAD'))
        $fetchedCommit = Get-HarvesterGitCommitId $fetched
        # If the release ref moved during this run, the next launch validates the new tip.
        if ($fetchedCommit -cne $releaseCommit) { return $failureReason }

        # A fast-forward-only merge alone also accepts a checkout that is already ahead.
        $failureReason = 'checkout_diverged'
        $ancestor = Invoke-HarvesterMorningGit @git -ArgumentList ($inCheckout + @('merge-base', '--is-ancestor', 'HEAD', 'FETCH_HEAD'))
        if ($ancestor.Code -ne 0) { return $failureReason }

        $failureReason = 'fast_forward_failed'
        $merge = Invoke-HarvesterMorningGit @git -ArgumentList ($inCheckout + @('merge', '--ff-only', 'FETCH_HEAD'))
        if ($merge.Code -ne 0) { return $failureReason }
        $head = Invoke-HarvesterMorningGit @git -ArgumentList ($inCheckout + @('rev-parse', 'HEAD'))
        $CheckoutCommit.Value = Get-HarvesterGitCommitId $head
        if ($CheckoutCommit.Value -cne $fetchedCommit) { return $failureReason }
    }
    catch {
        return $failureReason
    }
}

function Invoke-HarvesterMorningLaunch {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string]$RepositoryRoot,

        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [AllowEmptyString()]
        [string[]]$ArgumentList,

        [Parameter(Mandatory)]
        [ref]$ExitCode,

        [string]$MorningCheckout = 'C:\Users\larsp\src\harvester-morning',

        [scriptblock]$ProcessRunner,

        [scriptblock]$EnvironmentVariableProbe
    )

    Initialize-HarvesterLaunch $EnvironmentVariableProbe

    $archiveOut = $null
    $archivePrefix = '--archive-out='
    if ($ArgumentList.Count -eq 2 -and [string]::Equals($ArgumentList[0], '--archive-out', [StringComparison]::Ordinal)) {
        $archiveOut = $ArgumentList[1]
    } elseif (
        $ArgumentList.Count -eq 2 -and
        $ArgumentList[0] -cmatch '^--archive-out=[A-Za-z]\z' -and
        ([string]$ArgumentList[1]).StartsWith('\', [StringComparison]::Ordinal)
    ) {
        # pwsh -File can split the equals form at the drive colon.
        $archiveOut = $ArgumentList[0].Substring($archivePrefix.Length) + ':' + $ArgumentList[1]
    } elseif ($ArgumentList.Count -eq 1 -and ([string]$ArgumentList[0]).StartsWith($archivePrefix, [StringComparison]::Ordinal)) {
        $archiveOut = $ArgumentList[0].Substring($archivePrefix.Length)
    }
    if ([string]::IsNullOrWhiteSpace($archiveOut)) {
        $ExitCode.Value = 2
        Write-HarvesterMorningFailure 'usage_error' 'invalid_arguments'
        return
    }
    if (-not [System.IO.Path]::IsPathFullyQualified($archiveOut)) {
        $ExitCode.Value = 2
        Write-HarvesterMorningFailure 'usage_error' 'invalid_archive_out'
        return
    }

    $main = [System.IO.Path]::TrimEndingDirectorySeparator([System.IO.Path]::GetFullPath($RepositoryRoot))
    $checkout = [System.IO.Path]::TrimEndingDirectorySeparator([System.IO.Path]::GetFullPath($MorningCheckout))
    $runner = Get-HarvesterMorningProcessRunner $ProcessRunner
    $checkoutCommit = $null
    $updateFailure = Get-HarvesterMorningUpdateFailure $main $checkout $runner ([ref]$checkoutCommit)
    if ($null -ne $updateFailure) {
        $ExitCode.Value = 23
        Write-HarvesterMorningFailure 'update_failed' $updateFailure $checkoutCommit
        return
    }

    $spec = Get-HarvesterLaunchSpec -Name Batch -RepositoryRoot $checkout
    $buildEnvironment = @{
        CARGO_INCREMENTAL = '0'
        CARGO_TARGET_DIR = Join-Path $checkout 'target'
    }
    $buildArguments = @('build', '--locked', '-p', $spec.Package, '--bin', $spec.Package)
    $buildCode = -1
    try {
        & $runner 'cargo' $buildArguments $checkout ([ref]$buildCode) $buildEnvironment | Out-Host
    }
    catch {
        $buildCode = 1
    }
    if ($buildCode -ne 0) {
        $ExitCode.Value = 24
        Write-HarvesterMorningFailure 'build_failed' 'cargo_failed' $checkoutCommit
        return
    }
    if (-not (Test-Path -LiteralPath $spec.ExecutablePath -PathType Leaf)) {
        $ExitCode.Value = 24
        Write-HarvesterMorningFailure 'build_failed' 'binary_missing' $checkoutCommit
        return
    }

    $childArguments = $spec.RuntimeArguments + @(
        '--output-dir', (Join-Path $main 'output')
        '--sources', (Join-Path $main 'output\.sources.ron')
        '--contexts-dir', (Join-Path $main 'contexts')
        '--prompts-dir', (Join-Path $main 'prompts')
        '--archive-out', $archiveOut
    )
    $childEnvironment = @{ HARVESTER_MORNING_COMMIT = $checkoutCommit }
    try {
        # Stream the batch's result directly and leave it as the final line.
        & $runner $spec.ExecutablePath $childArguments $main $ExitCode $childEnvironment
    }
    catch {
        $ExitCode.Value = 24
        Write-HarvesterMorningFailure 'build_failed' 'child_start_failed' $checkoutCommit
    }
}

function Invoke-HarvesterMorningBranchAdvance {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string]$RepositoryRoot,

        [Parameter(Mandatory)]
        [ValidatePattern('\A[0-9a-fA-F]{4,40}\z')]
        [string]$Commit,

        [scriptblock]$ProcessRunner
    )

    $git = @{
        RepositoryRoot = $RepositoryRoot
        ProcessRunner = Get-HarvesterMorningProcessRunner $ProcessRunner
    }
    $resolved = Invoke-HarvesterMorningGit @git -ArgumentList @(
        'rev-parse', '--verify', '--quiet', '--end-of-options', "$Commit^{commit}"
    )
    $next = Get-HarvesterGitCommitId $resolved
    if (-not $next -or -not $next.StartsWith($Commit, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Cannot resolve '$Commit' to a commit hash."
    }
    $onMain = Invoke-HarvesterMorningGit @git -ArgumentList @('merge-base', '--is-ancestor', $next, 'refs/heads/main')
    if ($onMain.Code -ne 0) { throw "Cannot verify that commit $next is on local main." }

    $ref = 'refs/heads/morning'
    $current = Invoke-HarvesterMorningGit @git -ArgumentList @('rev-parse', '--verify', '--quiet', $ref)
    if ($current.Code -eq 1) {
        $old = '0' * 40
        $oldLabel = '(absent)'
    } else {
        $old = Get-HarvesterGitCommitId $current
        if (-not $old) { throw 'Cannot resolve the current morning branch.' }
        $ancestor = Invoke-HarvesterMorningGit @git -ArgumentList @('merge-base', '--is-ancestor', $old, $next)
        if ($ancestor.Code -ne 0) { throw "Cannot fast-forward morning from $old to $next." }
        $oldLabel = $old
    }

    # Compare-and-swap protects the release ref if it changed during the checks.
    $update = Invoke-HarvesterMorningGit @git -ArgumentList @('update-ref', '--no-deref', $ref, $next, $old)
    if ($update.Code -ne 0) { throw "Cannot update morning from $oldLabel to $next." }
    "morning $oldLabel -> $next"
}

Export-ModuleMember -Function Get-HarvesterLaunchSpec, Invoke-HarvesterLaunch, Invoke-HarvesterMorningLaunch, Invoke-HarvesterMorningBranchAdvance
