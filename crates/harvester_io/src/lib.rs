//! Harvester IO: shared runtime paths, effect execution, and persistence.

mod effect_helpers;
mod effect_runner;
mod entity_index_store;
mod persistence;
mod persistence_worker;
mod prompt_template_store;
mod runtime_paths;
mod seen_set_store;
mod source_loader;
mod summary_cache_store;
mod triage_cache_store;

pub use effect_runner::{EffectRunner, NoOpPlatformHandler, PlatformEffectHandler};
pub use entity_index_store::{load_entity_index, save_entity_index, EntityIndexPatch};
pub use persistence::{
    load_briefing_checkpoint, load_briefing_history, load_completed_jobs,
    load_pre_triage_overrides, load_window_size, persist_completed_jobs, persist_runtime_state,
    persist_window_size, save_briefing_checkpoint, save_briefing_history,
};
pub use persistence_worker::{PersistenceSnapshot, PersistenceWorker};
pub use prompt_template_store::{load_prompt_templates, save_prompt_template};
pub use runtime_paths::RuntimePaths;
pub use seen_set_store::{
    load_brave_seen_set, load_seen_set, persist_brave_metadata, persist_brave_seen_set,
    persist_seen_set, BraveMetadataEntry,
};
pub use source_loader::load_sources;
pub use summary_cache_store::{load_summary_cache, persist_summary_cache};
pub use triage_cache_store::{load_triage_cache, persist_triage_cache};
