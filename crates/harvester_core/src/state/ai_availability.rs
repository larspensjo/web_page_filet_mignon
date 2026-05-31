use super::{AiAvailability, AiUnavailableReason, AppState, PreTriageLoadContext};
use crate::pre_triage_coordinator::PreTriageRefreshReason;
use crate::pre_triage_filter::PreTriagePhase;
use crate::InlineWarningView;
use harvester_engine::llm::prompt::PromptId;

impl AppState {
    pub fn ai_availability(&self) -> &AiAvailability {
        &self.ai_availability
    }

    pub fn triage_ai_available(&self) -> bool {
        matches!(self.ai_availability, AiAvailability::Available)
    }

    pub fn briefing_ai_available(&self) -> bool {
        matches!(self.ai_availability, AiAvailability::Available)
    }

    pub(crate) fn set_ai_availability(&mut self, availability: AiAvailability) {
        self.llm_quota.ai_available = matches!(availability, AiAvailability::Available);
        self.ai_availability = availability;
    }

    pub(crate) fn reconcile_ai_availability_from_metadata(&mut self) {
        let triage_model_available = self.effective_models.contains_key(&PromptId::ArticleTriage);
        match (&self.ai_availability, triage_model_available) {
            (
                AiAvailability::Unavailable {
                    reason: AiUnavailableReason::MissingApiKey,
                },
                _,
            ) => {}
            (_, true) => {
                self.ai_availability = AiAvailability::Available;
                self.llm_quota.ai_available = true;
            }
            (_, false) => {
                self.ai_availability = AiAvailability::Unavailable {
                    reason: AiUnavailableReason::NoTriageModel,
                };
                self.llm_quota.ai_available = false;
            }
        }
    }

    pub(super) fn ai_unavailable_reason(&self) -> Option<AiUnavailableReason> {
        match self.ai_availability {
            AiAvailability::Available => None,
            AiAvailability::Unavailable { reason } => Some(reason),
        }
    }

    fn ai_unavailable_reason_text(&self) -> Option<&'static str> {
        match self.ai_unavailable_reason() {
            Some(AiUnavailableReason::MissingApiKey) => Some("OPENAI_API_KEY is not set"),
            Some(AiUnavailableReason::NoTriageModel) => Some("no triage model is available"),
            None => None,
        }
    }

    pub(super) fn ai_unavailable_message(&self) -> Option<String> {
        self.ai_unavailable_reason_text()
            .map(|reason| format!("AI features unavailable: {reason}"))
    }

    pub(super) fn ai_warning_banner(&self) -> Option<InlineWarningView> {
        matches!(
            self.ai_unavailable_reason(),
            Some(AiUnavailableReason::MissingApiKey)
        )
        .then(|| InlineWarningView {
            title: "AI features are disabled".to_string(),
            body: "Set OPENAI_API_KEY in the launch environment and restart to enable triage and briefing.".to_string(),
        })
    }

    pub(super) fn triage_blocked_reason(&self) -> Option<String> {
        if let Some(reason) = self.ai_unavailable_reason() {
            return Some(match reason {
                AiUnavailableReason::MissingApiKey => {
                    "AI setup is incomplete because OPENAI_API_KEY is not set".to_string()
                }
                AiUnavailableReason::NoTriageModel => "no triage model is available".to_string(),
            });
        }

        if matches!(self.pre_triage.phase(), PreTriagePhase::LoadingArticles) {
            return Some(match self.pre_triage_load_context {
                Some(PreTriageLoadContext {
                    reason: PreTriageRefreshReason::RestoreCompletedJobs,
                }) => "Triage is unavailable while startup prepares the article set".to_string(),
                _ => "Triage is unavailable while the article set is being prepared".to_string(),
            });
        }

        None
    }

    pub(super) fn briefing_blocked_reason(&self) -> Option<String> {
        self.ai_unavailable_reason().map(|reason| match reason {
            AiUnavailableReason::MissingApiKey => {
                "AI setup is incomplete because OPENAI_API_KEY is not set".to_string()
            }
            AiUnavailableReason::NoTriageModel => "no triage model is available".to_string(),
        })
    }
}
