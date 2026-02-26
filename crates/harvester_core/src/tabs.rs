use engine_logging::engine_warn;

/// The active content tab in the right pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppTab {
    Triage,
    #[default]
    Summary,
    Briefing,
    Trends,
}

impl AppTab {
    /// Returns the zero-based index of this tab in the canonical order.
    pub fn to_index(self) -> usize {
        match self {
            AppTab::Triage => 0,
            AppTab::Summary => 1,
            AppTab::Briefing => 2,
            AppTab::Trends => 3,
        }
    }

    /// Returns the tab for the given zero-based index, or `None` if out of range.
    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(AppTab::Triage),
            1 => Some(AppTab::Summary),
            2 => Some(AppTab::Briefing),
            3 => Some(AppTab::Trends),
            _ => {
                engine_warn!("[tabs] AppTab::from_index: out-of-range index {index}");
                None
            }
        }
    }
}

/// The active left-pane tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LeftTab {
    #[default]
    JobList,
    PromptLab,
}

impl LeftTab {
    /// Returns the zero-based index of this tab in the canonical order.
    pub fn to_index(self) -> usize {
        match self {
            LeftTab::JobList => 0,
            LeftTab::PromptLab => 1,
        }
    }

    /// Returns the tab for the given zero-based index, or `None` if out of range.
    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(LeftTab::JobList),
            1 => Some(LeftTab::PromptLab),
            _ => {
                engine_warn!("[tabs] LeftTab::from_index: out-of-range index {index}");
                None
            }
        }
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

impl TrendCategory {
    /// Returns the zero-based index of this category in the canonical order.
    pub fn to_index(self) -> usize {
        match self {
            TrendCategory::Companies => 0,
            TrendCategory::Technologies => 1,
            TrendCategory::Products => 2,
            TrendCategory::Themes => 3,
        }
    }

    /// Returns the category for the given zero-based index, or `None` if out of range.
    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(TrendCategory::Companies),
            1 => Some(TrendCategory::Technologies),
            2 => Some(TrendCategory::Products),
            3 => Some(TrendCategory::Themes),
            _ => {
                engine_warn!("[tabs] TrendCategory::from_index: out-of-range index {index}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_tab_round_trip() {
        let variants = [
            AppTab::Triage,
            AppTab::Summary,
            AppTab::Briefing,
            AppTab::Trends,
        ];
        for tab in variants {
            assert_eq!(AppTab::from_index(tab.to_index()), Some(tab));
        }
    }

    #[test]
    fn app_tab_from_index_out_of_range_returns_none() {
        assert_eq!(AppTab::from_index(4), None);
        assert_eq!(AppTab::from_index(usize::MAX), None);
    }

    #[test]
    fn left_tab_round_trip() {
        let variants = [LeftTab::JobList, LeftTab::PromptLab];
        for tab in variants {
            assert_eq!(LeftTab::from_index(tab.to_index()), Some(tab));
        }
    }

    #[test]
    fn left_tab_from_index_out_of_range_returns_none() {
        assert_eq!(LeftTab::from_index(2), None);
    }

    #[test]
    fn trend_category_round_trip() {
        let variants = [
            TrendCategory::Companies,
            TrendCategory::Technologies,
            TrendCategory::Products,
            TrendCategory::Themes,
        ];
        for cat in variants {
            assert_eq!(TrendCategory::from_index(cat.to_index()), Some(cat));
        }
    }

    #[test]
    fn trend_category_from_index_out_of_range_returns_none() {
        assert_eq!(TrendCategory::from_index(4), None);
    }
}
