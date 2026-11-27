#Requires -Version 7.0
Set-StrictMode -Version Latest

BeforeAll {
    Import-Module (Join-Path $PSScriptRoot '..\lib\HarvesterLaunch.psm1') -Force

    function New-MorningTestCase {
        param([string]$TestRoot, [switch]$MissingCheckout, [switch]$MissingBinary)

        $root = Join-Path $TestRoot ([guid]::NewGuid().ToString())
        $repository = Join-Path $root 'main repository'
        $checkout = Join-Path $root 'morning checkout'
        $binary = Join-Path $checkout 'target\debug\harvester_batch.exe'
        $target = Join-Path $root 'portfolio archive.md'
        New-Item -ItemType Directory -Path $repository -Force | Out-Null
        if (-not $MissingCheckout) {
            New-Item -ItemType Directory -Path (Split-Path -Parent $binary) -Force | Out-Null
            if (-not $MissingBinary) {
                New-Item -ItemType File -Path $binary | Out-Null
            }
        }

        $commit = 'a' * 40
        $initialCommit = 'b' * 40
        $childResult = 'HARVESTER_RESULT {"status":"nothing_new","commit":"' + $commit + '"}'
        $responses = @{
            'clone' = @{ Code = 0; Output = @() }
            'rev-parse --show-toplevel' = @{ Code = 0; Output = $checkout.Replace('\', '/') }
            'rev-parse --verify HEAD' = @{ Code = 0; Output = $initialCommit }
            'remote get-url origin' = @{ Code = 0; Output = $repository }
            'symbolic-ref --quiet HEAD' = @{ Code = 0; Output = 'refs/heads/morning' }
            'status --porcelain --untracked-files=normal' = @{ Code = 0; Output = @() }
            'rev-parse --verify --quiet refs/heads/morning' = @{ Code = 0; Output = $commit }
            "merge-base --is-ancestor $commit refs/heads/main" = @{ Code = 0; Output = @() }
            'fetch origin morning' = @{ Code = 0; Output = @() }
            'rev-parse FETCH_HEAD' = @{ Code = 0; Output = $commit }
            'merge-base --is-ancestor HEAD FETCH_HEAD' = @{ Code = 0; Output = @() }
            'merge --ff-only FETCH_HEAD' = @{ Code = 0; Output = @() }
            'rev-parse HEAD' = @{ Code = 0; Output = $commit }
            'cargo' = @{ Code = 0; Output = @() }
            'child' = @{ Code = 10; Output = @('child output', $childResult) }
        }
        $calls = [System.Collections.Generic.List[object]]::new()
        $runner = {
            param($Command, $ArgumentList, $WorkingDirectory, $ExitCode, $Environment)

            $step = switch -CaseSensitive ($Command) {
                'git' {
                    if ($ArgumentList[0] -ceq '-C') {
                        $ArgumentList[2..($ArgumentList.Count - 1)] -join ' '
                    } elseif ($ArgumentList[0] -ceq 'clone') {
                        'clone'
                    } else {
                        $ArgumentList -join ' '
                    }
                }
                'cargo' { 'cargo' }
                default { if ($Command -ceq $binary) { 'child' } else { $Command } }
            }
            $calls.Add([pscustomobject]@{
                Command = $Command
                Arguments = $ArgumentList
                Directory = $WorkingDirectory
                Environment = $Environment
                Step = $step
            })
            if (-not $responses.ContainsKey($step)) { throw "Unexpected process: $Command $ArgumentList" }
            $response = $responses[$step]
            if ($response.ContainsKey('StartError')) { throw $response.StartError }
            if ($step -ceq 'clone' -and $response.Code -eq 0) {
                New-Item -ItemType Directory -Path (Split-Path -Parent $binary) -Force | Out-Null
                New-Item -ItemType File -Path $binary | Out-Null
                $responses['rev-parse --verify HEAD'].Output = $commit
            }
            $response.Output
            $ExitCode.Value = $response.Code
        }.GetNewClosure()

        [pscustomobject]@{
            Repository = $repository
            Checkout = $checkout
            Binary = $binary
            Target = $target
            Commit = $commit
            InitialCommit = $initialCommit
            Responses = $responses
            Calls = $calls
            ChildResult = $childResult
            Parameters = @{
                RepositoryRoot = $repository
                MorningCheckout = $checkout
                ProcessRunner = $runner
                EnvironmentVariableProbe = {}
            }
        }
    }

    function Assert-MorningFailure {
        param($Output, [string]$Status, [string]$Reason, $Commit)

        $Output.Count | Should -Be 1
        $Output[0] | Should -Match '^HARVESTER_RESULT \{[^\r\n]*\}$'
        $Output[0] | Should -Not -Match '[^\x20-\x7e]'
        $result = $Output[0].Substring('HARVESTER_RESULT '.Length) | ConvertFrom-Json
        $expected = [ordered]@{
            v = 2
            status = $Status
            articles = 0
            path = $null
            previous_kept = $null
            previous_superseded = $false
            cost_usd = $null
            cost_basis = 'openai_price_table'
            archive_schema = 2
            since_utc = $null
            until_utc = $null
            failed_sources = 0
            left_out_articles = 0
            left_out = @()
            lock_holder = $null
            reason = $Reason
            commit = $Commit
        }
        ($result.PSObject.Properties.Name -join ',') | Should -BeExactly ($expected.Keys -join ',')
        ($result.left_out -is [array]) | Should -BeTrue
        $result.left_out.Count | Should -Be 0
        $result.left_out_articles | Should -Be $result.left_out.Count
        $Output[0] | Should -BeExactly ('HARVESTER_RESULT ' + ($expected | ConvertTo-Json -Compress))
    }

    function Assert-MorningGitSafety {
        param($Calls)
        foreach ($call in $Calls | Where-Object Command -CEQ 'git') {
            $verb = if ($call.Arguments[0] -ceq '-C') { $call.Arguments[2] } else { $call.Arguments[0] }
            @('reset', 'clean', 'checkout', 'stash', 'rebase', 'worktree') | Should -Not -Contain $verb
            if ($verb -ceq 'fetch') { ($call.Arguments[-3..-1] -join ' ') | Should -BeExactly 'fetch origin morning' }
        }
    }
}

AfterAll {
    Remove-Module HarvesterLaunch -ErrorAction SilentlyContinue
}

Describe 'Harvester morning launch policy' {
    BeforeEach {
        Mock -ModuleName HarvesterLaunch Set-HarvesterLaunchUtf8ProcessEncoding {}
        $fixture = New-MorningTestCase $TestDrive
        $code = -1
    }

    AfterEach {
        Assert-MorningGitSafety $fixture.Calls
    }

    It 'refuses <name> before running git or cargo' -ForEach @(
        @{ name = 'an unknown flag'; inputArguments = @('--force-unlock'); reason = 'invalid_arguments' }
        @{ name = 'a misspelled flag'; inputArguments = @('--ARCHIVE-OUT', '<target>'); reason = 'invalid_arguments' }
        @{ name = 'an extra argument'; inputArguments = @('--archive-out', '<target>', '--force-unlock'); reason = 'invalid_arguments' }
        @{ name = 'an extra argument after the equals form'; inputArguments = @('--archive-out=<target>', 'extra'); reason = 'invalid_arguments' }
        @{ name = 'a missing value'; inputArguments = @('--archive-out'); reason = 'invalid_arguments' }
        @{ name = 'an empty equals value'; inputArguments = @('--archive-out='); reason = 'invalid_arguments' }
        @{ name = 'a split drive without a leading backslash in the rest'; inputArguments = @('--archive-out=C', 'Users\x\archive.md'); reason = 'invalid_arguments' }
        @{ name = 'a relative path'; inputArguments = @('--archive-out', 'archive.md'); reason = 'invalid_archive_out' }
        @{ name = 'a drive-relative path'; inputArguments = @('--archive-out=C:archive.md'); reason = 'invalid_archive_out' }
        @{ name = 'a root-relative path'; inputArguments = @('--archive-out', '\archive.md'); reason = 'invalid_archive_out' }
    ) {
        $params = $fixture.Parameters
        $argumentList = @($inputArguments | ForEach-Object { $_.Replace('<target>', $fixture.Target) })
        $output = @(Invoke-HarvesterMorningLaunch @params -ArgumentList $argumentList -ExitCode ([ref]$code))

        $fixture.Calls.Count | Should -Be 0
        $code | Should -Be 2
        Assert-MorningFailure $output 'usage_error' $reason
    }

    It 'clones a missing checkout once, then verifies the fetched tip before building' {
        $fixture = New-MorningTestCase $TestDrive -MissingCheckout
        $params = $fixture.Parameters
        $output = @(Invoke-HarvesterMorningLaunch @params -ArgumentList @('--archive-out', $fixture.Target) -ExitCode ([ref]$code))

        @($fixture.Calls | Where-Object Step -CEQ 'clone').Count | Should -Be 1
        $clone = $fixture.Calls[2]
        $clone.Command | Should -BeExactly 'git'
        ($clone.Arguments -join '|') | Should -BeExactly "clone|--branch|morning|--single-branch|$($fixture.Repository)|$($fixture.Checkout)"
        $clone.Directory | Should -BeExactly $fixture.Repository
        ($fixture.Calls.Step -join '|') | Should -Match '\|clone\|.*\|rev-parse HEAD\|cargo\|child$'
        $code | Should -Be 10
        $output[-1] | Should -BeExactly $fixture.ChildResult
    }

    It 'stops immediately if the clone fails' {
        $fixture = New-MorningTestCase $TestDrive -MissingCheckout
        $fixture.Responses['clone'].Code = 1
        $params = $fixture.Parameters
        $output = @(Invoke-HarvesterMorningLaunch @params -ArgumentList @('--archive-out', $fixture.Target) -ExitCode ([ref]$code))

        ($fixture.Calls.Step -join '|') |
            Should -BeExactly "rev-parse --verify --quiet refs/heads/morning|merge-base --is-ancestor $($fixture.Commit) refs/heads/main|clone"
        $code | Should -Be 23
        Assert-MorningFailure $output 'update_failed' 'clone_failed'
    }

    It 'refuses to clone when the release branch is missing' {
        $fixture = New-MorningTestCase $TestDrive -MissingCheckout
        $fixture.Responses['rev-parse --verify --quiet refs/heads/morning'] = @{ Code = 1; Output = @() }
        $params = $fixture.Parameters
        $output = @(Invoke-HarvesterMorningLaunch @params -ArgumentList @('--archive-out', $fixture.Target) -ExitCode ([ref]$code))

        ($fixture.Calls.Step -join '|') | Should -BeExactly 'rev-parse --verify --quiet refs/heads/morning'
        $code | Should -Be 23
        Assert-MorningFailure $output 'update_failed' 'no_morning_branch'
    }

    It 'validates and fast-forwards a clean checkout in order' {
        $params = $fixture.Parameters
        Invoke-HarvesterMorningLaunch @params -ArgumentList @('--archive-out', $fixture.Target) -ExitCode ([ref]$code) | Out-Null

        $expected = @(
            'rev-parse --show-toplevel', 'rev-parse --verify HEAD', 'remote get-url origin', 'symbolic-ref --quiet HEAD',
            'status --porcelain --untracked-files=normal', 'rev-parse --verify --quiet refs/heads/morning',
            "merge-base --is-ancestor $($fixture.Commit) refs/heads/main", 'fetch origin morning', 'rev-parse FETCH_HEAD',
            'merge-base --is-ancestor HEAD FETCH_HEAD', 'merge --ff-only FETCH_HEAD', 'rev-parse HEAD',
            'cargo', 'child'
        )
        ($fixture.Calls.Step -join '|') | Should -BeExactly ($expected -join '|')
        foreach ($call in $fixture.Calls | Where-Object Command -CEQ 'git') {
            $call.Directory | Should -BeExactly $fixture.Repository
            if ($call.Arguments[0] -ceq '-C') { $call.Arguments[1] | Should -BeExactly $fixture.Checkout }
        }
        $fixture.Calls[4].Environment.GIT_OPTIONAL_LOCKS | Should -BeExactly '0'
        $code | Should -Be 10
    }

    It 'builds the same release when main advances and the morning merge is already up to date' {
        # A newer descendant main still yields a successful ancestry check for the same release.
        $fixture.Responses['rev-parse --verify HEAD'].Output = $fixture.Commit
        $fixture.Responses['merge --ff-only FETCH_HEAD'].Output = 'Already up to date.'
        $params = $fixture.Parameters
        $output = @(Invoke-HarvesterMorningLaunch @params -ArgumentList @('--archive-out', $fixture.Target) -ExitCode ([ref]$code))

        $code | Should -Be 10
        $fixture.Calls.Step | Should -Contain "merge-base --is-ancestor $($fixture.Commit) refs/heads/main"
        $fixture.Calls.Step | Should -Contain 'fetch origin morning'
        $fixture.Calls.Step | Should -Contain 'merge --ff-only FETCH_HEAD'
        $fixture.Calls[-2].Command | Should -BeExactly 'cargo'
        $fixture.Calls[-1].Environment.HARVESTER_MORNING_COMMIT | Should -BeExactly $fixture.Commit
        $output[-1] | Should -BeExactly $fixture.ChildResult
    }

    It 'stops after <name> without building or running' -ForEach @(
        @{ name = 'a non-repository'; step = 'rev-parse --show-toplevel'; response = @{ Code = 128; Output = @() }; count = 1; reason = 'checkout_foreign' }
        @{ name = 'a mismatched top level'; step = 'rev-parse --show-toplevel'; response = @{ Code = 0; Output = 'C:\other' }; count = 1; reason = 'checkout_foreign' }
        @{ name = 'the wrong origin'; step = 'remote get-url origin'; response = @{ Code = 0; Output = 'C:\other' }; count = 3; reason = 'checkout_foreign' }
        @{ name = 'the old main branch'; step = 'symbolic-ref --quiet HEAD'; response = @{ Code = 0; Output = 'refs/heads/main' }; count = 4; reason = 'checkout_foreign' }
        @{ name = 'a detached HEAD'; step = 'symbolic-ref --quiet HEAD'; response = @{ Code = 1; Output = @() }; count = 4; reason = 'checkout_foreign' }
        @{ name = 'a dirty checkout'; step = 'status --porcelain --untracked-files=normal'; response = @{ Code = 0; Output = '?? local-note.txt' }; count = 5; reason = 'checkout_dirty' }
        @{ name = 'a missing release branch'; step = 'rev-parse --verify --quiet refs/heads/morning'; response = @{ Code = 1; Output = @() }; count = 6; reason = 'no_morning_branch' }
        @{ name = 'a release branch outside main'; step = 'merge-base --is-ancestor <commit> refs/heads/main'; response = @{ Code = 1; Output = @() }; count = 7; reason = 'morning_not_on_main' }
        @{ name = 'fetch failure'; step = 'fetch origin morning'; response = @{ Code = 1; Output = @() }; count = 8; reason = 'fast_forward_failed' }
        @{ name = 'an unreadable FETCH_HEAD'; step = 'rev-parse FETCH_HEAD'; response = @{ Code = 128; Output = @() }; count = 9; reason = 'fast_forward_failed' }
        @{ name = 'the release moving during the fetch'; step = 'rev-parse FETCH_HEAD'; response = @{ Code = 0; Output = ('d' * 40) }; count = 9; reason = 'fast_forward_failed' }
        # Both histories are reported by Git as HEAD not being an ancestor of FETCH_HEAD.
        @{ name = 'a clean but ahead checkout'; step = 'merge-base --is-ancestor HEAD FETCH_HEAD'; response = @{ Code = 1; Output = @() }; count = 10; reason = 'checkout_diverged' }
        @{ name = 'a diverged checkout'; step = 'merge-base --is-ancestor HEAD FETCH_HEAD'; response = @{ Code = 1; Output = @() }; count = 10; reason = 'checkout_diverged' }
        @{ name = 'merge failure'; step = 'merge --ff-only FETCH_HEAD'; response = @{ Code = 1; Output = @() }; count = 11; reason = 'fast_forward_failed' }
        @{ name = 'HEAD differs from the fetched commit after merge'; step = 'rev-parse HEAD'; response = @{ Code = 0; Output = ('d' * 40) }; count = 12; reason = 'fast_forward_failed' }
    ) {
        $expectedStep = $step.Replace('<commit>', $fixture.Commit)
        $fixture.Responses[$expectedStep] = $response
        $params = $fixture.Parameters
        $output = @(Invoke-HarvesterMorningLaunch @params -ArgumentList @('--archive-out', $fixture.Target) -ExitCode ([ref]$code))

        $fixture.Calls.Count | Should -Be $count
        $fixture.Calls[-1].Step | Should -BeExactly $expectedStep
        $fixture.Calls.Step | Should -Not -Contain 'cargo'
        $fixture.Calls.Step | Should -Not -Contain 'child'
        if ($count -le 10) {
            $fixture.Calls.Step | Should -Not -Contain 'merge --ff-only FETCH_HEAD'
        }
        $code | Should -Be 23
        $expectedCommit = if ($step -ceq 'rev-parse HEAD') { $response.Output } elseif ($count -ge 3) { $fixture.InitialCommit }
        Assert-MorningFailure $output 'update_failed' $reason $expectedCommit
    }

    It 'stops after a failed cargo build' {
        $fixture.Responses['cargo'].Code = 101
        $params = $fixture.Parameters
        $output = @(Invoke-HarvesterMorningLaunch @params -ArgumentList @('--archive-out', $fixture.Target) -ExitCode ([ref]$code))

        $fixture.Calls[-1].Command | Should -BeExactly 'cargo'
        $fixture.Calls.Step | Should -Not -Contain 'child'
        $code | Should -Be 24
        Assert-MorningFailure $output 'build_failed' 'cargo_failed' $fixture.Commit
    }

    It 'stops if cargo succeeds without the binary' {
        $fixture = New-MorningTestCase $TestDrive -MissingBinary
        $params = $fixture.Parameters
        $output = @(Invoke-HarvesterMorningLaunch @params -ArgumentList @('--archive-out', $fixture.Target) -ExitCode ([ref]$code))

        $fixture.Calls[-1].Command | Should -BeExactly 'cargo'
        $fixture.Calls.Step | Should -Not -Contain 'child'
        $code | Should -Be 24
        Assert-MorningFailure $output 'build_failed' 'binary_missing' $fixture.Commit
    }

    It 'reports a child startup exception as a build failure' {
        $fixture.Responses['child'].StartError = 'Access is denied.'
        $params = $fixture.Parameters
        $output = @(Invoke-HarvesterMorningLaunch @params -ArgumentList @('--archive-out', $fixture.Target) -ExitCode ([ref]$code) *>&1)

        $fixture.Calls[-2].Command | Should -BeExactly 'cargo'
        $fixture.Calls[-1].Command | Should -BeExactly $fixture.Binary
        $code | Should -Be 24
        Assert-MorningFailure $output 'build_failed' 'child_start_failed' $fixture.Commit
    }

    It 'builds only the morning batch and passes through the child output and exit code for <form>' -ForEach @(
        @{ form = 'the space-separated form'; inputArguments = @('--archive-out', '<target>'); expectedTarget = '<target>'; childCode = 10 }
        @{ form = 'the equals form'; inputArguments = @('--archive-out=<target>'); expectedTarget = '<target>'; childCode = 21 }
        @{
            form = 'the drive-colon split from pwsh -File'
            inputArguments = @('--archive-out=c', '\Users\x\portfolio archive.md')
            expectedTarget = 'c:\Users\x\portfolio archive.md'
            childCode = 10
        }
    ) {
        $fixture.Responses['child'].Code = $childCode
        $params = $fixture.Parameters
        $argumentList = @($inputArguments | ForEach-Object { $_.Replace('<target>', $fixture.Target) })
        $output = @(Invoke-HarvesterMorningLaunch @params -ArgumentList $argumentList -ExitCode ([ref]$code) *>&1)

        $build = $fixture.Calls[-2]
        $build.Command | Should -BeExactly 'cargo'
        ($build.Arguments -join '|') | Should -BeExactly 'build|--locked|-p|harvester_batch|--bin|harvester_batch'
        $build.Directory | Should -BeExactly $fixture.Checkout
        $build.Environment.CARGO_INCREMENTAL | Should -BeExactly '0'
        $build.Environment.CARGO_TARGET_DIR | Should -BeExactly (Join-Path $fixture.Checkout 'target')

        $child = $fixture.Calls[-1]
        $child.Command | Should -BeExactly $fixture.Binary
        $child.Directory | Should -BeExactly $fixture.Repository
        $child.Environment.Count | Should -Be 1
        $child.Environment.HARVESTER_MORNING_COMMIT | Should -BeExactly $fixture.Commit
        $expected = @(
            '--use-stored-keys'
            '--output-dir', (Join-Path $fixture.Repository 'output')
            '--sources', (Join-Path $fixture.Repository 'output\.sources.ron')
            '--contexts-dir', (Join-Path $fixture.Repository 'contexts')
            '--prompts-dir', (Join-Path $fixture.Repository 'prompts')
            '--archive-out', $expectedTarget.Replace('<target>', $fixture.Target)
        )
        ($child.Arguments -join '|') | Should -BeExactly ($expected -join '|')
        $code | Should -Be $childCode
        $output.Count | Should -Be 2
        $output[0] | Should -BeExactly 'child output'
        $output[1] | Should -BeExactly $fixture.ChildResult
    }
}

Describe 'Harvester release branch advance' {
    BeforeEach {
        $fixture = New-MorningTestCase $TestDrive
        $next = $fixture.Commit
        $old = $fixture.InitialCommit
        $commitInput = $next.Substring(0, 8)
        $resolveStep = "rev-parse --verify --quiet --end-of-options $($commitInput)^{commit}"
        $onMainStep = "merge-base --is-ancestor $next refs/heads/main"
        $currentStep = 'rev-parse --verify --quiet refs/heads/morning'
        $ancestorStep = "merge-base --is-ancestor $old $next"
        $updateStep = "update-ref --no-deref refs/heads/morning $next $old"
        $fixture.Responses[$resolveStep] = @{ Code = 0; Output = $next }
        $fixture.Responses[$currentStep].Output = $old
        $fixture.Responses[$ancestorStep] = @{ Code = 0; Output = @() }
        $fixture.Responses[$updateStep] = @{ Code = 0; Output = @() }
        $advanceParameters = @{
            RepositoryRoot = $fixture.Repository
            ProcessRunner = $fixture.Parameters.ProcessRunner
            Commit = $commitInput
        }
    }

    AfterEach {
        Assert-MorningGitSafety $fixture.Calls
    }

    It 'refuses a candidate outside local main' {
        $fixture.Responses[$onMainStep].Code = 1
        { Invoke-HarvesterMorningBranchAdvance @advanceParameters } | Should -Throw '*on local main*'
        ($fixture.Calls.Step -join '|') | Should -BeExactly "$resolveStep|$onMainStep"
    }

    It 'refuses a candidate that cannot fast-forward the release branch' {
        $fixture.Responses[$ancestorStep].Code = 1
        { Invoke-HarvesterMorningBranchAdvance @advanceParameters } | Should -Throw '*fast-forward morning*'
        ($fixture.Calls.Step -join '|') | Should -BeExactly "$resolveStep|$onMainStep|$currentStep|$ancestorStep"
    }

    It 'creates the release branch with an expected absent value' {
        $fixture.Responses[$currentStep] = @{ Code = 1; Output = @() }
        $createStep = "update-ref --no-deref refs/heads/morning $next $('0' * 40)"
        $fixture.Responses[$createStep] = @{ Code = 0; Output = @() }

        Invoke-HarvesterMorningBranchAdvance @advanceParameters | Should -BeExactly "morning (absent) -> $next"
        ($fixture.Calls.Step -join '|') | Should -BeExactly "$resolveStep|$onMainStep|$currentStep|$createStep"
    }

    It 'advances the release branch using the previously observed ref value' {
        Invoke-HarvesterMorningBranchAdvance @advanceParameters | Should -BeExactly "morning $old -> $next"
        ($fixture.Calls.Step -join '|') | Should -BeExactly "$resolveStep|$onMainStep|$currentStep|$ancestorStep|$updateStep"
        foreach ($call in $fixture.Calls) { $call.Directory | Should -BeExactly $fixture.Repository }
    }

    It 'refuses a concurrent ref change without retrying' {
        $fixture.Responses[$updateStep].Code = 128
        { Invoke-HarvesterMorningBranchAdvance @advanceParameters } | Should -Throw '*Cannot update morning*'
        $fixture.Calls.Count | Should -Be 5
        $fixture.Calls[-1].Step | Should -BeExactly $updateStep
    }
}

Describe 'Harvester batch script dispatch' {
    It 'routes supplied arguments to the morning policy and keeps the Phase 3 launch for no arguments' {
        $path = Join-Path $PSScriptRoot '..\Start-HarvesterBatch.ps1'
        $tokens = $null
        $errors = $null
        $ast = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)

        $errors | Should -BeNullOrEmpty
        $statements = $ast.EndBlock.Statements
        $route = $statements[-2]
        $route | Should -BeOfType ([System.Management.Automation.Language.IfStatementAst])
        $route.Clauses[0].Item1.Extent.Text | Should -BeExactly '$args.Count -gt 0'
        $route.Clauses[0].Item2.Statements[0].Extent.Text |
            Should -BeExactly 'Invoke-HarvesterMorningLaunch -RepositoryRoot $root -ArgumentList $args -ExitCode ([ref]$code)'
        ($route.ElseClause.Statements.Extent.Text -join '|') |
            Should -BeExactly '$spec = Get-HarvesterLaunchSpec -Name Batch -RepositoryRoot $root|Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code)'
        $statements[-1].Extent.Text | Should -BeExactly 'exit $code'
    }
}
