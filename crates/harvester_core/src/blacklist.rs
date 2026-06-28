//! Pure, persistent model tracking per-domain fetch failures and deriving a
//! self-healing blacklist. Reduced from `Msg::FetchOutcomeClassified` and
//! consulted before enqueuing URLs. Time is always passed in as `now`.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use harvester_engine::{registrable_domain, FetchOutcomeClass};
use serde::{Deserialize, Serialize};

pub const BLACKLIST_STRIKE_THRESHOLD: u32 = 3;
pub const INITIAL_COOLDOWN_DAYS: i64 = 7;
pub const MAX_COOLDOWN_DAYS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DomainRecord {
    /// Consecutive permanent-block strikes since the last success.
    pub strikes: u32,
    /// Lifetime permanent-block count (for display; never reset).
    pub total_failures: u64,
    /// Human-readable last failure (e.g. "http status 403").
    pub last_failure_kind: Option<String>,
    /// When the domain was most recently (re)blacklisted.
    pub blacklisted_at: Option<DateTime<Utc>>,
    /// Skip the domain until this instant; `None` means not currently blacklisted.
    pub cooldown_until: Option<DateTime<Utc>>,
    /// How many times the cooldown has been armed (drives exponential backoff).
    pub cooldown_streak: u32,
    /// Timestamp of the most recent recorded outcome.
    pub last_outcome_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlacklistState {
    domains: BTreeMap<String, DomainRecord>,
}

fn cooldown_days_for_streak(streak: u32) -> i64 {
    // streak is >= 1 when this is called.
    let exp = streak.saturating_sub(1).min(10);
    let days = INITIAL_COOLDOWN_DAYS.saturating_mul(1_i64 << exp);
    days.min(MAX_COOLDOWN_DAYS)
}

impl BlacklistState {
    /// Records one classified outcome for `domain`. Returns `true` when the
    /// stored record changed, so callers can drive persistence/render precisely.
    ///
    /// Cooldown is armed **only on a transition into the blocked state**: the
    /// first time strikes cross the threshold, or when a probe fails after the
    /// previous cooldown has expired. Additional permanent failures that arrive
    /// while the domain is already cooling down (e.g. several in-flight jobs for
    /// the same site finishing after the 3rd strike) accumulate strikes but do
    /// **not** re-arm or escalate the cooldown.
    pub fn record_outcome(
        &mut self,
        domain: &str,
        class: FetchOutcomeClass,
        failure_label: Option<&str>,
        now: DateTime<Utc>,
    ) -> bool {
        match class {
            FetchOutcomeClass::Success => {
                // A success clears the active blacklist but keeps lifetime history.
                if let Some(record) = self.domains.get_mut(domain) {
                    let was_active = record.strikes != 0
                        || record.cooldown_until.is_some()
                        || record.cooldown_streak != 0
                        || record.last_outcome_at != Some(now);
                    record.strikes = 0;
                    record.blacklisted_at = None;
                    record.cooldown_until = None;
                    record.cooldown_streak = 0;
                    record.last_outcome_at = Some(now);
                    was_active
                } else {
                    false
                }
            }
            FetchOutcomeClass::PermanentBlock => {
                let record = self.domains.entry(domain.to_string()).or_default();
                record.strikes = record.strikes.saturating_add(1);
                record.total_failures = record.total_failures.saturating_add(1);
                record.last_failure_kind = failure_label.map(|s| s.to_string());
                record.last_outcome_at = Some(now);
                // Only (re)arm on a transition into the blocked state. While the
                // domain is still within an active cooldown window, extra failures
                // must not escalate 7 -> 14 -> 28 days.
                let currently_blocked = record
                    .cooldown_until
                    .map(|until| now < until)
                    .unwrap_or(false);
                if record.strikes >= BLACKLIST_STRIKE_THRESHOLD && !currently_blocked {
                    record.cooldown_streak = record.cooldown_streak.saturating_add(1);
                    record.blacklisted_at = Some(now);
                    record.cooldown_until = Some(
                        now + Duration::days(cooldown_days_for_streak(record.cooldown_streak)),
                    );
                }
                true
            }
            // Transient / Ignored: no state change.
            FetchOutcomeClass::Transient | FetchOutcomeClass::Ignored => false,
        }
    }

    pub fn is_blocked(&self, domain: &str, now: DateTime<Utc>) -> bool {
        self.domains
            .get(domain)
            .and_then(|r| r.cooldown_until)
            .map(|until| now < until)
            .unwrap_or(false)
    }

    pub fn record_for_url(
        &mut self,
        url: &str,
        class: FetchOutcomeClass,
        failure_label: Option<&str>,
        now: DateTime<Utc>,
    ) -> bool {
        if let Some(domain) = registrable_domain(url) {
            self.record_outcome(&domain, class, failure_label, now)
        } else {
            false
        }
    }

    pub fn is_url_blocked(&self, url: &str, now: DateTime<Utc>) -> bool {
        registrable_domain(url)
            .map(|domain| self.is_blocked(&domain, now))
            .unwrap_or(false)
    }

    pub fn rows(&self) -> Vec<(&String, &DomainRecord)> {
        let mut rows: Vec<_> = self.domains.iter().collect();
        rows.sort_by(|a, b| b.1.strikes.cmp(&a.1.strikes).then_with(|| a.0.cmp(b.0)));
        rows
    }

    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(day: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(day * 86_400, 0).unwrap()
    }

    #[test]
    fn blacklists_after_three_permanent_strikes() {
        let mut bl = BlacklistState::default();
        for _ in 0..2 {
            bl.record_outcome(
                "bloomberg.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 403"),
                t(0),
            );
        }
        assert!(
            !bl.is_blocked("bloomberg.com", t(0)),
            "2 strikes is not enough"
        );
        bl.record_outcome(
            "bloomberg.com",
            FetchOutcomeClass::PermanentBlock,
            Some("http status 403"),
            t(0),
        );
        assert!(
            bl.is_blocked("bloomberg.com", t(0)),
            "3rd strike blacklists"
        );
    }

    #[test]
    fn transient_failures_never_blacklist() {
        let mut bl = BlacklistState::default();
        for _ in 0..5 {
            bl.record_outcome(
                "thecentersquare.com",
                FetchOutcomeClass::Transient,
                Some("http status 429"),
                t(0),
            );
        }
        assert!(!bl.is_blocked("thecentersquare.com", t(0)));
    }

    #[test]
    fn cooldown_expires_then_allows_probe() {
        let mut bl = BlacklistState::default();
        for _ in 0..3 {
            bl.record_outcome(
                "wsj.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 401"),
                t(0),
            );
        }
        assert!(
            bl.is_blocked("wsj.com", t(6)),
            "still cooling at day 6 (7-day window)"
        );
        assert!(
            !bl.is_blocked("wsj.com", t(8)),
            "probe allowed after 7 days"
        );
    }

    #[test]
    fn success_clears_blacklist() {
        let mut bl = BlacklistState::default();
        for _ in 0..3 {
            bl.record_outcome(
                "wsj.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 401"),
                t(0),
            );
        }
        bl.record_outcome("wsj.com", FetchOutcomeClass::Success, None, t(8));
        assert!(!bl.is_blocked("wsj.com", t(8)));
    }

    #[test]
    fn repeated_probe_failure_extends_cooldown() {
        let mut bl = BlacklistState::default();
        for _ in 0..3 {
            bl.record_outcome(
                "wsj.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 401"),
                t(0),
            );
        }
        // probe after first cooldown still blocked -> second arming = 14 days
        bl.record_outcome(
            "wsj.com",
            FetchOutcomeClass::PermanentBlock,
            Some("http status 401"),
            t(8),
        );
        assert!(
            bl.is_blocked("wsj.com", t(20)),
            "second cooldown is 14 days"
        );
    }

    #[test]
    fn simultaneous_failures_do_not_escalate_cooldown() {
        // Several in-flight jobs for the same domain all fail at the same instant
        // after the 3rd strike. This must NOT escalate 7 -> 14 -> 28 days: the
        // cooldown only arms on the transition into the blocked state.
        let mut bl = BlacklistState::default();
        for _ in 0..5 {
            bl.record_outcome(
                "bloomberg.com",
                FetchOutcomeClass::PermanentBlock,
                Some("http status 403"),
                t(0),
            );
        }
        // First (and only) arming is 7 days: still blocked at day 6, free at day 8.
        assert!(
            bl.is_blocked("bloomberg.com", t(6)),
            "first cooldown is 7 days"
        );
        assert!(
            !bl.is_blocked("bloomberg.com", t(8)),
            "not a 14/28-day cooldown"
        );
    }

    #[test]
    fn record_outcome_reports_change() {
        let mut bl = BlacklistState::default();
        assert!(bl.record_outcome("a.com", FetchOutcomeClass::PermanentBlock, None, t(0)));
        assert!(!bl.record_outcome("a.com", FetchOutcomeClass::Transient, None, t(0)));
        assert!(!bl.record_outcome("a.com", FetchOutcomeClass::Ignored, None, t(0)));
        // Success on an active record reports a change; a no-op success does not.
        assert!(bl.record_outcome("a.com", FetchOutcomeClass::Success, None, t(0)));
        assert!(!bl.record_outcome("a.com", FetchOutcomeClass::Success, None, t(0)));
    }

    #[test]
    fn success_on_inactive_record_reports_change_when_timestamp_advances() {
        let mut bl = BlacklistState::default();
        // Establish and then clear an active record at t(0).
        for _ in 0..3 {
            bl.record_outcome("a.com", FetchOutcomeClass::PermanentBlock, None, t(0));
        }
        bl.record_outcome("a.com", FetchOutcomeClass::Success, None, t(0));
        // A later success advances last_outcome_at — that is a real change.
        assert!(bl.record_outcome("a.com", FetchOutcomeClass::Success, None, t(1)));
        // Same timestamp again: nothing changes.
        assert!(!bl.record_outcome("a.com", FetchOutcomeClass::Success, None, t(1)));
    }

    #[test]
    fn rows_sorted_by_strikes_desc() {
        let mut bl = BlacklistState::default();
        bl.record_outcome("a.com", FetchOutcomeClass::PermanentBlock, None, t(0));
        bl.record_outcome("b.com", FetchOutcomeClass::PermanentBlock, None, t(0));
        bl.record_outcome("b.com", FetchOutcomeClass::PermanentBlock, None, t(0));
        let rows = bl.rows();
        assert_eq!(rows[0].0, "b.com");
    }
}
