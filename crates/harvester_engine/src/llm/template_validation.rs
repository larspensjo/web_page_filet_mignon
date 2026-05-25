use std::collections::HashMap;

use crate::llm::prompt::{render_template, PromptId, RenderError, TemplateVars};

/// Which template field the error occurred in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateField {
    System,
    User,
}

/// Validation error emitted while checking placeholders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateValidationError {
    pub field: TemplateField,
    pub message: String,
}

fn synthetic_vars(prompt_id: PromptId) -> HashMap<String, String> {
    let mut vars = TemplateVars::new();
    vars.insert("context", "Sample context value");
    match prompt_id {
        PromptId::AggregateBriefing => {
            vars.set_document("collection", "Doc A\nDoc B");
            vars.insert("previous_briefings", "(none)");
            vars.insert("briefing_time_window", "All available articles");
        }
        PromptId::ArticleSignalCandidate => {
            vars.set_document("content", "Sample article text");
            vars.insert("url", "https://example.com/article");
            vars.insert("outlet", "example.com");
            vars.insert("title", "Sample article");
            vars.insert("published_at", "2026-05-25");
            vars.insert("triage_priority", "3");
            vars.insert("triage_tags", "ai-infrastructure, inference-scarcity");
            vars.insert("summary", "Sample summary text.");
            vars.insert("key_points", "- Point one\n- Point two");
        }
        _ => {
            vars.set_document("content", "Sample article text");
        }
    }
    vars.to_map()
}

fn format_render_error(field: TemplateField, error: RenderError) -> TemplateValidationError {
    let message = match error {
        RenderError::UnresolvedVariable {
            variable,
            template_fragment,
        } => {
            if variable == "malformed" {
                format!("malformed placeholder near '{}'", template_fragment)
            } else {
                format!(
                    "unresolved variable '{{{{{}}}}}' near '{}'",
                    variable, template_fragment
                )
            }
        }
        RenderError::ExceedsTokenBudget {
            rendered_len,
            budget,
        } => format!("rendered length {} exceeds budget {}", rendered_len, budget),
    };
    TemplateValidationError { field, message }
}

/// Validates that both template fields can be rendered with stage-appropriate variables.
pub fn validate_template(
    prompt_id: PromptId,
    system_template: &str,
    user_template: &str,
) -> Vec<TemplateValidationError> {
    let vars = synthetic_vars(prompt_id);
    let mut errors = Vec::new();
    if let Err(err) = render_template(system_template, &vars) {
        errors.push(format_render_error(TemplateField::System, err));
    }
    if let Err(err) = render_template(user_template, &vars) {
        errors.push(format_render_error(TemplateField::User, err));
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::prompt::PromptId;

    #[test]
    fn missing_placeholder_reports_system_error() {
        let errors = validate_template(
            PromptId::ArticleTriage,
            "System uses {{missing}}",
            "User uses {{content}}",
        );
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, TemplateField::System);
        assert!(errors[0]
            .message
            .contains("unresolved variable '{{missing}}'"));
    }

    #[test]
    fn malformed_placeholder_reported() {
        let errors = validate_template(
            PromptId::ArticleSummary,
            "System has {{bad",
            "User {{content}}",
        );
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, TemplateField::System);
        assert!(errors[0].message.contains("malformed placeholder"));
    }

    #[test]
    fn unknown_placeholder_reports_user_error() {
        let errors = validate_template(
            PromptId::ArticleSummary,
            "System {{content}}",
            "User uses {{unknown}}",
        );
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, TemplateField::User);
        assert!(errors[0]
            .message
            .contains("unresolved variable '{{unknown}}'"));
    }

    #[test]
    fn both_fields_reported_independently() {
        let errors = validate_template(
            PromptId::AggregateBriefing,
            "System {{missing}}",
            "User {{unknown}}",
        );
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|e| e.field == TemplateField::System));
        assert!(errors.iter().any(|e| e.field == TemplateField::User));
    }

    #[test]
    fn valid_template_produces_no_errors() {
        let errors = validate_template(
            PromptId::ArticleTriage,
            "System uses {{context}}",
            "User {{content}}",
        );
        assert!(errors.is_empty());
    }

    #[test]
    fn aggregate_briefing_supports_briefing_specific_variables() {
        let errors = validate_template(
            PromptId::AggregateBriefing,
            "System {{context}} {{previous_briefings}} {{briefing_time_window}}",
            "User {{collection}}",
        );
        assert!(errors.is_empty());
    }
}
