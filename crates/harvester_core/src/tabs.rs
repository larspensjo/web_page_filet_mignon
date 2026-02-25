/// The active content tab in the right pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppTab {
    Triage,
    Summary,
    Briefing,
    Trends,
    PromptLab,
}

impl Default for AppTab {
    fn default() -> Self {
        AppTab::Summary
    }
}

/// The active trend category in the Trends tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrendCategory {
    #[default]
    Companies,
    Technologies,
    Products,
    Themes,
}
