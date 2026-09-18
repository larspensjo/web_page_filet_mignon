use engine_logging::engine_warn;
use serde::{Deserialize, Serialize};

/// The new desktop workspace; retained alongside the Win32 tabs until phase 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WorkspaceView {
    #[default]
    Review,
    Trends,
    PollStats,
    Blacklist,
}

/// Reducer-owned selection for the desktop job list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JobListMode {
    Results,
    #[default]
    SinceCheckpoint,
    Last24Hours,
}

/// The active trend category in the Trends tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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
    fn job_list_mode_default_is_since_checkpoint() {
        assert_eq!(JobListMode::default(), JobListMode::SinceCheckpoint);
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
