use super::*;
use crate::{host_bootstrap::hydrate_result_stores, RuntimePaths};
use harvester_core::{update, AppState, Msg};

fn legacy(path: &Path) -> io::Result<Vec<(String, u32)>> {
    read_ron::<String, u32, String, u32>(path)
}
fn read(path: &Path) -> Vec<(String, u32)> {
    load::<String, u32, String, u32>(path, legacy).unwrap()
}
fn append(path: &Path, key: &str, value: u32) {
    AppendFile::open::<String, u32, String, u32>(path, legacy)
        .unwrap()
        .append::<_, _, String, u32>(&[(key.to_owned(), value)])
        .unwrap();
}
fn artifacts(dir: &Path, fragment: &str) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().contains(fragment))
        .collect()
}

fn declared_internal_state(path: &Path) -> bool {
    let name = path.file_name().unwrap().to_string_lossy();
    harvester_engine::build_corpus_manifest("2026-09-28T00:00:00Z")["layout"]["internal_state"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|pattern| {
            pattern
                .split_once('*')
                .is_some_and(|(prefix, suffix)| name.starts_with(prefix) && name.ends_with(suffix))
        })
}

#[test]
fn interrupted_migration_restarts_then_append_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".summary_cache.jsonl");
    let ron = b"(version:1,entries:[(\"old\",1),(\"older\",2)])";
    fs::write(path.with_extension("ron"), ron).unwrap();
    let stale = artifact_path(&path, "migrating");
    assert!(declared_internal_state(&stale));
    fs::write(&stale, b"[\"old\",1]\n[\"old").unwrap();
    assert_eq!(read(&path).len(), 2);
    append(&path, "new", 3);
    assert_eq!(
        read(&path),
        vec![("old".into(), 1), ("older".into(), 2), ("new".into(), 3)]
    );
    assert_eq!(fs::read(path.with_extension("ron")).unwrap(), ron);
    assert!(artifacts(dir.path(), ".migrating-").is_empty());
}

#[test]
fn torn_tail_is_saved_before_restart_append_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".summary_cache.jsonl");
    fs::write(&path, b"[\"old\",1]\n[\"torn").unwrap();
    assert_eq!(read(&path), vec![("old".into(), 1)]);
    let sidecars = artifacts(dir.path(), ".torn-");
    assert_eq!(sidecars.len(), 1);
    assert!(declared_internal_state(&sidecars[0]));
    assert_eq!(fs::read(&sidecars[0]).unwrap(), b"[\"torn");
    append(&path, "new", 2);
    assert_eq!(read(&path), vec![("old".into(), 1), ("new".into(), 2)]);
    assert_eq!(fs::read(&path).unwrap().last(), Some(&b'\n'));
}

#[test]
fn interrupted_multi_record_append_keeps_complete_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".triage_cache.jsonl");
    append(&path, "original", 0);
    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(b"[\"first\",1]\n[\"second\",2]\n[\"third\", ")
        .unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert_eq!(read(&path).len(), 3);
    append(&path, "next", 4);
    assert_eq!(
        read(&path),
        vec![
            ("original".into(), 0),
            ("first".into(), 1),
            ("second".into(), 2),
            ("next".into(), 4)
        ]
    );
}

#[test]
fn malformed_middle_and_final_complete_lines_are_counted_and_append_stays_readable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".summary_cache.jsonl");
    let bytes = b"[\"first\",1]\nnot json\n[\"second\",2]\n[bad final]\n";
    fs::write(&path, bytes).unwrap();
    let (parsed, skipped) = parse_lines::<String, u32, String, u32>(&path, bytes);
    assert_eq!(skipped, 2);
    assert_eq!(read(&path), parsed);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    append(&path, "new", 3);
    assert_eq!(
        read(&path),
        vec![("first".into(), 1), ("second".into(), 2), ("new".into(), 3)]
    );
    assert!(fs::read(&path).unwrap().starts_with(bytes));
}

fn migration(kind: &str) {
    let dir = tempfile::tempdir().unwrap();
    let filename = format!(".{kind}_cache.ron");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../harvester_batch/tests/fixtures/carry_over")
        .join(&filename);
    let original = fs::read(fixture).unwrap();
    let path = dir.path().join(&filename);
    fs::write(&path, &original).unwrap();
    macro_rules! check {
        ($legacy:path, $load:path, $iter:ident) => {{
            let before: std::collections::HashMap<_, _> =
                $legacy(&path).unwrap().into_iter().collect();
            let after = $load(&path).unwrap();
            let entries: std::collections::HashMap<_, _> =
                after.$iter().map(|(k, e)| (k.clone(), e.clone())).collect();
            assert_eq!(entries, before);
        }};
    }
    match kind {
        "triage" => check!(
            crate::triage_cache_store::legacy,
            crate::load_triage_cache,
            iter
        ),
        "summary" => check!(
            crate::summary_cache_store::legacy,
            crate::load_summary_cache,
            iter
        ),
        "signal_candidate" => {
            let before: std::collections::HashMap<_, _> =
                crate::signal_candidate_cache_store::legacy(&path)
                    .unwrap()
                    .into_iter()
                    .collect();
            let after = crate::load_signal_candidate_cache(&path).unwrap();
            assert_eq!(&*after.entries, &before);
        }
        _ => unreachable!(),
    }
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(path.with_extension("jsonl").is_file());
}

#[test]
fn triage_migration_preserves_every_key_entry_and_ron_byte() {
    migration("triage");
}
#[test]
fn summary_migration_preserves_every_key_entry_and_ron_byte() {
    migration("summary");
}
#[test]
fn signal_migration_preserves_every_key_entry_and_ron_byte() {
    migration("signal_candidate");
}

#[test]
fn golden_jsonl_files_load_every_entry() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/results");
    assert_eq!(
        crate::load_triage_cache(&fixtures.join(".triage_cache.jsonl"))
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        crate::load_summary_cache(&fixtures.join(".summary_cache.jsonl"))
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        crate::load_signal_candidate_cache(&fixtures.join(".signal_candidate_cache.jsonl"))
            .unwrap()
            .len(),
        2
    );
}

fn refusal(kind: &str, bytes: &[u8]) {
    let dir = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::with_defaults(dir.path().to_owned());
    let filename = format!(".{kind}_cache.ron");
    let path = dir.path().join(&filename);
    fs::write(&path, bytes).unwrap();
    let (state, _) = hydrate_result_stores(AppState::new(), &paths);
    assert!(!state.triage_ai_available());
    assert!(!state.briefing_ai_available());
    assert!(state.result_store_failure().unwrap().contains(&filename));
    assert!(state
        .view()
        .ai_unavailable_message
        .unwrap()
        .contains(&filename));
    let (state, _) = update(
        state,
        Msg::AiAvailabilityDetected {
            availability: harvester_core::AiAvailability::Available,
        },
    );
    assert!(
        !state.triage_ai_available(),
        "host availability must not clear a refusal"
    );
    assert!(matches!(
        state.ai_availability(),
        harvester_core::AiAvailability::Unavailable { .. }
    ));
    let (_, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Full,
        },
    );
    assert!(!effects
        .iter()
        .any(|effect| matches!(effect, harvester_core::Effect::RequestLlmCompletion { .. })));
    let save_error = match kind {
        "triage" => crate::persist_triage_cache(&harvester_core::TriageCache::new(), &path),
        "summary" => crate::persist_summary_cache(&harvester_core::SummaryCache::new(), &path),
        "signal_candidate" => crate::signal_candidate_cache_store::save(
            &path,
            &harvester_core::SignalCandidateCache::default(),
        ),
        _ => unreachable!(),
    }
    .unwrap_err();
    assert!(save_error.to_string().contains(&filename));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(!path.with_extension("jsonl").exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn corrupt_triage_ron_refuses_ai_and_names_untouched_file() {
    refusal("triage", b"invalid RON");
}
#[test]
fn corrupt_summary_ron_refuses_ai_and_names_untouched_file() {
    refusal("summary", b"invalid RON");
}
#[test]
fn corrupt_signal_ron_refuses_ai_and_names_untouched_file() {
    refusal("signal_candidate", b"invalid RON");
}
#[test]
fn unknown_signal_ron_version_refuses_ai_and_names_untouched_file() {
    refusal("signal_candidate", b"(version:999,entries:[])");
}

#[test]
fn unreadable_jsonl_refuses_ai_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::with_defaults(dir.path().to_owned());
    fs::create_dir(&paths.summary_cache_path).unwrap();
    let (state, _) = hydrate_result_stores(AppState::new(), &paths);
    assert!(!state.triage_ai_available());
    assert!(state
        .view()
        .ai_unavailable_message
        .unwrap()
        .contains(".summary_cache.jsonl"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn all_three_stores_keep_more_than_ten_thousand_entries_and_later_lines_win() {
    let dir = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::with_defaults(dir.path().to_owned());
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../harvester_batch/tests/fixtures/carry_over");
    let (tk, te) = crate::triage_cache_store::legacy(&fixtures.join(".triage_cache.ron"))
        .unwrap()
        .remove(0);
    let (sk, se) = crate::summary_cache_store::legacy(&fixtures.join(".summary_cache.ron"))
        .unwrap()
        .remove(0);
    let (ck, ce) =
        crate::signal_candidate_cache_store::legacy(&fixtures.join(".signal_candidate_cache.ron"))
            .unwrap()
            .remove(0);
    let mut triage = harvester_core::TriageCache::new();
    let mut summary = harvester_core::SummaryCache::new();
    let mut signal = harvester_core::SignalCandidateCache::default();
    for index in 0..10_050 {
        let mut key = tk.clone();
        key.content_hash = index.to_string();
        triage.insert_entry(key, te.clone());
        let mut key = sk.clone();
        key.content_hash = index.to_string();
        summary.insert(key, se.clone());
        let mut key = ck.clone();
        key.signal_input_hash = index.to_string();
        signal.insert(key, ce.clone());
    }
    assert_eq!(
        (triage.len(), summary.len(), signal.len()),
        (10_050, 10_050, 10_050)
    );
    crate::persist_triage_cache(&triage, &paths.triage_cache_path).unwrap();
    crate::persist_summary_cache(&summary, &paths.summary_cache_path).unwrap();
    crate::signal_candidate_cache_store::save(&paths.signal_candidate_cache_path, &signal).unwrap();
    assert_eq!(
        crate::load_triage_cache(&paths.triage_cache_path).unwrap(),
        triage
    );
    assert_eq!(
        crate::load_summary_cache(&paths.summary_cache_path).unwrap(),
        summary
    );
    assert_eq!(
        crate::load_signal_candidate_cache(&paths.signal_candidate_cache_path).unwrap(),
        signal
    );
    let mut key = sk;
    key.content_hash = "0".into();
    let mut newer = se;
    newer.result.summary = "newer paid result".into();
    AppendFile::open::<
        harvester_core::SummaryCacheKey,
        harvester_core::SummaryCacheEntry,
        crate::summary_cache_store::PersistedCacheKey,
        crate::summary_cache_store::PersistedCacheEntry,
    >(
        &paths.summary_cache_path,
        crate::summary_cache_store::legacy,
    )
    .unwrap()
    .append::<_, _, crate::summary_cache_store::PersistedCacheKey, crate::summary_cache_store::PersistedCacheEntry>(&[(key.clone(), newer.clone())])
    .unwrap();
    let loaded = crate::load_summary_cache(&paths.summary_cache_path).unwrap();
    assert_eq!(loaded.len(), 10_050);
    assert_eq!(loaded.lookup(&key), Some(&newer));
}

#[test]
fn retired_aggregate_briefing_summary_entries_are_skipped_during_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".summary_cache.ron");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../harvester_batch/tests/fixtures/carry_over/.summary_cache.ron");
    let ron =
        fs::read_to_string(fixture)
            .unwrap()
            .replacen("ArticleSummary", "AggregateBriefing", 1);
    fs::write(&path, &ron).unwrap();
    let before = crate::summary_cache_store::legacy(&path).unwrap();
    let after = crate::load_summary_cache(&path).unwrap();
    assert_eq!(after.len(), 2);
    assert_eq!(after.len(), before.len());
    for (key, entry) in before {
        assert_eq!(after.lookup(&key), Some(&entry));
    }
    assert_eq!(fs::read_to_string(path).unwrap(), ron);
}
