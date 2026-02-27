# PSScriptAnalyzer settings for scripts/
# Pass explicitly: Invoke-ScriptAnalyzer -Path .\scripts\ -Recurse -Settings .\scripts\PSScriptAnalyzerSettings.psd1
@{
    ExcludeRules = @(
        # These scripts are internal tooling, not public modules.
        # Plural nouns and unapproved verbs (Count-, Ensure-, Normalize-, Build-, etc.)
        # are kept as-is for readability.
        'PSUseSingularNouns'
        'PSUseApprovedVerbs'

        # TUI and batch scripts intentionally write to the host for interactive output.
        'PSAvoidUsingWriteHost'

        # Internal helper functions don't need comment-based help.
        'PSProvideCommentHelp'

        # New-* functions here construct objects/strings; they don't change system state.
        'PSUseShouldProcessForStateChangingFunctions'

        # Write-Warning and a few other cmdlets are called with positional args intentionally.
        'PSAvoidUsingPositionalParameters'
    )
}
