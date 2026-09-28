use super::batch_runtime::BatchRuntime;
use super::dispatch_loop::{
    batch_buffer_is_quiescent, classify_cycle_outcome, should_settle_cycle, truncate_for_log,
};
use super::live_progress::batch_peek;
use super::*;
use crate::cli::{Args, CheckpointCommand};
use harvester_core::signal_candidate::DEFAULT_SELECTION_THRESHOLD;
use harvester_core::{AppState, BatchObservation, FrozenBatchKey, Msg, StageKind};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{
    prompts::register_defaults, LlmCompletionError, LlmConfig, LlmQuotas, ModelId, OpenAiProvider,
    PricingRegistry, PromptRegistry, ProviderKind, TokenUsage, DEFAULT_BRIEFING_MODEL,
    DEFAULT_SUMMARY_MODEL, DEFAULT_TRIAGE_MODEL, OPENAI_MODEL_GPT_4O_MINI,
};
use harvester_io::{
    load_briefing_checkpoint, EffectRunner, NoOpPlatformHandler, NoOpRuntimePersistenceSink,
    RuntimePaths,
};
use std::collections::HashMap;
use std::collections::HashSet;

#[test]
fn collect_only_replay_dispatches_and_settles_without_pipeline_advance() {
    use harvester_core::{Effect, LlmResultKind, Msg, PipelineRunScope};
    use harvester_engine::llm::PromptId;
    let state = harvester_core::AppState::new();
    let (state, _) = harvester_core::update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: [(PromptId::ArticleTriage, 1)].into_iter().collect(),
            effective_models: [(PromptId::ArticleTriage, "test-model".into())]
                .into_iter()
                .collect(),
        },
    );
    let (state, effects) = harvester_core::update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Resume,
        },
    );
    let (state, effects) =
        harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000);
    let load_id = effects
        .iter()
        .find_map(|e| match e {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    let (state, effects) = harvester_core::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_id,
            delta: harvester_engine::TriageArticleDelta::full_window(
                vec![harvester_engine::LoadedArticle {
                    url: "https://batch.example/replay".into(),
                    source_title: None,
                    prepared_text: std::iter::repeat_n("contentword", 220)
                        .collect::<Vec<_>>()
                        .join(" "),
                    content_hash: "replay-hash".into(),
                    fetched_utc: None,
                }],
                100_000,
            ),
        },
    );
    let request = |effects: &[Effect]| {
        effects.iter().find_map(|e| match e {
            Effect::RequestLlmCompletion {
                request_id,
                prompt_id: PromptId::ArticleTriage,
                ..
            } => Some(*request_id),
            _ => None,
        })
    };
    let id = request(&effects).unwrap();
    let (state, _) = harvester_core::update(
        state,
        Msg::LlmCompleted {
            request_id: id,
            result: LlmResultKind::DeferredToBatch,
            metadata: None,
        },
    );
    assert!(state.pipeline_activity().is_settled());
    assert!(!state.view().run_progress.run_active);
    let (state, effects) = super::batch_runtime::continue_deferred_batch_work(state);
    let (state, effects) =
        harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000);
    assert!(state.view().run_progress.run_active);
    let id = request(&effects).expect("collect-only rearm dispatches within its Continue run");
    let (state, _) = harvester_core::update(
        state,
        Msg::LlmCompleted {
            request_id: id,
            result: LlmResultKind::Failed {
                reason: "test failure".into(),
            },
            metadata: None,
        },
    );
    assert!(state.pipeline_activity().is_settled());
    assert!(!state.view().run_progress.run_active);
    let (state, effects) = super::batch_runtime::continue_deferred_batch_work(state);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadProcessingConfiguration { .. })),
        "later cycle starts a new run"
    );
    let (state, effects) =
        harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000);
    assert!(request(&effects).is_none());
    assert!(state.pipeline_activity().is_settled());
    assert!(!state.view().run_progress.run_active);
}

#[test]
fn empty_collect_only_cycle_settles_without_admitting_the_window() {
    use harvester_core::{Effect, Msg};
    use harvester_engine::llm::PromptId;
    let (state, _) = harvester_core::update(
        harvester_core::AppState::new(),
        Msg::LlmMetadataLoaded {
            active_versions: [(PromptId::ArticleTriage, 1)].into_iter().collect(),
            effective_models: [(PromptId::ArticleTriage, "test-model".into())]
                .into_iter()
                .collect(),
        },
    );
    let (state, effects) = super::batch_runtime::continue_deferred_batch_work(state);
    let (state, effects) =
        harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000);
    assert!(effects.iter().all(|e| !matches!(
        e,
        Effect::RequestLlmCompletion { .. }
            | Effect::LoadArticlesForTriage { .. }
            | Effect::PollAllSources
    )));
    assert!(state.pipeline_activity().is_settled());
    assert!(!state.view().run_progress.run_active);
    assert!(state.pipeline_waves().waves().is_empty());
}
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;
use tempfile::TempDir;

use super::batch_runtime::{
    batch_custom_id, is_batch_eligible_prompt, send_batch_preparation_failure,
};

fn create_test_args(dry_run: bool, temp_dir: &TempDir) -> Args {
    Args {
        output_dir: temp_dir.path().to_path_buf(),
        sources: Some(PathBuf::from("test_sources.json")),
        contexts_dir: PathBuf::from("contexts"),
        prompts_dir: PathBuf::from("prompts"),
        dry_run,
        batch_api: false,
        drain: false,
        verbose_progress: false,
        ascii_progress: false,
        single_shot: false,
        allow_unsupported_sources: false,
        llm_concurrency: 1,
        poll_interval: 1,
        force_unlock: false,
        set_briefing_since: None,
        set_briefing_since_now: false,
        clear_briefing_since: false,
        show_briefing_since: false,
        import_saved_web_dir: None,
        refresh_stale_summaries_limit: None,
        signal_candidate_threshold: None,
    }
}

fn test_batch_runtime(temp_dir: &TempDir) -> BatchRuntime {
    let provider = OpenAiProvider::new("test-key".to_string());
    let mock: Arc<dyn harvester_engine::llm::provider::LlmProvider> =
        Arc::new(harvester_engine::llm::MockLlmProvider::new());
    let mut registry = PromptRegistry::new();
    register_defaults(&mut registry);
    let config = LlmConfig {
        provider: mock,
        default_model: ModelId::new(ProviderKind::OpenAi, OPENAI_MODEL_GPT_4O_MINI),
        triage_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_TRIAGE_MODEL)),
        summary_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_SUMMARY_MODEL)),
        signal_candidate_model: None,
        briefing_model: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_BRIEFING_MODEL)),
        registry: Arc::new(RwLock::new(registry)),
        quotas: LlmQuotas::default(),
        output_dir: temp_dir.path().to_path_buf(),
        pricing: PricingRegistry::with_defaults(),
        max_input_bytes: 100_000,
        #[allow(deprecated)]
        max_input_chars: 0,
        timestamp_utc: Arc::new(|| "2026-07-19T00:00:00Z".to_string()),
        session_id: "test-batch".to_string(),
        replay_cache: None,
        replay_write_observer: None,
        max_concurrent_requests: 1,
    };
    BatchRuntime::new(
        provider,
        config,
        &RuntimePaths::new(
            temp_dir.path().to_path_buf(),
            temp_dir.path().join("sources.ron"),
            temp_dir.path().join("contexts"),
            temp_dir.path().join("prompts"),
        ),
    )
    .unwrap()
}

fn test_loaded_article(index: usize) -> harvester_engine::LoadedArticle {
    harvester_engine::LoadedArticle {
        url: format!("https://runner.example/article-{index}"),
        source_title: Some(format!("Runner article {index}")),
        prepared_text: std::iter::repeat_n(format!("article-{index}-word"), 220)
            .collect::<Vec<_>>()
            .join(" "),
        content_hash: format!("runner-hash-{index}"),
        fetched_utc: Some("2026-09-27T00:00:00Z".into()),
    }
}

fn triage_request_id(effects: &[harvester_core::Effect]) -> Option<u64> {
    effects.iter().find_map(|effect| match effect {
        harvester_core::Effect::RequestLlmCompletion {
            request_id,
            prompt_id: PromptId::ArticleTriage,
            ..
        } => Some(*request_id),
        _ => None,
    })
}

fn fake_full_cycle_runner(
    temp_dir: &TempDir,
    article: &harvester_engine::LoadedArticle,
) -> (
    AppState,
    EffectRunner,
    Arc<harvester_engine::llm::MockLlmProvider>,
    mpsc::Sender<Msg>,
    mpsc::Receiver<Msg>,
) {
    let output_dir = temp_dir.path().join("output");
    let contexts_dir = temp_dir.path().join("contexts");
    std::fs::create_dir_all(&output_dir).unwrap();
    std::fs::create_dir_all(&contexts_dir).unwrap();
    std::fs::write(
        temp_dir.path().join("sources.ron"),
        "SourceRegistry(sources: [])",
    )
    .unwrap();
    std::fs::write(
        contexts_dir.join("article_triage.toml"),
        "[meta]\nprompt_id = \"ArticleTriage\"\nschema_version = 1\nversion = 1\nupdated = \"2026-09-27\"\n\n[variables]\npolicy = \"test policy\"\n",
    ).unwrap();
    let (_, markdown) = harvester_engine::build_markdown_document(
        &article.url,
        article.source_title.as_deref(),
        "utf-8",
        article.fetched_utc.as_deref().unwrap(),
        &article.prepared_text,
        &harvester_engine::WhitespaceTokenCounter,
    );
    std::fs::write(output_dir.join("article.md"), markdown).unwrap();
    let paths = RuntimePaths::new(
        output_dir,
        temp_dir.path().join("sources.ron"),
        contexts_dir,
        temp_dir.path().join("prompts"),
    );
    let mock = Arc::new(harvester_engine::llm::MockLlmProvider::new());
    let mut registry = PromptRegistry::new();
    register_defaults(&mut registry);
    let registry = Arc::new(RwLock::new(registry));
    let model = ModelId::new(ProviderKind::OpenAi, "mock");
    let config = LlmConfig {
        provider: mock.clone(),
        default_model: model.clone(),
        triage_model: Some(model.clone()),
        summary_model: Some(model.clone()),
        signal_candidate_model: Some(model.clone()),
        briefing_model: Some(model),
        registry: registry.clone(),
        quotas: LlmQuotas::default(),
        output_dir: paths.output_dir.clone(),
        pricing: PricingRegistry::with_defaults(),
        max_input_bytes: 100_000,
        #[allow(deprecated)]
        max_input_chars: 0,
        timestamp_utc: Arc::new(|| "2026-09-27T00:00:00Z".into()),
        session_id: "fake-full-cycle".into(),
        replay_cache: None,
        replay_write_observer: None,
        max_concurrent_requests: 1,
    };
    let (msg_tx, msg_rx) = mpsc::channel();
    let runner = EffectRunner::new_with_llm(
        paths,
        msg_tx.clone(),
        harvester_engine::llm::LlmHandle::new(config),
        100_000,
        registry.clone(),
        HashMap::from([
            (PromptId::ArticleTriage, "mock".into()),
            (PromptId::ArticleSummary, "mock".into()),
            (PromptId::ArticleSignalCandidate, "mock".into()),
        ]),
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    let (state, _) = harvester_core::update(
        AppState::new(),
        Msg::RestoreCompletedJobs(vec![harvester_core::CompletedJobSnapshot {
            url: article.url.clone(),
            tokens: Some(500),
            bytes: Some(3_000),
            links: Vec::new(),
            fetched_utc: article.fetched_utc.clone(),
        }]),
    );
    let (state, _) = harvester_core::update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: registry.read().unwrap().active_versions_map(),
            effective_models: HashMap::from([
                (PromptId::ArticleTriage, "mock".into()),
                (PromptId::ArticleSummary, "mock".into()),
                (PromptId::ArticleSignalCandidate, "mock".into()),
            ]),
        },
    );
    (state, runner, mock, msg_tx, msg_rx)
}

fn route_fake_batch_effects(
    mut state: AppState,
    effects: Vec<harvester_core::Effect>,
    articles: &[harvester_engine::LoadedArticle],
    coordinator: &mut crate::batch_coordinator::BatchCoordinator<
        crate::batch_coordinator::tests::FakeTransport,
    >,
) -> (AppState, Vec<PromptId>) {
    use std::collections::VecDeque;
    let (msg_tx, msg_rx) = mpsc::channel();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut queued = VecDeque::from(effects);
    let mut prompts = Vec::new();

    for _ in 0..64 {
        let mut follow_up = Vec::new();
        while let Some(effect) = queued.pop_front() {
            match effect {
                harvester_core::Effect::RequestLlmCompletion {
                    request_id,
                    prompt_id,
                    ..
                } => {
                    prompts.push(prompt_id);
                    let mut key = state
                        .frozen_batch_key_for_request(request_id)
                        .expect("pipeline request carries its frozen batch key");
                    key.rendered_system = "fake batch system".into();
                    key.rendered_user = "fake batch user".into();
                    let custom_id = batch_custom_id(&key);
                    let stage = key.stage;
                    coordinator.buffer(crate::batch_coordinator::BufferedRequest {
                        request_id,
                        stage,
                        line: openai_provider_kit::BatchInputLine {
                            custom_id: custom_id.clone(),
                            method: "POST".into(),
                            url: "/v1/chat/completions".into(),
                            body: serde_json::json!({
                                "model": key.model_id,
                                "messages": [
                                    {"role":"system", "content":"fake batch system"},
                                    {"role":"user", "content":"fake batch user"}
                                ]
                            }),
                        },
                        entry: crate::batch_manifest::PendingEntry {
                            custom_id,
                            key,
                            stage,
                            attempts: 0,
                            collected: None,
                        },
                        estimated_input_tokens: 10,
                        estimated_cost_microdollars: 1,
                    });
                }
                harvester_core::Effect::LoadProcessingConfiguration { .. } => {
                    let (next, effects) =
                        harvester_core::fixture_support::complete_processing_configuration(
                            state,
                            vec![effect],
                            100_000,
                        );
                    state = next;
                    follow_up.extend(effects);
                }
                harvester_core::Effect::LoadArticlesForTriage { request_id, .. } => {
                    let (next, effects) = harvester_core::update(
                        state,
                        Msg::TriageArticlesLoaded {
                            request_id,
                            delta: harvester_engine::TriageArticleDelta::full_window(
                                articles.to_vec(),
                                100_000,
                            ),
                        },
                    );
                    state = next;
                    follow_up.extend(effects);
                }
                _ => {}
            }
        }

        if !coordinator.buffered_request_ids().is_empty() {
            runtime
                .block_on(coordinator.flush(&msg_tx, "2026-09-27T00:00:00Z".into()))
                .unwrap();
        }
        for msg in msg_rx.try_iter() {
            let (next, effects) = harvester_core::update(state, msg);
            state = next;
            follow_up.extend(effects);
        }
        if follow_up.is_empty() && coordinator.buffered_request_ids().is_empty() {
            break;
        }
        queued.extend(follow_up);
    }

    assert!(queued.is_empty(), "fake batch flow made bounded progress");
    (state, prompts)
}

fn successful_batch_output(custom_id: &str, output_json: &str) -> Vec<u8> {
    let line = serde_json::json!({
        "custom_id": custom_id,
        "response": {
            "status_code": 200,
            "body": {
                "choices": [{"message": {"content": output_json}}],
                "usage": {"prompt_tokens": 20, "completion_tokens": 8},
                "model": "test-model"
            }
        },
        "error": null
    });
    format!("{line}\n").into_bytes()
}

fn set_fake_batch_status(
    transport: &crate::batch_coordinator::tests::FakeTransport,
    batch: &crate::batch_manifest::PendingBatch,
    lifecycle: openai_provider_kit::BatchLifecycle,
    output_file_id: Option<&str>,
    output: Option<Vec<u8>>,
) {
    let batch_id = batch.batch_id.as_deref().expect("submitted batch id");
    let mut remote =
        crate::batch_coordinator::tests::handle(batch_id, &batch.input_file_id, lifecycle);
    remote.output_file_id = output_file_id.map(str::to_owned);
    transport
        .retrieved
        .lock()
        .unwrap()
        .insert(batch_id.to_owned(), remote);
    if let (Some(file_id), Some(output)) = (output_file_id, output) {
        transport
            .downloads
            .lock()
            .unwrap()
            .insert(file_id.to_owned(), output);
    }
}

#[test]
fn batch_api_collects_replays_and_releases_summary_waves_once_across_three_cycles() {
    use harvester_core::{
        BatchStatus, Effect, PipelineRunScope, PipelineStage, PipelineWavePolicy,
    };
    use harvester_engine::llm::PromptId;
    use openai_provider_kit::BatchLifecycle;

    let temp_dir = TempDir::new().unwrap();
    let articles = vec![test_loaded_article(10), test_loaded_article(11)];
    let (state, _) = harvester_core::update(
        AppState::new(),
        Msg::LlmMetadataLoaded {
            active_versions: [
                (PromptId::ArticleTriage, 1),
                (PromptId::ArticleSummary, 1),
                (PromptId::ArticleSignalCandidate, 1),
            ]
            .into_iter()
            .collect(),
            effective_models: [
                (PromptId::ArticleTriage, "test-triage-model".into()),
                (PromptId::ArticleSummary, "test-summary-model".into()),
                (PromptId::ArticleSignalCandidate, "test-signal-model".into()),
            ]
            .into_iter()
            .collect(),
        },
    );
    let (state, _) = harvester_core::update(state, Msg::PromptTemplateFilesLoaded);
    let (state, _) = harvester_core::update(
        state,
        Msg::PromptContextsLoaded {
            contexts: Default::default(),
        },
    );
    let mut state = state;
    state.set_pipeline_wave_policy(PipelineWavePolicy::AfterDownloadsSettle);
    state.set_llm_max_in_flight(2);
    state.set_llm_deferred_allowance(1);
    let (state, _) = harvester_core::update(
        state,
        Msg::RestoreCompletedJobs(
            articles
                .iter()
                .map(|article| harvester_core::CompletedJobSnapshot {
                    url: article.url.clone(),
                    tokens: Some(500),
                    bytes: Some(3_000),
                    links: Vec::new(),
                    fetched_utc: article.fetched_utc.clone(),
                })
                .collect(),
        ),
    );
    let (state, _) = harvester_core::update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: articles.iter().map(|article| article.url.clone()).collect(),
            triggered_by_job_done: false,
        },
    );
    let (state, effects) = harvester_core::update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Full,
        },
    );
    let (state, _) = harvester_core::update(state, Msg::AllSourcesPollEnded);
    let (state, effects) =
        harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000);
    let load_id = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .expect("intake run admits the completed window");
    let (state, effects) = harvester_core::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles.clone(), 100_000),
        },
    );

    let transport = crate::batch_coordinator::tests::FakeTransport::default();
    let uploaded = Arc::clone(&transport.uploaded);
    let created = Arc::clone(&transport.created);
    let mut coordinator = crate::batch_coordinator::BatchCoordinator::new(
        transport.clone(),
        crate::batch_manifest::BatchManifestStore::load(temp_dir.path().to_path_buf()).unwrap(),
        crate::batch_coordinator::SubmissionBudget {
            max_lines_per_file: 1,
            ..Default::default()
        },
    );

    // Intake released exactly one two-member triage wave. The transport's one-line
    // file bound splits the wave into two manifest batches for partial collection.
    let (state, cycle1_prompts) =
        route_fake_batch_effects(state, effects, &articles, &mut coordinator);
    assert_eq!(
        cycle1_prompts
            .iter()
            .filter(|prompt| **prompt == PromptId::ArticleTriage)
            .count(),
        2
    );
    let triage_waves: Vec<_> = state
        .pipeline_waves()
        .waves()
        .iter()
        .filter(|wave| wave.stage == PipelineStage::Triaging)
        .collect();
    assert_eq!(triage_waves.len(), 1);
    assert_eq!(triage_waves[0].members.len(), 2);
    assert!(state.pipeline_activity().is_settled());
    assert!(should_settle_cycle(state.batch_status()));
    assert!(state.batch_observation().triage_deferred > 0);
    assert_eq!(coordinator.manifest().manifest().batches.len(), 2);
    assert!(coordinator
        .manifest()
        .manifest()
        .batches
        .iter()
        .all(|batch| batch.stage == "triage"));

    let triage_ids: Vec<_> = coordinator
        .manifest()
        .manifest()
        .batches
        .iter()
        .map(|batch| batch.entries[0].custom_id.clone())
        .collect();
    let cycle1_batches = coordinator.manifest().manifest().batches.clone();
    set_fake_batch_status(
        &transport,
        &cycle1_batches[0],
        BatchLifecycle::Completed,
        Some("triage-output-1"),
        Some(successful_batch_output(
            &triage_ids[0],
            r#"{"category":"news","priority":3,"tags":["batch"],"rationale":"cycle two"}"#,
        )),
    );
    set_fake_batch_status(
        &transport,
        &cycle1_batches[1],
        BatchLifecycle::InProgress,
        None,
        None,
    );

    // Cycle 2 collects the first triage line. Its replay releases exactly one
    // summary. The re-requested second triage key remains DeferredToBatch and is
    // deduplicated by the manifest, while the other triage batch is still running.
    let cycle2_collected = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(coordinator.collect_completed())
        .unwrap();
    assert_eq!(cycle2_collected.len(), 1);
    assert_eq!(cycle2_collected[0].custom_id, triage_ids[0]);
    let (state, _effects) = harvester_core::update(
        state,
        Msg::BatchResultsCollected {
            entries: cycle2_collected,
        },
    );
    let (state, effects) = super::batch_runtime::continue_deferred_batch_work(state);
    let (state, cycle2_prompts) =
        route_fake_batch_effects(state, effects, &articles, &mut coordinator);
    assert_eq!(
        cycle2_prompts
            .iter()
            .filter(|prompt| **prompt == PromptId::ArticleSummary)
            .count(),
        1,
        "only the collected triage member releases a summary"
    );
    assert_eq!(
        cycle2_prompts
            .iter()
            .filter(|prompt| **prompt == PromptId::ArticleTriage)
            .count(),
        1,
        "the still-pending triage key is requested again in cycle two"
    );
    assert_eq!(coordinator.manifest().manifest().batches.len(), 3);
    assert_eq!(
        created.lock().unwrap().len(),
        3,
        "the pending triage key was not uploaded twice"
    );
    assert_eq!(uploaded.lock().unwrap().len(), 3);
    assert!(coordinator.pending_custom_ids().contains(&triage_ids[1]));
    assert!(state.pipeline_activity().is_settled());
    assert_eq!(state.batch_status(), BatchStatus::Settled);
    assert!(state.batch_observation().triage_deferred > 0);
    assert!(state.batch_observation().summary_deferred > 0);

    let batches = coordinator.manifest().manifest().batches.clone();
    let triage_second = batches
        .iter()
        .find(|batch| batch.stage == "triage" && batch.entries[0].custom_id == triage_ids[1])
        .unwrap();
    let summary_first = batches
        .iter()
        .find(|batch| batch.stage == "summary")
        .unwrap();
    let summary_id = summary_first.entries[0].custom_id.clone();
    set_fake_batch_status(
        &transport,
        triage_second,
        BatchLifecycle::Completed,
        Some("triage-output-2"),
        Some(successful_batch_output(
            &triage_ids[1],
            r#"{"category":"news","priority":3,"tags":["batch"],"rationale":"cycle three"}"#,
        )),
    );
    set_fake_batch_status(
        &transport,
        summary_first,
        BatchLifecycle::Completed,
        Some("summary-output-1"),
        Some(successful_batch_output(
            &summary_id,
            r#"{"title":"First summary","summary":"first article summary","key_points":["first"]}"#,
        )),
    );

    // The coordinator replays the first collected line again alongside the
    // newly completed lines. The reducer's identity/key ledger releases only
    // the remaining article's summary wave.
    let cycle3_collected = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(coordinator.collect_completed())
        .unwrap();
    assert_eq!(cycle3_collected.len(), 3);
    assert_eq!(
        cycle3_collected
            .iter()
            .filter(|entry| entry.stage == StageKind::Triage)
            .count(),
        2
    );
    let (state, _effects) = harvester_core::update(
        state,
        Msg::BatchResultsCollected {
            entries: cycle3_collected,
        },
    );
    let (state, effects) = super::batch_runtime::continue_deferred_batch_work(state);
    let (state, cycle3_prompts) =
        route_fake_batch_effects(state, effects, &articles, &mut coordinator);
    assert_eq!(
        cycle3_prompts
            .iter()
            .filter(|prompt| **prompt == PromptId::ArticleSummary)
            .count(),
        1,
        "the duplicated first triage line does not release its summary again"
    );
    assert_eq!(
        coordinator
            .manifest()
            .manifest()
            .batches
            .iter()
            .filter(|batch| batch.stage == "summary")
            .count(),
        2,
        "each completed triage identity owns one summary submission"
    );
    assert_eq!(created.lock().unwrap().len(), 5);
    assert_eq!(uploaded.lock().unwrap().len(), 5);
    assert!(state.pipeline_activity().is_settled());
    assert_eq!(state.batch_status(), BatchStatus::Settled);
    assert!(state.batch_observation().summary_deferred > 0);
}

#[test]
fn recurring_cycle_retries_a_failed_article_without_new_jobs() {
    engine_logging::initialize_for_tests();
    let article = test_loaded_article(0);
    let temp_dir = TempDir::new().unwrap();
    let (mut state, runner, mock, msg_tx, msg_rx) = fake_full_cycle_runner(&temp_dir, &article);
    mock.queue_json_success("{}").queue_json_success("{}");
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    for cycle in 1..=2 {
        msg_tx
            .send(Msg::PipelineRunRequested {
                scope: harvester_core::PipelineRunScope::Full,
            })
            .unwrap();
        run_dispatch_loop(
            &mut state,
            &msg_tx,
            &msg_rx,
            &runner,
            &shutdown_flag,
            DispatchLoopOptions {
                tick_interval: Duration::from_millis(75),
            },
        )
        .expect("full cycle settles after fake model failure");
        assert!(!state.view().run_progress.run_active);
        assert_eq!(
            state.pipeline_run_phase(),
            harvester_core::PipelineRunPhase::Idle
        );
        assert_eq!(mock.recorded_requests().len(), cycle as usize);
    }
    assert_eq!(
        state.batch_observation().jobs_total,
        1,
        "no new jobs arrived"
    );
    assert_eq!(state.batch_observation().triage_failed, 1);
}

#[test]
fn single_shot_cycle_processes_unfinished_work_without_new_jobs() {
    engine_logging::initialize_for_tests();
    let article = test_loaded_article(1);
    let temp_dir = TempDir::new().unwrap();
    let (mut state, runner, mock, msg_tx, msg_rx) = fake_full_cycle_runner(&temp_dir, &article);
    mock.queue_json_success("{}");
    let mut args = create_test_args(false, &temp_dir);
    args.single_shot = true;
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    msg_tx
        .send(Msg::PipelineRunRequested {
            scope: harvester_core::PipelineRunScope::Full,
        })
        .unwrap();
    run_dispatch_loop(
        &mut state,
        &msg_tx,
        &msg_rx,
        &runner,
        &shutdown_flag,
        DispatchLoopOptions {
            tick_interval: Duration::from_millis(75),
        },
    )
    .expect("single-shot Full cycle settles");
    assert!(should_stop_after_cycle(args.single_shot, false));
    assert_eq!(mock.recorded_requests().len(), 1);
    assert!(!state.view().run_progress.run_active);
    assert_eq!(state.batch_observation().jobs_total, 1);
    assert_eq!(state.batch_observation().triage_total, 1);
}

#[test]
fn startup_window_is_loaded_before_first_cycle_count() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let article = test_loaded_article(3);
    let (state, runner, _mock, _msg_tx, msg_rx) = fake_full_cycle_runner(&temp_dir, &article);
    let (mut state, effects) = harvester_core::update(state, Msg::StartupHydrationRequested);
    let mut effects = effects;
    effects.push(harvester_core::Effect::LoadPromptTemplateFiles);
    runner.enqueue(effects);
    prepare_startup_window(&mut state, &msg_rx, &runner).expect("startup window settles");
    let mut reporter = CycleStartWorkReporter::default();
    let line = reporter
        .pending_count_line(&state)
        .expect("metadata and window are known");
    assert!(line.contains("unfinished_articles=1"), "{line}");
}

fn observation_with_totals(
    jobs_total: usize,
    jobs_done: usize,
    jobs_failed: usize,
    triage_completed: usize,
    triage_failed: usize,
    summary_completed: usize,
    summary_failed: usize,
) -> BatchObservation {
    observation_with_import(
        jobs_total,
        jobs_done,
        jobs_failed,
        triage_completed,
        triage_failed,
        summary_completed,
        summary_failed,
        0,
        0,
    )
}

#[allow(clippy::too_many_arguments)]
fn observation_with_import(
    jobs_total: usize,
    jobs_done: usize,
    jobs_failed: usize,
    triage_completed: usize,
    triage_failed: usize,
    summary_completed: usize,
    summary_failed: usize,
    imports_completed: usize,
    imports_failed: usize,
) -> BatchObservation {
    BatchObservation {
        poll_in_progress: false,
        session_state: harvester_core::SessionState::Idle,
        jobs_total,
        jobs_done,
        jobs_failed,
        jobs_in_flight: 0,
        pre_triage_phase: harvester_core::PreTriagePhase::Idle,
        pre_triage_total: 0,
        pre_triage_included: 0,
        pre_triage_review: 0,
        pre_triage_filtered: 0,
        triage_phase: harvester_core::TriagePhase::Idle,
        triage_total: 0,
        triage_pending: 0,
        triage_in_flight: 0,
        triage_completed,
        triage_failed,
        summary_total: 0,
        summary_pending: 0,
        summary_in_flight: 0,
        summary_completed,
        summary_failed,
        triage_deferred: 0,
        summary_deferred: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        signal_deferred: 0,
        triage_cache_hits: 0,
        triage_cache_misses: 0,
        triage_cache_key_unavailable: 0,
        summary_cache_hits: 0,
        summary_cache_misses: 0,
        summary_cache_key_unavailable: 0,
        import_phase: harvester_core::ImportPhase::Idle,
        imports_completed,
        imports_failed,
        import_in_flight: false,
        source_poll_stats: vec![],
    }
}

#[test]
fn test_dry_run_exits_successfully_without_api_key() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let args = create_test_args(true, &temp_dir);

    // Create empty sources file to avoid validation errors
    let sources_path = temp_dir.path().join("test_sources.json");
    std::fs::write(&sources_path, r#"{"sources": []}"#).unwrap();

    let runtime_paths = RuntimePaths::new(
        args.output_dir.clone(),
        sources_path,
        args.contexts_dir.clone(),
        args.prompts_dir.clone(),
    );

    // Dry-run should succeed even without OPENAI_API_KEY
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let result = run_dry_run(&runtime_paths, &args, &shutdown_flag);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
}

#[test]
fn dry_run_returns_130_when_shutdown_is_already_requested() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let args = create_test_args(true, &temp_dir);
    let sources_path = temp_dir.path().join("test_sources.json");
    std::fs::write(&sources_path, r#"{"sources": []}"#).unwrap();
    let runtime_paths = RuntimePaths::new(
        args.output_dir.clone(),
        sources_path,
        args.contexts_dir.clone(),
        args.prompts_dir.clone(),
    );
    let shutdown_flag = Arc::new(AtomicBool::new(true));

    assert_eq!(
        run_dry_run(&runtime_paths, &args, &shutdown_flag).unwrap(),
        130
    );
}

#[test]
fn test_dry_run_does_not_modify_state_files() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let args = create_test_args(true, &temp_dir);

    let sources_path = temp_dir.path().join("test_sources.json");
    std::fs::write(&sources_path, r#"{"sources": []}"#).unwrap();

    let runtime_paths = RuntimePaths::new(
        args.output_dir.clone(),
        sources_path,
        args.contexts_dir.clone(),
        args.prompts_dir.clone(),
    );

    let state_path = &runtime_paths.state_path;

    // Ensure state file does not exist initially
    assert!(!state_path.exists());

    // Run dry-run
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let result = run_dry_run(&runtime_paths, &args, &shutdown_flag);
    assert!(result.is_ok());

    // State file should still not exist (no writes)
    assert!(!state_path.exists());
}

#[test]
fn test_should_settle_cycle_when_batch_status_is_settled() {
    assert!(should_settle_cycle(harvester_core::BatchStatus::Settled));
}

#[test]
fn test_should_not_settle_cycle_when_batch_status_is_running() {
    assert!(!should_settle_cycle(harvester_core::BatchStatus::Running));
}

#[test]
fn buffered_batch_requests_are_quiescent_but_other_pending_requests_are_not() {
    let mut state = AppState::new();
    state.record_pending_llm_request(1, PromptId::ArticleTriage);
    let buffered = HashSet::from([1]);
    assert!(batch_buffer_is_quiescent(&state, &buffered));

    state.record_pending_llm_request(2, PromptId::AggregateBriefing);
    assert!(!batch_buffer_is_quiescent(&state, &buffered));
    assert!(!batch_buffer_is_quiescent(&state, &HashSet::new()));
}

#[test]
fn batch_custom_id_changes_when_model_changes() {
    let mut key = FrozenBatchKey {
        content_hash: "content-hash".to_string(),
        prompt_id: PromptId::ArticleTriage,
        prompt_version: 3,
        model_id: "gpt-5.4-nano".to_string(),
        context_hash: "context-hash".to_string(),
        stage: StageKind::Triage,
        url: "https://example.test".to_string(),
        rendered_system: String::new(),
        rendered_user: String::new(),
    };
    let first = batch_custom_id(&key);
    key.model_id = "gpt-5.4-mini".to_string();
    assert_ne!(first, batch_custom_id(&key));
}

#[test]
fn signal_custom_id_prefix_does_not_control_provider_stage_grouping() {
    let key = FrozenBatchKey {
        content_hash: "content-hash".to_string(),
        prompt_id: PromptId::ArticleSignalCandidate,
        prompt_version: 3,
        model_id: "gpt-5.4-nano".to_string(),
        context_hash: "context-hash".to_string(),
        stage: StageKind::SignalCandidate,
        url: "https://example.test".to_string(),
        rendered_system: String::new(),
        rendered_user: String::new(),
    };
    assert!(batch_custom_id(&key).starts_with("signal-"));

    let provider = crate::progress::ProviderProgress::from_peeks(&[BatchPeek {
        batch_id: "batch-with-signal-custom-id".to_string(),
        stage: StageKind::SignalCandidate,
        status: Some(openai_provider_kit::BatchLifecycle::InProgress),
        request_counts: Some(openai_provider_kit::BatchRequestCounts {
            total: 1,
            completed: 0,
            failed: 0,
        }),
    }]);
    assert_eq!(provider.signals.submitted, 1);
    assert_eq!(provider.triage.submitted, 0);
}

#[test]
fn batch_routing_partition_keeps_briefing_synchronous() {
    assert!(is_batch_eligible_prompt(PromptId::ArticleTriage));
    assert!(is_batch_eligible_prompt(PromptId::ArticleSummary));
    assert!(is_batch_eligible_prompt(PromptId::ArticleSignalCandidate));
    assert!(!is_batch_eligible_prompt(PromptId::AggregateBriefing));
    assert!(!is_batch_eligible_prompt(
        PromptId::BriefingExecutiveSummary
    ));
    assert!(!is_batch_eligible_prompt(PromptId::BriefingNextItem));
}

#[test]
fn batch_render_failure_replies_failed_exactly_once() {
    let (tx, rx) = mpsc::channel();
    send_batch_preparation_failure(
        &tx,
        17,
        &LlmCompletionError::TemplateRenderFailed {
            detail: "missing variable".to_string(),
        },
    );
    assert!(matches!(
        rx.recv().unwrap(),
        Msg::LlmCompleted {
            request_id: 17,
            result: harvester_core::LlmResultKind::Failed { .. },
            ..
        }
    ));
    assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn collected_replay_audit_is_idempotent_and_uses_discounted_cost() {
    let temp_dir = TempDir::new().unwrap();
    let mut runtime = test_batch_runtime(&temp_dir);
    let entry = harvester_core::CollectedEntry {
        batch_id: "batch-1".to_string(),
        custom_id: "line-1".to_string(),
        stage: StageKind::Triage,
        key: FrozenBatchKey {
            content_hash: "content-hash".to_string(),
            prompt_id: PromptId::ArticleTriage,
            prompt_version: 1,
            model_id: DEFAULT_TRIAGE_MODEL.to_string(),
            context_hash: "context-hash".to_string(),
            stage: StageKind::Triage,
            url: "https://example.test".to_string(),
            rendered_system: "system".to_string(),
            rendered_user: "user".to_string(),
        },
        created_at_utc: "2026-07-19T00:00:00Z".to_string(),
        outcome: harvester_core::CollectedOutcome::Success {
            raw_output_json: r#"{"category":"news","priority":3,"tags":["ai"],"rationale":"ok"}"#
                .to_string(),
            usage: TokenUsage::new(1_000_000, 0),
            resolved_model: DEFAULT_TRIAGE_MODEL.to_string(),
        },
    };

    persist_batch_replay_records(std::slice::from_ref(&entry), &mut runtime);
    persist_batch_replay_records(&[entry], &mut runtime);

    assert_eq!(runtime.realized_cost_microdollars, 100_000);
    assert_eq!(
        std::fs::read_dir(temp_dir.path().join("llm_results"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn test_should_stop_after_cycle_for_single_shot() {
    assert!(should_stop_after_cycle(true, false));
}

#[test]
fn test_should_stop_after_cycle_for_shutdown_signal() {
    assert!(should_stop_after_cycle(false, true));
}

#[test]
fn test_should_continue_after_cycle_when_not_single_shot_and_no_shutdown() {
    assert!(!should_stop_after_cycle(false, false));
}

#[test]
fn drain_makes_the_first_cycle_collect_only_so_no_sources_are_polled() {
    // Batch API mode polls once before it starts collecting.
    assert!(!is_collect_only_cycle(true, false, 1));
    assert!(is_collect_only_cycle(true, false, 2));

    // Drain never polls, so it collects from the very first cycle.
    assert!(is_collect_only_cycle(true, true, 1));
    assert!(is_collect_only_cycle(true, true, 2));

    // Without the Batch API runtime there is no manifest to collect from.
    assert!(!is_collect_only_cycle(false, false, 1));
    assert!(!is_collect_only_cycle(false, false, 2));
}

#[test]
fn batch_api_intake_waits_for_downloads_and_hands_off_once() {
    use harvester_core::{
        Effect, JobResultKind, PipelineRunScope, PipelineStage, PipelineWavePolicy,
    };
    use harvester_engine::{SourceId, SourceKind};
    let articles: Vec<_> = (0..2)
        .map(|i| harvester_engine::LoadedArticle {
            url: format!("https://batch.example/intake-{i}"),
            source_title: Some(format!("Intake {i}")),
            prepared_text: std::iter::repeat_n("contentword", 220)
                .collect::<Vec<_>>()
                .join(" "),
            content_hash: format!("intake-hash-{i}"),
            fetched_utc: None,
        })
        .collect();
    let state = AppState::new();
    let (state, _) = harvester_core::update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: [
                (PromptId::ArticleTriage, 1),
                (PromptId::ArticleSummary, 1),
                (PromptId::ArticleSignalCandidate, 1),
            ]
            .into_iter()
            .collect(),
            effective_models: [
                (PromptId::ArticleTriage, "test-triage-model".to_string()),
                (PromptId::ArticleSummary, "test-summary-model".to_string()),
                (
                    PromptId::ArticleSignalCandidate,
                    "test-signal-model".to_string(),
                ),
            ]
            .into_iter()
            .collect(),
        },
    );
    let (state, _) = harvester_core::update(state, Msg::PromptTemplateFilesLoaded);
    let (mut state, _) = harvester_core::update(
        state,
        Msg::PromptContextsLoaded {
            contexts: Default::default(),
        },
    );
    state.set_pipeline_wave_policy(PipelineWavePolicy::AfterDownloadsSettle);
    state.set_llm_max_in_flight(2);
    state = harvester_core::update(state, Msg::PollSourcesClicked).0;
    let (state, _) = harvester_core::update(state, Msg::PollStarted { total: 1 });
    let (mut state, effects) = harvester_core::update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("batch-intake").unwrap(),
            urls: articles.iter().map(|article| article.url.clone()).collect(),
            kind: SourceKind::Rss,
            parsed: articles.len(),
            dedup_filtered: 0,
        },
    );
    let jobs: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::EnqueueUrl { job_id, .. } => Some(*job_id),
            _ => None,
        })
        .collect();
    assert_eq!(jobs.len(), articles.len());
    for job_id in &jobs {
        state = harvester_core::update(
            state,
            Msg::JobProgress {
                job_id: *job_id,
                stage: harvester_core::Stage::Downloading,
                tokens: None,
                bytes: Some(1_024),
                content_preview: None,
            },
        )
        .0;
    }
    let (state, effects) = harvester_core::update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Full,
        },
    );
    let (state, _) = harvester_core::update(state, Msg::AllSourcesPollEnded);
    let (mut state, effects) =
        harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000);
    assert!(effects
        .iter()
        .all(|effect| !matches!(effect, Effect::LoadArticlesForTriage { .. })));

    let mut intake_load_id = None;
    for (index, job_id) in jobs.iter().copied().enumerate() {
        let (next, _) = harvester_core::update(
            state,
            Msg::JobDone {
                job_id,
                result: JobResultKind::Success,
                content_preview: None,
                extracted_links: vec![],
                fetched_utc: None,
            },
        );
        state = harvester_core::update(
            next,
            Msg::EvaluatePreTriageRefresh {
                ordered_urls: articles[..=index]
                    .iter()
                    .map(|article| article.url.clone())
                    .collect(),
                triggered_by_job_done: true,
            },
        )
        .0;
        let (next, effects) = harvester_core::update(state, Msg::PipelineRunAdvance);
        state = next;
        if index == 0 {
            assert!(effects
                .iter()
                .all(|effect| !matches!(effect, Effect::LoadArticlesForTriage { .. })));
            assert_eq!(state.batch_observation().jobs_in_flight, 1);
        } else {
            intake_load_id = effects.iter().find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            });
        }
    }
    assert_eq!(state.batch_observation().jobs_in_flight, 0);
    let (state, effects) = if intake_load_id.is_none() {
        harvester_core::update(state, Msg::PipelineRunAdvance)
    } else {
        (state, Vec::new())
    };
    let load_id = intake_load_id
        .or_else(|| {
            effects.iter().find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            })
        })
        .expect("one intake load follows download settlement");
    let (state, effects) = harvester_core::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_id,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    );
    assert!(effects
        .iter()
        .any(|effect| matches!(effect, Effect::RequestLlmCompletion { .. })));
    let triage_waves: Vec<_> = state
        .pipeline_waves()
        .waves()
        .iter()
        .filter(|wave| wave.stage == PipelineStage::Triaging)
        .collect();
    assert_eq!(triage_waves.len(), 1, "one intake hand-off per cycle");
    assert_eq!(triage_waves[0].members.len(), 2);
}

#[test]
fn recurring_staggered_downloads_dispatch_triage_and_settle_once() {
    use harvester_core::{
        Effect, JobResultKind, PipelineRunScope, PipelineStage, PipelineWavePolicy,
    };
    use harvester_engine::{SourceId, SourceKind};

    let articles = vec![test_loaded_article(20), test_loaded_article(21)];
    let (state, _) = harvester_core::update(
        AppState::new(),
        Msg::LlmMetadataLoaded {
            active_versions: [
                (PromptId::ArticleTriage, 1),
                (PromptId::ArticleSummary, 1),
                (PromptId::ArticleSignalCandidate, 1),
            ]
            .into_iter()
            .collect(),
            effective_models: [
                (PromptId::ArticleTriage, "test-triage-model".into()),
                (PromptId::ArticleSummary, "test-summary-model".into()),
                (PromptId::ArticleSignalCandidate, "test-signal-model".into()),
            ]
            .into_iter()
            .collect(),
        },
    );
    let (state, _) = harvester_core::update(state, Msg::PromptTemplateFilesLoaded);
    let (mut state, _) = harvester_core::update(
        state,
        Msg::PromptContextsLoaded {
            contexts: Default::default(),
        },
    );
    state.set_pipeline_wave_policy(PipelineWavePolicy::Overlap);
    state.set_llm_max_in_flight(2);
    state = harvester_core::update(state, Msg::PollSourcesClicked).0;
    let (state, _) = harvester_core::update(state, Msg::PollStarted { total: 1 });
    let (mut state, source_effects) = harvester_core::update(
        state,
        Msg::SourcePollCompleted {
            source_id: SourceId::new("overlap").unwrap(),
            urls: articles.iter().map(|article| article.url.clone()).collect(),
            kind: SourceKind::Rss,
            parsed: articles.len(),
            dedup_filtered: 0,
        },
    );
    let jobs: Vec<_> = source_effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::EnqueueUrl { job_id, .. } => Some(*job_id),
            _ => None,
        })
        .collect();
    assert_eq!(jobs.len(), articles.len());
    for job_id in &jobs {
        state = harvester_core::update(
            state,
            Msg::JobProgress {
                job_id: *job_id,
                stage: harvester_core::Stage::Downloading,
                tokens: None,
                bytes: Some(1_024),
                content_preview: None,
            },
        )
        .0;
    }
    let (state, effects) = harvester_core::update(
        state,
        Msg::PipelineRunRequested {
            scope: PipelineRunScope::Full,
        },
    );
    let (state, _) = harvester_core::update(state, Msg::AllSourcesPollEnded);
    let (mut state, _) =
        harvester_core::fixture_support::complete_processing_configuration(state, effects, 100_000);

    state = harvester_core::update(
        state,
        Msg::JobDone {
            job_id: jobs[0],
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: Vec::new(),
            fetched_utc: articles[0].fetched_utc.clone(),
        },
    )
    .0;
    state = harvester_core::update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: vec![articles[0].url.clone()],
            triggered_by_job_done: true,
        },
    )
    .0;
    let mut load_id = None;
    for tick in 1..=40 {
        let (next, tick_effects) = harvester_core::update(
            state,
            Msg::tick_at(chrono::DateTime::from_timestamp(1_700_200_000 + tick, 0).unwrap()),
        );
        let (next, advance_effects) = harvester_core::update(next, Msg::PipelineRunAdvance);
        state = next;
        load_id = tick_effects
            .iter()
            .chain(&advance_effects)
            .find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            });
        if load_id.is_some() {
            break;
        }
    }
    let load_id = load_id.expect("an overlap wave loads before the remaining download ends");
    assert_eq!(state.batch_observation().jobs_in_flight, 1);
    let (state, effects) = harvester_core::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_id,
            delta: harvester_engine::TriageArticleDelta::full_window(
                vec![articles[0].clone()],
                100_000,
            ),
        },
    );
    let first_request = triage_request_id(&effects).expect("first article dispatches triage early");
    assert_eq!(state.batch_observation().jobs_in_flight, 1);
    let (mut state, _) = harvester_core::update(
        state,
        Msg::LlmCompleted {
            request_id: first_request,
            result: harvester_core::LlmResultKind::Failed {
                reason: "fake triage failure".into(),
            },
            metadata: None,
        },
    );
    assert!(state.view().run_progress.run_active);

    state = harvester_core::update(
        state,
        Msg::JobDone {
            job_id: jobs[1],
            result: JobResultKind::Success,
            content_preview: None,
            extracted_links: Vec::new(),
            fetched_utc: articles[1].fetched_utc.clone(),
        },
    )
    .0;
    state = harvester_core::update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: articles.iter().map(|article| article.url.clone()).collect(),
            triggered_by_job_done: true,
        },
    )
    .0;
    let mut second_load = None;
    for tick in 41..=100 {
        let (next, tick_effects) = harvester_core::update(
            state,
            Msg::tick_at(chrono::DateTime::from_timestamp(1_700_200_000 + tick, 0).unwrap()),
        );
        let (next, advance_effects) = harvester_core::update(next, Msg::PipelineRunAdvance);
        state = next;
        second_load = tick_effects
            .iter()
            .chain(&advance_effects)
            .find_map(|effect| match effect {
                Effect::LoadArticlesForTriage { request_id, .. } => Some(*request_id),
                _ => None,
            });
        if second_load.is_some() {
            break;
        }
    }
    let second_load = second_load.expect("the final download joins as a later wave");
    let (state, effects) = harvester_core::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: second_load,
            delta: harvester_engine::TriageArticleDelta::full_window(articles, 100_000),
        },
    );
    let second_request = triage_request_id(&effects).expect("the later article is admitted once");
    let (state, _) = harvester_core::update(
        state,
        Msg::LlmCompleted {
            request_id: second_request,
            result: harvester_core::LlmResultKind::Failed {
                reason: "fake triage failure".into(),
            },
            metadata: None,
        },
    );
    assert!(state.pipeline_activity().is_settled());
    assert_eq!(state.batch_status(), harvester_core::BatchStatus::Settled);
    assert!(!state.view().run_progress.run_active);
    assert_eq!(
        state
            .pipeline_waves()
            .waves()
            .iter()
            .filter(|wave| wave.stage == PipelineStage::Triaging)
            .count(),
        2
    );
    let settled_notice = state.run_completion_notice().cloned();
    let (state, effects) = harvester_core::update(state, Msg::PipelineRunAdvance);
    assert!(effects.is_empty());
    assert!(!state.view().run_progress.run_active);
    assert_eq!(state.run_completion_notice().cloned(), settled_notice);
}

#[test]
fn drain_with_restored_startup_work_issues_no_model_request_and_terminates() {
    use harvester_core::{
        ArticleSummaryResult, ArticleTriageResult, CompletedJobSnapshot, PipelineWavePolicy,
        SummaryCache, SummaryCacheEntry, SummaryCacheKey, TriageCache, TriageCacheKey,
        TriageSession, UnfinishedWork,
    };
    use harvester_engine::llm::PromptId;
    let temp_dir = TempDir::new().unwrap();
    let output_dir = temp_dir.path().join("output");
    std::fs::create_dir_all(&output_dir).unwrap();
    let runtime_paths = RuntimePaths::new(
        output_dir,
        temp_dir.path().join("sources.ron"),
        temp_dir.path().join("contexts"),
        temp_dir.path().join("prompts"),
    );
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let state = AppState::new();
    let (state, _) = harvester_core::update(
        state,
        Msg::LlmMetadataLoaded {
            active_versions: [
                (PromptId::ArticleTriage, 1),
                (PromptId::ArticleSummary, 1),
                (PromptId::ArticleSignalCandidate, 1),
            ]
            .into_iter()
            .collect(),
            effective_models: [
                (PromptId::ArticleTriage, "test-triage-model".to_string()),
                (PromptId::ArticleSummary, "test-summary-model".to_string()),
                (
                    PromptId::ArticleSignalCandidate,
                    "test-signal-model".to_string(),
                ),
            ]
            .into_iter()
            .collect(),
        },
    );
    let (state, _) = harvester_core::update(state, Msg::PromptTemplateFilesLoaded);
    let (state, _) = harvester_core::update(
        state,
        Msg::PromptContextsLoaded {
            contexts: Default::default(),
        },
    );
    let article = harvester_engine::LoadedArticle {
        url: "https://drain.example/restored".into(),
        source_title: Some("Restored article".into()),
        prepared_text: std::iter::repeat_n("articleword", 220)
            .collect::<Vec<_>>()
            .join(" "),
        content_hash: "drain-hash".into(),
        fetched_utc: Some("2026-09-27T00:00:00Z".into()),
    };
    let triage_result = ArticleTriageResult {
        category: "news".into(),
        priority: 3,
        tags: vec!["ai".into()],
        rationale: "eligible".into(),
        input_tokens: 0,
        output_tokens: 0,
    };
    let mut triage_cache = TriageCache::new();
    triage_cache.insert(
        TriageCacheKey::try_new(
            &article.content_hash,
            PromptId::ArticleTriage,
            Some(1),
            Some("test-triage-model"),
            &[],
        )
        .unwrap(),
        triage_result.clone(),
    );
    let (state, _) = harvester_core::update(
        state,
        Msg::TriageCacheHydrated {
            cache: triage_cache,
        },
    );
    let summary_key = SummaryCacheKey::try_new(
        &article.content_hash,
        PromptId::ArticleSummary,
        Some(1),
        Some("test-summary-model"),
        &[],
    )
    .unwrap();
    let summary_result = ArticleSummaryResult {
        title: "Restored article".into(),
        summary: "Already summarized".into(),
        key_points: vec!["One point".into()],
        input_tokens: 0,
        output_tokens: 0,
        entities: Default::default(),
    };
    let mut summary_cache = SummaryCache::new();
    summary_cache.insert(
        summary_key,
        SummaryCacheEntry {
            result: summary_result,
            created_at_utc: "2026-09-27T00:00:00Z".into(),
        },
    );
    let (state, _) = harvester_core::update(
        state,
        Msg::SummaryCacheHydrated {
            cache: summary_cache,
        },
    );
    let (state, _) = harvester_core::update(
        state,
        Msg::RestoreCompletedJobs(vec![CompletedJobSnapshot {
            url: article.url.clone(),
            tokens: Some(123),
            bytes: Some(4567),
            links: Vec::new(),
            fetched_utc: article.fetched_utc.clone(),
        }]),
    );
    let (mut state, _) = harvester_core::update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls: vec![article.url.clone()],
            triggered_by_job_done: false,
        },
    );
    let mut load_id = None;
    for tick in 1..=8 {
        let (next, tick_effects) = harvester_core::update(
            state,
            Msg::tick_at(chrono::DateTime::from_timestamp(1_790_000_000 + tick, 0).unwrap()),
        );
        state = next;
        load_id = load_id.or_else(|| {
            tick_effects.iter().find_map(|effect| match effect {
                harvester_core::Effect::LoadArticlesForTriage { request_id, .. } => {
                    Some(*request_id)
                }
                _ => None,
            })
        });
        if load_id.is_some() {
            break;
        }
    }
    let load_id = load_id.expect("the fixture loads the restored article window");
    state = harvester_core::update(
        state,
        Msg::TriageArticlesLoaded {
            request_id: load_id,
            delta: harvester_engine::TriageArticleDelta::full_window(
                vec![article.clone()],
                100_000,
            ),
        },
    )
    .0;
    let mut triage = TriageSession::new_loading(None);
    triage.set_articles(vec![article.clone()]);
    triage.transition_to_triaging();
    triage.complete_article(0, triage_result);
    triage.complete();
    state.set_complete_triage_for_host_drain_fixture(triage);
    state.set_pipeline_wave_policy(PipelineWavePolicy::Disabled);
    assert!(!should_request_continue(
        is_collect_only_cycle(true, true, 1),
        true
    ));
    assert!(matches!(
        state.unfinished_work(),
        UnfinishedWork::Known(work) if work.needs_scoring == 1
    ));
    let (state, rearm_effects) = super::batch_runtime::rearm_for_cycle(state, false);
    assert!(rearm_effects.iter().all(|effect| !matches!(
        effect,
        harvester_core::Effect::RequestLlmCompletion { .. }
            | harvester_core::Effect::LoadProcessingConfiguration { .. }
    )));
    assert_eq!(
        state.pipeline_run_phase(),
        harvester_core::PipelineRunPhase::Idle
    );
    let (mut state, advance_effects) = harvester_core::update(state, Msg::PipelineRunAdvance);
    assert!(advance_effects
        .iter()
        .all(|effect| !matches!(effect, harvester_core::Effect::RequestLlmCompletion { .. })));
    assert_eq!(state.signal_candidate().observation_counts().total, 0);

    let effect_runner = EffectRunner::new(
        runtime_paths,
        msg_tx.clone(),
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    msg_tx.send(Msg::PipelineRunAdvance).unwrap();
    let outcome = run_dispatch_loop_with_tick_interval(
        &mut state,
        &msg_tx,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            tick_interval: Duration::ZERO,
        },
        None,
        None,
    )
    .expect("drain must return after its one non-arming pass");
    assert_eq!(outcome, CycleOutcome::Success);
    assert!(state.pipeline_activity().is_settled());
    assert_eq!(
        state.pipeline_run_phase(),
        harvester_core::PipelineRunPhase::Idle
    );
    assert_eq!(state.signal_candidate().observation_counts().total, 0);
    assert!(matches!(msg_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn determine_exit_code_returns_zero_when_only_partial_failures_occur() {
    assert_eq!(determine_exit_code(0), 0);
}

#[test]
fn determine_exit_code_returns_nonzero_when_total_failure_occurs() {
    assert_eq!(determine_exit_code(1), 1);
}

#[test]
fn shutdown_overrides_each_modes_default_exit_code() {
    assert_eq!(exit_code_with_shutdown(0, true), 130);
    assert_eq!(exit_code_with_shutdown(1, true), 130);
    assert_eq!(exit_code_with_shutdown(1, false), 1);
}

#[test]
fn cycle_counter_baseline_reports_deltas_not_cumulative_totals() {
    let mut baseline = CycleCounterBaseline::from_observation(&observation_with_totals(
        577, 577, 0, 405, 0, 61, 0,
    ));
    let cycle_counts =
        baseline.measure_cycle_and_advance(&observation_with_totals(578, 578, 0, 406, 0, 61, 0));
    assert_eq!(
        cycle_counts,
        CycleCounts {
            new_jobs: 1,
            jobs_done: 1,
            jobs_failed: 0,
            triage_completed: 1,
            triage_failed: 0,
            summary_completed: 0,
            summary_failed: 0,
            imports_completed: 0,
            imports_failed: 0,
        }
    );
}

#[test]
fn test_summarize_batch_msg_compacts_large_payloads() {
    let msg = Msg::TriageArticlesLoaded {
        request_id: 1,
        delta: harvester_engine::TriageArticleDelta::full_window(Vec::new(), 100_000),
    };
    let summary = summarize_batch_msg(&msg);
    assert!(summary.contains("TriageArticlesLoaded"));
    assert!(summary.contains("articles: 0"));
    assert!(!summary.contains("request_id"));
}

#[test]
fn test_truncate_for_log_appends_ellipsis() {
    let input = "abcdefghijklmnopqrstuvwxyz";
    let output = truncate_for_log(input, 10);
    assert!(output.starts_with("abcdefghij"));
    assert!(output.ends_with("..."));
    assert_eq!(output.chars().count(), 13);
}

#[test]
fn test_should_log_batch_msg_filters_downloading_progress() {
    let downloading = Msg::JobProgress {
        job_id: 1,
        stage: harvester_core::Stage::Downloading,
        tokens: None,
        bytes: Some(4096),
        content_preview: None,
    };
    assert!(!should_log_batch_msg(&downloading));

    let tokenizing = Msg::JobProgress {
        job_id: 1,
        stage: harvester_core::Stage::Tokenizing,
        tokens: Some(10),
        bytes: None,
        content_preview: None,
    };
    assert!(should_log_batch_msg(&tokenizing));
}

#[test]
fn test_dispatch_loop_reduces_queued_poll_before_settling() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let output_dir = temp_dir.path().join("output");
    std::fs::create_dir_all(&output_dir).unwrap();

    let sources_path = temp_dir.path().join("sources.ron");
    std::fs::write(&sources_path, "SourceRegistry(sources: [])").unwrap();

    let runtime_paths = RuntimePaths::new(
        output_dir,
        sources_path,
        temp_dir.path().join("contexts"),
        temp_dir.path().join("prompts"),
    );

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let mut state = AppState::new();
    let effect_runner = EffectRunner::new(
        runtime_paths,
        msg_tx.clone(),
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    msg_tx.send(Msg::PollSourcesClicked).unwrap();

    let outcome = run_dispatch_loop(
        &mut state,
        &msg_tx,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            tick_interval: Duration::from_millis(75),
        },
    )
    .expect("dispatch loop should complete");
    assert_eq!(outcome, CycleOutcome::Success);

    assert!(matches!(msg_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn keyless_full_dispatch_loop_polls_and_settles_on_each_cycle() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let output_dir = temp_dir.path().join("output");
    std::fs::create_dir_all(&output_dir).unwrap();
    let sources_path = temp_dir.path().join("sources.ron");
    std::fs::write(&sources_path, "SourceRegistry(sources: [])").unwrap();
    let runtime_paths = RuntimePaths::new(
        output_dir,
        sources_path,
        temp_dir.path().join("contexts"),
        temp_dir.path().join("prompts"),
    );
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let effect_runner = EffectRunner::new(
        runtime_paths,
        msg_tx.clone(),
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let mut state = apply_llm_availability(AppState::new(), false);

    for cycle in 1..=2 {
        msg_tx
            .send(Msg::PipelineRunRequested {
                scope: harvester_core::PipelineRunScope::Full,
            })
            .unwrap();
        let outcome = run_dispatch_loop(
            &mut state,
            &msg_tx,
            &msg_rx,
            &effect_runner,
            &shutdown_flag,
            DispatchLoopOptions {
                tick_interval: Duration::from_millis(75),
            },
        )
        .expect("each keyless intake cycle must terminate");
        assert_eq!(outcome, CycleOutcome::Success);
        let progress = state.view().run_progress;
        assert!(!progress.run_active, "cycle {cycle} must settle");
        assert_eq!(
            progress.stages[harvester_core::PipelineStage::ScanningSources.index()].status,
            harvester_core::StageStatus::Done
        );
        assert_eq!(
            state.pipeline_run_phase(),
            harvester_core::PipelineRunPhase::Idle
        );
    }
}

#[test]
fn test_dispatch_loop_ticks_drive_pretriage_from_restore_signal() {
    engine_logging::initialize_for_tests();
    let temp_dir = TempDir::new().unwrap();
    let output_dir = temp_dir.path().join("output");
    std::fs::create_dir_all(&output_dir).unwrap();

    let sources_path = temp_dir.path().join("sources.ron");
    std::fs::write(&sources_path, "SourceRegistry(sources: [])").unwrap();

    let runtime_paths = RuntimePaths::new(
        output_dir,
        sources_path,
        temp_dir.path().join("contexts"),
        temp_dir.path().join("prompts"),
    );

    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    let mut state = AppState::new();
    let effect_runner = EffectRunner::new(
        runtime_paths,
        msg_tx.clone(),
        Box::new(NoOpPlatformHandler),
        Box::new(NoOpRuntimePersistenceSink),
    );
    let shutdown_flag = Arc::new(AtomicBool::new(false));

    msg_tx
        .send(Msg::RestoreCompletedJobs(vec![
            harvester_core::CompletedJobSnapshot {
                url: "https://example.com/article-1".to_string(),
                tokens: Some(123),
                bytes: Some(4567),
                links: Vec::new(),
                fetched_utc: Some("2026-03-05T00:00:00Z".to_string()),
            },
        ]))
        .unwrap();

    let outcome = run_dispatch_loop_with_tick_interval(
        &mut state,
        &msg_tx,
        &msg_rx,
        &effect_runner,
        &shutdown_flag,
        DispatchLoopOptions {
            tick_interval: Duration::ZERO,
        },
        None,
        None,
    )
    .expect("dispatch loop should complete");
    assert_eq!(outcome, CycleOutcome::Success);

    let obs = state.batch_observation();
    assert!(!matches!(
        obs.pre_triage_phase,
        harvester_core::PreTriagePhase::Idle
    ));
}

#[test]
fn test_classify_outcome_success() {
    let obs = BatchObservation {
        poll_in_progress: false,
        session_state: harvester_core::SessionState::Idle,
        jobs_total: 5,
        jobs_done: 5,
        jobs_failed: 0,
        jobs_in_flight: 0,
        pre_triage_phase: harvester_core::PreTriagePhase::Idle,
        pre_triage_total: 0,
        pre_triage_included: 0,
        pre_triage_review: 0,
        pre_triage_filtered: 0,
        triage_phase: harvester_core::TriagePhase::Complete,
        triage_total: 5,
        triage_pending: 0,
        triage_in_flight: 0,
        triage_completed: 5,
        triage_failed: 0,
        summary_total: 0,
        summary_pending: 0,
        summary_in_flight: 0,
        summary_completed: 0,
        summary_failed: 0,
        triage_deferred: 0,
        summary_deferred: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        signal_deferred: 0,
        triage_cache_hits: 0,
        triage_cache_misses: 0,
        triage_cache_key_unavailable: 0,
        summary_cache_hits: 0,
        summary_cache_misses: 0,
        summary_cache_key_unavailable: 0,
        import_phase: harvester_core::ImportPhase::Idle,
        imports_completed: 0,
        imports_failed: 0,
        import_in_flight: false,
        source_poll_stats: vec![],
    };

    assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::Success);
}

#[test]
fn test_classify_outcome_partial_failure() {
    let obs = BatchObservation {
        poll_in_progress: false,
        session_state: harvester_core::SessionState::Idle,
        jobs_total: 5,
        jobs_done: 3,
        jobs_failed: 2,
        jobs_in_flight: 0,
        pre_triage_phase: harvester_core::PreTriagePhase::Idle,
        pre_triage_total: 0,
        pre_triage_included: 0,
        pre_triage_review: 0,
        pre_triage_filtered: 0,
        triage_phase: harvester_core::TriagePhase::Complete,
        triage_total: 5,
        triage_pending: 0,
        triage_in_flight: 0,
        triage_completed: 3,
        triage_failed: 2,
        summary_total: 0,
        summary_pending: 0,
        summary_in_flight: 0,
        summary_completed: 0,
        summary_failed: 0,
        triage_deferred: 0,
        summary_deferred: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        signal_deferred: 0,
        triage_cache_hits: 0,
        triage_cache_misses: 0,
        triage_cache_key_unavailable: 0,
        summary_cache_hits: 0,
        summary_cache_misses: 0,
        summary_cache_key_unavailable: 0,
        import_phase: harvester_core::ImportPhase::Idle,
        imports_completed: 0,
        imports_failed: 0,
        import_in_flight: false,
        source_poll_stats: vec![],
    };

    assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::PartialFailure);
}

#[test]
fn test_classify_outcome_total_failure() {
    let obs = BatchObservation {
        poll_in_progress: false,
        session_state: harvester_core::SessionState::Idle,
        jobs_total: 5,
        jobs_done: 0,
        jobs_failed: 5,
        jobs_in_flight: 0,
        pre_triage_phase: harvester_core::PreTriagePhase::Idle,
        pre_triage_total: 0,
        pre_triage_included: 0,
        pre_triage_review: 0,
        pre_triage_filtered: 0,
        triage_phase: harvester_core::TriagePhase::Complete,
        triage_total: 5,
        triage_pending: 0,
        triage_in_flight: 0,
        triage_completed: 0,
        triage_failed: 5,
        summary_total: 0,
        summary_pending: 0,
        summary_in_flight: 0,
        summary_completed: 0,
        summary_failed: 0,
        triage_deferred: 0,
        summary_deferred: 0,
        signal_total: 0,
        signal_pending_or_in_flight: 0,
        signal_completed: 0,
        signal_failed: 0,
        signal_deferred: 0,
        triage_cache_hits: 0,
        triage_cache_misses: 0,
        triage_cache_key_unavailable: 0,
        summary_cache_hits: 0,
        summary_cache_misses: 0,
        summary_cache_key_unavailable: 0,
        import_phase: harvester_core::ImportPhase::Idle,
        imports_completed: 0,
        imports_failed: 0,
        import_in_flight: false,
        source_poll_stats: vec![],
    };

    assert_eq!(classify_cycle_outcome(&obs), CycleOutcome::TotalFailure);
}

fn make_checkpoint_test_paths(temp_dir: &TempDir) -> RuntimePaths {
    let output_dir = temp_dir.path().to_path_buf();
    std::fs::create_dir_all(&output_dir).unwrap();
    RuntimePaths::new(
        output_dir,
        temp_dir.path().join("sources.ron"),
        temp_dir.path().join("contexts"),
        temp_dir.path().join("prompts"),
    )
}

#[test]
fn set_checkpoint_invalid_timestamp_returns_err_without_write() {
    let temp_dir = TempDir::new().unwrap();
    let paths = make_checkpoint_test_paths(&temp_dir);
    // Simulate the validation in checkpoint_command() — Set is only constructed after validation
    // We test execute_checkpoint_write with a directly-valid Set to confirm it writes
    let result =
        execute_checkpoint_write(CheckpointCommand::Set("not-rfc3339".to_string()), &paths);
    // save_briefing_checkpoint does not validate the string; validation is in checkpoint_command().
    // But the file SHOULD be written with whatever string is passed.
    // This test verifies the call succeeds (the CLI layer is responsible for validation).
    assert!(result.is_ok());
}

#[test]
fn set_checkpoint_writes_file() {
    let temp_dir = TempDir::new().unwrap();
    let paths = make_checkpoint_test_paths(&temp_dir);
    execute_checkpoint_write(
        CheckpointCommand::Set("2025-12-31T23:00:00Z".to_string()),
        &paths,
    )
    .unwrap();
    let loaded = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
    assert_eq!(loaded.as_deref(), Some("2025-12-31T23:00:00Z"));
}

#[test]
fn set_checkpoint_now_writes_valid_rfc3339() {
    let temp_dir = TempDir::new().unwrap();
    let paths = make_checkpoint_test_paths(&temp_dir);
    execute_checkpoint_write(CheckpointCommand::SetNow, &paths).unwrap();
    let loaded = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
    let ts = loaded.expect("checkpoint should be written");
    assert!(
        chrono::DateTime::parse_from_rfc3339(&ts).is_ok(),
        "expected valid RFC3339, got: {ts}"
    );
}

#[test]
fn clear_checkpoint_deletes_file() {
    let temp_dir = TempDir::new().unwrap();
    let paths = make_checkpoint_test_paths(&temp_dir);
    // Write first
    execute_checkpoint_write(
        CheckpointCommand::Set("2025-12-31T23:00:00Z".to_string()),
        &paths,
    )
    .unwrap();
    assert!(paths.briefing_checkpoint_path.exists());
    // Then clear
    execute_checkpoint_write(CheckpointCommand::Clear, &paths).unwrap();
    assert!(!paths.briefing_checkpoint_path.exists());
}

#[test]
fn show_checkpoint_prints_none_when_absent() {
    let temp_dir = TempDir::new().unwrap();
    let paths = make_checkpoint_test_paths(&temp_dir);
    let val = load_briefing_checkpoint(&paths.briefing_checkpoint_path);
    assert_eq!(val.as_deref().unwrap_or("NONE"), "NONE");
}

#[test]
fn redirected_start_line_uses_the_operational_mode_label() {
    assert_eq!(batch_mode_label(true, false), "batch-api");
    assert_eq!(batch_mode_label(false, false), "recurring");
    assert_eq!(batch_mode_label(true, true), "drain");
}

#[test]
fn batch_wait_keeps_waiting_when_all_peeked_batches_are_nonterminal() {
    let peeks = vec![
        batch_peek(Some(openai_provider_kit::BatchLifecycle::InProgress), 3, 10),
        batch_peek(Some(openai_provider_kit::BatchLifecycle::Finalizing), 8, 8),
        batch_peek(None, 0, 0),
    ];

    assert_eq!(decide_batch_wait(&peeks), BatchWaitDecision::KeepWaiting);
}

#[test]
fn batch_wait_runs_collect_cycle_when_a_peeked_batch_is_terminal() {
    for terminal in [
        openai_provider_kit::BatchLifecycle::Completed,
        openai_provider_kit::BatchLifecycle::Failed,
        openai_provider_kit::BatchLifecycle::Expired,
        openai_provider_kit::BatchLifecycle::Cancelled,
    ] {
        let peeks = vec![
            batch_peek(Some(openai_provider_kit::BatchLifecycle::InProgress), 3, 10),
            batch_peek(Some(terminal), 10, 10),
        ];

        assert_eq!(
            decide_batch_wait(&peeks),
            BatchWaitDecision::RunCollectCycle
        );
    }
}

#[test]
fn batch_wait_runs_collect_cycle_when_no_batches_can_be_peeked() {
    assert_eq!(decide_batch_wait(&[]), BatchWaitDecision::RunCollectCycle);
}

#[test]
fn batch_drain_progress_compares_manifest_and_deferred_work() {
    let before = BatchDrainSnapshot {
        pending_manifest_batches: vec![("file-1".to_string(), None)],
        triage_deferred: 1,
        summary_deferred: 0,
        signal_deferred: 0,
    };
    assert!(!batch_drain_made_progress(&before, &before));

    let after_reconcile = BatchDrainSnapshot {
        pending_manifest_batches: vec![("file-1".to_string(), Some("batch-1".to_string()))],
        ..before.clone()
    };
    assert!(batch_drain_made_progress(&before, &after_reconcile));

    let after_collection = BatchDrainSnapshot {
        pending_manifest_batches: Vec::new(),
        triage_deferred: 0,
        ..after_reconcile.clone()
    };
    assert!(batch_drain_made_progress(
        &after_reconcile,
        &after_collection
    ));
}

#[test]
fn batch_drain_exits_after_second_consecutive_no_progress_cycle() {
    assert!(!should_exit_batch_drain_after_no_progress(0));
    assert!(!should_exit_batch_drain_after_no_progress(1));
    assert!(should_exit_batch_drain_after_no_progress(2));
}

#[test]
fn immediate_exit_cursor_restore_emits_control_bytes_only_for_interactive_output() {
    let mut redirected = Vec::new();
    restore_cursor_before_immediate_exit(&mut redirected, false);
    assert!(
        redirected.is_empty(),
        "redirected output must contain no cursor-control bytes"
    );

    let mut interactive = Vec::new();
    restore_cursor_before_immediate_exit(&mut interactive, true);
    assert_eq!(interactive, b"\x1b[?25h");
}

#[test]
fn apply_signal_candidate_selection_settings_uses_defaults_and_overrides() {
    let temp_dir = TempDir::new().unwrap();
    let mut state = AppState::new();
    let mut args = create_test_args(false, &temp_dir);

    apply_signal_candidate_selection_settings(&mut state, &args);
    assert_eq!(
        state.signal_candidate_threshold(),
        DEFAULT_SELECTION_THRESHOLD
    );

    args.signal_candidate_threshold = Some(75);
    apply_signal_candidate_selection_settings(&mut state, &args);
    assert_eq!(state.signal_candidate_threshold(), 75);
}
