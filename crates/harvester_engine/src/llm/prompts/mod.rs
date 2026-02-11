pub mod briefing;
pub mod summary;
pub mod triage;

use super::PromptId;

pub use briefing::BRIEFING_PROMPT_V3 as BRIEFING_PROMPT;
pub use briefing::{BRIEFING_PROMPT_V1, BRIEFING_PROMPT_V2, BRIEFING_PROMPT_V3};
pub use summary::SUMMARY_PROMPT_V3 as SUMMARY_PROMPT;
pub use summary::{SUMMARY_PROMPT_V1, SUMMARY_PROMPT_V2, SUMMARY_PROMPT_V3};
pub use triage::TRIAGE_PROMPT_V3 as TRIAGE_PROMPT;
pub use triage::{TRIAGE_PROMPT_V1, TRIAGE_PROMPT_V2, TRIAGE_PROMPT_V3};

pub fn register_defaults(registry: &mut super::PromptRegistry) {
    registry.register(triage::TRIAGE_PROMPT_V1);
    registry.register(triage::TRIAGE_PROMPT_V2);
    registry.register(triage::TRIAGE_PROMPT_V3);
    registry.set_active(PromptId::ArticleTriage, triage::TRIAGE_PROMPT_V3.version);
    registry.register(summary::SUMMARY_PROMPT_V1);
    registry.register(summary::SUMMARY_PROMPT_V2);
    registry.register(summary::SUMMARY_PROMPT_V3);
    registry.set_active(PromptId::ArticleSummary, summary::SUMMARY_PROMPT_V3.version);
    registry.register(briefing::BRIEFING_PROMPT_V1);
    registry.register(briefing::BRIEFING_PROMPT_V2);
    registry.register(briefing::BRIEFING_PROMPT_V3);
    registry.set_active(
        PromptId::AggregateBriefing,
        briefing::BRIEFING_PROMPT_V3.version,
    );
}
