//! Priority's ordinal representation and high-priority probability helper.

use std::collections::BTreeMap;

/// Priority 1 is lowest and priority 5 is highest.
pub type Priority = u8;

/// A complete priority distribution that has passed Jev response validation.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedPriorityDistribution(BTreeMap<String, f64>);

impl ValidatedPriorityDistribution {
    pub(crate) fn new(probabilities: BTreeMap<String, f64>) -> Self {
        Self(probabilities)
    }

    /// Returns the validated probabilities keyed by priority label `"1"` through `"5"`.
    pub fn probabilities(&self) -> &BTreeMap<String, f64> {
        &self.0
    }

    /// Returns the probability for a priority label.
    pub fn get(&self, priority: &str) -> Option<f64> {
        self.0.get(priority).copied()
    }
}

/// Probability of priority 4 or 5 from an already validated distribution.
pub fn p_high(distribution: &ValidatedPriorityDistribution) -> f64 {
    distribution
        .get("4")
        .expect("validated distribution contains priority 4")
        + distribution
            .get("5")
            .expect("validated distribution contains priority 5")
}
