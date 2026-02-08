use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

/// Identifier for built-in prompts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PromptId {
    ArticleTriage,
    ArticleSummary,
    AggregateBriefing,
}

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
}

impl PromptRegistry {
    pub fn new() -> Self {
        Self {
            templates: HashMap::new(),
            active_versions: HashMap::new(),
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
