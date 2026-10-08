use crate::host_bootstrap::{prepare_desktop_startup_state, pump_pre_triage_refresh};
use crate::*;
use harvester_core::{update, AppState, Effect, JobListMode, LlmResultKind, Msg};
use harvester_engine::llm::{
    prompt::PromptId, PromptRegistry, DEFAULT_SUMMARY_MODEL, DEFAULT_TRIAGE_MODEL,
};
use std::{
    collections::HashMap,
    sync::mpsc,
    time::{Duration, Instant},
};

fn date() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-10-03T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}
fn models() -> HashMap<PromptId, String> {
    [
        (PromptId::ArticleTriage, DEFAULT_TRIAGE_MODEL),
        (PromptId::ArticleSummary, DEFAULT_SUMMARY_MODEL),
        (PromptId::ArticleSignalCandidate, DEFAULT_SUMMARY_MODEL),
    ]
    .into_iter()
    .map(|(id, model)| (id, model.to_string()))
    .collect()
}

fn corpus() -> (tempfile::TempDir, RuntimePaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::new(
        dir.path().to_path_buf(),
        dir.path().join("sources.ron"),
        dir.path().join("contexts"),
        dir.path().join("prompts"),
    );
    std::fs::create_dir_all(&paths.contexts_dir).unwrap();
    std::fs::write(paths.contexts_dir.join("article_triage.toml"), "[meta]\nprompt_id = \"ArticleTriage\"\nschema_version = 1\nversion = 1\nupdated = \"2026-10-03\"\ndescription = \"fixture\"\n[variables]\n").unwrap();
    for (i, fetched) in ["2026-10-03T09:00:00Z", "2026-10-03T11:00:00Z"]
        .into_iter()
        .enumerate()
    {
        let (_, doc) = harvester_engine::build_markdown_document(
            &format!("https://restart.example/{i}"),
            Some(&format!("Article {i}")),
            "utf-8",
            fetched,
            &"substantial business news ".repeat(220),
            &harvester_engine::WhitespaceTokenCounter,
        );
        std::fs::write(dir.path().join(format!("article-{i}.md")), doc).unwrap();
    }
    (dir, paths)
}

fn canned_run(paths: &RuntimePaths) -> (AppState, Vec<harvester_core::SavedResult>) {
    let registry = PromptRegistry::with_defaults();
    let urls: Vec<_> = (0..2)
        .map(|i| format!("https://restart.example/{i}"))
        .collect();
    let mut scan = harvester_engine::CorpusScanIndex::default();
    let (delta, _) = scan
        .load_delta(
            &paths.output_dir,
            100_000,
            &registry,
            &urls,
            None,
            &[],
            |_| {},
        )
        .unwrap();
    let jobs = delta
        .members
        .iter()
        .map(|a| harvester_core::CompletedJobSnapshot {
            url: a.url.clone(),
            fetched_utc: a.fetched_utc.clone(),
            tokens: Some(660),
            bytes: Some(1000),
            links: vec![],
        })
        .collect();
    let mut state = update(AppState::new(), Msg::RestoreCompletedJobs(jobs)).0;
    state = update(
        state,
        Msg::PromptContextsLoaded {
            contexts: HashMap::new(),
        },
    )
    .0;
    state = update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: registry.active_versions_map(),
            effective_models: models(),
        },
    )
    .0;
    state = update(
        state,
        Msg::BriefingCheckpointLoaded {
            since_utc: Some("2026-10-02T00:00:00Z".into()),
        },
    )
    .0;
    let (next, _, _) = pump_pre_triage_refresh(state);
    state = next;
    let (next, effects) = update(
        state,
        Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Resume,
        },
    );
    state = next;
    let mut pending: std::collections::VecDeque<_> = effects.into();
    let mut records = Vec::new();
    let mut steps = 0;
    while let Some(effect) = pending.pop_front() {
        steps += 1;
        assert!(steps < 100, "canned run must settle");
        let message = match effect {
            Effect::SaveResults { records: completed } => {
                records.extend(completed);
                continue;
            }
            Effect::LoadProcessingConfiguration { request_id, .. } => {
                Msg::ProcessingConfigurationLoaded {
                    request_id,
                    contexts: HashMap::new(),
                    active_versions: registry.active_versions_map(),
                    effective_models: models(),
                    preparation_budget: delta.preparation_budget,
                }
            }
            Effect::LoadArticlesForTriage { request_id, .. } => {
                state = update(
                    state,
                    Msg::SavedArticlesLoaded {
                        request_id,
                        articles: scan.article_metadata(),
                    },
                )
                .0;
                Msg::TriageArticlesLoaded {
                    request_id,
                    delta: delta.clone(),
                }
            }
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                prompt_version,
                ..
            } => {
                let output_json = match prompt_id {
                    PromptId::ArticleTriage => {
                        r#"{"category":"news","priority":4,"tags":["ai"],"rationale":"Relevant"}"#
                    }
                    PromptId::ArticleSummary => {
                        r#"{"title":"Current saved title","summary":"Current saved summary","key_points":["A point"]}"#
                    }
                    PromptId::ArticleSignalCandidate => {
                        r#"{"signal_score":90,"signal_key":"event","themes":["ai"],"draft_gist":"Concrete company development","source_tier":"Tier1","confidence":"High","reasoning":"Relevant"}"#
                    }
                };
                Msg::LlmCompleted {
                    request_id,
                    result: LlmResultKind::Success {
                        output_json: output_json.into(),
                        input_tokens: 100,
                        output_tokens: 20,
                        prompt_version: prompt_version
                            .unwrap_or(registry.active_versions_map()[&prompt_id]),
                        resolved_model: models()[&prompt_id].clone(),
                    },
                    metadata: None,
                }
            }
            _ => continue,
        };
        let (next, effects) = update(state, message);
        state = next;
        pending.extend(effects);
    }
    assert_eq!(state.run_state(), harvester_core::RunState::Idle);
    assert!(!records.is_empty());
    (state, records)
}

fn archive(state: AppState) -> Effect {
    let (state, _) = update(state, Msg::ArchiveClicked);
    let request_id = state.archive_request_id();
    update(
        state,
        Msg::ArchiveDialogSubmitted {
            request_id,
            basename: "archive.md".into(),
            set_checkpoint: false,
            submitted_at: date(),
            use_summaries: true,
            use_signal_candidates: false,
        },
    )
    .1
    .into_iter()
    .find(|e| matches!(e, Effect::ArchiveRequested { .. }))
    .unwrap()
}

fn write_archive(paths: &RuntimePaths, effect: &Effect) -> Vec<u8> {
    let Effect::ArchiveRequested {
        basename,
        ordered_urls,
        since_utc,
        use_summaries,
        summaries,
        annotations,
        priority_snapshot,
        ..
    } = effect
    else {
        panic!("archive effect")
    };
    harvester_engine::build_triage_archive(
        &paths.output_dir,
        basename,
        ordered_urls,
        *since_utc,
        *use_summaries,
        summaries,
        annotations,
        priority_snapshot,
    )
    .unwrap();
    std::fs::read(paths.output_dir.join(basename)).unwrap()
}

fn restart(paths: &RuntimePaths) -> AppState {
    let (tx, rx) = mpsc::channel();
    let runner = EffectRunner::new(
        paths.clone(),
        tx,
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    let (mut state, effects) = prepare_desktop_startup_state(
        AppState::new(),
        paths,
        3,
        Some(harvester_core::AiAvailability::Unavailable {
            reason: harvester_core::AiUnavailableReason::MissingApiKey,
        }),
        None,
    );
    state = update(
        state,
        Msg::RestoreDesktopView {
            mode: Some(JobListMode::Last24Hours),
            selected_article_url: Some("https://restart.example/0".into()),
            now: date(),
        },
    )
    .0;
    runner.enqueue(effects);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            Instant::now() < deadline,
            "startup saved-view hydration timed out"
        );
        let (next, effects, _) = pump_pre_triage_refresh(state);
        state = next;
        runner.enqueue(effects);
        let (next, effects) = update(state, Msg::tick_at(date()));
        state = next;
        runner.enqueue(effects);
        if state
            .view()
            .desktop_job_list
            .rows
            .iter()
            .filter(|row| row.triage_annotation.is_some() && row.has_summary)
            .count()
            == 2
            && state.selected_job_id().is_some()
        {
            break;
        }
        if let Ok(message) = rx.recv_timeout(Duration::from_millis(5)) {
            let (next, effects) = update(state, message);
            state = next;
            runner.enqueue(effects);
        }
    }
    assert!(state.run_progress().is_none());
    assert!(state.view().right_pane.summary_markdown.is_some());
    state
}

#[test]
fn post_run_and_real_keyless_restart_export_identical_effects_and_archive_bytes() {
    let (_dir, paths) = corpus();
    let (state, records) = canned_run(&paths);
    let mut worker = PersistenceWorker::new(paths.state_path.clone(), paths.blacklist_path.clone());
    worker.enqueue(harvester_core::PersistenceSnapshot::capture(&state));
    worker.shutdown();
    save_briefing_checkpoint(
        &paths.briefing_checkpoint_path,
        Some("2026-10-02T00:00:00Z"),
    )
    .unwrap();
    let (tx, _) = mpsc::channel();
    let runner = EffectRunner::new(
        paths.clone(),
        tx,
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    runner.enqueue(vec![Effect::SaveResults { records }, Effect::FlushResults]);
    drop(runner);
    let post_run = archive(state);
    let before = write_archive(&paths, &post_run);
    let restarted = archive(restart(&paths));
    assert_eq!(post_run, restarted);
    assert_eq!(before, write_archive(&paths, &restarted));
    let text = String::from_utf8(before).unwrap();
    assert!(text.contains("export_schema: 2"));
    assert!(text.contains("Current saved summary"));
    // A window URL absent from the current-key snapshot counts as unavailable.
    // The reducer's stale-key regression covers the omission of such URLs.
    let mut partial = restarted;
    if let Effect::ArchiveRequested {
        ordered_urls,
        priority_snapshot,
        ..
    } = &mut partial
    {
        ordered_urls.retain(|url| url.ends_with("/0"));
        priority_snapshot.remove("https://restart.example/1");
    }
    let partial = String::from_utf8(write_archive(&paths, &partial)).unwrap();
    assert!(partial.contains("window_count: 2"));
    assert!(partial.contains("\"unavailable\":1"));
}

#[test]
fn startup_keyless_metadata_loads_saved_overlays_and_hashes_before_checkpoint() {
    let (_dir, paths) = corpus();
    let (state, records) = canned_run(&paths);
    let mut worker = PersistenceWorker::new(paths.state_path.clone(), paths.blacklist_path.clone());
    worker.enqueue(harvester_core::PersistenceSnapshot::capture(&state));
    worker.shutdown();
    save_briefing_checkpoint(
        &paths.briefing_checkpoint_path,
        Some("2026-10-03T10:00:00Z"),
    )
    .unwrap();
    let (tx, _) = mpsc::channel();
    let runner = EffectRunner::new(
        paths.clone(),
        tx,
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    runner.enqueue(vec![Effect::SaveResults { records }, Effect::FlushResults]);
    drop(runner);
    let state = restart(&paths);
    let view = state.view();
    assert_eq!(view.desktop_job_list.rows.len(), 2);
    assert_eq!(
        view.desktop_job_list.rows[0]
            .triage_annotation
            .as_ref()
            .unwrap()
            .priority,
        4
    );
    let since = update(
        state.clone(),
        Msg::JobListModeSet {
            mode: JobListMode::SinceCheckpoint,
        },
    )
    .0
    .view();
    assert_eq!(since.desktop_job_list.rows.len(), 1);
    if let Effect::ArchiveRequested {
        ordered_urls,
        priority_snapshot,
        ..
    } = archive(state)
    {
        assert_eq!(ordered_urls, ["https://restart.example/1"]);
        assert_eq!(priority_snapshot.len(), 1);
    } else {
        unreachable!()
    }
    // Read the saved overlay using the actual startup effect: no processing run.
    std::fs::create_dir_all(&paths.prompts_dir).unwrap();
    let overlay_dir = paths.prompts_dir.join("article_triage");
    std::fs::create_dir_all(&overlay_dir).unwrap();
    let template = crate::prompt_template_store::PromptTemplateFile {
        schema_version: 1,
        version: 88,
        updated: "2026-10-03".into(),
        prompt_id: "ArticleTriage".into(),
        system_template: "Saved overlay".into(),
        user_template: "{{content}}".into(),
        description: "Fixture".into(),
        expected_format: "json".into(),
    };
    std::fs::write(
        overlay_dir.join("v88.toml"),
        toml::to_string(&template).unwrap(),
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let runner = EffectRunner::new(
        paths,
        tx,
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    runner.enqueue(vec![Effect::LoadLlmMetadata]);
    let Msg::LlmMetadataLoaded {
        active_versions,
        effective_models,
    } = rx.recv_timeout(Duration::from_secs(3)).unwrap()
    else {
        panic!("metadata")
    };
    assert_eq!(
        active_versions,
        PromptRegistry::with_defaults().active_versions_map()
    );
    assert_eq!(effective_models, models());
    assert!(matches!(
        runner
            .prompt_registry
            .read()
            .unwrap()
            .get_effective(PromptId::ArticleTriage, 88),
        Some(harvester_engine::llm::prompt::EffectiveTemplate::Overlay(_))
    ));
}

#[test]
fn optional_view_fields_load_old_state_and_new_state_is_readable_by_old_reader() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".harvester_state.ron");
    std::fs::write(&path, "(completed: [], links_in_store: true)").unwrap();
    let old = load_runtime_hydration(&path, dir.path());
    assert!(old.job_list_mode.is_none());
    assert!(old.selected_article_url.is_none());
    crate::persistence::persist_snapshot_with_notices(
        &path,
        &[],
        &[],
        true,
        Some(JobListMode::Results),
        Some("https://restart.example/0"),
        |_| {},
    )
    .unwrap();
    let new = load_runtime_hydration(&path, dir.path());
    assert_eq!(new.job_list_mode, Some(JobListMode::Results));
    assert_eq!(
        new.selected_article_url.as_deref(),
        Some("https://restart.example/0")
    );
    #[derive(serde::Deserialize)]
    struct OldReader {
        completed: Vec<harvester_core::CompletedJobSnapshot>,
        #[serde(default)]
        pending_intake: Vec<String>,
    }
    let old_reader: OldReader = ron::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(old_reader.completed.is_empty());
    assert!(old_reader.pending_intake.is_empty());
}

#[test]
fn worker_shutdown_flushes_latest_tab_and_selection_and_geometry_save_preserves_them() {
    let dir = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::with_defaults(dir.path().to_path_buf());
    let mut worker = PersistenceWorker::new(paths.state_path.clone(), paths.blacklist_path.clone());
    let mut state = update(
        AppState::new(),
        Msg::RestoreCompletedJobs(vec![harvester_core::CompletedJobSnapshot {
            url: "https://restart.example/0".into(),
            fetched_utc: Some(date().to_rfc3339()),
            tokens: Some(10),
            bytes: Some(100),
            links: vec![],
        }]),
    )
    .0;
    for mode in [
        JobListMode::Results,
        JobListMode::SinceCheckpoint,
        JobListMode::Last24Hours,
    ] {
        state = update(state, Msg::JobListModeSet { mode }).0;
        worker.enqueue(harvester_core::PersistenceSnapshot::capture(&state));
    }
    state = update(state, Msg::JobSelected { job_id: 1 }).0;
    worker.enqueue(harvester_core::PersistenceSnapshot::capture(&state));
    worker.shutdown();
    persist_desktop_window_size(&paths.state_path, 1200, 800);
    let restored = load_runtime_hydration(&paths.state_path, dir.path());
    assert_eq!(restored.job_list_mode, Some(JobListMode::Last24Hours));
    assert_eq!(
        restored.selected_article_url.as_deref(),
        Some("https://restart.example/0")
    );
}
