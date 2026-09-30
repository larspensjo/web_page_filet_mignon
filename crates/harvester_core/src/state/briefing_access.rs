#[cfg(test)]
use super::PendingBriefingCheckpointSaveSnapshot;
use super::{AppState, PendingBriefingCheckpointSave, CHECKPOINT_SAVING_STATUS_MESSAGE};
use crate::briefing::BriefingSession;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

impl AppState {
    pub(crate) fn briefing_triage_policy(&self) -> crate::briefing::TriageSelectionPolicy {
        crate::briefing::TriageSelectionPolicy {
            cutoff_exclusive: 1,
            exclude_untriaged: true,
        }
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
        self.note_unfinished_inputs_changed();
        &mut self.briefing
    }

    pub(crate) fn set_briefing(&mut self, briefing: BriefingSession) {
        self.note_unfinished_inputs_changed();
        self.briefing = briefing;
        self.dirty = true;
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

    pub(crate) fn set_briefing_checkpoint_status_message(&mut self, message: Option<String>) {
        if self.briefing_checkpoint_status_message != message {
            self.briefing_checkpoint_status_message = message;
            self.mark_dirty();
        }
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
