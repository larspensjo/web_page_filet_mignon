use crate::FailureKind;

/// Download caps that apply to one pipeline run. The host resets them when a
/// new run begins, so a long-lived app does not accumulate usage across runs.
#[derive(Debug, Clone)]
pub struct RunQuotas {
    pub max_urls_per_run: Option<usize>,
}

impl Default for RunQuotas {
    fn default() -> Self {
        Self {
            max_urls_per_run: Some(500),
        }
    }
}

/// Tracks download usage for the current run.
pub struct QuotaTracker {
    quotas: RunQuotas,
    urls: usize,
}

impl QuotaTracker {
    pub fn new(quotas: RunQuotas) -> Self {
        Self { quotas, urls: 0 }
    }

    /// Check if a new URL can be started.
    pub fn check_url(&mut self) -> Result<(), FailureKind> {
        if let Some(limit) = self.quotas.max_urls_per_run {
            if self.urls >= limit {
                return Err(FailureKind::QuotaExceeded {
                    description: format!("max urls per run ({limit}) reached"),
                });
            }
        }
        self.urls += 1;
        Ok(())
    }

    /// Start counting a new run from zero.
    pub fn reset(&mut self) {
        self.urls = 0;
    }

    /// URLs started in the current run.
    pub fn urls_started(&self) -> usize {
        self.urls
    }
}
