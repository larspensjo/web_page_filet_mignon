use crate::FailureKind;

/// Session-level quota configuration.
#[derive(Debug, Clone)]
pub struct SessionQuotas {
    pub max_urls_per_session: Option<usize>,
    pub max_bytes_per_session: Option<u64>,
    pub max_total_tokens_per_session: Option<u64>,
}

impl Default for SessionQuotas {
    fn default() -> Self {
        Self {
            max_urls_per_session: Some(500),
            max_bytes_per_session: Some(500 * 1024 * 1024),
            max_total_tokens_per_session: Some(2_000_000),
        }
    }
}

/// Tracks session-level resource consumption.
pub struct QuotaTracker {
    quotas: SessionQuotas,
    urls: usize,
    bytes: u64,
    tokens: u64,
}

impl QuotaTracker {
    pub fn new(quotas: SessionQuotas) -> Self {
        Self {
            quotas,
            urls: 0,
            bytes: 0,
            tokens: 0,
        }
    }

    /// Check if a new URL can be started.
    pub fn check_url(&mut self) -> Result<(), FailureKind> {
        if let Some(limit) = self.quotas.max_urls_per_session {
            if self.urls >= limit {
                return Err(FailureKind::QuotaExceeded {
                    description: format!("max urls per session ({limit}) reached"),
                });
            }
        }
        self.urls += 1;
        Ok(())
    }

    /// Record tokens/bytes after a job finishes.
    pub fn record_job(&mut self, bytes: u64, tokens: u64) {
        self.bytes = self.bytes.saturating_add(bytes);
        self.tokens = self.tokens.saturating_add(tokens);
    }

    /// Expose current usage totals for testing.
    pub fn totals(&self) -> (usize, u64, u64) {
        (self.urls, self.bytes, self.tokens)
    }
}
