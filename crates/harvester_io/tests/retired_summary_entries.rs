use std::{
    fs,
    sync::{Mutex, Once},
};

use harvester_core::AppState;
use harvester_io::{host_bootstrap::hydrate_result_stores, load_summary_cache, RuntimePaths};

struct WarningLog(Mutex<Vec<String>>);
static LOG: WarningLog = WarningLog(Mutex::new(Vec::new()));
impl log::Log for WarningLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() == log::Level::Warn
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.0.lock().unwrap().push(record.args().to_string());
        }
    }
    fn flush(&self) {}
}
fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        log::set_logger(&LOG).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
    });
}
fn fixture() -> String {
    include_str!("../../harvester_batch/tests/fixtures/carry_over/.summary_cache.ron").into()
}
fn assert_loads_without_refusal(paths: &RuntimePaths) {
    let cache = load_summary_cache(&paths.summary_cache_path).unwrap();
    assert_eq!(cache.len(), 2);
    assert!(cache
        .iter()
        .all(|(key, _)| key.prompt_id == harvester_engine::llm::PromptId::ArticleSummary));
    assert!(cache.iter().any(|(_, entry)| entry.result.summary
        == "A canned summary for deterministic replay measurement."));
    let (state, _) = hydrate_result_stores(AppState::new(), paths);
    assert!(state.result_store_failure().is_none());
}
fn paths(dir: &std::path::Path) -> RuntimePaths {
    RuntimePaths::new(
        dir.into(),
        dir.join(".sources.ron"),
        dir.join("contexts"),
        dir.join("prompts"),
    )
}

#[test]
fn jsonl_skips_aggregate_entries_warns_with_file_and_count_and_keeps_paid_results() {
    init();
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    fs::write(paths.summary_cache_path.with_extension("ron"), fixture()).unwrap();
    let original = load_summary_cache(&paths.summary_cache_path).unwrap();
    assert_eq!(original.len(), 3);
    let jsonl = paths.summary_cache_path.with_extension("jsonl");
    let text =
        fs::read_to_string(&jsonl)
            .unwrap()
            .replacen("ArticleSummary", "AggregateBriefing", 1);
    fs::write(&jsonl, &text).unwrap();
    assert_loads_without_refusal(&paths);
    assert_eq!(fs::read_to_string(&jsonl).unwrap(), text);
    let warnings = LOG.0.lock().unwrap();
    let file_warnings: Vec<_> = warnings
        .iter()
        .filter(|line| line.contains(&jsonl.display().to_string()))
        .collect();
    // Both loading and hydration read this store. Each read reports one
    // aggregate retirement warning rather than a per-line parse failure.
    assert_eq!(file_warnings.len(), 2);
    assert!(file_warnings
        .iter()
        .all(|line| line.contains("skipped 1 entries with retired/unknown prompt id")));
    assert!(file_warnings
        .iter()
        .all(|line| !line.contains("malformed complete lines") && !line.contains("skipped line")));
}

#[test]
fn ron_migration_skips_aggregate_entries_warns_with_file_and_count_and_keeps_paid_results() {
    init();
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    let text = fixture().replacen("ArticleSummary", "AggregateBriefing", 1);
    let ron = paths.summary_cache_path.with_extension("ron");
    fs::write(&ron, &text).unwrap();
    assert_loads_without_refusal(&paths);
    assert_eq!(fs::read_to_string(&ron).unwrap(), text);
    assert!(paths.summary_cache_path.with_extension("jsonl").is_file());
    let warnings = LOG.0.lock().unwrap();
    assert!(warnings
        .iter()
        .any(|line| line.contains(&ron.display().to_string())
            && line.contains("skipped 1 entries with unknown prompt id during RON migration")));
}
