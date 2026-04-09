use super::AppState;

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
