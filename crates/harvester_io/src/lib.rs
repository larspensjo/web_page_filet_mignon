//! Harvester IO: shared runtime paths, effect execution, and persistence.

mod blacklist_store;
mod effect_helpers;
mod effect_runner;
mod entity_index_store;
pub mod host_bootstrap;
mod persistence;
mod persistence_worker;
mod prompt_template_store;
pub mod run_lock;
mod runtime_paths;
mod seen_set_store;
pub mod signal_candidate_cache_store;
pub mod signal_candidate_overrides_store;
mod source_loader;
mod summary_cache_store;
mod triage_cache_store;

pub use blacklist_store::{default_blacklist_path, load_blacklist, save_blacklist};
pub use effect_runner::{
    EffectRunner, FileWriteObserver, NoOpPlatformHandler, NoOpRuntimePersistenceSink,
    PlatformEffectHandler, RuntimePersistenceSink,
};
pub use entity_index_store::{
    load_entity_index, save_entity_index, upsert_entry, EntityIndexPatch,
};
pub use persistence::{
    load_briefing_checkpoint, load_completed_jobs, load_desktop_window_size, load_pending_intake,
    load_window_size, persist_completed_jobs, persist_desktop_window_size, persist_runtime_state,
    persist_window_size, save_briefing_checkpoint, try_persist_runtime_state,
    try_persist_runtime_state_with_pending,
};
pub use persistence_worker::PersistenceWorker;
pub use prompt_template_store::load_prompt_templates;
pub use run_lock::{
    acquire_lock, LockGuard, LockIdentity, COMMAND_LINE_LOCK_IDENTITY, DESKTOP_LOCK_IDENTITY,
    LOCK_FILENAME,
};
pub use runtime_paths::{default_sources_path, RuntimePaths, DEFAULT_SOURCES_FILENAME};
pub use seen_set_store::{
    load_brave_seen_set, load_seen_set, persist_brave_metadata, persist_brave_seen_set,
    persist_seen_set, BraveMetadataEntry,
};
pub use signal_candidate_cache_store::load as load_signal_candidate_cache;
pub use signal_candidate_overrides_store::{
    load as load_signal_candidate_overrides, save as save_signal_candidate_overrides,
};
pub use source_loader::load_sources;
pub use summary_cache_store::load_summary_cache;
#[cfg(test)]
pub(crate) use summary_cache_store::persist_summary_cache;
pub use triage_cache_store::load_triage_cache;
#[cfg(test)]
pub(crate) use triage_cache_store::persist_triage_cache;

pub mod result_sink;
pub mod result_store;
