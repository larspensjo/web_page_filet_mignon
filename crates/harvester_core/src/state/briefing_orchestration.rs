use super::{AppState, PendingBriefingCheckpointSave, CHECKPOINT_SAVING_STATUS_MESSAGE};
use crate::briefing::{BriefingHistoryEntry, BriefingSession};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[cfg(test)]
use super::PendingBriefingCheckpointSaveSnapshot;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct BriefingOrchestration {
    requested: bool,
    skip_aggregate_briefing: bool,
    priority_cutoff_exclusive: u8,
    prereq_articles: Option<Vec<crate::briefing::LoadedArticle>>,
}

impl Default for BriefingOrchestration {
    fn default() -> Self {
        Self {
            requested: false,
            skip_aggregate_briefing: false,
            priority_cutoff_exclusive: 1,
            prereq_articles: None,
        }
    }
}

impl BriefingOrchestration {
    fn request(&mut self, skip_aggregate_briefing: bool) {
        self.requested = true;
        self.skip_aggregate_briefing = skip_aggregate_briefing;
    }

    fn store_prereq(&mut self, articles: Vec<crate::briefing::LoadedArticle>) {
        self.prereq_articles = Some(articles);
    }

    fn take_prereq(&mut self) -> Option<Vec<crate::briefing::LoadedArticle>> {
        self.prereq_articles.take()
    }

    fn clear(&mut self) {
        self.requested = false;
        self.skip_aggregate_briefing = false;
        self.prereq_articles = None;
    }

    pub(super) fn is_requested(&self) -> bool {
        self.requested
    }

    fn policy(&self) -> crate::briefing::TriageSelectionPolicy {
        crate::briefing::TriageSelectionPolicy {
            cutoff_exclusive: self.priority_cutoff_exclusive,
            exclude_untriaged: true,
        }
    }

    fn clear_request(&mut self) {
        self.requested = false;
    }

    fn skip_aggregate_briefing(&self) -> bool {
        self.skip_aggregate_briefing
    }
}

impl AppState {
    pub(crate) fn briefing_orchestration_requested(&self) -> bool {
        self.briefing_orchestration.is_requested()
    }

    pub(crate) fn request_briefing_orchestration(&mut self) {
        self.briefing_orchestration.request(false);
    }

    pub(crate) fn request_summary_preparation(&mut self) {
        self.briefing_orchestration.request(true);
    }

    pub(crate) fn store_briefing_prereq_articles(
        &mut self,
        articles: Vec<crate::briefing::LoadedArticle>,
    ) {
        self.briefing_orchestration.store_prereq(articles);
    }

    pub(crate) fn take_briefing_prereq_articles(
        &mut self,
    ) -> Option<Vec<crate::briefing::LoadedArticle>> {
        self.briefing_orchestration.take_prereq()
    }

    pub(crate) fn clear_briefing_orchestration(&mut self) {
        self.briefing_orchestration.clear()
    }

    pub(crate) fn clear_briefing_orchestration_request(&mut self) {
        self.briefing_orchestration.clear_request();
    }

    pub(crate) fn briefing_triage_policy(&self) -> crate::briefing::TriageSelectionPolicy {
        self.briefing_orchestration.policy()
    }

    pub(crate) fn briefing_orchestration_skip_aggregate(&self) -> bool {
        self.briefing_orchestration.skip_aggregate_briefing()
    }
}

impl AppState {
    pub(crate) fn allocate_next_briefing_checkpoint_save_id(&mut self) -> u64 {
        let save_id = self.next_briefing_checkpoint_save_id;
        self.next_briefing_checkpoint_save_id =
            self.next_briefing_checkpoint_save_id.saturating_add(1);
        save_id
    }

    pub(crate) fn briefing(&self) -> &BriefingSession {
        &self.briefing
    }

    pub(crate) fn briefing_mut(&mut self) -> &mut BriefingSession {
        &mut self.briefing
    }

    pub(crate) fn set_briefing(&mut self, briefing: BriefingSession) {
        self.briefing = briefing;
        self.dirty = true;
    }

    pub fn briefing_history(&self) -> &[BriefingHistoryEntry] {
        &self.briefing_history
    }

    /// Prepends `entry` (newest first) and caps the list at 3 entries.
    pub fn push_briefing_history(&mut self, entry: BriefingHistoryEntry) {
        self.briefing_history.insert(0, entry);
        self.briefing_history.truncate(3);
    }

    pub fn set_briefing_history(&mut self, entries: Vec<BriefingHistoryEntry>) {
        self.briefing_history = entries;
    }

    pub fn briefing_since_utc(&self) -> Option<DateTime<Utc>> {
        self.briefing_since_utc
    }

    pub(crate) fn set_briefing_since_utc(&mut self, v: Option<DateTime<Utc>>) {
        self.briefing_since_utc = v;
    }

    #[cfg(test)]
    pub(crate) fn pending_briefing_checkpoint_save(
        &self,
    ) -> Option<PendingBriefingCheckpointSaveSnapshot> {
        self.pending_briefing_checkpoint_save
            .as_ref()
            .map(|pending| PendingBriefingCheckpointSaveSnapshot {
                save_id: pending.save_id,
                previous_since_utc: pending.previous_since_utc,
                pending_since_utc: pending.pending_since_utc,
            })
    }

    pub(crate) fn begin_briefing_checkpoint_save(
        &mut self,
        pending_since_utc: Option<DateTime<Utc>>,
    ) -> u64 {
        let save_id = self.allocate_next_briefing_checkpoint_save_id();
        // A newer user-driven checkpoint change replaces any older pending save.
        // Matching is done by save_id, so late acks for older requests are dropped.
        self.pending_briefing_checkpoint_save = Some(PendingBriefingCheckpointSave {
            save_id,
            previous_since_utc: self.briefing_since_utc,
            pending_since_utc,
        });
        self.briefing_since_utc = pending_since_utc;
        self.briefing_checkpoint_status_message =
            Some(CHECKPOINT_SAVING_STATUS_MESSAGE.to_string());
        save_id
    }

    pub(crate) fn finish_briefing_checkpoint_save_success(&mut self, save_id: u64) -> bool {
        match self.pending_briefing_checkpoint_save.as_ref() {
            Some(pending) if pending.save_id == save_id => {
                self.pending_briefing_checkpoint_save = None;
                self.briefing_checkpoint_status_message = None;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn finish_briefing_checkpoint_save_failure(
        &mut self,
        save_id: u64,
        reason: &str,
    ) -> bool {
        match self.pending_briefing_checkpoint_save.as_ref() {
            Some(pending) if pending.save_id == save_id => {
                self.briefing_since_utc = pending.previous_since_utc;
                self.pending_briefing_checkpoint_save = None;
                self.briefing_checkpoint_status_message =
                    Some(format!("Checkpoint save failed: {reason}"));
                true
            }
            _ => false,
        }
    }

    pub(crate) fn clear_briefing_checkpoint_save_tracking(&mut self) {
        self.pending_briefing_checkpoint_save = None;
        self.briefing_checkpoint_status_message = None;
    }

    pub fn briefing_checkpoint_status_message(&self) -> Option<&str> {
        self.briefing_checkpoint_status_message.as_deref()
    }

    /// Backfills `fetched_utc` on jobs that have it as `None`, keyed by URL.
    /// Used to recover timestamps for jobs restored from pre-feature persisted state.
    pub(crate) fn backfill_jobs_fetched_utc(
        &mut self,
        url_to_fetched: &HashMap<String, DateTime<Utc>>,
    ) {
        for job in self.jobs.values_mut() {
            if job.fetched_utc.is_none() {
                if let Some(&dt) = url_to_fetched.get(&job.url) {
                    job.fetched_utc = Some(dt);
                }
            }
        }
    }
}
