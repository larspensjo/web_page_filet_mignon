use serde::{Deserialize, Serialize};

/// Reducer-owned selection for the desktop job list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JobListMode {
    Results,
    #[default]
    SinceCheckpoint,
    Last24Hours,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_list_mode_default_is_since_checkpoint() {
        assert_eq!(JobListMode::default(), JobListMode::SinceCheckpoint);
    }
}
