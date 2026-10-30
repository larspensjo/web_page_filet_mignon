use harvester_engine::llm::{LlmQuotaTracker, LlmQuotas};
use harvester_engine::FailureKind;

#[test]
fn default_call_limit_supports_large_summary_runs() {
    assert_eq!(LlmQuotas::default().max_calls_per_run, Some(1_000));
}

#[test]
fn call_limit_blocks() {
    let quotas = LlmQuotas {
        max_calls_per_run: Some(2),
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
fn reset_usage_clears_totals_and_reopens_every_limit() {
    let quotas = LlmQuotas {
        max_calls_per_run: Some(1),
        max_input_tokens_per_run: Some(10),
        max_output_tokens_per_run: Some(5),
        max_cost_microdollars_per_run: Some(100),
    };
    let mut tracker = LlmQuotaTracker::new(quotas);
    tracker.reserve_call().unwrap();
    tracker.record_call_usage(10, 5, 100).unwrap();
    assert!(tracker.reserve_call().is_err());
    assert!(tracker.check_call(0, 1, 0).is_err());

    tracker.reset_usage();

    assert_eq!(tracker.totals().calls, 0);
    assert_eq!(tracker.totals().input_tokens, 0);
    assert!(tracker.reserve_call().is_ok());
    assert!(tracker.record_call_usage(10, 5, 100).is_ok());
}

#[test]
fn byte_and_token_limits_block() {
    let quotas = LlmQuotas {
        max_input_tokens_per_run: Some(10),
        max_output_tokens_per_run: Some(5),
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
        max_cost_microdollars_per_run: Some(100),
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
        max_calls_per_run: None,
        max_input_tokens_per_run: None,
        max_output_tokens_per_run: None,
        max_cost_microdollars_per_run: None,
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
