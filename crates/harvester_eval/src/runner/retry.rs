//! Pure retry disposition and delay calculations.

use std::time::Duration;

use crate::config::JevConfig;
use crate::jev::transport::TransportOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    Success,
    Terminal { reason: &'static str },
    Retryable { reason: &'static str },
}

pub fn classify(outcome: &TransportOutcome) -> Disposition {
    match outcome {
        TransportOutcome::Response {
            status: 200..=299, ..
        } => Disposition::Success,
        TransportOutcome::Response { status: 401, .. } => Disposition::Terminal {
            reason: "access_denied",
        },
        TransportOutcome::Response { status: 422, .. } => Disposition::Terminal {
            reason: "invalid_input",
        },
        TransportOutcome::Response { status: 429, .. } => Disposition::Retryable {
            reason: "rate_limited",
        },
        TransportOutcome::Response { status: 529, .. } => Disposition::Retryable {
            reason: "provider_overloaded",
        },
        TransportOutcome::Response {
            status: 500..=599, ..
        } => Disposition::Retryable {
            reason: "server_error",
        },
        TransportOutcome::Response { .. } => Disposition::Terminal {
            reason: "http_error",
        },
        TransportOutcome::Timeout { .. } => Disposition::Retryable { reason: "timeout" },
        TransportOutcome::ConnectionError { .. } => Disposition::Retryable {
            reason: "connection_error",
        },
    }
}

/// Capped exponential delay. `jitter` is injected so tests and decisions stay
/// deterministic; callers pass a sampled value in `[0, 1]` only when enabled.
pub fn backoff_delay(attempt: u32, config: &JevConfig, jitter: f64) -> Duration {
    let exponent = attempt.saturating_sub(1).min(63);
    let base = config
        .backoff_initial_ms
        .saturating_mul(1_u64 << exponent)
        .min(config.backoff_max_ms);
    let multiplier = if config.jitter {
        0.5 + jitter.clamp(0.0, 1.0)
    } else {
        1.0
    };
    Duration::from_millis((((base as f64) * multiplier).round() as u64).min(config.backoff_max_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16) -> TransportOutcome {
        TransportOutcome::Response {
            status,
            body: Vec::new(),
            model_latency: Duration::ZERO,
        }
    }

    #[test]
    fn classifies_documented_failure_classes() {
        assert_eq!(
            classify(&response(401)),
            Disposition::Terminal {
                reason: "access_denied"
            }
        );
        assert_eq!(
            classify(&response(422)),
            Disposition::Terminal {
                reason: "invalid_input"
            }
        );
        for status in [429, 529, 500, 503] {
            assert!(matches!(
                classify(&response(status)),
                Disposition::Retryable { .. }
            ));
        }
        assert!(matches!(
            classify(&TransportOutcome::Timeout {
                model_latency: Duration::ZERO
            }),
            Disposition::Retryable { reason: "timeout" }
        ));
        assert!(matches!(
            classify(&TransportOutcome::ConnectionError {
                detail: "reset".into(),
                model_latency: Duration::ZERO
            }),
            Disposition::Retryable {
                reason: "connection_error"
            }
        ));
        assert_eq!(classify(&response(200)), Disposition::Success);
    }

    #[test]
    fn backoff_is_capped_and_injectable() {
        let config = JevConfig {
            backoff_initial_ms: 100,
            backoff_max_ms: 350,
            jitter: false,
            ..JevConfig::default()
        };
        let actual: Vec<_> = (1..=5)
            .map(|attempt| backoff_delay(attempt, &config, 0.2).as_millis())
            .collect();
        assert_eq!(actual, vec![100, 200, 350, 350, 350]);
        let jittered = JevConfig {
            jitter: true,
            ..config
        };
        let fixed_low: Vec<_> = (1..=5)
            .map(|attempt| backoff_delay(attempt, &jittered, 0.0).as_millis())
            .collect();
        assert_eq!(fixed_low, [50, 100, 175, 175, 175]);
        assert_eq!(backoff_delay(2, &jittered, 1.0), Duration::from_millis(300));
        assert_eq!(backoff_delay(5, &jittered, 1.0), Duration::from_millis(350));
    }
}
