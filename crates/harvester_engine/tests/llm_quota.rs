use harvester_engine::llm::{LlmQuotaTracker, LlmQuotas};
use harvester_engine::FailureKind;

#[test]
fn call_limit_blocks() {
    let quotas = LlmQuotas {
        max_calls_per_session: Some(2),
        ..Default::default()
    };
    let mut tracker = LlmQuotaTracker::new(quotas);
    assert!(tracker.check_call(0, 0, 0).is_ok());
    tracker.record_call(0, 0, 0);
    assert!(tracker.check_call(0, 0, 0).is_ok());
    tracker.record_call(0, 0, 0);
    let err = tracker.check_call(0, 0, 0).unwrap_err();
    assert!(matches!(err, FailureKind::QuotaExceeded { .. }));
}

#[test]
fn byte_and_token_limits_block() {
    let quotas = LlmQuotas {
        max_input_tokens_per_session: Some(10),
        max_output_tokens_per_session: Some(5),
        ..Default::default()
    };
    let mut tracker = LlmQuotaTracker::new(quotas);
    assert!(tracker.check_call(7, 3, 0).is_ok());
    tracker.record_call(7, 3, 0);
    assert!(tracker.check_call(4, 2, 0).is_err());
}

#[test]
fn cost_limit_blocks() {
    let quotas = LlmQuotas {
        max_cost_microdollars_per_session: Some(100),
        ..Default::default()
    };
    let mut tracker = LlmQuotaTracker::new(quotas);
    assert!(tracker.check_call(0, 0, 80).is_ok());
    tracker.record_call(0, 0, 80);
    assert!(tracker.check_call(0, 0, 30).is_err());
}

#[test]
fn unlimited_quota_never_blocks() {
    let quotas = LlmQuotas {
        max_calls_per_session: None,
        max_input_tokens_per_session: None,
        max_output_tokens_per_session: None,
        max_cost_microdollars_per_session: None,
    };
    let mut tracker = LlmQuotaTracker::new(quotas);
    for _ in 0..1000 {
        assert!(tracker.check_call(1, 1, 1).is_ok());
        tracker.record_call(1, 1, 1);
    }
}

#[test]
fn totals_saturate() {
    let quotas = LlmQuotas::default();
    let mut tracker = LlmQuotaTracker::new(quotas);
    tracker.record_call(u64::MAX, u64::MAX, u64::MAX);
    let totals = tracker.totals();
    assert_eq!(totals.input_tokens, u64::MAX);
    assert_eq!(totals.output_tokens, u64::MAX);
    assert_eq!(totals.cost_microdollars, u64::MAX);
}
