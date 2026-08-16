Set-StrictMode -Version Latest

$script:HarvesterSecretEnvironmentMap = [ordered]@{
    BraveSearchApiKey   = 'BRAVE_SEARCH_API_KEY'
    OpenAIProductionKey = 'OPENAI_API_KEY'
}

$script:HarvesterLaunchPolicies = [ordered]@{
    App = [pscustomobject]@{
        Package              = 'harvester_app'
        BinaryName           = 'harvester_app.exe'
        RuntimeArguments     = [string[]]@()
        SecretEnvironmentMap = $script:HarvesterSecretEnvironmentMap
    }
    Batch = [pscustomobject]@{
        Package              = 'harvester_batch'
        BinaryName           = 'harvester_batch.exe'
        RuntimeArguments     = [string[]]@('--single-shot', '--batch-api')
        SecretEnvironmentMap = $script:HarvesterSecretEnvironmentMap
    }
}

function Get-HarvesterLaunchSpec {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [ValidateSet('App', 'Batch')]
        [string]$Name,

        [Parameter(Mandatory)]
        [string]$RepositoryRoot
    )

    $root = [System.IO.Path]::GetFullPath($RepositoryRoot)
    $policy = $script:HarvesterLaunchPolicies[$Name]
    $secretMap = [ordered]@{}
    foreach ($entry in $policy.SecretEnvironmentMap.GetEnumerator()) {
        $secretMap[[string]$entry.Key] = [string]$entry.Value
    }

    [pscustomobject]@{
        Name             = $Name
        RepositoryRoot   = $root
        Package          = $policy.Package
        BinaryName       = $policy.BinaryName
        ExecutablePath   = Join-Path $root (Join-Path 'target\debug' $policy.BinaryName)
        RuntimeArguments = [string[]]$policy.RuntimeArguments.Clone()
        SecretEnvironmentMap = $secretMap
    }
}

function Invoke-DefaultHarvesterBuild {
    param(
        [Parameter(Mandatory)]
        [string]$Package
    )

    & cargo build -p $Package
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build -p $Package failed with exit code $LASTEXITCODE."
    }
}

function Invoke-DefaultHarvesterSecret {
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

    Invoke-WithSecretMap `
        -SecretEnvironmentMap $SecretEnvironmentMap `
        -Executable $Executable `
        -ArgumentList $ArgumentList `
        -ExitCode $ExitCode
}

function Test-HarvesterCommandAvailable {
    param(
        [Parameter(Mandatory)]
        [string]$Name
    )

    $null -ne (Get-Command -Name $Name -ErrorAction SilentlyContinue)
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
        [scriptblock]$SecretInvoker,

        [Parameter()]
        [scriptblock]$PromptCheck,

        [Parameter()]
        [scriptblock]$EnvironmentVariableProbe
    )

    $ExitCode.Value = 1

    if (-not (Test-HarvesterCommandAvailable -Name 'Invoke-WithSecretMap')) {
        throw "Invoke-WithSecretMap is unavailable; load your PowerShell profile; this launcher does not dot-source it."
    }
    if (-not (Test-HarvesterCommandAvailable -Name 'Test-SecretStorePromptAvailable')) {
        throw "Test-SecretStorePromptAvailable is unavailable; load your PowerShell profile; this launcher does not dot-source it."
    }

    $promptIsAvailable = if ($null -eq $PromptCheck) {
        Test-SecretStorePromptAvailable
    }
    else {
        & $PromptCheck
    }
    if (-not [bool]$promptIsAvailable) {
        throw 'SecretStore prompting is unavailable in this non-interactive session; run this launcher from an interactive PowerShell terminal.'
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

    foreach ($environmentName in $Spec.SecretEnvironmentMap.Values) {
        $environmentName = [string]$environmentName
        $processValue = & $environmentProbe $environmentName ([System.EnvironmentVariableTarget]::Process)
        if (-not [string]::IsNullOrEmpty([string]$processValue)) {
            $userValue = & $environmentProbe $environmentName ([System.EnvironmentVariableTarget]::User)
            $machineValue = & $environmentProbe $environmentName ([System.EnvironmentVariableTarget]::Machine)
            $sessionRemedy = "[System.Environment]::SetEnvironmentVariable('$environmentName', [NullString]::Value, 'Process')"

            if (-not [string]::IsNullOrEmpty([string]$userValue)) {
                $scopeDescription = 'persistently for the Windows user account'
                if (-not [string]::IsNullOrEmpty([string]$machineValue)) {
                    $scopeDescription += ' and machine'
                }
                Write-Warning "Parent environment variable $environmentName is set $scopeDescription. cargo build and the child process will inherit it. To prevent inheritance for this launch while keeping the persistent setting, remove it from the current session environment first: $sessionRemedy. Permanent removal from the persistent scope is a secondary option."
            }
            elseif (-not [string]::IsNullOrEmpty([string]$machineValue)) {
                Write-Warning "Parent environment variable $environmentName is set persistently for the machine. cargo build and the child process will inherit it. To prevent inheritance for this launch while keeping the persistent setting, remove it from the current session environment first: $sessionRemedy. Permanent removal from the persistent scope is a secondary option."
            }
            else {
                Write-Warning "Parent environment variable $environmentName is set only in the current session. cargo build and the child process will inherit it. If Unlock-Secrets populated it, run Lock-Secrets; otherwise remove it from the current session environment first: $sessionRemedy."
            }
        }
    }

    $build = if ($null -eq $BuildInvoker) {
        ${function:Invoke-DefaultHarvesterBuild}
    }
    else {
        $BuildInvoker
    }
    $secret = if ($null -eq $SecretInvoker) {
        ${function:Invoke-DefaultHarvesterSecret}
    }
    else {
        $SecretInvoker
    }

    $pushed = $false
    try {
        Push-Location -LiteralPath $Spec.RepositoryRoot
        $pushed = $true

        & $build $Spec.Package

        if (-not (Test-Path -LiteralPath $Spec.ExecutablePath -PathType Leaf)) {
            throw "Built binary was not found at '$($Spec.ExecutablePath)'."
        }

        $childExitCode = 1
        & $secret `
            $Spec.SecretEnvironmentMap `
            ([string]$Spec.ExecutablePath) `
            ([string[]]$Spec.RuntimeArguments) `
            ([ref]$childExitCode)

        $ExitCode.Value = [int]$childExitCode
    }
    finally {
        if ($pushed) {
            Pop-Location
        }
    }
}

Export-ModuleMember -Function Get-HarvesterLaunchSpec, Invoke-HarvesterLaunch
