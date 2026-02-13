use crate::types::FailureKind;

/// Session-level caps for LLM usage.
#[derive(Clone)]
pub struct LlmQuotas {
    pub max_calls_per_session: Option<u32>,
    pub max_input_tokens_per_session: Option<u64>,
    pub max_output_tokens_per_session: Option<u64>,
    pub max_cost_microdollars_per_session: Option<u64>,
}

impl Default for LlmQuotas {
    fn default() -> Self {
        Self {
            max_calls_per_session: Some(100),
            max_input_tokens_per_session: Some(2_000_000),
            max_output_tokens_per_session: Some(500_000),
            max_cost_microdollars_per_session: Some(5_000_000),
        }
    }
}

/// Tracks cumulative usage across an LLM session.
pub struct LlmQuotaTracker {
    quotas: LlmQuotas,
    total_calls: u64,
    total_input_tokens: u64,
    total_output_tokens: u64,
    total_cost_microdollars: u64,
}

impl LlmQuotaTracker {
    pub fn new(quotas: LlmQuotas) -> Self {
        Self {
            quotas,
            total_calls: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cost_microdollars: 0,
        }
    }

    /// Pre-call reservation: atomically check and reserve a call slot.
    /// Returns `Ok(())` if the call is allowed and increments the call counter.
    /// Returns `Err` if the call count limit is already reached.
    pub fn reserve_call(&mut self) -> Result<(), FailureKind> {
        if let Some(max_calls) = self.quotas.max_calls_per_session {
            if self.total_calls >= max_calls as u64 {
                return Err(FailureKind::QuotaExceeded {
                    description: "too many LLM calls this session".into(),
                });
            }
        }
        self.total_calls = self.total_calls.saturating_add(1);
        Ok(())
    }

    /// Release a previously reserved call slot. Used when a call fails after pre-reservation.
    pub fn release_call(&mut self) {
        self.total_calls = self.total_calls.saturating_sub(1);
    }

    /// Post-call: record actual token usage and cost, checking token/cost limits.
    /// Returns `Err` if token or cost limits are exceeded (caller should emit QuotaExhausted).
    pub fn record_call_usage(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        cost_microdollars: u64,
    ) -> Result<(), FailureKind> {
        if let Some(max_input) = self.quotas.max_input_tokens_per_session {
            if self.total_input_tokens + input_tokens > max_input {
                return Err(FailureKind::QuotaExceeded {
                    description: "too many LLM input tokens this session".into(),
                });
            }
        }

        if let Some(max_output) = self.quotas.max_output_tokens_per_session {
            if self.total_output_tokens + output_tokens > max_output {
                return Err(FailureKind::QuotaExceeded {
                    description: "too many LLM output tokens this session".into(),
                });
            }
        }

        if let Some(max_cost) = self.quotas.max_cost_microdollars_per_session {
            if self.total_cost_microdollars + cost_microdollars > max_cost {
                return Err(FailureKind::QuotaExceeded {
                    description: "LLM cost limit exceeded this session".into(),
                });
            }
        }

        self.total_input_tokens = self.total_input_tokens.saturating_add(input_tokens);
        self.total_output_tokens = self.total_output_tokens.saturating_add(output_tokens);
        self.total_cost_microdollars = self
            .total_cost_microdollars
            .saturating_add(cost_microdollars);
        Ok(())
    }

    /// Legacy combined check (without reservation). Useful for testing and inspection.
    pub fn check_call(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cost_microdollars: u64,
    ) -> Result<(), FailureKind> {
        if let Some(max_calls) = self.quotas.max_calls_per_session {
            if self.total_calls >= max_calls as u64 {
                return Err(FailureKind::QuotaExceeded {
                    description: "too many LLM calls this session".into(),
                });
            }
        }
        if let Some(max_input) = self.quotas.max_input_tokens_per_session {
            if self.total_input_tokens + input_tokens > max_input {
                return Err(FailureKind::QuotaExceeded {
                    description: "too many LLM input tokens this session".into(),
                });
            }
        }
        if let Some(max_output) = self.quotas.max_output_tokens_per_session {
            if self.total_output_tokens + output_tokens > max_output {
                return Err(FailureKind::QuotaExceeded {
                    description: "too many LLM output tokens this session".into(),
                });
            }
        }
        if let Some(max_cost) = self.quotas.max_cost_microdollars_per_session {
            if self.total_cost_microdollars + cost_microdollars > max_cost {
                return Err(FailureKind::QuotaExceeded {
                    description: "LLM cost limit exceeded this session".into(),
                });
            }
        }
        Ok(())
    }

    pub fn record_call(&mut self, input_tokens: u64, output_tokens: u64, cost_microdollars: u64) {
        self.total_calls = self.total_calls.saturating_add(1);
        self.total_input_tokens = self.total_input_tokens.saturating_add(input_tokens);
        self.total_output_tokens = self.total_output_tokens.saturating_add(output_tokens);
        self.total_cost_microdollars = self
            .total_cost_microdollars
            .saturating_add(cost_microdollars);
    }

    pub fn totals(&self) -> LlmUsageTotals {
        LlmUsageTotals {
            calls: self.total_calls,
            input_tokens: self.total_input_tokens,
            output_tokens: self.total_output_tokens,
            cost_microdollars: self.total_cost_microdollars,
        }
    }
}

pub struct LlmUsageTotals {
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_microdollars: u64,
}
