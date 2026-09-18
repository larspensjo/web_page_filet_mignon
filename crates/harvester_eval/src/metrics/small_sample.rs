//! Shared rendering for rates whose support is too small to interpret.

use serde::{Deserialize, Serialize};

pub const DEFAULT_FLOOR: u64 = 30;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Rate {
    pub numerator: u64,
    pub denominator: u64,
}

impl Rate {
    pub const fn new(numerator: u64, denominator: u64) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    pub fn value(self) -> Option<f64> {
        (self.denominator > 0).then(|| self.numerator as f64 / self.denominator as f64)
    }

    pub fn label(self, floor: u64) -> String {
        rate_label(self.numerator, self.denominator, floor)
    }
}

pub fn rate_label(numerator: u64, denominator: u64, floor: u64) -> String {
    if denominator < floor {
        format!("n = {denominator}, rate not reported (count {numerator})")
    } else {
        format!(
            "{numerator} ({:.1}%)",
            numerator as f64 * 100.0 / denominator as f64
        )
    }
}
