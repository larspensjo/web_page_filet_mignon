pub mod briefing;
pub mod summary;
pub mod triage;

use super::PromptId;

pub use briefing::BRIEFING_PROMPT_V7 as BRIEFING_PROMPT;
pub use briefing::{
    BRIEFING_PROMPT_V1, BRIEFING_PROMPT_V2, BRIEFING_PROMPT_V3, BRIEFING_PROMPT_V4,
    BRIEFING_PROMPT_V5, BRIEFING_PROMPT_V6, BRIEFING_PROMPT_V7,
};
pub use summary::SUMMARY_PROMPT_V4 as SUMMARY_PROMPT;
pub use summary::{SUMMARY_PROMPT_V1, SUMMARY_PROMPT_V2, SUMMARY_PROMPT_V3, SUMMARY_PROMPT_V4};
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
    registry.register(summary::SUMMARY_PROMPT_V4);
    registry.set_active(PromptId::ArticleSummary, summary::SUMMARY_PROMPT_V4.version);
    registry.register(briefing::BRIEFING_PROMPT_V1);
    registry.register(briefing::BRIEFING_PROMPT_V2);
    registry.register(briefing::BRIEFING_PROMPT_V3);
    registry.register(briefing::BRIEFING_PROMPT_V4);
    registry.register(briefing::BRIEFING_PROMPT_V5);
    registry.register(briefing::BRIEFING_PROMPT_V6);
    registry.register(briefing::BRIEFING_PROMPT_V7);
    registry.set_active(
        PromptId::AggregateBriefing,
        briefing::BRIEFING_PROMPT_V7.version,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::PromptRegistry;

    #[test]
    fn aggregate_briefing_active_version_is_v7() {
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        let active = registry
            .active(PromptId::AggregateBriefing)
            .expect("active AggregateBriefing prompt");
        assert_eq!(active.version, 7);
    }

    #[test]
    fn register_defaults_registers_seven_aggregate_briefing_versions() {
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        assert_eq!(registry.versions(PromptId::AggregateBriefing).len(), 7);
    }

    #[test]
    fn article_summary_active_version_is_v4() {
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        let active = registry
            .active(PromptId::ArticleSummary)
            .expect("active ArticleSummary prompt");
        assert_eq!(active.version, 4);
    }

    #[test]
    fn register_defaults_registers_four_article_summary_versions() {
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        assert_eq!(registry.versions(PromptId::ArticleSummary).len(), 4);
    }
}
