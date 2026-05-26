#Requires -Version 5.1
Set-StrictMode -Version Latest

function ConvertFrom-KeyInfoToAction {
    param(
        [Parameter(Mandatory = $true)][System.ConsoleKeyInfo]$KeyInfo
    )

    switch ($KeyInfo.Key) {
        'Q' { return [pscustomobject]@{ Type = 'Quit' } }
        'Tab' { return [pscustomobject]@{ Type = 'SwitchPane' } }
        'UpArrow' { return [pscustomobject]@{ Type = 'MoveUp' } }
        'DownArrow' { return [pscustomobject]@{ Type = 'MoveDown' } }
        'PageUp' { return [pscustomobject]@{ Type = 'PageUp' } }
        'PageDown' { return [pscustomobject]@{ Type = 'PageDown' } }
        'Home' { return [pscustomobject]@{ Type = 'MoveHome' } }
        'End' { return [pscustomobject]@{ Type = 'MoveEnd' } }
        'Spacebar' { return [pscustomobject]@{ Type = 'ToggleTag' } }
        default { return $null }
    }
}

function ConvertFrom-KeyInfoToLauncherAction {
    param([System.ConsoleKeyInfo]$KeyInfo)

    # Launcher-specific overrides — evaluated before submodule fallback
    switch ($KeyInfo.Key) {
        'Enter'      { return [pscustomobject]@{ Type = 'Activate'      } }
        'Escape'     { return [pscustomobject]@{ Type = 'Cancel'        } }
        'RightArrow' { return [pscustomobject]@{ Type = 'ValueIncrease' } }
        'LeftArrow'  { return [pscustomobject]@{ Type = 'ValueDecrease' } }
        'Spacebar'   { return [pscustomobject]@{ Type = 'ValueToggle'   } }
        'Add'        { return [pscustomobject]@{ Type = 'ValueIncrease' } }
        'Subtract'   { return [pscustomobject]@{ Type = 'ValueDecrease' } }
    }

    # Character-based overrides (handles S/s and +/-)
    switch ($KeyInfo.KeyChar) {
        'S'  { return [pscustomobject]@{ Type = 'SaveDefaults'  } }
        's'  { return [pscustomobject]@{ Type = 'SaveDefaults'  } }
        '+'  { return [pscustomobject]@{ Type = 'ValueIncrease' } }
        '-'  { return [pscustomobject]@{ Type = 'ValueDecrease' } }
    }

    # Fall through to the shared navigation mappings.
    ConvertFrom-KeyInfoToAction -KeyInfo $KeyInfo
}

Export-ModuleMember -Function ConvertFrom-KeyInfoToLauncherAction
