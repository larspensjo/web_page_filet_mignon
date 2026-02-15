use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

/// Identifier for built-in prompts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PromptId {
    ArticleTriage,
    ArticleSummary,
    AggregateBriefing,
}

impl FromStr for PromptId {
    type Err = ParsePromptIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ArticleTriage" => Ok(PromptId::ArticleTriage),
            "ArticleSummary" => Ok(PromptId::ArticleSummary),
            "AggregateBriefing" => Ok(PromptId::AggregateBriefing),
            _ => Err(ParsePromptIdError::Unknown(s.to_string())),
        }
    }
}

impl std::fmt::Display for PromptId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromptId::ArticleTriage => write!(f, "ArticleTriage"),
            PromptId::ArticleSummary => write!(f, "ArticleSummary"),
            PromptId::AggregateBriefing => write!(f, "AggregateBriefing"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsePromptIdError {
    Unknown(String),
}

impl std::fmt::Display for ParsePromptIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(s) => write!(f, "unknown prompt ID: '{}'", s),
        }
    }
}

impl std::error::Error for ParsePromptIdError {}

pub type PromptVersion = u32;

#[derive(Clone)]
pub struct PromptTemplate {
    pub id: PromptId,
    pub version: PromptVersion,
    pub system_template: &'static str,
    pub user_template: &'static str,
    pub description: &'static str,
    pub expected_format: &'static str,
}

pub const PROMPT_VERSION_DRAFT: PromptVersion = u32::MAX;

pub fn is_draft_version(version: PromptVersion) -> bool {
    version == PROMPT_VERSION_DRAFT
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptTemplateOwned {
    pub id: PromptId,
    pub version: PromptVersion,
    pub system_template: String,
    pub user_template: String,
    pub description: String,
    pub expected_format: String,
}

impl From<&PromptTemplate> for PromptTemplateOwned {
    fn from(template: &PromptTemplate) -> Self {
        Self {
            id: template.id,
            version: template.version,
            system_template: template.system_template.to_string(),
            user_template: template.user_template.to_string(),
            description: template.description.to_string(),
            expected_format: template.expected_format.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateSource {
    Static,
    Overlay,
}

pub enum EffectiveTemplate<'a> {
    Static(&'a PromptTemplate),
    Overlay(&'a PromptTemplateOwned),
}

impl<'a> EffectiveTemplate<'a> {
    pub fn source(&self) -> TemplateSource {
        match self {
            Self::Static(_) => TemplateSource::Static,
            Self::Overlay(_) => TemplateSource::Overlay,
        }
    }

    pub fn id(&self) -> PromptId {
        match self {
            Self::Static(template) => template.id,
            Self::Overlay(template) => template.id,
        }
    }

    pub fn version(&self) -> PromptVersion {
        match self {
            Self::Static(template) => template.version,
            Self::Overlay(template) => template.version,
        }
    }

    pub fn system_template(&self) -> &'a str {
        match self {
            Self::Static(template) => template.system_template,
            Self::Overlay(template) => template.system_template.as_str(),
        }
    }

    pub fn user_template(&self) -> &'a str {
        match self {
            Self::Static(template) => template.user_template,
            Self::Overlay(template) => template.user_template.as_str(),
        }
    }

    pub fn description(&self) -> &'a str {
        match self {
            Self::Static(template) => template.description,
            Self::Overlay(template) => template.description.as_str(),
        }
    }

    pub fn expected_format(&self) -> &'a str {
        match self {
            Self::Static(template) => template.expected_format,
            Self::Overlay(template) => template.expected_format.as_str(),
        }
    }

    pub fn to_owned(&self) -> PromptTemplateOwned {
        match self {
            Self::Static(template) => PromptTemplateOwned::from(*template),
            Self::Overlay(template) => (*template).clone(),
        }
    }
}

pub struct TemplateVars {
    entries: HashMap<String, String>,
}

impl TemplateVars {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.entries.insert(key.into(), value.into());
        self
    }

    pub fn set_document(&mut self, key: &str, content: &str) -> &mut Self {
        let nonce = content_nonce(content);
        let escaped = content.replace(&nonce, "");
        let wrapped = format!("<document-{nonce}>\n{escaped}\n</document-{nonce}>");
        self.entries.insert(key.to_string(), wrapped);
        self
    }

    pub fn to_map(&self) -> HashMap<String, String> {
        self.entries.clone()
    }
}

pub fn content_nonce(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    let digest = hasher.finalize();
    hex::encode(digest)[..12].to_string()
}

impl Default for TemplateVars {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct PromptRegistry {
    templates: HashMap<PromptId, HashMap<PromptVersion, PromptTemplate>>,
    active_versions: HashMap<PromptId, PromptVersion>,
    overlays: HashMap<(PromptId, PromptVersion), PromptTemplateOwned>,
}

impl PromptRegistry {
    pub fn new() -> Self {
        Self {
            templates: HashMap::new(),
            active_versions: HashMap::new(),
            overlays: HashMap::new(),
        }
    }

    pub fn register(&mut self, template: PromptTemplate) {
        let versions = self.templates.entry(template.id).or_default();
        let version = template.version;
        let id = template.id;
        versions.insert(version, template);
        self.active_versions.entry(id).or_insert(version);
    }

    pub fn set_active(&mut self, id: PromptId, version: PromptVersion) {
        if let Some(versions) = self.templates.get(&id) {
            if versions.contains_key(&version) {
                self.active_versions.insert(id, version);
            }
        }
    }

    pub fn active(&self, id: PromptId) -> Option<&PromptTemplate> {
        self.active_versions
            .get(&id)
            .and_then(|version| self.templates.get(&id).and_then(|m| m.get(version)))
    }

    pub fn get(&self, id: PromptId, version: PromptVersion) -> Option<&PromptTemplate> {
        self.templates.get(&id).and_then(|m| m.get(&version))
    }

    pub fn versions(&self, id: PromptId) -> HashSet<PromptVersion> {
        self.templates
            .get(&id)
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default()
    }

    pub fn active_versions_map(&self) -> HashMap<PromptId, PromptVersion> {
        self.active_versions.clone()
    }

    pub fn register_overlay(&mut self, template: PromptTemplateOwned) {
        debug_assert!(
            !is_draft_version(template.version),
            "draft versions must not live in the registry overlays"
        );
        self.overlays
            .insert((template.id, template.version), template);
    }

    pub fn remove_overlay(&mut self, id: PromptId, version: PromptVersion) {
        self.overlays.remove(&(id, version));
    }

    pub fn get_effective(
        &self,
        id: PromptId,
        version: PromptVersion,
    ) -> Option<EffectiveTemplate<'_>> {
        if let Some(template) = self.overlays.get(&(id, version)) {
            return Some(EffectiveTemplate::Overlay(template));
        }
        self.get(id, version).map(EffectiveTemplate::Static)
    }

    pub fn active_effective(&self, id: PromptId) -> Option<EffectiveTemplate<'_>> {
        self.active_versions
            .get(&id)
            .and_then(|version| self.get_effective(id, *version))
    }

    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        crate::llm::prompts::register_defaults(&mut registry);
        registry
    }
}

impl Default for PromptRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur during template rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// A `{{variable}}` in the template has no value in the variable map.
    UnresolvedVariable {
        variable: String,
        template_fragment: String,
    },
    /// The rendered prompt exceeds the configured token budget.
    ExceedsTokenBudget { rendered_len: usize, budget: usize },
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnresolvedVariable {
                variable,
                template_fragment,
            } => {
                write!(
                    f,
                    "unresolved variable '{{{{{}}}}}' near: {}",
                    variable, template_fragment
                )
            }
            Self::ExceedsTokenBudget {
                rendered_len,
                budget,
            } => {
                write!(
                    f,
                    "rendered prompt length {} exceeds budget {}",
                    rendered_len, budget
                )
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// Render a template string by replacing `{{key}}` placeholders with values
/// from the variable map. This is a single-pass scanner that only replaces
/// placeholders in the original template text, never inside injected values.
///
/// This function is **pure**: no IO, no mutation, deterministic output.
pub fn render_template(
    template: &str,
    vars: &HashMap<String, String>,
) -> Result<String, RenderError> {
    let mut result = String::with_capacity(template.len());
    let mut pos = 0;
    let bytes = template.as_bytes();

    while pos < bytes.len() {
        // Look for the start of a placeholder
        if let Some(open_offset) = template[pos..].find("{{") {
            let open_pos = pos + open_offset;

            // Append literal text before the placeholder
            result.push_str(&template[pos..open_pos]);

            let after_open = open_pos + 2;

            // Find the closing }}
            if let Some(close_offset) = template[after_open..].find("}}") {
                let close_pos = after_open + close_offset;
                let key = template[after_open..close_pos].trim();

                // Look up the variable
                let value = vars.get(key).ok_or_else(|| {
                    let fragment_start = open_pos;
                    let fragment_end = (close_pos + 2).min(template.len());
                    let fragment = template[fragment_start..fragment_end].to_string();
                    RenderError::UnresolvedVariable {
                        variable: key.to_string(),
                        template_fragment: fragment,
                    }
                })?;

                // Append the replacement value
                result.push_str(value);

                // Advance past the closing }}
                pos = close_pos + 2;
            } else {
                // Malformed placeholder (no closing }})
                let fragment = template[open_pos..].chars().take(40).collect::<String>();
                return Err(RenderError::UnresolvedVariable {
                    variable: "malformed".to_string(),
                    template_fragment: fragment,
                });
            }
        } else {
            // No more placeholders, append the rest
            result.push_str(&template[pos..]);
            break;
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_static_template(version: PromptVersion) -> PromptTemplate {
        PromptTemplate {
            id: PromptId::ArticleTriage,
            version,
            system_template: "system",
            user_template: "user",
            description: "desc",
            expected_format: "json",
        }
    }

    #[test]
    fn overlay_shadows_static_effective_template() {
        let base = make_static_template(1);
        let mut registry = PromptRegistry::new();
        registry.register(base.clone());

        let mut overlay = PromptTemplateOwned::from(&base);
        overlay.system_template = "override-system".to_string();
        overlay.user_template = "override-user".to_string();
        registry.register_overlay(overlay.clone());

        let effective = registry
            .get_effective(base.id, base.version)
            .expect("expected effective template");

        assert_eq!(effective.source(), TemplateSource::Overlay);
        assert_eq!(effective.system_template(), overlay.system_template.as_str());
        assert_eq!(effective.user_template(), overlay.user_template.as_str());
    }

    #[test]
    fn get_effective_falls_back_to_static() {
        let base = make_static_template(2);
        let mut registry = PromptRegistry::new();
        registry.register(base.clone());

        let effective = registry
            .get_effective(base.id, base.version)
            .expect("expected effective template");

        assert_eq!(effective.source(), TemplateSource::Static);
        assert_eq!(effective.system_template(), base.system_template);
    }

    #[test]
    fn active_effective_observes_overlay() {
        let base = make_static_template(3);
        let mut registry = PromptRegistry::new();
        registry.register(base.clone());

        let mut overlay = PromptTemplateOwned::from(&base);
        overlay.system_template = "active override".to_string();
        registry.register_overlay(overlay.clone());

        let effective = registry
            .active_effective(base.id)
            .expect("expected active effective template");
        assert_eq!(effective.source(), TemplateSource::Overlay);
        assert_eq!(effective.system_template(), overlay.system_template.as_str());
    }

    #[test]
    #[should_panic(expected = "draft versions must not live in the registry overlays")]
    fn register_overlay_rejects_draft_version() {
        let mut registry = PromptRegistry::new();
        let mut overlay = PromptTemplateOwned::from(&make_static_template(4));
        overlay.version = PROMPT_VERSION_DRAFT;
        registry.register_overlay(overlay);
    }
}
