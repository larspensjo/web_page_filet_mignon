#Requires -Version 5.1
Set-StrictMode -Version Latest

function Get-LauncherActionItems {
    @(
        [pscustomobject]@{ Id='run-batch';   Label='Run batch (continuous)';    IsSeparator=$false; IsCheckpoint=$false; IsDryRun=$false }
        [pscustomobject]@{ Id='run-dry';     Label='Run dry-run (single poll)'; IsSeparator=$false; IsCheckpoint=$false; IsDryRun=$true  }
        [pscustomobject]@{ Id='sep-1';       Label='';                          IsSeparator=$true;  IsCheckpoint=$false; IsDryRun=$false }
        [pscustomobject]@{ Id='cp-set-now';  Label='Set checkpoint to now';     IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false }
        [pscustomobject]@{ Id='cp-set-date'; Label='Set checkpoint to date...'; IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false }
        [pscustomobject]@{ Id='cp-clear';    Label='Clear checkpoint';          IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false }
        [pscustomobject]@{ Id='cp-show';     Label='Show current checkpoint';   IsSeparator=$false; IsCheckpoint=$true;  IsDryRun=$false }
    )
}

function Get-LauncherParamDefs {
    # Order matters: determines right-pane cursor index
    @(
        [pscustomobject]@{ Name='LlmConcurrency';   Label='LLM concurrency';   Type='Int';  Min=1;    Max=10;   Unit='';     Flag='--llm-concurrency'           }
        [pscustomobject]@{ Name='PollInterval';     Label='Poll interval';     Type='Int';  Min=1;    Max=1440; Unit=' min'; Flag='--poll-interval'             }
        [pscustomobject]@{ Name='ForceUnlock';      Label='Force unlock';      Type='Bool'; Min=$null; Max=$null; Unit='';   Flag='--force-unlock'              }
        [pscustomobject]@{ Name='AllowUnsupported'; Label='Allow unsupported'; Type='Bool'; Min=$null; Max=$null; Unit='';   Flag='--allow-unsupported-sources' }
        [pscustomobject]@{ Name='Sources';          Label='Sources file';      Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--sources'                   }
        [pscustomobject]@{ Name='OutputDir';        Label='Output dir';        Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--output-dir'                }
        [pscustomobject]@{ Name='ContextsDir';      Label='Contexts dir';      Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--contexts-dir'              }
        [pscustomobject]@{ Name='PromptsDir';       Label='Prompts dir';       Type='Path'; Min=$null; Max=$null; Unit='';   Flag='--prompts-dir'               }
    )
}

function New-LauncherDefaults {
    @{
        LlmConcurrency   = 6
        PollInterval     = 15
        ForceUnlock      = $false
        AllowUnsupported = $false
        Sources          = 'sources.ron'
        OutputDir        = 'output'
        ContextsDir      = 'contexts'
        PromptsDir       = 'prompts'
    }
}

function Get-DefaultsFilePath {
    # Resolves to scripts/harvester_launcher_defaults.json
    Join-Path (Split-Path -Parent $PSScriptRoot) 'harvester_launcher_defaults.json'
}

Export-ModuleMember -Function Get-LauncherActionItems, Get-LauncherParamDefs, New-LauncherDefaults, Get-DefaultsFilePath
