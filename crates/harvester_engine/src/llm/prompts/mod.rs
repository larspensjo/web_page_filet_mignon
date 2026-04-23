pub mod briefing;
pub mod summary;
pub mod triage;

use super::PromptId;

pub use briefing::BRIEFING_PROMPT_V8 as BRIEFING_PROMPT;
pub use briefing::{
    BRIEFING_PROMPT_V1, BRIEFING_PROMPT_V2, BRIEFING_PROMPT_V3, BRIEFING_PROMPT_V4,
    BRIEFING_PROMPT_V5, BRIEFING_PROMPT_V6, BRIEFING_PROMPT_V7, BRIEFING_PROMPT_V8,
};
pub use summary::SUMMARY_PROMPT_V5 as SUMMARY_PROMPT;
pub use summary::{
    SUMMARY_PROMPT_V1, SUMMARY_PROMPT_V2, SUMMARY_PROMPT_V3, SUMMARY_PROMPT_V4, SUMMARY_PROMPT_V5,
};
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
    registry.register(summary::SUMMARY_PROMPT_V5);
    registry.set_active(PromptId::ArticleSummary, summary::SUMMARY_PROMPT_V5.version);
    registry.register(briefing::BRIEFING_PROMPT_V1);
    registry.register(briefing::BRIEFING_PROMPT_V2);
    registry.register(briefing::BRIEFING_PROMPT_V3);
    registry.register(briefing::BRIEFING_PROMPT_V4);
    registry.register(briefing::BRIEFING_PROMPT_V5);
    registry.register(briefing::BRIEFING_PROMPT_V6);
    registry.register(briefing::BRIEFING_PROMPT_V7);
    registry.register(briefing::BRIEFING_PROMPT_V8);
    registry.set_active(
        PromptId::AggregateBriefing,
        briefing::BRIEFING_PROMPT_V8.version,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::PromptRegistry;

    #[test]
    fn register_defaults_activates_exported_default_aliases() {
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        assert_eq!(
            registry
                .active(PromptId::ArticleTriage)
                .expect("active ArticleTriage prompt")
                .version,
            TRIAGE_PROMPT.version
        );
        assert_eq!(
            registry
                .active(PromptId::ArticleSummary)
                .expect("active ArticleSummary prompt")
                .version,
            SUMMARY_PROMPT.version
        );
        assert_eq!(
            registry
                .active(PromptId::AggregateBriefing)
                .expect("active AggregateBriefing prompt")
                .version,
            BRIEFING_PROMPT.version
        );
    }

    #[test]
    fn register_defaults_keeps_older_exported_versions_addressable() {
        let mut registry = PromptRegistry::new();
        register_defaults(&mut registry);
        for template in [TRIAGE_PROMPT_V1, SUMMARY_PROMPT_V1, BRIEFING_PROMPT_V1] {
            let registered = registry
                .get(template.id, template.version)
                .expect("older exported template should remain registered");
            assert_eq!(registered.description, template.description);
        }
    }
}
