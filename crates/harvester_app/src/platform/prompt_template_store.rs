use chrono::Utc;
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use harvester_engine::AtomicFileWriter;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub const PROMPT_TEMPLATE_SCHEMA_VERSION: u32 = 1;
pub type LoadedPromptTemplate = Result<(PromptId, PromptTemplateFile, PathBuf), String>;

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

pub fn prompts_directory() -> PathBuf {
    PathBuf::from("prompts")
}

pub fn save_template_file(
    prompt_id: PromptId,
    system_template: &str,
    user_template: &str,
    description: &str,
    expected_format: &str,
) -> Result<(PromptVersion, PathBuf), String> {
    let prompts_dir = prompts_directory();
    let prompt_dir = prompts_dir.join(prompt_id.to_string());
    ensure_directory_in_base(&prompts_dir, &prompt_dir)?;
    let version = next_template_version(&prompt_dir)?;
    let template_file = PromptTemplateFile {
        schema_version: PROMPT_TEMPLATE_SCHEMA_VERSION,
        version,
        updated: Utc::now().to_rfc3339(),
        prompt_id: prompt_id.to_string(),
        system_template: system_template.to_string(),
        user_template: user_template.to_string(),
        description: description.to_string(),
        expected_format: expected_format.to_string(),
    };
    let mut toml_string = toml::to_string(&template_file)
        .map_err(|err| format!("failed to serialize template: {}", err))?;
    toml_string.push('\n');
    let filename = format!("v{}.toml", version);
    let writer = AtomicFileWriter::new(prompt_dir.clone());
    let path = writer
        .write(&filename, &toml_string)
        .map_err(|err| format!("failed to write template file: {}", err))?;
    Ok((version, path))
}

pub fn load_prompt_template_files(base_dir: &Path) -> Vec<LoadedPromptTemplate> {
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
                            results.push(Ok((prompt_id, template_file, path.clone())));
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

fn ensure_directory_in_base(base: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|err| {
        format!(
            "failed to create prompt directory '{}': {}",
            target.display(),
            err
        )
    })?;
    let canonical_base = canonicalize_path(base)?;
    let canonical_target = canonicalize_path(target)?;
    if !canonical_target.starts_with(&canonical_base) {
        return Err(format!(
            "prompt directory '{}' is outside base '{}'",
            canonical_target.display(),
            canonical_base.display()
        ));
    }
    Ok(())
}

fn canonicalize_path(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|err| format!("failed to canonicalize '{}': {}", path.display(), err))
}

fn next_template_version(prompt_dir: &Path) -> Result<PromptVersion, String> {
    let mut max_version = 0;
    if prompt_dir.exists() {
        for entry in fs::read_dir(prompt_dir).map_err(|err| {
            format!(
                "failed to list prompt directory '{}': {}",
                prompt_dir.display(),
                err
            )
        })? {
            let entry = entry.map_err(|err| {
                format!(
                    "failed to inspect entry in '{}': {}",
                    prompt_dir.display(),
                    err
                )
            })?;
            let name = entry.file_name();
            let name = match name.to_str() {
                Some(name) => name,
                None => continue,
            };
            if let Some(stripped) = name.strip_prefix('v') {
                if let Some(version_str) = stripped.strip_suffix(".toml") {
                    if let Ok(parsed) = version_str.parse::<PromptVersion>() {
                        if parsed > max_version {
                            max_version = parsed;
                        }
                    }
                }
            }
        }
    }
    Ok(max_version.saturating_add(1))
}
