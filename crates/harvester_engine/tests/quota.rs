use harvester_engine::{FailureKind, QuotaTracker, SessionQuotas};

#[test]
fn url_limit_blocks_after_limit() {
    let quotas = SessionQuotas {
        max_urls_per_session: Some(2),
        ..SessionQuotas::default()
    };
    let mut tracker = QuotaTracker::new(quotas);
    assert!(tracker.check_url().is_ok());
    assert!(tracker.check_url().is_ok());
    let err = tracker.check_url().unwrap_err();
    assert!(matches!(err, FailureKind::QuotaExceeded { .. }));
}

#[test]
fn record_job_increments_bytes_and_tokens() {
    let quotas = SessionQuotas {
        max_bytes_per_session: Some(100),
        max_total_tokens_per_session: Some(100),
        ..SessionQuotas::default()
    };
    let mut tracker = QuotaTracker::new(quotas);
    tracker.record_job(10, 5);
    tracker.record_job(20, 15);
    assert_eq!(tracker.totals(), (0, 30, 20));
}

#[test]
fn unlimited_quotas_never_block() {
    let quotas = SessionQuotas {
        max_urls_per_session: None,
        max_bytes_per_session: None,
        max_total_tokens_per_session: None,
    };
    let mut tracker = QuotaTracker::new(quotas);
    for _ in 0..100 {
        assert!(tracker.check_url().is_ok());
    }
    tracker.record_job(1, 1);
    assert_eq!(tracker.totals().0, 100);
}
