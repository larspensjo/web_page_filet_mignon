//! Classifies a completed fetch by how it should affect the domain blacklist.
//! Lives in the engine because that is where the typed `FailureKind` exists;
//! the core blacklist reducer consumes the resulting `FetchOutcomeClass`.

use serde::{Deserialize, Serialize};

use crate::{FailureKind, JobOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FetchOutcomeClass {
    /// Fetch succeeded — clears accumulated strikes for the domain.
    Success,
    /// Site structurally refuses us (auth / forbidden / consent wall) — a strike.
    PermanentBlock,
    /// Temporary failure (timeout, rate-limit, 5xx) — does not affect the blacklist.
    Transient,
    /// Failure unrelated to domain hostility (bad URL, 404, too large, …) — ignored.
    Ignored,
}

/// Classify a full job result.
pub fn classify_fetch_outcome(result: &Result<JobOutcome, FailureKind>) -> FetchOutcomeClass {
    match result {
        Ok(_) => FetchOutcomeClass::Success,
        Err(kind) => classify_failure(kind),
    }
}

/// Classify a failure kind in isolation.
pub fn classify_failure(kind: &FailureKind) -> FetchOutcomeClass {
    match kind {
        FailureKind::HttpStatus(401 | 403 | 407 | 451) => FetchOutcomeClass::PermanentBlock,
        FailureKind::BlockedContent { .. } => FetchOutcomeClass::PermanentBlock,
        FailureKind::Timeout
        | FailureKind::Network
        | FailureKind::HttpStatus(408 | 429 | 500 | 502 | 503 | 504) => {
            FetchOutcomeClass::Transient
        }
        _ => FetchOutcomeClass::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permanent_blocks() {
        assert_eq!(
            classify_failure(&FailureKind::HttpStatus(403)),
            FetchOutcomeClass::PermanentBlock
        );
        assert_eq!(
            classify_failure(&FailureKind::HttpStatus(401)),
            FetchOutcomeClass::PermanentBlock
        );
        assert_eq!(
            classify_failure(&FailureKind::BlockedContent {
                description: "yahoo consent interstitial".to_string()
            }),
            FetchOutcomeClass::PermanentBlock
        );
    }

    #[test]
    fn transient_failures() {
        assert_eq!(
            classify_failure(&FailureKind::HttpStatus(429)),
            FetchOutcomeClass::Transient
        );
        assert_eq!(
            classify_failure(&FailureKind::Timeout),
            FetchOutcomeClass::Transient
        );
        assert_eq!(
            classify_failure(&FailureKind::Network),
            FetchOutcomeClass::Transient
        );
    }

    #[test]
    fn ignored_failures() {
        assert_eq!(
            classify_failure(&FailureKind::HttpStatus(404)),
            FetchOutcomeClass::Ignored
        );
        assert_eq!(
            classify_failure(&FailureKind::InvalidUrl),
            FetchOutcomeClass::Ignored
        );
    }
}
