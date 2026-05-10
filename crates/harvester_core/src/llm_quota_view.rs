use crate::SourcePollStat;

pub const WARNING_PERCENT: u8 = 70;
pub const DANGER_PERCENT: u8 = 90;
pub const POLL_WARNING_REMAINING_PERCENT: u8 = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmQuotaLimits {
    pub max_calls_per_session: Option<u64>,
    pub max_input_tokens_per_session: Option<u64>,
    pub max_output_tokens_per_session: Option<u64>,
    pub max_cost_microdollars_per_session: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LlmQuotaUsage {
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_microdollars: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LlmQuotaState {
    pub limits: Option<LlmQuotaLimits>,
    pub usage: LlmQuotaUsage,
    pub ai_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmQuotaView {
    pub label: String,
    pub used: u64,
    pub limit: Option<u64>,
    pub percent: Option<u8>,
    pub severity: LlmQuotaSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmQuotaSeverity {
    Normal,
    Warning,
    Danger,
    Exhausted,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollQuotaWarning {
    pub severity: LlmQuotaSeverity,
    pub estimated_triage_calls: u64,
    pub remaining_calls: u64,
    pub max_calls: u64,
}

pub fn build_llm_quota_view(quota: &LlmQuotaState) -> LlmQuotaView {
    if !quota.ai_available {
        return LlmQuotaView {
            label: "LLM calls unavailable".to_string(),
            used: quota.usage.calls,
            limit: None,
            percent: None,
            severity: LlmQuotaSeverity::Unavailable,
        };
    }

    let used = quota.usage.calls;
    let limit = quota
        .limits
        .as_ref()
        .and_then(|limits| limits.max_calls_per_session);
    let Some(limit_value) = limit else {
        return LlmQuotaView {
            label: format!("LLM calls {used} / unlimited"),
            used,
            limit: None,
            percent: None,
            severity: LlmQuotaSeverity::Normal,
        };
    };

    let percent = usage_percent(used, limit_value);
    let severity = if used >= limit_value {
        LlmQuotaSeverity::Exhausted
    } else if percent >= DANGER_PERCENT {
        LlmQuotaSeverity::Danger
    } else if percent >= WARNING_PERCENT {
        LlmQuotaSeverity::Warning
    } else {
        LlmQuotaSeverity::Normal
    };

    LlmQuotaView {
        label: format!("LLM calls {used} / {limit_value}"),
        used,
        limit,
        percent: Some(percent),
        severity,
    }
}

pub fn build_poll_quota_warning(
    stats: &[SourcePollStat],
    quota: &LlmQuotaState,
) -> Option<PollQuotaWarning> {
    if !quota.ai_available {
        return None;
    }
    let estimated_triage_calls: u64 = stats.iter().map(|stat| stat.emitted as u64).sum::<u64>();
    if estimated_triage_calls == 0 {
        return None;
    }
    let max_calls = quota
        .limits
        .as_ref()
        .and_then(|limits| limits.max_calls_per_session)?;
    let remaining_calls = max_calls.saturating_sub(quota.usage.calls);
    let current_percent = usage_percent(quota.usage.calls, max_calls);

    let severity = if estimated_triage_calls > remaining_calls {
        LlmQuotaSeverity::Danger
    } else if estimated_triage_calls.saturating_mul(100)
        >= remaining_calls.saturating_mul(u64::from(POLL_WARNING_REMAINING_PERCENT))
        || current_percent >= DANGER_PERCENT
    {
        LlmQuotaSeverity::Warning
    } else {
        return None;
    };

    Some(PollQuotaWarning {
        severity,
        estimated_triage_calls,
        remaining_calls,
        max_calls,
    })
}

fn usage_percent(used: u64, limit: u64) -> u8 {
    if limit == 0 {
        return 100;
    }
    let percent = used.saturating_mul(100) / limit;
    percent.min(100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::{SourceId, SourceKind};

    fn quota(used: u64, limit: Option<u64>) -> LlmQuotaState {
        LlmQuotaState {
            limits: Some(LlmQuotaLimits {
                max_calls_per_session: limit,
                max_input_tokens_per_session: None,
                max_output_tokens_per_session: None,
                max_cost_microdollars_per_session: None,
            }),
            usage: LlmQuotaUsage {
                calls: used,
                ..Default::default()
            },
            ai_available: true,
        }
    }

    fn stat(emitted: usize) -> SourcePollStat {
        SourcePollStat {
            source_id: SourceId::new("source").expect("valid"),
            kind: SourceKind::Rss,
            parsed: emitted,
            dedup_filtered: 0,
            emitted,
        }
    }

    #[test]
    fn quota_view_uses_expected_thresholds() {
        assert_eq!(
            build_llm_quota_view(&quota(69, Some(100))).severity,
            LlmQuotaSeverity::Normal
        );
        assert_eq!(
            build_llm_quota_view(&quota(70, Some(100))).severity,
            LlmQuotaSeverity::Warning
        );
        assert_eq!(
            build_llm_quota_view(&quota(90, Some(100))).severity,
            LlmQuotaSeverity::Danger
        );
        assert_eq!(
            build_llm_quota_view(&quota(100, Some(100))).severity,
            LlmQuotaSeverity::Exhausted
        );
    }

    #[test]
    fn unlimited_quota_is_neutral() {
        let view = build_llm_quota_view(&quota(37, None));
        assert_eq!(view.label, "LLM calls 37 / unlimited");
        assert_eq!(view.limit, None);
        assert_eq!(view.severity, LlmQuotaSeverity::Normal);
    }

    #[test]
    fn unavailable_quota_hides_numeric_capacity() {
        let view = build_llm_quota_view(&LlmQuotaState::default());
        assert_eq!(view.label, "LLM calls unavailable");
        assert_eq!(view.limit, None);
        assert_eq!(view.severity, LlmQuotaSeverity::Unavailable);
    }

    #[test]
    fn poll_warning_rules_follow_remaining_quota() {
        assert!(build_poll_quota_warning(&[stat(10)], &quota(0, None)).is_none());
        assert!(build_poll_quota_warning(&[stat(10)], &quota(0, Some(100))).is_none());
        assert_eq!(
            build_poll_quota_warning(&[stat(80)], &quota(0, Some(100)))
                .expect("warning")
                .severity,
            LlmQuotaSeverity::Warning
        );
        assert_eq!(
            build_poll_quota_warning(&[stat(101)], &quota(0, Some(100)))
                .expect("danger")
                .severity,
            LlmQuotaSeverity::Danger
        );
    }
}
