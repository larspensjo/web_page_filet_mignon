pub mod briefing;
pub mod summary;
pub mod triage;

use super::{PromptId, PromptTemplate};

pub fn register_defaults(registry: &mut super::PromptRegistry) {
    registry.register(triage::TRIAGE_PROMPT);
    registry.register(summary::SUMMARY_PROMPT);
    registry.register(briefing::BRIEFING_PROMPT);
}
