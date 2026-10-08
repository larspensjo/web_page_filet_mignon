use std::sync::Mutex;

struct CapturingLogger(Mutex<Vec<String>>);
static LOGGER: CapturingLogger = CapturingLogger(Mutex::new(Vec::new()));

impl log::Log for CapturingLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.0.lock().unwrap().push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

#[test]
fn unknown_prompt_id_entry_is_skipped_with_warning() {
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".triage_cache.ron");
    let source = r#"(
        version: 1,
        entries: [
            ((content_hash: "unknown", prompt_id: "UnknownPrompt", prompt_version: 1,
              model_id: "model", context_hash: "ctx"),
             (result: (category: "technology", priority: 1, tags: [], rationale: "unknown",
                       input_tokens: 1, output_tokens: 1), created_at_utc: "2026-01-01T00:00:00Z")),
            ((content_hash: "valid", prompt_id: "ArticleTriage", prompt_version: 1,
              model_id: "model", context_hash: "ctx"),
             (result: (category: "technology", priority: 2, tags: [], rationale: "valid",
                       input_tokens: 1, output_tokens: 1), created_at_utc: "2026-01-01T00:00:00Z")),
        ],
    )"#;
    std::fs::write(&path, source).unwrap();

    let cache = harvester_io::load_triage_cache(&path).unwrap();
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.iter().next().unwrap().0.content_hash, "valid");
    assert_eq!(
        std::fs::read_to_string(path.with_extension("jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    let warnings = LOGGER.0.lock().unwrap();
    assert!(warnings.iter().any(|line| {
        line.contains(".triage_cache.ron")
            && line.contains("skipped 1 entries with unknown prompt id")
    }));
}
