use engine_logging::{engine_info, engine_warn};
use harvester_engine::llm::prompt::{
    PromptId, PromptTemplateOwned, PromptVersion, PROMPT_VERSION_DRAFT,
};
use harvester_engine::llm::{load_context_file, PromptRegistry};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, RwLock},
};

type Contexts = HashMap<PromptId, Vec<(String, String)>>;
type Configuration = (Contexts, HashMap<PromptId, PromptVersion>, usize);

/// Shared context loader for startup hydration and processing snapshots.
/// Startup can publish partial contexts before reporting the required-file error.
pub(super) fn load_contexts(
    contexts_dir: &Path,
    require_triage: bool,
) -> (Contexts, Option<String>) {
    if !contexts_dir.exists() {
        let reason = format!(
            "required prompt contexts directory not found at {:?}",
            contexts_dir
        );
        return (HashMap::new(), Some(reason));
    }
    let mut contexts = HashMap::new();
    let mut required_failure = None;
    for id in [
        PromptId::ArticleTriage,
        PromptId::ArticleSummary,
        PromptId::ArticleSignalCandidate,
        PromptId::AggregateBriefing,
        PromptId::BriefingExecutiveSummary,
        PromptId::BriefingNextItem,
    ] {
        let path = contexts_dir.join(crate::effect_helpers::prompt_context_filename(id));
        if !path.exists() {
            if id == PromptId::ArticleTriage && require_triage {
                required_failure = Some(format!(
                    "required ArticleTriage context file missing at {:?}",
                    path
                ));
            }
            continue;
        }
        match load_context_file(&path) {
            Ok(file) => {
                contexts.insert(id, super::dispatch::ordered_context_pairs(&file));
            }
            Err(error) if id == PromptId::ArticleTriage && require_triage => {
                required_failure = Some(format!(
                    "required ArticleTriage context failed to load from {:?}: {}",
                    path, error
                ));
            }
            Err(error) => engine_warn!("[PromptContext] Failed to load {:?}: {}", path, error),
        }
    }
    (contexts, required_failure)
}

/// Register saved non-draft overlays and retain the established diagnostic logs.
pub(super) fn load_overlays(prompts_dir: &Path, registry: &Arc<RwLock<PromptRegistry>>) {
    for entry in crate::load_prompt_templates(prompts_dir) {
        let loaded = match entry {
            Ok(loaded) => loaded,
            Err(reason) => {
                engine_warn!(
                    "[prompt-lab-template] Failed to load saved template: {}",
                    reason
                );
                continue;
            }
        };
        if loaded.template_file.version == PROMPT_VERSION_DRAFT {
            engine_warn!(
                "[prompt-lab-template] skipping draft saved template prompt_id={:?} path={}",
                loaded.prompt_id,
                loaded.path.display()
            );
            continue;
        }
        let version = loaded.template_file.version;
        let overlay = PromptTemplateOwned {
            id: loaded.prompt_id,
            version,
            system_template: loaded.template_file.system_template,
            user_template: loaded.template_file.user_template,
            description: loaded.template_file.description,
            expected_format: loaded.template_file.expected_format,
        };
        match registry.write() {
            Ok(mut guard) => guard.register_overlay(overlay),
            Err(error) => {
                engine_warn!("[prompt-lab-template] registry lock: {}", error);
                continue;
            }
        }
        engine_info!(
            "[prompt-lab-template] Loaded saved template prompt_id={:?} version={} path={}",
            loaded.prompt_id,
            version,
            loaded.path.display()
        );
    }
}

/// Publish a snapshot only after contexts and template overlays have loaded.
pub(super) fn load(
    paths: &crate::RuntimePaths,
    registry: &Arc<RwLock<PromptRegistry>>,
    max_input_bytes: usize,
    require_triage_context: bool,
) -> Result<Configuration, String> {
    let (contexts, failure) = load_contexts(&paths.contexts_dir, require_triage_context);
    if let Some(reason) = failure {
        return Err(reason);
    }
    load_overlays(&paths.prompts_dir, registry);
    let registry = registry.read().map_err(|e| e.to_string())?;
    let budget = harvester_engine::summary_preparation_budget(max_input_bytes, &registry)?;
    Ok((contexts, registry.active_versions_map(), budget))
}
