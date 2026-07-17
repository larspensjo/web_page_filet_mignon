use super::AppState;
use crate::InlineWarningView;

/// Consecutive provider rate-limit failures tolerated before a run stops.
pub(crate) const RATE_LIMIT_ABORT_THRESHOLD: u32 = 3;

/// A run-stopping provider problem the user must resolve or wait out.
/// Raised only by completions owned by the active run; cleared when the
/// user starts the next LLM run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAlert {
    OutOfCredits { detail: String },
    RateLimited,
}

impl AppState {
    pub(crate) fn provider_alert(&self) -> Option<&ProviderAlert> {
        self.provider_alert.as_ref()
    }

    pub(crate) fn note_provider_rate_limited(&mut self) -> bool {
        self.consecutive_rate_limit_failures =
            self.consecutive_rate_limit_failures.saturating_add(1);
        if self.consecutive_rate_limit_failures >= RATE_LIMIT_ABORT_THRESHOLD {
            self.provider_alert = Some(ProviderAlert::RateLimited);
            self.mark_dirty();
            return true;
        }
        false
    }

    pub(crate) fn note_provider_out_of_credits(&mut self, detail: String) {
        self.provider_alert = Some(ProviderAlert::OutOfCredits { detail });
        self.mark_dirty();
    }

    pub(crate) fn note_owned_llm_success(&mut self) {
        self.consecutive_rate_limit_failures = 0;
    }

    pub(crate) fn clear_provider_alert(&mut self) {
        if self.provider_alert.is_some() {
            self.mark_dirty();
        }
        self.provider_alert = None;
        self.consecutive_rate_limit_failures = 0;
    }

    pub(super) fn provider_alert_banner(&self) -> Option<InlineWarningView> {
        self.provider_alert().map(|alert| match alert {
            ProviderAlert::OutOfCredits { detail } => InlineWarningView {
                title: "LLM run stopped: OpenAI account out of credits".to_string(),
                body: format!(
                    "{detail}. Refill credits at platform.openai.com, then start the run again. \
                     Queued articles were skipped; calls already in flight may still finish."
                ),
            },
            ProviderAlert::RateLimited => InlineWarningView {
                title: "LLM run stopped: provider rate limiting".to_string(),
                body: format!(
                    "OpenAI refused {RATE_LIMIT_ABORT_THRESHOLD} consecutive requests with a \
                     rate-limit response, so the queued articles were skipped. Calls already in \
                     flight may still finish. Wait a moment, then start the run again."
                ),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    #[test]
    fn rate_limit_threshold_raises_alert_after_three_consecutive_failures() {
        let mut state = AppState::default();
        assert!(!state.note_provider_rate_limited());
        assert!(!state.note_provider_rate_limited());
        assert!(state.note_provider_rate_limited());
        assert!(matches!(
            state.provider_alert(),
            Some(ProviderAlert::RateLimited)
        ));
    }

    #[test]
    fn owned_success_resets_consecutive_counter() {
        let mut state = AppState::default();
        assert!(!state.note_provider_rate_limited());
        assert!(!state.note_provider_rate_limited());
        state.note_owned_llm_success();
        assert!(!state.note_provider_rate_limited());
        assert!(state.provider_alert().is_none());
    }

    #[test]
    fn out_of_credits_raises_alert_immediately() {
        let mut state = AppState::default();
        state.note_provider_out_of_credits("provider quota exhausted: billing".to_string());
        assert!(matches!(
            state.provider_alert(),
            Some(ProviderAlert::OutOfCredits { .. })
        ));
    }

    #[test]
    fn clear_provider_alert_resets_alert_and_counter() {
        let mut state = AppState::default();
        state.note_provider_out_of_credits("x".to_string());
        assert!(!state.note_provider_rate_limited());
        state.clear_provider_alert();
        assert!(state.provider_alert().is_none());
        assert!(!state.note_provider_rate_limited());
        assert!(!state.note_provider_rate_limited());
        assert!(state.note_provider_rate_limited());
    }

    #[test]
    fn banner_text_for_out_of_credits_mentions_credits_and_detail() {
        let mut state = AppState::default();
        state.note_provider_out_of_credits("provider quota exhausted: billing".to_string());
        let banner = state.provider_alert_banner().expect("banner");
        assert!(banner.body.contains("credits"));
        assert!(banner.body.contains("provider quota exhausted: billing"));
    }

    #[test]
    fn rate_limited_banner_describes_best_effort_stop() {
        let mut state = AppState::default();
        for _ in 0..3 {
            state.note_provider_rate_limited();
        }
        let banner = state.provider_alert_banner().expect("banner");
        assert!(banner.body.contains("queued"));
    }
}
