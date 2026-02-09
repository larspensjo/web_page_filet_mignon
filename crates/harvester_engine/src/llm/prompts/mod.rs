pub mod briefing;
pub mod summary;
pub mod triage;

use super::PromptId;

pub use briefing::BRIEFING_PROMPT_V2 as BRIEFING_PROMPT;
pub use briefing::{BRIEFING_PROMPT_V1, BRIEFING_PROMPT_V2};
pub use summary::SUMMARY_PROMPT_V2 as SUMMARY_PROMPT;
pub use summary::{SUMMARY_PROMPT_V1, SUMMARY_PROMPT_V2};
pub use triage::TRIAGE_PROMPT;

pub fn register_defaults(registry: &mut super::PromptRegistry) {
    registry.register(triage::TRIAGE_PROMPT);
    registry.register(summary::SUMMARY_PROMPT_V1);
    registry.register(summary::SUMMARY_PROMPT_V2);
    registry.set_active(PromptId::ArticleSummary, summary::SUMMARY_PROMPT_V2.version);
    registry.register(briefing::BRIEFING_PROMPT_V1);
    registry.register(briefing::BRIEFING_PROMPT_V2);
    registry.set_active(
        PromptId::AggregateBriefing,
        briefing::BRIEFING_PROMPT_V2.version,
    );
}
