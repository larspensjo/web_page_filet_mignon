use std::collections::HashMap;
use std::sync::{mpsc, Arc, RwLock};

use chrono::Utc;
use engine_logging::{engine_info, engine_warn};
use harvester_core::{update, AiAvailability, AppState, Effect, LlmQuotaLimits, Msg};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::prompts::register_defaults;
use harvester_engine::llm::{
    LlmConfig, LlmHandle, LlmQuotas, ModelId, OpenAiProvider, PricingRegistry, PromptRegistry,
    ProviderKind, DEFAULT_SUMMARY_MODEL, DEFAULT_TRIAGE_MODEL,
};

use crate::{
    load_blacklist, load_pending_intake, load_runtime_hydration, load_signal_candidate_cache,
    load_signal_candidate_overrides, load_summary_cache, load_triage_cache, EffectRunner,
    PlatformEffectHandler, RuntimePaths, RuntimePersistenceSink,
};

/// The LLM settings that intentionally differ between executable hosts.
#[derive(Debug, Clone)]
pub struct HostLlmDefaults {
    pub default_model: ModelId,
    pub session_id_prefix: &'static str,
}

pub const DEFAULT_LLM_MAX_CONCURRENT_REQUESTS: usize = 3;
pub use harvester_engine::llm::MAX_LLM_CONCURRENT_REQUESTS;

pub fn parse_llm_max_concurrency_requests(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_LLM_MAX_CONCURRENT_REQUESTS)
        .clamp(1, MAX_LLM_CONCURRENT_REQUESTS)
}

pub fn llm_max_concurrency_requests_from_env() -> usize {
    let raw = std::env::var("LLM_MAX_CONCURRENT_REQUESTS").ok();
    let value = parse_llm_max_concurrency_requests(raw.as_deref());
    if let Some(raw) = raw {
        engine_info!("[llm-concurrency] LLM_MAX_CONCURRENT_REQUESTS='{raw}' -> {value}");
    }
    value
}

/// Seeds shared GUI facts before hydration and returns effects that must be enqueued.
pub fn prepare_desktop_startup_state(
    mut state: AppState,
    paths: &RuntimePaths,
    llm_max_concurrent_requests: usize,
    startup_ai_availability: Option<AiAvailability>,
    llm_quota_limits: Option<LlmQuotaLimits>,
) -> (AppState, Vec<Effect>) {
    let mut startup_effects = Vec::new();
    state.set_llm_max_in_flight(llm_max_concurrent_requests);
    for message in [
        startup_ai_availability.map(|availability| Msg::AiAvailabilityDetected { availability }),
        llm_quota_limits.map(|limits| Msg::LlmQuotaConfigured { limits }),
    ]
    .into_iter()
    .flatten()
    {
        let (next, effects) = update(state, message);
        state = next;
        startup_effects.extend(effects);
    }
    let (state, hydration_effects) = hydrate_state_from_disk(state, paths);
    startup_effects.extend(hydration_effects);
    (state, startup_effects)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAiEnvironment {
    pub api_key: Option<String>,
    pub availability: AiAvailability,
}

pub type EffectRunnerBuild = (EffectRunner, Option<LlmQuotaLimits>, AiAvailability);

/// Resolve the shared API-key state for every host, preserving each host's
/// established warning text.
pub fn host_ai_environment_from_env(
    missing_api_key_warning: &'static str,
    empty_api_key_warning: Option<&'static str>,
) -> HostAiEnvironment {
    host_ai_environment_from_value(
        std::env::var("OPENAI_API_KEY"),
        missing_api_key_warning,
        empty_api_key_warning,
    )
}

pub fn host_ai_environment_from_value(
    value: Result<String, std::env::VarError>,
    missing_api_key_warning: &'static str,
    empty_api_key_warning: Option<&'static str>,
) -> HostAiEnvironment {
    match value {
        Ok(api_key) if !api_key.trim().is_empty() => HostAiEnvironment {
            api_key: Some(api_key),
            availability: AiAvailability::Available,
        },
        Ok(_) => {
            engine_warn!(
                "{}",
                empty_api_key_warning.unwrap_or(missing_api_key_warning)
            );
            HostAiEnvironment {
                api_key: None,
                availability: AiAvailability::Unavailable {
                    reason: harvester_core::AiUnavailableReason::MissingApiKey,
                },
            }
        }
        Err(_) => {
            engine_warn!("{}", missing_api_key_warning);
            HostAiEnvironment {
                api_key: None,
                availability: AiAvailability::Unavailable {
                    reason: harvester_core::AiUnavailableReason::MissingApiKey,
                },
            }
        }
    }
}

pub fn effective_model_map(config: &LlmConfig) -> HashMap<PromptId, String> {
    resolved_model_map(
        &config.default_model,
        config.triage_model.as_ref(),
        config.summary_model.as_ref(),
        config.signal_candidate_model.as_ref(),
    )
}

struct HostPromptModels {
    triage: Option<ModelId>,
    summary: Option<ModelId>,
    signal_candidate: Option<ModelId>,
}

impl Default for HostPromptModels {
    fn default() -> Self {
        Self {
            triage: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_TRIAGE_MODEL)),
            summary: Some(ModelId::new(ProviderKind::OpenAi, DEFAULT_SUMMARY_MODEL)),
            signal_candidate: None,
        }
    }
}

pub(crate) fn default_effective_model_map() -> HashMap<PromptId, String> {
    let models = HostPromptModels::default();
    resolved_model_map(
        &ModelId::new(ProviderKind::OpenAi, DEFAULT_SUMMARY_MODEL),
        models.triage.as_ref(),
        models.summary.as_ref(),
        models.signal_candidate.as_ref(),
    )
}

fn resolved_model_map(
    default_model: &ModelId,
    triage_model: Option<&ModelId>,
    summary_model: Option<&ModelId>,
    signal_candidate_model: Option<&ModelId>,
) -> HashMap<PromptId, String> {
    let mut map = HashMap::new();

    let triage_model = triage_model
        .unwrap_or(default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleTriage, triage_model);

    let summary_name = summary_model
        .unwrap_or(default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleSummary, summary_name);

    let signal_candidate_model = signal_candidate_model
        .or(summary_model)
        .unwrap_or(default_model)
        .model_name()
        .to_string();
    map.insert(PromptId::ArticleSignalCandidate, signal_candidate_model);

    map
}

pub fn llm_quota_limits_from_engine(quotas: &LlmQuotas) -> LlmQuotaLimits {
    LlmQuotaLimits {
        max_calls_per_session: quotas.max_calls_per_session.map(u64::from),
        max_input_tokens_per_session: quotas.max_input_tokens_per_session,
        max_output_tokens_per_session: quotas.max_output_tokens_per_session,
        max_cost_microdollars_per_session: quotas.max_cost_microdollars_per_session,
    }
}

/// Builds a host's runner while leaving host-specific batch scheduling and UI
/// platform handling with their owning crates.
#[allow(clippy::too_many_arguments)]
pub fn build_effect_runner(
    paths: &RuntimePaths,
    msg_tx: mpsc::Sender<Msg>,
    llm_concurrency: usize,
    defaults: &HostLlmDefaults,
    platform_handler: Box<dyn PlatformEffectHandler>,
    persistence_sink: Box<dyn RuntimePersistenceSink>,
    missing_api_key_warning: &'static str,
    empty_api_key_warning: Option<&'static str>,
) -> Result<EffectRunnerBuild, String> {
    let ai_environment =
        host_ai_environment_from_env(missing_api_key_warning, empty_api_key_warning);
    if let Some(api_key) = ai_environment.api_key {
        let provider: Arc<dyn harvester_engine::llm::provider::LlmProvider> =
            Arc::new(OpenAiProvider::new(api_key));
        let (runner, config) = build_effect_runner_with_provider(
            paths,
            msg_tx,
            llm_concurrency,
            defaults,
            provider,
            platform_handler,
            persistence_sink,
            None,
        );
        let quota_limits = llm_quota_limits_from_engine(&config.quotas);
        Ok((runner, Some(quota_limits), ai_environment.availability))
    } else {
        Ok((
            EffectRunner::new(paths.clone(), msg_tx, platform_handler, persistence_sink),
            None,
            ai_environment.availability,
        ))
    }
}

/// Build the real effect runner while substituting only the model provider.
#[allow(clippy::too_many_arguments)]
pub fn build_effect_runner_with_provider(
    paths: &RuntimePaths,
    msg_tx: mpsc::Sender<Msg>,
    llm_concurrency: usize,
    defaults: &HostLlmDefaults,
    provider: Arc<dyn harvester_engine::llm::provider::LlmProvider>,
    platform_handler: Box<dyn PlatformEffectHandler>,
    persistence_sink: Box<dyn RuntimePersistenceSink>,
    file_write_observer: Option<crate::FileWriteObserver>,
) -> (EffectRunner, LlmConfig) {
    let (mut config, registry) =
        llm_config_with_provider(paths, llm_concurrency, defaults, provider);
    config.replay_write_observer = file_write_observer.clone();
    let model_map = effective_model_map(&config);
    let handle = LlmHandle::new(config.clone());
    let runner = if let Some(observer) = file_write_observer {
        EffectRunner::new_with_llm_and_file_write_observer(
            paths.clone(),
            msg_tx,
            handle,
            config.max_input_bytes,
            registry,
            model_map,
            platform_handler,
            persistence_sink,
            observer,
        )
    } else {
        EffectRunner::new_with_llm(
            paths.clone(),
            msg_tx,
            handle,
            config.max_input_bytes,
            registry,
            model_map,
            platform_handler,
            persistence_sink,
        )
    };
    (runner, config)
}

/// Keep the model and quota configuration shared between real and canned providers.
pub fn llm_config_with_provider(
    paths: &RuntimePaths,
    llm_concurrency: usize,
    defaults: &HostLlmDefaults,
    provider: Arc<dyn harvester_engine::llm::provider::LlmProvider>,
) -> (LlmConfig, Arc<RwLock<PromptRegistry>>) {
    let mut registry = PromptRegistry::new();
    register_defaults(&mut registry);
    let registry = Arc::new(RwLock::new(registry));
    let models = HostPromptModels::default();
    let config = LlmConfig {
        provider,
        default_model: defaults.default_model.clone(),
        triage_model: models.triage,
        summary_model: models.summary,
        signal_candidate_model: models.signal_candidate,
        registry: Arc::clone(&registry),
        quotas: LlmQuotas::default(),
        output_dir: paths.output_dir.clone(),
        pricing: PricingRegistry::with_defaults(),
        max_input_bytes: 100_000,
        timestamp_utc: Arc::new(|| Utc::now().to_rfc3339()),
        session_id: format!(
            "{}{}",
            defaults.session_id_prefix,
            Utc::now().format("%Y%m%d-%H%M%S")
        ),
        replay_write_observer: None,
        max_concurrent_requests: llm_concurrency,
    };
    (config, registry)
}

/// Hydrates startup state shared by executable hosts. Manual pre-triage
/// overrides are deliberately excluded; no host honours invisible saved decisions.
pub fn hydrate_state_from_disk(
    mut state: AppState,
    paths: &RuntimePaths,
) -> (AppState, Vec<Effect>) {
    let mut startup_effects = vec![Effect::LoadPromptTemplateFiles];

    let (next_state, effects) = update(state, Msg::StartupHydrationRequested);
    state = next_state;
    startup_effects.extend(effects);

    let hydration = load_runtime_hydration(&paths.state_path, &paths.output_dir);
    let completed_jobs = hydration.jobs;
    let completed_job_count = completed_jobs.len();
    (state, _) = update(state, Msg::RestoreCompletedJobs(completed_jobs));
    let pending_intake = load_pending_intake(&paths.state_path);
    if !pending_intake.is_empty() {
        (state, _) = update(state, Msg::RestorePendingIntake(pending_intake));
    }

    let (next, effects) = hydrate_result_stores(state, paths);
    state = next;
    startup_effects.extend(effects);

    match load_signal_candidate_overrides(&paths.signal_candidate_overrides_path) {
        Ok(overrides) if !overrides.is_empty() => {
            let (next_state, effects) =
                update(state, Msg::SignalCandidateOverridesLoaded { overrides });
            state = next_state;
            startup_effects.extend(effects);
        }
        Ok(_) => {}
        Err(err) => engine_warn!(
            "[signal-overrides] failed to hydrate {}: {}",
            paths.signal_candidate_overrides_path.display(),
            err
        ),
    }

    let blacklist = load_blacklist(&paths.blacklist_path);
    if !blacklist.is_empty() {
        (state, _) = update(state, Msg::BlacklistHydrated { state: blacklist });
    }

    let (next, effects) = update(
        state,
        Msg::RestoreDesktopView {
            mode: hydration.job_list_mode,
            selected_article_url: hydration.selected_article_url,
            now: Utc::now(),
        },
    );
    state = next;
    startup_effects.extend(effects);

    for message in hydration.notices {
        (state, _) = update(state, Msg::RuntimeStateNotice { message });
    }
    if hydration.recovery_needs_persist {
        let (next, effects) = update(state, Msg::FetchTimeRecoveryCompleted);
        state = next;
        startup_effects.extend(effects);
    }

    engine_info!(
        "[host-bootstrap] Hydrated startup state: completed_jobs={} output_dir={}",
        completed_job_count,
        paths.output_dir.display()
    );

    (state, startup_effects)
}

/// Hydrate every paid-result kind and report unreadable stores through actions.
pub fn hydrate_result_stores(mut state: AppState, paths: &RuntimePaths) -> (AppState, Vec<Effect>) {
    let mut startup_effects = Vec::new();
    for result in [
        load_summary_cache(&paths.summary_cache_path)
            .map(|cache| Msg::SummaryCacheHydrated { cache }),
        load_triage_cache(&paths.triage_cache_path).map(|cache| Msg::TriageCacheHydrated { cache }),
        load_signal_candidate_cache(&paths.signal_candidate_cache_path)
            .map(|cache| Msg::SignalCandidateCacheLoaded { cache }),
    ] {
        let message = match result {
            Ok(message) => message,
            Err(error) => {
                let reason = error.to_string();
                engine_warn!("[host-bootstrap] AI unavailable: {}", reason);
                Msg::ResultStoreUnavailable { reason }
            }
        };
        let (next, effects) = update(state, message);
        state = next;
        startup_effects.extend(effects);
    }

    (state, startup_effects)
}

/// Emits the reducer-owned pre-triage refresh evaluation when the state asks
/// the host loop to perform it.
pub fn pump_pre_triage_refresh(mut state: AppState) -> (AppState, Vec<Effect>, bool) {
    let Some(triggered_by_job_done) = state.take_pre_triage_refresh_evaluation_request() else {
        return (state, Vec::new(), false);
    };
    let ordered_urls = state.ordered_completed_job_urls_snapshot();
    let (next_state, effects) = update(
        state,
        Msg::EvaluatePreTriageRefresh {
            ordered_urls,
            triggered_by_job_done,
        },
    );
    (next_state, effects, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hydrate_empty_output_folder_records_empty_article_window() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::with_defaults(dir.path().to_path_buf());
        let (state, effects) = hydrate_state_from_disk(AppState::new(), &paths);
        assert_eq!(
            state.startup_inputs().initial_article_window,
            harvester_core::InitialArticleWindowOutcome::Empty
        );
        assert_eq!(
            state.startup_inputs().result_stores,
            harvester_core::StartupInputOutcome::Loaded
        );
        assert!(state.completed_jobs_snapshot().is_empty());
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadArticlesForTriage { .. })));
        let (mut state, pump_effects, pumped) = pump_pre_triage_refresh(state);
        assert!(!pumped);
        assert!(pump_effects.is_empty());
        for msg in [
            Msg::BriefingCheckpointLoaded { since_utc: None },
            Msg::PromptTemplateFilesLoaded,
            Msg::PromptContextsLoaded {
                contexts: HashMap::new(),
            },
            Msg::LlmMetadataLoaded {
                active_versions: [
                    PromptId::ArticleTriage,
                    PromptId::ArticleSummary,
                    PromptId::ArticleSignalCandidate,
                ]
                .into_iter()
                .map(|id| (id, 1))
                .collect(),
                effective_models: [
                    PromptId::ArticleTriage,
                    PromptId::ArticleSummary,
                    PromptId::ArticleSignalCandidate,
                ]
                .into_iter()
                .map(|id| (id, "fixture-model".into()))
                .collect(),
            },
        ] {
            state = update(state, msg).0;
        }
        assert_eq!(
            state.startup_readiness(),
            harvester_core::StartupReadinessStatus::Ready
        );
        assert_eq!(
            state.archive_meter_view().status,
            harvester_core::ArchiveMeterStatus::NotScoredYet
        );
        assert_eq!(state.archive_meter_view().selected_count, 0);
    }

    #[test]
    fn hydrate_state_from_disk_reduces_exclusions_before_startup_replies() {
        let dir = tempfile::tempdir().unwrap();
        let paths = missing_dates_fixture(dir.path());
        let overrides = [harvester_core::OverrideKey {
            signal_key: "saved-exclusion".into(),
            prompt_id: PromptId::ArticleSignalCandidate.to_string(),
            prompt_version: 1,
        }]
        .into_iter()
        .collect();
        crate::signal_candidate_overrides_store::save(
            &paths.signal_candidate_overrides_path,
            &overrides,
        )
        .unwrap();
        let (state, effects) = hydrate_state_from_disk(AppState::new(), &paths);
        assert_eq!(state.signal_exclusions().excluded(), &overrides);
        let inputs = state.startup_inputs();
        assert_eq!(
            inputs.checkpoint,
            harvester_core::StartupInputOutcome::Pending
        );
        assert_eq!(
            inputs.prompt_metadata,
            harvester_core::StartupInputOutcome::Pending
        );
        assert_eq!(
            inputs.prompt_contexts,
            harvester_core::StartupInputOutcome::Pending
        );
        assert_eq!(
            inputs.initial_article_window,
            harvester_core::InitialArticleWindowOutcome::Pending
        );
        for required in [
            Effect::LoadBriefingCheckpoint,
            Effect::LoadLlmMetadata,
            Effect::LoadPromptContexts,
            Effect::LoadPromptTemplateFiles,
        ] {
            assert!(effects.contains(&required));
        }
        let (state, _, pumped) = pump_pre_triage_refresh(state);
        assert!(pumped);
        assert_eq!(state.signal_exclusions().excluded(), &overrides);
    }

    fn missing_dates_fixture(output: &std::path::Path) -> RuntimePaths {
        let paths = RuntimePaths::with_defaults(output.to_path_buf());
        std::fs::write(&paths.state_path, r#"(completed: [
            (url: "https://example.com/recover#fragment", tokens: Some(12), bytes: Some(240), links: []),
            (url: "https://example.com/no-file", tokens: Some(10), bytes: Some(120), links: [])
        ], pending_intake: ["https://example.com/pending"])"#).unwrap();
        let (_, document) = harvester_engine::build_markdown_document(
            "https://example.com/recover",
            Some("Recovered date"),
            "utf-8",
            "2026-09-20T09:15:00Z",
            "An article with frontmatter and a missing runtime timestamp.",
            &harvester_engine::WhitespaceTokenCounter,
        );
        std::fs::write(output.join("recover.md"), document).unwrap();
        paths
    }

    fn window(state: AppState) -> AppState {
        update(
            state,
            Msg::BriefingCheckpointLoaded {
                since_utc: Some("2026-09-01T00:00:00Z".into()),
            },
        )
        .0
    }

    #[test]
    fn startup_recovers_frontmatter_fetch_time_and_lowers_hidden_count() {
        let dir = tempfile::tempdir().unwrap();
        let paths = missing_dates_fixture(dir.path());
        let before = update(
            AppState::new(),
            Msg::RestoreCompletedJobs(crate::load_completed_jobs(&paths.state_path)),
        )
        .0;
        assert_eq!(
            window(before)
                .view()
                .desktop_job_list
                .hidden_without_fetch_time,
            2
        );
        let (state, _) = hydrate_state_from_disk(AppState::new(), &paths);
        let state = window(state);
        assert_eq!(state.view().desktop_job_list.hidden_without_fetch_time, 1);
        assert_eq!(state.view().desktop_job_list.rows.len(), 1);
        assert_eq!(
            state.completed_jobs_snapshot()[0].fetched_utc.as_deref(),
            Some("2026-09-20T09:15:00+00:00")
        );
        assert_eq!(
            harvester_core::PersistenceSnapshot::capture(&state).pending_intake,
            ["https://example.com/pending"]
        );
    }

    #[test]
    fn job_without_article_stays_hidden_survives_restart_and_blocks_redownload() {
        let dir = tempfile::tempdir().unwrap();
        let paths = missing_dates_fixture(dir.path());
        let (state, _) = hydrate_state_from_disk(AppState::new(), &paths);
        let (tx, _) = mpsc::channel();
        let runner = EffectRunner::new(
            paths.clone(),
            tx,
            Box::new(crate::NoOpPlatformHandler),
            Box::new(crate::PersistenceWorker::new(
                paths.state_path.clone(),
                paths.blacklist_path.clone(),
            )),
        );
        // Persistence still travels through the reducer-emitted effect and runner.
        let (state, effects) = update(
            state,
            Msg::FetchOutcomeClassified {
                job_id: 2,
                class: harvester_engine::FetchOutcomeClass::Success,
                failure_label: None,
                recorded_at: Utc::now(),
            },
        );
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistRuntimeState { .. })));
        runner.enqueue(effects);
        drop(runner);
        let (restored, _) = hydrate_state_from_disk(AppState::new(), &paths);
        let restored = window(restored);
        assert_eq!(
            restored.view().desktop_job_list.hidden_without_fetch_time,
            1
        );
        assert_eq!(restored.view().desktop_job_list.rows.len(), 1);
        assert_eq!(restored.completed_jobs_snapshot().len(), 2);
        let missing = restored
            .completed_jobs_snapshot()
            .into_iter()
            .find(|job| job.url.ends_with("no-file"))
            .unwrap();
        assert!(missing.fetched_utc.is_none());
        assert_eq!(state.completed_jobs_snapshot().len(), 2);
        let (restored, _) = update(
            restored,
            Msg::InputChanged("https://example.com/no-file".into()),
        );
        let (restored, effects) = update(restored, Msg::UrlsSubmitted);
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, Effect::EnqueueUrl { .. })));
        assert_eq!(restored.completed_jobs_snapshot().len(), 2);
    }

    #[test]
    fn recovery_persists_once_and_second_start_does_not_rescan_hidden_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let paths = missing_dates_fixture(dir.path());
        let (state, effects) = hydrate_state_from_disk(AppState::new(), &paths);
        let persistence = effects
            .into_iter()
            .filter(|effect| matches!(effect, Effect::PersistRuntimeState { .. }))
            .collect::<Vec<_>>();
        assert_eq!(persistence.len(), 1);
        let (tx, _) = mpsc::channel();
        let runner = EffectRunner::new(
            paths.clone(),
            tx,
            Box::new(crate::NoOpPlatformHandler),
            Box::new(crate::PersistenceWorker::new(
                paths.state_path.clone(),
                paths.blacklist_path.clone(),
            )),
        );
        runner.enqueue(persistence);
        drop(runner);
        assert!(std::fs::read_to_string(&paths.state_path)
            .unwrap()
            .contains("fetch_time_recovery_done: true"));
        assert_eq!(state.completed_jobs_snapshot().len(), 2);
        // If another scan runs it would now find this previously unrecoverable date.
        std::fs::write(
            dir.path().join("late.md"),
            "---\nurl: https://example.com/no-file\nfetched_utc: 2026-09-20T09:15:00Z\n---\nBody",
        )
        .unwrap();
        let (restored, effects) = hydrate_state_from_disk(AppState::new(), &paths);
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistRuntimeState { .. })));
        assert_eq!(restored.completed_jobs_snapshot().len(), 2);
        assert!(restored
            .completed_jobs_snapshot()
            .iter()
            .find(|job| job.url.ends_with("no-file"))
            .unwrap()
            .fetched_utc
            .is_none());
        assert_eq!(
            window(restored)
                .view()
                .desktop_job_list
                .hidden_without_fetch_time,
            1
        );
    }

    #[test]
    fn startup_backup_notices_reach_the_existing_owner_status() {
        for corrupt in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let paths = RuntimePaths::with_defaults(dir.path().to_path_buf());
            let original = if corrupt {
                "truncated state"
            } else {
                "(completed: [])"
            };
            std::fs::write(&paths.state_path, original).unwrap();
            std::fs::write(
                dir.path().join(".harvester_state.pre-slim.ron"),
                "owner backup",
            )
            .unwrap();
            let (state, _) = hydrate_state_from_disk(AppState::new(), &paths);
            let notice = state.view().checkpoint_status_message.unwrap();
            assert_eq!(notice.lines().count(), 1);
            let name = notice
                .split("preserved in ")
                .nth(1)
                .unwrap()
                .split(';')
                .next()
                .unwrap();
            assert!(name.starts_with(".harvester_state.pre-slim-"));
            assert!(!name.starts_with(".harvester_state.pre-slim-partial-"));
            assert_eq!(
                std::fs::read_to_string(dir.path().join(name)).unwrap(),
                original
            );
            assert_eq!(
                std::fs::read_to_string(dir.path().join(".harvester_state.pre-slim.ron")).unwrap(),
                "owner backup"
            );
        }
    }

    #[test]
    fn parse_llm_max_concurrency_uses_default_when_missing_or_invalid() {
        assert_eq!(
            parse_llm_max_concurrency_requests(None),
            DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(
            parse_llm_max_concurrency_requests(Some("not-a-number")),
            DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(
            parse_llm_max_concurrency_requests(Some("")),
            DEFAULT_LLM_MAX_CONCURRENT_REQUESTS
        );
    }

    #[test]
    fn parse_llm_max_concurrency_clamps_to_valid_range() {
        assert_eq!(parse_llm_max_concurrency_requests(Some("1")), 1);
        assert_eq!(parse_llm_max_concurrency_requests(Some("3")), 3);
        assert_eq!(
            parse_llm_max_concurrency_requests(Some("999")),
            MAX_LLM_CONCURRENT_REQUESTS
        );
        assert_eq!(parse_llm_max_concurrency_requests(Some(" 2 ")), 2);
    }

    #[test]
    fn hydrate_state_schedules_llm_metadata_load_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = RuntimePaths::with_defaults(dir.path().to_path_buf());

        let (_, effects) = hydrate_state_from_disk(AppState::new(), &paths);

        assert_eq!(
            effects
                .iter()
                .filter(|effect| matches!(effect, Effect::LoadLlmMetadata))
                .count(),
            1
        );
    }

    #[test]
    fn desktop_bootstrap_marks_missing_and_blank_keys_unavailable() {
        let missing_warning = "test missing key warning";
        let empty_warning = "test empty key warning";
        let cases = [
            Err(std::env::VarError::NotPresent),
            Ok(String::new()),
            Ok(" \t ".to_string()),
        ];

        for value in cases {
            let environment =
                host_ai_environment_from_value(value, missing_warning, Some(empty_warning));
            assert!(environment.api_key.is_none());
            assert_eq!(
                environment.availability,
                AiAvailability::Unavailable {
                    reason: harvester_core::AiUnavailableReason::MissingApiKey,
                }
            );

            let temp = tempfile::tempdir().expect("tempdir");
            let paths = RuntimePaths::with_defaults(temp.path().to_path_buf());
            let (state, _) = prepare_desktop_startup_state(
                AppState::new(),
                &paths,
                DEFAULT_LLM_MAX_CONCURRENT_REQUESTS,
                Some(environment.availability),
                None,
            );
            assert_eq!(
                state.ai_availability(),
                &AiAvailability::Unavailable {
                    reason: harvester_core::AiUnavailableReason::MissingApiKey,
                }
            );
        }

        let available = host_ai_environment_from_value(
            Ok("test-key".to_string()),
            missing_warning,
            Some(empty_warning),
        );
        assert_eq!(available.api_key.as_deref(), Some("test-key"));
        assert_eq!(available.availability, AiAvailability::Available);
    }
}
