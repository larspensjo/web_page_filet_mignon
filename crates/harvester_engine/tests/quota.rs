use harvester_engine::{FailureKind, QuotaTracker, RunQuotas};

#[test]
fn url_limit_blocks_after_limit() {
    let quotas = RunQuotas {
        max_urls_per_run: Some(2),
    };
    let mut tracker = QuotaTracker::new(quotas);
    assert!(tracker.check_url().is_ok());
    assert!(tracker.check_url().is_ok());
    let err = tracker.check_url().unwrap_err();
    assert!(matches!(err, FailureKind::QuotaExceeded { .. }));
}

#[test]
fn reset_starts_a_new_run_allowance() {
    let quotas = RunQuotas {
        max_urls_per_run: Some(2),
    };
    let mut tracker = QuotaTracker::new(quotas);
    assert!(tracker.check_url().is_ok());
    assert!(tracker.check_url().is_ok());
    assert!(tracker.check_url().is_err());
    tracker.reset();
    assert_eq!(tracker.urls_started(), 0);
    assert!(tracker.check_url().is_ok());
    assert!(tracker.check_url().is_ok());
    assert!(tracker.check_url().is_err());
}

#[test]
fn unlimited_quotas_never_block() {
    let quotas = RunQuotas {
        max_urls_per_run: None,
    };
    let mut tracker = QuotaTracker::new(quotas);
    for _ in 0..100 {
        assert!(tracker.check_url().is_ok());
    }
    assert_eq!(tracker.urls_started(), 100);
}
