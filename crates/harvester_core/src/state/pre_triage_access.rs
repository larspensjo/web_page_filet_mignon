use super::{AppState, JobId, PreTriageActionability, PreTriageLoadContext, PreTriageLoadProgress};
use crate::pre_triage_coordinator::PreTriageRefreshReason;
use crate::pre_triage_filter::{
    ArticleFilterKey, ManualDecision, PreTriagePhase, PreTriageSession,
};
use crate::triage::TriageSession;
use std::collections::HashMap;

impl AppState {
    pub(crate) fn triage(&self) -> &TriageSession {
        &self.triage
    }

    pub(crate) fn triage_mut(&mut self) -> &mut TriageSession {
        &mut self.triage
    }

    pub(crate) fn set_triage(&mut self, triage: TriageSession) {
        self.triage = triage;
        self.dirty = true;
    }

    pub(crate) fn pre_triage(&self) -> &PreTriageSession {
        &self.pre_triage
    }

    pub fn pre_triage_actionability(&self) -> PreTriageActionability {
        match self.pre_triage.phase() {
            PreTriagePhase::LoadingArticles => PreTriageActionability::Loading,
            PreTriagePhase::ReadyToTriage => {
                if self.pre_triage.resolved_included_articles().is_empty() {
                    PreTriageActionability::Unavailable
                } else {
                    PreTriageActionability::Ready
                }
            }
            PreTriagePhase::Reviewing => {
                if self.pre_triage.resolved_included_articles().is_empty() {
                    PreTriageActionability::Unavailable
                } else {
                    PreTriageActionability::ReadyWithPendingReview
                }
            }
            PreTriagePhase::Idle | PreTriagePhase::Failed { .. } => {
                PreTriageActionability::Unavailable
            }
        }
    }

    pub fn can_start_triage_from_pre_triage(&self) -> bool {
        matches!(
            self.pre_triage_actionability(),
            PreTriageActionability::Ready | PreTriageActionability::ReadyWithPendingReview
        )
    }

    /// Consumes the pre-triage included articles for use in a triage session,
    /// resetting pre-triage to Idle. Returns `None` if pre-triage is not in an
    /// interactive phase or has no resolved articles. This is a one-way
    /// transition that ensures pre-triage cannot remain action-ready after its
    /// articles have been handed off.
    pub(crate) fn consume_interactive_pre_triage_articles_for_triage(
        &mut self,
    ) -> Option<Vec<crate::briefing::LoadedArticle>> {
        if !self.can_start_triage_from_pre_triage() {
            return None;
        }
        let articles = self.pre_triage.resolved_included_articles();
        if articles.is_empty() {
            return None;
        }
        self.pre_triage.reset();
        self.dirty = true;
        Some(articles)
    }

    pub(crate) fn set_pre_triage(&mut self, pre_triage: PreTriageSession) {
        if !matches!(pre_triage.phase(), PreTriagePhase::LoadingArticles) {
            self.pre_triage_load_context = None;
            self.pre_triage_load_progress = None;
        }
        self.pre_triage = pre_triage;
        self.dirty = true;
    }

    pub(crate) fn set_pre_triage_load_context(&mut self, reason: PreTriageRefreshReason) {
        self.pre_triage_load_context = Some(PreTriageLoadContext { reason });
        self.dirty = true;
    }

    pub(crate) fn set_pre_triage_load_progress(
        &mut self,
        request_id: u64,
        files_scanned: usize,
        files_total: usize,
    ) {
        let progress = PreTriageLoadProgress {
            request_id,
            files_scanned,
            files_total,
        };
        if self.pre_triage_load_progress != Some(progress) {
            self.pre_triage_load_progress = Some(progress);
            self.dirty = true;
        }
    }

    pub(crate) fn clear_pre_triage_load_progress(&mut self) {
        if self.pre_triage_load_progress.take().is_some() {
            self.dirty = true;
        }
    }

    pub(crate) fn pre_triage_load_progress(&self) -> Option<(usize, usize, u64)> {
        self.pre_triage_load_progress.map(
            |PreTriageLoadProgress {
                 request_id,
                 files_scanned,
                 files_total,
             }| { (files_scanned, files_total, request_id) },
        )
    }

    pub fn is_pre_triage_reviewing(&self) -> bool {
        self.pre_triage.is_interactive()
    }

    pub fn pre_triage_key_for_job(&self, job_id: JobId) -> Option<ArticleFilterKey> {
        self.pre_triage.key_for_job(job_id)
    }

    pub fn pre_triage_manual_overrides(&self) -> &HashMap<ArticleFilterKey, ManualDecision> {
        &self.pre_triage_manual_overrides
    }

    pub(crate) fn set_pre_triage_manual_overrides(
        &mut self,
        overrides: HashMap<ArticleFilterKey, ManualDecision>,
    ) {
        self.pre_triage_manual_overrides = overrides;
        self.pre_triage
            .apply_manual_overrides(&self.pre_triage_manual_overrides);
        self.dirty = true;
    }

    pub(crate) fn set_pre_triage_manual_decision(
        &mut self,
        key: ArticleFilterKey,
        decision: ManualDecision,
    ) -> bool {
        if self.pre_triage.set_manual_decision(&key, decision).is_err() {
            return false;
        }
        self.pre_triage_manual_overrides.insert(key, decision);
        self.dirty = true;
        true
    }

    pub(crate) fn clear_pre_triage_manual_overrides(&mut self) {
        self.pre_triage_manual_overrides.clear();
        self.pre_triage.clear_manual_decisions();
        self.dirty = true;
    }

    /// Allocate the next request ID for a pre-triage load.
    ///
    /// Only available in tests - used to inject a request ID into messages
    /// without driving the coordinator. In production, IDs are allocated
    /// exclusively by `PreTriageRefreshCoordinator`.
    #[cfg(test)]
    pub(crate) fn alloc_triage_request_id(&mut self) -> u64 {
        let id = self.next_triage_request_id;
        self.next_triage_request_id += 1;
        id
    }

    /// Record that a pre-triage load with the given request ID is in flight.
    pub(crate) fn set_triage_in_flight(&mut self, id: u64) {
        self.triage_in_flight_request_id = Some(id);
    }

    /// Clear the in-flight pre-triage load request (call on response or cancellation).
    pub(crate) fn clear_triage_in_flight(&mut self) {
        self.triage_in_flight_request_id = None;
    }

    /// Return the request ID of the currently in-flight pre-triage load, if any.
    pub fn triage_in_flight_request_id(&self) -> Option<u64> {
        self.triage_in_flight_request_id
    }

    pub(crate) fn request_pre_triage_refresh_evaluation(&mut self, triggered_by_job_done: bool) {
        self.pre_triage_refresh_eval_pending = true;
        if triggered_by_job_done {
            self.pre_triage_refresh_eval_job_done = true;
        }
    }

    pub fn take_pre_triage_refresh_evaluation_request(&mut self) -> Option<bool> {
        if !self.pre_triage_refresh_eval_pending {
            return None;
        }
        self.pre_triage_refresh_eval_pending = false;
        let triggered_by_job_done = self.pre_triage_refresh_eval_job_done;
        self.pre_triage_refresh_eval_job_done = false;
        Some(triggered_by_job_done)
    }
}
