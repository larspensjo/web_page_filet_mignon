/// The active content tab in the right pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppTab {
    Triage,
    #[default]
    Summary,
    Briefing,
    Trends,
}

/// The active left-pane tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LeftTab {
    #[default]
    JobList,
    PromptLab,
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
