//! Shared thresholding for categories and tags.

use std::collections::BTreeMap;

/// Selects values at or above the inclusive threshold in deterministic order.
pub fn select_above_threshold(
    probabilities: &BTreeMap<String, f64>,
    threshold: f64,
) -> Vec<String> {
    probabilities
        .iter()
        .filter(|(_, probability)| **probability >= threshold)
        .map(|(label, _)| label.clone())
        .collect()
}
