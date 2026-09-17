//! Reproducible identity for the current triage system prompt.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use harvester_engine::llm::{
    load_context_file, prompt::render_template, prompts::triage::TRIAGE_PROMPT_V4,
    PromptContextFile, PromptId, ReplayRecord,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The stable identity of the prompt policy used to select recordings.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PromptIdentity {
    pub system_message: String,
    pub system_sha256: String,
    pub template_sha256: String,
    pub rubric_text: String,
    pub rubric_sha256: String,
    pub context_version: u32,
}

/// Why a replay record did or did not match the expected prompt identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMatch {
    Match,
    PromptMismatch,
    WrongPromptOrVersion,
}

/// Loads a context file and creates the expected prompt identity.
pub fn load_prompt_identity(path: &Path) -> anyhow::Result<PromptIdentity> {
    let context = load_context_file(path)
        .with_context(|| format!("loading prompt context {}", path.display()))?;
    prompt_identity(&context)
}

/// Creates the expected prompt identity without IO.
///
/// The rubric is normalised to LF line endings, which is how the context file
/// is stored in Git. A Windows checkout with `core.autocrlf=true` yields CRLF
/// inside the TOML multi-line string, and matching on those bytes would reject
/// every recording made from an LF checkout.
pub fn prompt_identity(context: &PromptContextFile) -> anyhow::Result<PromptIdentity> {
    let rubric_text = context
        .variables
        .get("triage_instructions")
        .context("context is missing variables.triage_instructions")?
        .replace("\r\n", "\n");
    let mut vars = HashMap::new();
    vars.insert(
        "context".to_string(),
        format!("triage_instructions: {rubric_text}"),
    );
    let system_message = render_template(TRIAGE_PROMPT_V4.system_template, &vars)
        .map_err(|error| anyhow::anyhow!(error))?;
    Ok(PromptIdentity {
        system_sha256: sha256(&system_message),
        template_sha256: sha256(TRIAGE_PROMPT_V4.system_template),
        rubric_sha256: sha256(&rubric_text),
        system_message,
        rubric_text,
        context_version: context.meta.version,
    })
}

/// Classifies a recording against the selected prompt identity.
pub fn classify_record(
    record: &ReplayRecord,
    requested_version: u32,
    expected: &PromptIdentity,
) -> PromptMatch {
    if record.prompt_id != PromptId::ArticleTriage || record.prompt_version != requested_version {
        PromptMatch::WrongPromptOrVersion
    } else if record.rendered_system_message == expected.system_message {
        PromptMatch::Match
    } else {
        PromptMatch::PromptMismatch
    }
}

pub(crate) fn sha256(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::PromptContextFile;

    #[test]
    fn context_is_joined_the_same_way_as_completion_preparation() {
        let context: PromptContextFile = toml::from_str(
            "[meta]\nprompt_id='ArticleTriage'\nschema_version=1\nversion=3\nupdated='x'\n[variables]\ntriage_instructions='rubric'",
        )
        .unwrap();
        let identity = prompt_identity(&context).unwrap();
        assert!(identity
            .system_message
            .contains("triage_instructions: rubric"));
    }

    #[test]
    fn crlf_checkout_of_context_matches_lf_identity() {
        let lf = "[meta]\nprompt_id='ArticleTriage'\nschema_version=1\nversion=3\nupdated='x'\n[variables]\ntriage_instructions='''\nline one\nline two\n'''\n";
        let crlf = lf.replace('\n', "\r\n");
        let lf_identity = prompt_identity(&toml::from_str(lf).unwrap()).unwrap();
        let crlf_identity = prompt_identity(&toml::from_str(&crlf).unwrap()).unwrap();
        assert_eq!(lf_identity, crlf_identity);
        assert!(!crlf_identity.rubric_text.contains('\r'));
    }
}
