use super::live_progress::LiveSystemBatchProgress;
use crate::progress::BatchDisplayPhase;
use crate::{
    batch_coordinator::{BatchCoordinator, BufferedRequest, SubmissionBudget},
    batch_manifest::{BatchManifestStore, PendingEntry},
};
use engine_logging::engine_warn;
use harvester_core::{
    update, AppState, FrozenBatchKey, Msg, SignalCandidateCacheKey, StageKind, SummaryCacheKey,
    TriageCacheKey,
};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::llm::{
    LlmCompletionCommand, LlmCompletionError, LlmConfig, OpenAiProvider, ReplayRecord, TokenUsage,
};
use harvester_io::{
    load_signal_candidate_cache, load_summary_cache, load_triage_cache, EffectRunner, RuntimePaths,
};
use std::collections::HashSet;
use std::sync::mpsc;

pub(crate) struct BatchRuntime {
    pub(super) coordinator: BatchCoordinator<OpenAiProvider>,
    config: LlmConfig,
    pub(super) runtime: tokio::runtime::Runtime,
    pub(super) reconciled: bool,
    pub(super) realized_cost_microdollars: u64,
    recorded_replay_lines: HashSet<String>,
}

impl BatchRuntime {
    pub(super) fn new(
        provider: OpenAiProvider,
        config: LlmConfig,
        paths: &RuntimePaths,
    ) -> Result<Self, String> {
        let manifest =
            BatchManifestStore::load(paths.output_dir.clone()).map_err(|err| err.to_string())?;
        let budget = SubmissionBudget::from_quotas(&config.quotas);
        let recorded_replay_lines = load_recorded_batch_replay_lines(&config.replay_output_dir());
        Ok(Self {
            coordinator: BatchCoordinator::new(provider, manifest, budget),
            config,
            runtime: tokio::runtime::Runtime::new().map_err(|err| err.to_string())?,
            reconciled: false,
            realized_cost_microdollars: 0,
            recorded_replay_lines,
        })
    }
}

pub(super) fn is_batch_eligible_prompt(prompt_id: PromptId) -> bool {
    matches!(
        prompt_id,
        PromptId::ArticleTriage | PromptId::ArticleSummary | PromptId::ArticleSignalCandidate
    )
}

pub(super) fn batch_custom_id(key: &FrozenBatchKey) -> String {
    let custom_stage = match key.stage {
        StageKind::Triage => "triage",
        StageKind::Summary => "summary",
        StageKind::SignalCandidate => "signal",
    };
    let model_hash = harvester_engine::llm::content_hash(&key.model_id);
    format!(
        "{}-{}-v{}-{}-{}",
        custom_stage,
        &key.content_hash[..key.content_hash.len().min(16)],
        key.prompt_version,
        &key.context_hash[..key.context_hash.len().min(8)],
        &model_hash[..8]
    )
}

/// Diverts only cache-keyed, non-interactive article stages. Every other
/// effect remains on the normal EffectRunner path.
pub(super) fn divert_batch_effects(
    state: &AppState,
    effects: Vec<harvester_core::Effect>,
    batch: &mut BatchRuntime,
    msg_tx: &mpsc::Sender<Msg>,
) -> Vec<harvester_core::Effect> {
    let mut passthrough = Vec::new();
    for effect in effects {
        let harvester_core::Effect::RequestLlmCompletion {
            request_id,
            prompt_id,
            prompt_version,
            model_override,
            input_content,
            context,
            template_override,
            extra_template_vars,
        } = effect
        else {
            passthrough.push(effect);
            continue;
        };
        if !is_batch_eligible_prompt(prompt_id) {
            passthrough.push(harvester_core::Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                prompt_version,
                model_override,
                input_content,
                context,
                template_override,
                extra_template_vars,
            });
            continue;
        }
        let Some(mut key) = state.frozen_batch_key_for_request(request_id) else {
            engine_logging::engine_warn!(
                "[batch-submit] request_id={} prompt_id={:?} missing frozen cache key; dispatching synchronously",
                request_id, prompt_id
            );
            passthrough.push(harvester_core::Effect::RequestLlmCompletion {
                request_id,
                prompt_id,
                prompt_version,
                model_override,
                input_content,
                context,
                template_override,
                extra_template_vars,
            });
            continue;
        };
        let command = LlmCompletionCommand {
            request_id,
            prompt_id,
            prompt_version,
            model_override,
            input_content,
            context,
            template_override,
            extra_template_vars,
        };
        match harvester_engine::llm::prepare_completion(&command, &batch.config) {
            Ok(prepared) => {
                let stage = key.stage;
                key.rendered_system = prepared.system_message;
                key.rendered_user = prepared.user_message;
                let custom_id = batch_custom_id(&key);
                if batch.coordinator.failed_attempts_for(&custom_id) >= 2 {
                    engine_logging::engine_warn!(
                        "[batch-submit] custom_id={} cache_key={} reached two batch attempts; falling back to synchronous dispatch",
                        custom_id, key.content_hash
                    );
                    passthrough.push(harvester_core::Effect::RequestLlmCompletion {
                        request_id: command.request_id,
                        prompt_id: command.prompt_id,
                        prompt_version: command.prompt_version,
                        model_override: command.model_override,
                        input_content: command.input_content,
                        context: command.context,
                        template_override: command.template_override,
                        extra_template_vars: command.extra_template_vars,
                    });
                    continue;
                }
                let estimated_input_tokens = prepared
                    .request
                    .messages()
                    .iter()
                    .map(|message| message.content().chars().count() as u64)
                    .sum::<u64>()
                    .div_ceil(4);
                let estimated_usage =
                    TokenUsage::new(estimated_input_tokens.min(u64::from(u32::MAX)) as u32, 0);
                let estimated_cost_microdollars = batch
                    .config
                    .pricing
                    .batch_cost_microdollars(prepared.model.model_name(), &estimated_usage);
                batch.coordinator.buffer(BufferedRequest {
                    request_id,
                    stage,
                    line: openai_provider_kit::BatchInputLine {
                        custom_id: custom_id.clone(),
                        method: "POST".to_string(),
                        url: "/v1/chat/completions".to_string(),
                        body: openai_provider_kit::openai_chat_completion_body(&prepared.request),
                    },
                    entry: PendingEntry {
                        custom_id,
                        key,
                        stage,
                        attempts: 0,
                        collected: None,
                    },
                    estimated_input_tokens,
                    estimated_cost_microdollars,
                });
            }
            Err(err) => {
                send_batch_preparation_failure(msg_tx, request_id, &err);
            }
        }
    }
    passthrough
}

pub(super) fn send_batch_preparation_failure(
    msg_tx: &mpsc::Sender<Msg>,
    request_id: u64,
    err: &LlmCompletionError,
) {
    engine_warn!(
        "[batch-submit] request_id={} render/preparation failed: {:?}",
        request_id,
        err
    );
    let _ = msg_tx.send(Msg::LlmCompleted {
        request_id,
        result: harvester_core::LlmResultKind::Failed {
            reason: format!("batch request preparation failed: {err:?}"),
        },
        metadata: None,
    });
}

pub(crate) fn persist_batch_replay_records(
    entries: &[harvester_core::CollectedEntry],
    batch: &mut BatchRuntime,
) {
    for entry in entries {
        let replay_line_id = format!("batch-{}-{}", entry.batch_id, entry.custom_id);
        if batch.recorded_replay_lines.contains(&replay_line_id) {
            continue;
        }
        let (raw_response, usage, resolved_model, validated_output, validation_error) = match &entry
            .outcome
        {
            harvester_core::CollectedOutcome::Success {
                raw_output_json,
                usage,
                resolved_model,
            } => {
                let validation_error = match entry.stage {
                    StageKind::Triage => harvester_engine::llm::validate_triage(raw_output_json)
                        .err()
                        .map(|err| err.to_string()),
                    StageKind::Summary => harvester_engine::llm::validate_summary(raw_output_json)
                        .err()
                        .map(|err| err.to_string()),
                    StageKind::SignalCandidate => {
                        harvester_engine::llm::validate_signal_candidate(raw_output_json)
                            .err()
                            .map(|err| err.to_string())
                    }
                };
                match validation_error {
                    None => (
                        raw_output_json.clone(),
                        *usage,
                        resolved_model.clone(),
                        serde_json::from_str(raw_output_json).ok(),
                        None,
                    ),
                    Some(err) => (
                        raw_output_json.clone(),
                        *usage,
                        resolved_model.clone(),
                        None,
                        Some(err),
                    ),
                }
            }
            harvester_core::CollectedOutcome::LineError { detail } => (
                detail.clone(),
                TokenUsage::new(0, 0),
                entry.key.model_id.clone(),
                None,
                Some(detail.clone()),
            ),
        };
        let priced_model = if resolved_model.trim().is_empty() {
            &entry.key.model_id
        } else {
            &resolved_model
        };
        let cost_microdollars = batch
            .config
            .pricing
            .batch_cost_microdollars(priced_model, &usage);
        let record = ReplayRecord {
            request_id: replay_line_id.clone(),
            input_content_hash: entry.key.content_hash.clone(),
            prompt_id: entry.key.prompt_id,
            prompt_version: entry.key.prompt_version,
            model_id: priced_model.to_string(),
            timestamp_utc: entry.created_at_utc.clone(),
            rendered_system_message: entry.key.rendered_system.clone(),
            rendered_user_message: entry.key.rendered_user.clone(),
            raw_response,
            usage,
            validated_output,
            validation_error,
            cost_microdollars,
            wall_ms: 0,
            cache_status: "batch_collected".to_string(),
        };
        match harvester_engine::llm::persist_replay_record(
            &batch.config.replay_output_dir(),
            &record,
        ) {
            Ok(_) => {
                batch.recorded_replay_lines.insert(replay_line_id);
                batch.realized_cost_microdollars = batch
                    .realized_cost_microdollars
                    .saturating_add(cost_microdollars);
            }
            Err(err) => {
                engine_logging::engine_warn!(
                    "[batch-replay] batch_id={} custom_id={} cache_key={} persist failed: {}",
                    entry.batch_id,
                    entry.custom_id,
                    entry.key.content_hash,
                    err
                );
            }
        }
    }
}

fn load_recorded_batch_replay_lines(dir: &std::path::Path) -> HashSet<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return HashSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| harvester_engine::llm::load_replay_record(&entry.path()).ok())
        .filter(|record| record.request_id.starts_with("batch-"))
        .map(|record| record.request_id)
        .collect()
}

pub(super) fn remove_collected_with_persisted_cache_confirmation(
    batch: &mut BatchRuntime,
    paths: &RuntimePaths,
) -> Result<(), String> {
    let triage_cache = load_triage_cache(&paths.triage_cache_path);
    let summary_cache = load_summary_cache(&paths.summary_cache_path);
    let signal_cache =
        load_signal_candidate_cache(&paths.signal_candidate_cache_path).map_err(|err| {
            format!(
                "signal cache {}: {err}",
                paths.signal_candidate_cache_path.display()
            )
        })?;
    batch
        .coordinator
        .remove_collected_if(|entry| match entry.stage {
            StageKind::Triage => TriageCacheKey::try_new_with_context_hash(
                &entry.key.content_hash,
                entry.key.prompt_id,
                Some(entry.key.prompt_version),
                Some(&entry.key.model_id),
                &entry.key.context_hash,
            )
            .ok()
            .is_some_and(|key| triage_cache.lookup(&key).is_some()),
            StageKind::Summary => summary_cache
                .lookup(&SummaryCacheKey {
                    content_hash: entry.key.content_hash.clone(),
                    prompt_id: entry.key.prompt_id,
                    prompt_version: entry.key.prompt_version,
                    model_id: entry.key.model_id.clone(),
                    context_hash: entry.key.context_hash.clone(),
                })
                .is_some(),
            StageKind::SignalCandidate => signal_cache
                .get(&SignalCandidateCacheKey {
                    signal_input_hash: entry.key.content_hash.clone(),
                    prompt_id: entry.key.prompt_id,
                    prompt_version: entry.key.prompt_version,
                    model_id: entry.key.model_id.clone(),
                    context_hash: entry.key.context_hash.clone(),
                })
                .is_some(),
        })
}

pub(super) fn collect_and_rearm_batch_cycle(
    mut state: AppState,
    batch: &mut BatchRuntime,
    paths: &RuntimePaths,
    effect_runner: &EffectRunner,
    msg_tx: &mpsc::Sender<Msg>,
    progress: &mut LiveSystemBatchProgress,
) -> AppState {
    // Batch results are collected at the cycle boundary before re-arming.
    // A collected manifest snapshot is durable before this reducer message,
    // so a crash after the snapshot is replayed safely on the next run.
    if !batch.reconciled {
        progress.set_phase(BatchDisplayPhase::Reconciling);
        progress.paint(&state, batch.realized_cost_microdollars, true);
        match batch.runtime.block_on(batch.coordinator.reconcile_once()) {
            Ok(()) => batch.reconciled = true,
            Err(err) => {
                engine_warn!("[batch-reconcile] failed; retrying next cycle: {}", err)
            }
        }
    }
    if let Err(err) = remove_collected_with_persisted_cache_confirmation(batch, paths) {
        engine_warn!(
            "[batch-collect] persisted cache confirmation failed; retaining snapshots: {}",
            err
        );
    }
    progress.set_phase(BatchDisplayPhase::Collecting);
    progress.paint(&state, batch.realized_cost_microdollars, true);
    let collected = match batch
        .runtime
        .block_on(batch.coordinator.collect_completed())
    {
        Ok(collected) => collected,
        Err(err) => {
            engine_warn!("[batch-collect] failed; retrying next cycle: {}", err);
            Vec::new()
        }
    };
    if !collected.is_empty() {
        let invalid = invalid_collected_custom_ids(&collected);
        if !invalid.is_empty() {
            if let Err(err) = batch.coordinator.release_invalid_collected(&invalid) {
                engine_warn!(
                    "[batch-collect] invalid-line release failed; snapshots retained for retry: {}",
                    err
                );
            }
        }
        persist_batch_replay_records(&collected, batch);
        let (new_state, collection_effects) =
            update(state, Msg::BatchResultsCollected { entries: collected });
        state = new_state;
        if !collection_effects.is_empty() {
            let collection_effects =
                divert_batch_effects(&state, collection_effects, batch, msg_tx);
            if !collection_effects.is_empty() {
                effect_runner.enqueue(collection_effects);
            }
        }
    }

    // Only the runner advances deferred work into a new dispatch
    // epoch. Pre-loop effects use the same diversion path as effects
    // reduced inside the dispatch loop.
    progress.set_phase(BatchDisplayPhase::Replaying);
    progress.paint(&state, batch.realized_cost_microdollars, true);
    let (new_state, rearm_effects) = update(state, Msg::RearmDeferredBatchStages);
    state = new_state;
    if !rearm_effects.is_empty() {
        let rearm_effects = divert_batch_effects(&state, rearm_effects, batch, msg_tx);
        if !rearm_effects.is_empty() {
            effect_runner.enqueue(rearm_effects);
        }
    }
    state
}

pub(super) fn invalid_collected_custom_ids(
    entries: &[harvester_core::CollectedEntry],
) -> HashSet<String> {
    entries
        .iter()
        .filter_map(|entry| match &entry.outcome {
            harvester_core::CollectedOutcome::Success {
                raw_output_json, ..
            } => {
                let valid = match entry.stage {
                    StageKind::Triage => {
                        harvester_engine::llm::validate_triage(raw_output_json).is_ok()
                    }
                    StageKind::Summary => {
                        harvester_engine::llm::validate_summary(raw_output_json).is_ok()
                    }
                    StageKind::SignalCandidate => {
                        harvester_engine::llm::validate_signal_candidate(raw_output_json).is_ok()
                    }
                };
                (!valid).then(|| entry.custom_id.clone())
            }
            harvester_core::CollectedOutcome::LineError { .. } => None,
        })
        .collect()
}
