use engine_logging::engine_info;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

use crate::llm::prompt::PromptId;

/// User-editable context that is loaded from a TOML file and injected
/// into a `PromptTemplate` at render time.
/// Pure data container — no behaviour beyond deserialization.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PromptContextFile {
    pub meta: ContextMeta,
    pub variables: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContextMeta {
    pub prompt_id: String,
    pub schema_version: u32,
    pub version: u32,
    pub updated: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub changelog: Option<String>,
}

/// Load and parse a prompt context file from disk.
/// This is an **effect handler** — it performs IO and should be called
/// only from the effect layer, never from a reducer.
pub fn load_context_file(path: &Path) -> Result<PromptContextFile, ContextLoadError> {
    let content = std::fs::read_to_string(path).map_err(|e| ContextLoadError::Io {
        path: path.display().to_string(),
        source: e,
    })?;

    let ctx: PromptContextFile = toml::from_str(&content).map_err(|e| ContextLoadError::Parse {
        path: path.display().to_string(),
        source: e,
    })?;

    // Validate schema_version
    if ctx.meta.schema_version != 1 {
        return Err(ContextLoadError::UnknownSchemaVersion {
            path: path.display().to_string(),
            version: ctx.meta.schema_version,
        });
    }

    // Validate prompt_id is recognized
    PromptId::from_str(&ctx.meta.prompt_id).map_err(|_| ContextLoadError::UnknownPromptId {
        path: path.display().to_string(),
        prompt_id: ctx.meta.prompt_id.clone(),
    })?;

    engine_info!(
        "[PromptContext] Loaded context for '{}' v{} (schema v{}, updated: {})",
        ctx.meta.prompt_id,
        ctx.meta.version,
        ctx.meta.schema_version,
        ctx.meta.updated
    );

    Ok(ctx)
}

#[derive(Debug)]
pub enum ContextLoadError {
    Io {
        path: String,
        source: std::io::Error,
    },
    Parse {
        path: String,
        source: toml::de::Error,
    },
    UnknownPromptId {
        path: String,
        prompt_id: String,
    },
    UnknownSchemaVersion {
        path: String,
        version: u32,
    },
}

impl std::fmt::Display for ContextLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "failed to read '{}': {}", path, source),
            Self::Parse { path, source } => write!(f, "failed to parse '{}': {}", path, source),
            Self::UnknownPromptId { path, prompt_id } => write!(
                f,
                "unknown prompt_id '{}' in '{}'. Valid values: ArticleTriage, ArticleSummary, ArticleSignalCandidate, AggregateBriefing",
                prompt_id, path
            ),
            Self::UnknownSchemaVersion { path, version } => write!(
                f,
                "unsupported schema_version {} in '{}'. Only schema_version = 1 is supported.",
                version, path
            ),
        }
    }
}

impl std::error::Error for ContextLoadError {}

/// Validate that a context file provides all variables required by
/// the template (excluding runtime variables like "content" and "collection").
///
/// `known_runtime_vars` lists keys that will be supplied at render time
/// and should not be required in the context.
///
/// Returns a list of missing variables (those in the template but not in
/// context or runtime vars) and a list of unused variables (those in context
/// but not referenced in any template).
pub fn validate_context_covers_template(
    template: &str,
    context: &PromptContextFile,
    known_runtime_vars: &[&str],
) -> (Vec<String>, Vec<String>) {
    let mut missing = Vec::new();
    let mut referenced_in_template = std::collections::HashSet::new();
    let mut pos = 0;

    // Scan template for {{key}} placeholders
    while let Some(open_offset) = template[pos..].find("{{") {
        let open_pos = pos + open_offset;
        let after_open = open_pos + 2;

        if let Some(close_offset) = template[after_open..].find("}}") {
            let close_pos = after_open + close_offset;
            let key = template[after_open..close_pos].trim();

            referenced_in_template.insert(key.to_string());

            let is_runtime = known_runtime_vars.contains(&key);
            let is_in_context = context.variables.contains_key(key);

            if !is_runtime && !is_in_context && !missing.contains(&key.to_string()) {
                missing.push(key.to_string());
            }

            pos = close_pos + 2;
        } else {
            break;
        }
    }

    // Check for unused context variables
    let mut unused = Vec::new();
    for key in context.variables.keys() {
        if !referenced_in_template.contains(key) {
            unused.push(key.clone());
        }
    }

    (missing, unused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::prompt::PromptId;

    #[test]
    fn prompt_context_file_roundtrip_serialization() {
        let mut variables = HashMap::new();
        variables.insert("first".to_string(), "value".to_string());
        variables.insert("second".to_string(), "value-two".to_string());
        let meta = ContextMeta {
            prompt_id: PromptId::ArticleTriage.to_string(),
            schema_version: 1,
            version: 5,
            updated: "2025-01-01T00:00:00Z".to_string(),
            description: Some("desc".to_string()),
            changelog: Some("changelog".to_string()),
        };
        let file = PromptContextFile {
            meta: meta.clone(),
            variables: variables.clone(),
        };
        let serialized = toml::to_string(&file).expect("serialize prompt context");
        let parsed: PromptContextFile =
            toml::from_str(&serialized).expect("deserialize prompt context");
        assert_eq!(parsed.meta, meta);
        assert_eq!(parsed.variables, variables);
    }

    #[test]
    fn signal_candidate_context_loads() {
        let p = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contexts/article_signal_candidate.toml"
        ));
        let ctx = load_context_file(p).expect("context file must load");
        assert_eq!(ctx.meta.prompt_id, "ArticleSignalCandidate");
        assert_eq!(ctx.meta.schema_version, 1);
        assert!(
            !ctx.variables.is_empty(),
            "must define at least one variable"
        );
        assert!(
            ctx.variables.contains_key("context"),
            "must define a {{{{context}}}} variable used by the system template"
        );
    }
}
