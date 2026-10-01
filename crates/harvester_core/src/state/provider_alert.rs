use super::AppState;

/// Consecutive provider rate-limit failures tolerated before a run stops.
pub(crate) const RATE_LIMIT_ABORT_THRESHOLD: u32 = 3;

impl AppState {
    pub(crate) fn note_provider_rate_limited(&mut self) -> bool {
        self.consecutive_rate_limit_failures =
            self.consecutive_rate_limit_failures.saturating_add(1);
        self.consecutive_rate_limit_failures >= RATE_LIMIT_ABORT_THRESHOLD
    }

    pub(crate) fn note_owned_llm_success(&mut self) {
        self.consecutive_rate_limit_failures = 0;
    }

    pub(crate) fn reset_provider_rate_limit_failures(&mut self) {
        self.consecutive_rate_limit_failures = 0;
    }
}

#[cfg(test)]
mod tests {
    use crate::state::AppState;

    #[test]
    fn rate_limit_threshold_stops_after_three_consecutive_failures() {
        let mut state = AppState::default();
        assert!(!state.note_provider_rate_limited());
        assert!(!state.note_provider_rate_limited());
        assert!(state.note_provider_rate_limited());
    }

    #[test]
    fn owned_success_resets_consecutive_counter() {
        let mut state = AppState::default();
        assert!(!state.note_provider_rate_limited());
        assert!(!state.note_provider_rate_limited());
        state.note_owned_llm_success();
        assert!(!state.note_provider_rate_limited());
    }

    #[test]
    fn reset_provider_rate_limit_failures_resets_counter() {
        let mut state = AppState::default();
        assert!(!state.note_provider_rate_limited());
        state.reset_provider_rate_limit_failures();
        assert!(!state.note_provider_rate_limited());
        assert!(!state.note_provider_rate_limited());
        assert!(state.note_provider_rate_limited());
    }
}
