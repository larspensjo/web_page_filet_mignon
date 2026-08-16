#Requires -Version 7.0
Set-StrictMode -Version Latest

BeforeAll {
    $script:ModulePath = Join-Path $PSScriptRoot '..\lib\HarvesterLaunch.psm1'
    Get-Module -Name HarvesterLaunch -All | Remove-Module -Force -ErrorAction SilentlyContinue
    Import-Module $script:ModulePath -Force -DisableNameChecking

    $script:TestRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('harvester launch tests {0}' -f [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $script:TestRoot | Out-Null
    $script:CommandsAvailable = $true

    Mock -ModuleName HarvesterLaunch Get-Command {
        param([string]$Name)
        if ($script:CommandsAvailable -and $Name -in @('Invoke-WithSecretMap', 'Test-SecretStorePromptAvailable')) {
            return [pscustomobject]@{ Name = $Name }
        }
        return $null
    }
}

AfterAll {
    Remove-Item -LiteralPath $script:TestRoot -Recurse -Force -ErrorAction SilentlyContinue
    Get-Module -Name HarvesterLaunch -All | Remove-Module -Force -ErrorAction SilentlyContinue
}

function script:New-BuildFake {
    param(
        [Parameter(Mandatory)][psobject]$Spec,
        [Parameter(Mandatory)][hashtable]$Calls,
        [switch]$Fail
    )

    $null = @($Spec, $Calls, $Fail)
    {
        param(
            [Parameter(Mandatory)]
            [string]$Package
        )
        $Calls.BuildPackages += $Package
        if ($Fail) {
            throw "fake build failed for $Package"
        }

        $binaryParent = Split-Path -Parent $Spec.ExecutablePath
        New-Item -ItemType Directory -Path $binaryParent -Force | Out-Null
        New-Item -ItemType File -Path $Spec.ExecutablePath -Force | Out-Null
    }.GetNewClosure()
}

function script:New-SecretFake {
    param(
        [Parameter(Mandatory)][hashtable]$Calls,
        [int]$ReportedExitCode = 0
    )

    $null = @($Calls, $ReportedExitCode)
    {
        param(
            [Parameter(Mandatory)]
            [System.Collections.IDictionary]$SecretEnvironmentMap,

            [Parameter(Mandatory)]
            [string]$Executable,

            [Parameter()]
            [AllowEmptyCollection()]
            [string[]]$ArgumentList = @(),

            [Parameter(Mandatory)]
            [ref]$ExitCode
        )

        $Calls.SecretInvocations++
        $Calls.Executable = $Executable
        $Calls.Arguments = [string[]]$ArgumentList
        $Calls.SecretMap = [ordered]@{}
        foreach ($entry in $SecretEnvironmentMap.GetEnumerator()) {
            $Calls.SecretMap[[string]$entry.Key] = [string]$entry.Value
        }
        Write-Host 'fake child stdout one'
        Write-Host 'fake child stdout two'
        $ExitCode.Value = $ReportedExitCode
    }.GetNewClosure()
}

function script:New-EnvironmentVariableProbe {
    param(
        [Parameter(Mandatory)]
        [hashtable]$Values
    )

    $null = $Values
    {
        param(
            [Parameter(Mandatory)]
            [string]$Name,

            [Parameter(Mandatory)]
            [System.EnvironmentVariableTarget]$Target
        )

        $Values[('{0}|{1}' -f $Name, $Target)]
    }.GetNewClosure()
}

function script:New-Calls {
    @{
        BuildPackages     = [System.Collections.Generic.List[string]]::new()
        SecretInvocations = 0
        Executable        = $null
        Arguments         = @()
        SecretMap         = $null
    }
}

Describe 'Harvester launch policy' {
    BeforeEach {
        $script:CommandsAvailable = $true
    }

    It 'returns the app policy and launches its debug binary' {
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot
        $calls = New-Calls
        $code = 1

        Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
            -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls) `
            -SecretInvoker (New-SecretFake -Calls $calls) `
            -PromptCheck { $true } `
            -EnvironmentVariableProbe (New-EnvironmentVariableProbe -Values @{})

        @($calls.BuildPackages) | Should -Be @('harvester_app')
        $calls.Executable | Should -Be (Join-Path $script:TestRoot 'target\debug\harvester_app.exe')
        $calls.SecretInvocations | Should -Be 1
    }

    It 'returns the batch policy with exactly its fixed runtime arguments' {
        $spec = Get-HarvesterLaunchSpec -Name Batch -RepositoryRoot $script:TestRoot
        @($spec.RuntimeArguments) | Should -Be @('--single-shot', '--batch-api')
        $spec.Package | Should -Be 'harvester_batch'
        $spec.BinaryName | Should -Be 'harvester_batch.exe'
    }

    It 'returns an independent copy of the runtime argument policy' {
        $first = Get-HarvesterLaunchSpec -Name Batch -RepositoryRoot $script:TestRoot
        $first.RuntimeArguments[0] = '--mutated-by-caller'
        $second = Get-HarvesterLaunchSpec -Name Batch -RepositoryRoot $script:TestRoot

        @($second.RuntimeArguments) | Should -Be @('--single-shot', '--batch-api')
    }

    It 'keeps the repository root on the spec as the launch location source of truth' {
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot

        $spec.RepositoryRoot | Should -Be ([System.IO.Path]::GetFullPath($script:TestRoot))
        $spec.ExecutablePath | Should -Be (Join-Path $spec.RepositoryRoot 'target\debug\harvester_app.exe')
        (Get-Command Invoke-HarvesterLaunch).Parameters.Keys | Should -Not -Contain 'RepositoryRoot'
    }

    It 'keeps the default secret wrapper compatible with an empty argument list' {
        InModuleScope HarvesterLaunch {
            function Invoke-WithSecretMap {
                param(
                    [Parameter(Mandatory)]
                    [System.Collections.IDictionary]$SecretEnvironmentMap,

                    [Parameter(Mandatory)]
                    [string]$Executable,

                    [Parameter()]
                    [AllowEmptyCollection()]
                    [string[]]$ArgumentList = @(),

                    [Parameter(Mandatory)]
                    [ref]$ExitCode
                )

                $null = @($SecretEnvironmentMap, $Executable)
                $script:DefaultWrapperArguments = [string[]]$ArgumentList
                Write-Host 'fake native child output'
                $ExitCode.Value = 0
            }

            try {
                $code = 1
                $output = @(
                    Invoke-DefaultHarvesterSecret `
                        -SecretEnvironmentMap ([ordered]@{ DummySecret = 'DUMMY_ENVIRONMENT_NAME' }) `
                        -Executable 'dummy.exe' `
                        -ArgumentList @() `
                        -ExitCode ([ref]$code)
                )

                $code | Should -Be 0
                @($script:DefaultWrapperArguments).Count | Should -Be 0
                $output.Count | Should -Be 0
            }
            finally {
                Remove-Item -LiteralPath Function:Invoke-WithSecretMap -ErrorAction SilentlyContinue
            }
        }
    }

    It 'uses exactly the two app secret mappings and no runtime arguments' {
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot
        $calls = New-Calls
        $code = 1

        Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
            -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls) `
            -SecretInvoker (New-SecretFake -Calls $calls) `
            -PromptCheck { $true } `
            -EnvironmentVariableProbe (New-EnvironmentVariableProbe -Values @{})

        @($calls.BuildPackages) | Should -Be @('harvester_app')
        @($calls.SecretMap.Keys) | Should -Be @('BraveSearchApiKey', 'OpenAIProductionKey')
        @($calls.SecretMap.Values) | Should -Be @('BRAVE_SEARCH_API_KEY', 'OPENAI_API_KEY')
        @($calls.Arguments).Count | Should -Be 0
    }

    It 'uses exactly the two batch secret mappings and fixed argument order' {
        $spec = Get-HarvesterLaunchSpec -Name Batch -RepositoryRoot $script:TestRoot
        $calls = New-Calls
        $code = 1

        Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
            -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls) `
            -SecretInvoker (New-SecretFake -Calls $calls) `
            -PromptCheck { $true } `
            -EnvironmentVariableProbe (New-EnvironmentVariableProbe -Values @{})

        @($calls.BuildPackages) | Should -Be @('harvester_batch')
        @($calls.SecretMap.Keys) | Should -Be @('BraveSearchApiKey', 'OpenAIProductionKey')
        @($calls.SecretMap.Values) | Should -Be @('BRAVE_SEARCH_API_KEY', 'OPENAI_API_KEY')
        @($calls.Arguments) | Should -Be @('--single-shot', '--batch-api')
    }

    It 'does not select forbidden secrets or inject any other environment variable' {
        foreach ($name in @('App', 'Batch')) {
            $spec = Get-HarvesterLaunchSpec -Name $name -RepositoryRoot $script:TestRoot
            @($spec.SecretEnvironmentMap.Keys) | Should -Not -Contain 'DeepSeekProductionKey'
            @($spec.SecretEnvironmentMap.Keys) | Should -Not -Contain 'MoonshotApiKey'
            @($spec.SecretEnvironmentMap.Keys) | Should -Not -Contain 'TEST_SECRET'
            @($spec.SecretEnvironmentMap.Values) | Should -BeIn @('BRAVE_SEARCH_API_KEY', 'OPENAI_API_KEY')
        }
    }

    It 'stops after a build failure without invoking secrets' {
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot
        $calls = New-Calls
        $code = 1

        {
            Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
                -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls -Fail) `
                -SecretInvoker (New-SecretFake -Calls $calls) `
                -PromptCheck { $true } `
                -EnvironmentVariableProbe (New-EnvironmentVariableProbe -Values @{})
        } | Should -Throw '*fake build failed*'

        $calls.SecretInvocations | Should -Be 0
    }

    It 'warns and continues for a key set persistently for the Windows user account' {
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot
        $spec.SecretEnvironmentMap = [ordered]@{ DummySecret = 'HARVESTER_TEST_PARENT_KEY' }
        $calls = New-Calls
        $probe = New-EnvironmentVariableProbe -Values @{
            'HARVESTER_TEST_PARENT_KEY|Process' = 'dummy-test-value'
            'HARVESTER_TEST_PARENT_KEY|User'    = 'dummy-test-value'
        }
        $code = 1

        Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
            -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls) `
            -SecretInvoker (New-SecretFake -Calls $calls) `
            -PromptCheck { $true } `
            -EnvironmentVariableProbe $probe `
            -WarningVariable warnings

        $calls.BuildPackages.Count | Should -Be 1
        $calls.SecretInvocations | Should -Be 1
        @($warnings).Count | Should -Be 1
        $warnings[0].Message | Should -BeLike '*HARVESTER_TEST_PARENT_KEY*persistently for the Windows user account*cargo build and the child process will inherit it*current session environment*NullString*Permanent removal*secondary option*'
    }

    It 'warns and continues for a key set only in the current session' {
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot
        $spec.SecretEnvironmentMap = [ordered]@{ DummySecret = 'HARVESTER_TEST_PARENT_KEY' }
        $calls = New-Calls
        $probe = New-EnvironmentVariableProbe -Values @{
            'HARVESTER_TEST_PARENT_KEY|Process' = 'dummy-test-value'
        }
        $code = 1

        Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
            -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls) `
            -SecretInvoker (New-SecretFake -Calls $calls) `
            -PromptCheck { $true } `
            -EnvironmentVariableProbe $probe `
            -WarningVariable warnings

        $calls.BuildPackages.Count | Should -Be 1
        $calls.SecretInvocations | Should -Be 1
        @($warnings).Count | Should -Be 1
        $warnings[0].Message | Should -BeLike '*HARVESTER_TEST_PARENT_KEY*only in the current session*cargo build and the child process will inherit it*Lock-Secrets*NullString*'
    }

    It 'does not warn or block for a blank parent variable' {
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot
        $spec.SecretEnvironmentMap = [ordered]@{ DummySecret = 'HARVESTER_TEST_PARENT_KEY' }
        $calls = New-Calls
        $probe = New-EnvironmentVariableProbe -Values @{
            'HARVESTER_TEST_PARENT_KEY|Process' = ''
            'HARVESTER_TEST_PARENT_KEY|User'    = 'dummy-test-value'
        }
        $code = 1

        Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
            -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls) `
            -SecretInvoker (New-SecretFake -Calls $calls) `
            -PromptCheck { $true } `
            -EnvironmentVariableProbe $probe `
            -WarningVariable warnings

        $calls.BuildPackages.Count | Should -Be 1
        $calls.SecretInvocations | Should -Be 1
        @($warnings).Count | Should -Be 0
    }

    It 'puts the child exit code in the ref and emits no success-stream output' {
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot
        $calls = New-Calls
        $code = 1
        $output = @(
            Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
                -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls) `
                -SecretInvoker (New-SecretFake -Calls $calls -ReportedExitCode 3) `
                -PromptCheck { $true } `
                -EnvironmentVariableProbe (New-EnvironmentVariableProbe -Values @{})
        )

        $code | Should -Be 3
        $code.GetType().Name | Should -Be 'Int32'
        $output.Count | Should -Be 0
    }

    It 'keeps a repository root containing spaces as one executable path argument' {
        $spec = Get-HarvesterLaunchSpec -Name Batch -RepositoryRoot $script:TestRoot
        $calls = New-Calls
        $code = 1

        Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
            -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls) `
            -SecretInvoker (New-SecretFake -Calls $calls) `
            -PromptCheck { $true } `
            -EnvironmentVariableProbe (New-EnvironmentVariableProbe -Values @{})

        $calls.Executable | Should -Be (Join-Path $script:TestRoot 'target\debug\harvester_batch.exe')
        $calls.Executable | Should -BeOfType [string]
        @($calls.Arguments).Count | Should -Be 2
    }

    It 'restores the working directory after a successful launch' {
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot
        $calls = New-Calls
        $before = (Get-Location).Path
        $code = 1

        Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
            -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls) `
            -SecretInvoker (New-SecretFake -Calls $calls) `
            -PromptCheck { $true } `
            -EnvironmentVariableProbe (New-EnvironmentVariableProbe -Values @{})

        (Get-Location).Path | Should -Be $before
    }

    It 'restores the working directory after a failed launch' {
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot
        $calls = New-Calls
        $before = (Get-Location).Path
        $code = 1

        {
            Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
                -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls -Fail) `
                -SecretInvoker (New-SecretFake -Calls $calls) `
                -PromptCheck { $true } `
                -EnvironmentVariableProbe (New-EnvironmentVariableProbe -Values @{})
        } | Should -Throw '*fake build failed*'

        (Get-Location).Path | Should -Be $before
    }

    It 'fails fast in a non-interactive session before building' {
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot
        $calls = New-Calls
        $code = 1

        {
            Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
                -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls) `
                -SecretInvoker (New-SecretFake -Calls $calls) `
                -PromptCheck { $false } `
                -EnvironmentVariableProbe (New-EnvironmentVariableProbe -Values @{})
        } | Should -Throw '*non-interactive*'

        $calls.BuildPackages.Count | Should -Be 0
    }

    It 'reports the missing secret helper and does not build' {
        $script:CommandsAvailable = $false
        $spec = Get-HarvesterLaunchSpec -Name App -RepositoryRoot $script:TestRoot
        $calls = New-Calls
        $code = 1

        {
            Invoke-HarvesterLaunch -Spec $spec -ExitCode ([ref]$code) `
                -BuildInvoker (New-BuildFake -Spec $spec -Calls $calls) `
                -SecretInvoker (New-SecretFake -Calls $calls) `
                -PromptCheck { $true } `
                -EnvironmentVariableProbe (New-EnvironmentVariableProbe -Values @{})
        } | Should -Throw '*Invoke-WithSecretMap*load your PowerShell profile*'

        $calls.BuildPackages.Count | Should -Be 0
    }
}

Describe 'Harvester launcher script contracts' {
    It '<file> parses, has no parameter block, and gets a launch spec' -ForEach @(
        @{ file = 'Start-HarvesterApp.ps1' }
        @{ file = 'Start-HarvesterBatch.ps1' }
    ) {
        $path = Join-Path $PSScriptRoot ('..\{0}' -f $file)
        $tokens = $null
        $errors = $null
        $ast = [System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)

        $errors | Should -BeNullOrEmpty
        $ast.ParamBlock | Should -BeNullOrEmpty
        $commands = @($ast.FindAll(
            { param($node) $node -is [System.Management.Automation.Language.CommandAst] },
            $true
        ) | ForEach-Object GetCommandName)
        $commands | Should -Contain 'Get-HarvesterLaunchSpec'
    }
}
