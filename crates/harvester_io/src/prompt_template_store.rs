use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub const PROMPT_TEMPLATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct LoadedPromptTemplate {
    pub prompt_id: PromptId,
    pub template_file: PromptTemplateFile,
    pub path: PathBuf,
}

/// File format for saved prompt templates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplateFile {
    pub schema_version: u32,
    pub version: PromptVersion,
    pub updated: String,
    pub prompt_id: String,
    pub system_template: String,
    pub user_template: String,
    pub description: String,
    pub expected_format: String,
}

impl PromptTemplateFile {
    fn is_valid_schema(&self) -> bool {
        self.schema_version == PROMPT_TEMPLATE_SCHEMA_VERSION
    }
}

pub fn load_prompt_templates(base_dir: &Path) -> Vec<Result<LoadedPromptTemplate, String>> {
    let mut results = Vec::new();
    let base_canonical = match canonicalize_path(base_dir) {
        Ok(path) => path,
        Err(_) => return results,
    };
    let entries = match fs::read_dir(base_dir) {
        Ok(entries) => entries,
        Err(_) => return results,
    };
    for entry in entries.filter_map(Result::ok) {
        let prompt_dir = entry.path();
        if !prompt_dir.is_dir() {
            continue;
        }
        let files = match fs::read_dir(&prompt_dir) {
            Ok(inner) => inner,
            Err(err) => {
                results.push(Err(format!(
                    "failed to read prompt directory '{}': {}",
                    prompt_dir.display(),
                    err
                )));
                continue;
            }
        };
        for file in files.filter_map(Result::ok) {
            let path = file.path();
            if !path.is_file() {
                continue;
            }
            match canonicalize_path(&path) {
                Ok(canonic) => {
                    if !canonic.starts_with(&base_canonical) {
                        results.push(Err(format!(
                            "template file '{}' is outside prompts directory",
                            path.display()
                        )));
                        continue;
                    }
                }
                Err(err) => {
                    results.push(Err(err));
                    continue;
                }
            }
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext != "toml")
                .unwrap_or(false)
            {
                continue;
            }
            let content = match fs::read_to_string(&path) {
                Ok(content) => content,
                Err(err) => {
                    results.push(Err(format!(
                        "failed to read template file '{}': {}",
                        path.display(),
                        err
                    )));
                    continue;
                }
            };
            match toml::from_str::<PromptTemplateFile>(&content) {
                Ok(template_file) => {
                    if !template_file.is_valid_schema() {
                        results.push(Err(format!(
                            "unsupported schema version {} in {}",
                            template_file.schema_version,
                            path.display()
                        )));
                        continue;
                    }
                    match PromptId::from_str(&template_file.prompt_id) {
                        Ok(prompt_id) => {
                            results.push(Ok(LoadedPromptTemplate {
                                prompt_id,
                                template_file,
                                path: path.clone(),
                            }));
                        }
                        Err(err) => {
                            results.push(Err(format!(
                                "invalid prompt id '{}' in {}: {}",
                                template_file.prompt_id,
                                path.display(),
                                err
                            )));
                        }
                    }
                }
                Err(err) => {
                    results.push(Err(format!(
                        "failed to parse template file '{}': {}",
                        path.display(),
                        err
                    )));
                }
            }
        }
    }
    results
}

fn canonicalize_path(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|err| format!("failed to canonicalize '{}': {}", path.display(), err))
}
