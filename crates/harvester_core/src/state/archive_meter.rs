//! Read-only projection over saved current-key results, with no corpus fallback.
use super::{AppState, StartupReadinessStatus};
use crate::{ArchiveMeterStatus, ArchiveMeterView, ARCHIVE_ARTICLE_TARGET};
impl AppState {
    pub fn archive_meter_view(&self) -> ArchiveMeterView {
        let status = match self.startup_readiness() {
            StartupReadinessStatus::Pending => ArchiveMeterStatus::Loading,
            StartupReadinessStatus::Unavailable => ArchiveMeterStatus::Unavailable,
            StartupReadinessStatus::Ready => {
                if self
                    .saved_results_entries()
                    .any(|entry| entry.in_window && entry.actionable && entry.signal.is_some())
                {
                    ArchiveMeterStatus::Scored
                } else {
                    ArchiveMeterStatus::NotScoredYet
                }
            }
        };
        let mut meter = ArchiveMeterView {
            selected_count: 0,
            target: ARCHIVE_ARTICLE_TARGET,
            token_estimate: 0,
            unsettled_count: 0,
            status,
        };
        if matches!(
            status,
            ArchiveMeterStatus::Loading | ArchiveMeterStatus::Unavailable
        ) {
            return meter;
        }
        let selection = self.signal_candidate_selection();
        meter.selected_count = selection.selected_urls.len();
        meter.token_estimate = self
            .archive_token_estimates(&selection.selected_urls)
            .summary_tokens;
        meter.unsettled_count = self
            .saved_results_entries()
            .filter(|entry| entry.is_unsettled(self.briefing_triage_policy()))
            .count();
        meter
    }
}
