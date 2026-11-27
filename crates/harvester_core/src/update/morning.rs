use super::archive;
use crate::morning_archive::{
    assess_backlog, morning_gate, resolve_window, target_key, BacklogArticle, BatchArchiveOutcome,
    BatchArchiveReason as Reason, BatchArchiveStatus as Status, BriefingCheckpoint,
    CheckpointWrite, ExternalArchiveTarget, FailedStage, GateDecision, GateFacts, PortfolioRecord,
    ResolvedWindow, TargetObservation,
};
use crate::{AppState, Effect, PipelineStage, StartupInputOutcome};
use chrono::{DateTime, Utc};
use engine_logging::{engine_info, engine_warn};
use harvester_engine::{archive_url_key, ArchiveWindow};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MorningArchive {
    target: PathBuf,
    target_key: String,
    loaded: Option<BriefingCheckpoint>,
    observed: Option<(TargetObservation, DateTime<Utc>)>,
    previous_until_utc: Option<DateTime<Utc>>,
    window: Option<ResolvedWindow>,
    ready: bool,
    pending: Option<(u64, SavePurpose)>,
    publishing: Option<(u64, BatchArchiveOutcome)>,
    outcome: Option<BatchArchiveOutcome>,
    interrupted: bool,
    failures: HashMap<String, Vec<FailedStage>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SavePurpose {
    Startup,
    Strikes(BatchArchiveOutcome),
    Intent {
        request: Box<Effect>,
        outcome: BatchArchiveOutcome,
    },
    Completion(BatchArchiveOutcome),
}

impl AppState {
    /// The host can start intake after the resolved window is durable.
    pub fn batch_archive_window_ready(&self) -> bool {
        self.morning_archive
            .as_ref()
            .is_some_and(|morning| morning.ready)
    }

    pub fn batch_archive_outcome(&self) -> Option<&BatchArchiveOutcome> {
        self.morning_archive.as_ref()?.outcome.as_ref()
    }

    pub fn batch_archive_since_utc(&self) -> Option<DateTime<Utc>> {
        Some(self.morning_archive.as_ref()?.window.as_ref()?.since_utc)
    }

    pub fn batch_archive_previous_until_utc(&self) -> Option<DateTime<Utc>> {
        self.morning_archive.as_ref()?.previous_until_utc
    }

    /// A worker acknowledgement is outstanding for this publication.
    pub fn batch_archive_publication(&self) -> Option<(u64, ArchiveWindow)> {
        let (request_id, outcome) = self.morning_archive.as_ref()?.publishing.as_ref()?;
        Some((*request_id, outcome.window?))
    }

    pub fn batch_archive_pending_record_save(&self) -> Option<u64> {
        self.morning_archive
            .as_ref()?
            .pending
            .as_ref()
            .map(|(id, _)| *id)
    }

    pub fn batch_archive_failed_sources(&self) -> usize {
        self.batch_archive_poll_counts().1
    }

    fn batch_archive_poll_counts(&self) -> (usize, usize) {
        self.run_progress().map_or((0, 0), |run| {
            let scan = run.stage(PipelineStage::ScanningSources);
            (
                scan.completed.saturating_add(scan.failed) as usize,
                scan.failed as usize,
            )
        })
    }

    pub(crate) fn archive_processing_window_available(&self) -> bool {
        self.morning_archive.is_none() || self.batch_archive_window_ready()
    }
}

impl MorningArchive {
    pub(crate) fn start_run(&mut self) {
        self.failures.clear();
    }

    pub(crate) fn note_failure(&mut self, url: &str, stage: FailedStage) {
        let stages = self.failures.entry(archive_url_key(url)).or_default();
        if let Err(index) = stages.binary_search(&stage) {
            stages.insert(index, stage);
        }
    }
}

pub(super) fn configure(state: &mut AppState, target: PathBuf) -> Vec<Effect> {
    if state.morning_archive.is_some() {
        return Vec::new();
    }
    let reload_checkpoint = state.restored_checkpoint_ready;
    state.startup_inputs.checkpoint = StartupInputOutcome::Pending;
    state.restored_checkpoint_ready = false;
    state.morning_archive = Some(MorningArchive {
        target: target.clone(),
        target_key: target_key(&target),
        loaded: None,
        observed: None,
        previous_until_utc: None,
        window: None,
        pending: None,
        publishing: None,
        outcome: None,
        ready: false,
        interrupted: false,
        failures: HashMap::new(),
    });
    let mut effects = vec![Effect::ObserveArchiveTarget { target }];
    if reload_checkpoint {
        effects.push(Effect::LoadBriefingCheckpoint);
    }
    effects
}

pub(super) fn checkpoint_loaded(
    state: &mut AppState,
    checkpoint: BriefingCheckpoint,
) -> Vec<Effect> {
    let morning = state.morning_archive.as_mut().expect("configured morning");
    if morning.window.is_some() || morning.outcome.is_some() {
        return Vec::new();
    }
    morning.loaded = Some(checkpoint);
    try_resolve(state)
}

pub(super) fn checkpoint_load_failed(state: &mut AppState, reason: &str) {
    let morning = state.morning_archive.as_mut().expect("configured morning");
    if morning.window.is_some() || morning.outcome.is_some() {
        return;
    }
    engine_warn!(
        "[morning-archive] target={} reason=record_unreadable checkpoint_error={reason}",
        morning.target.display()
    );
    morning.loaded = None;
    morning.observed = None;
    morning.outcome = Some(outcome(
        Status::CycleFailed,
        Some(Reason::RecordUnreadable),
        0,
        None,
    ));
    state.startup_inputs.checkpoint = StartupInputOutcome::Failed;
}

pub(super) fn observed(
    state: &mut AppState,
    target: &Path,
    observation: TargetObservation,
    now: DateTime<Utc>,
) -> Vec<Effect> {
    let Some(morning) = state
        .morning_archive
        .as_mut()
        .filter(|morning| morning.target == target)
    else {
        return Vec::new();
    };
    if morning.window.is_some() || morning.outcome.is_some() {
        return Vec::new();
    }
    if let crate::morning_archive::ObservedArchive::Present { index, .. } = observation.archive {
        morning.previous_until_utc = index.window_until();
    }
    morning.observed = Some((observation, now));
    try_resolve(state)
}

fn try_resolve(state: &mut AppState) -> Vec<Effect> {
    let morning = state.morning_archive.as_mut().expect("configured morning");
    let (Some(checkpoint), Some((observation, now))) = (&morning.loaded, &morning.observed) else {
        return Vec::new();
    };
    let resolved = match resolve_window(checkpoint, &morning.target_key, observation, *now) {
        Ok(resolved) => resolved,
        Err(reason) => {
            engine_warn!(
                "[morning-archive] target={} unresolved reason={} sidecar={:?}",
                morning.target.display(),
                reason.as_str(),
                observation.sidecar
            );
            morning.outcome = Some(outcome(Status::UsageError, Some(reason), 0, None));
            state.startup_inputs.checkpoint = StartupInputOutcome::Failed;
            return Vec::new();
        }
    };
    let previous = checkpoint.portfolio.get(&morning.target_key);
    let record = &resolved.record;
    let must_save = previous != Some(record) || resolved.checkpoint_advance.is_some();
    engine_info!(
        "[morning-archive] target={} R={:?} L={:?} P={:?} prior_P={:?} sidecar={:?} disposition={:?}",
        morning.target.display(), record.confirmed_until_utc, record.last_archive_until_utc,
        record.pending_until_utc, previous.and_then(|r| r.pending_until_utc), resolved.sidecar_status,
        resolved.disposition
    );
    let advance = resolved.checkpoint_advance;
    let record = resolved.record.clone();
    morning.window = Some(resolved);
    morning.loaded = None;
    morning.observed = None;
    if must_save {
        vec![save_record(
            state,
            record,
            advance.map_or(CheckpointWrite::Unchanged, CheckpointWrite::AdvanceTo),
            SavePurpose::Startup,
        )]
    } else {
        window_ready(state);
        Vec::new()
    }
}

fn window_ready(state: &mut AppState) {
    let morning = state.morning_archive.as_mut().expect("configured morning");
    let since = morning
        .window
        .as_ref()
        .expect("resolved before ready")
        .since_utc;
    morning.ready = true;
    state.set_briefing_since_utc(Some(since));
    state.startup_inputs.checkpoint = StartupInputOutcome::Loaded;
    state.restored_checkpoint_ready = true;
    state.clear_briefing_checkpoint_save_tracking();
    state.mark_dirty();
}

fn save_record(
    state: &mut AppState,
    record: PortfolioRecord,
    checkpoint: CheckpointWrite,
    purpose: SavePurpose,
) -> Effect {
    let save_id = state.allocate_next_briefing_checkpoint_save_id();
    let morning = state.morning_archive.as_mut().expect("configured morning");
    debug_assert!(morning.pending.is_none());
    morning.pending = Some((save_id, purpose));
    Effect::SavePortfolioRecord {
        save_id,
        target_key: morning.target_key.clone(),
        record,
        checkpoint,
    }
}

pub(super) fn record_saved(
    state: &mut AppState,
    save_id: u64,
    result: Result<(), String>,
) -> Vec<Effect> {
    let Some(morning) = state.morning_archive.as_mut() else {
        return Vec::new();
    };
    if !morning
        .pending
        .as_ref()
        .is_some_and(|(pending, _)| *pending == save_id)
    {
        return Vec::new();
    }
    let (_, purpose) = morning.pending.take().expect("matched save");
    if let Err(error) = &result {
        engine_warn!(
            "[morning-archive] target={} save_id={save_id} record write failed reason={error}",
            morning.target.display()
        );
    }
    match (purpose, result.is_ok()) {
        (SavePurpose::Startup, true) => window_ready(state),
        (SavePurpose::Startup, false) => {
            state.startup_inputs.checkpoint = StartupInputOutcome::Failed;
            morning.outcome = Some(outcome(
                Status::CycleFailed,
                Some(Reason::RecordNotSaved),
                0,
                None,
            ));
        }
        (SavePurpose::Strikes(mut result), saved) => {
            if !saved
                && !matches!(
                    result.reason,
                    Some(Reason::Interrupted | Reason::TotalFailure)
                )
            {
                result.status = Status::CycleFailed;
                result.reason = Some(Reason::RecordNotSaved);
                result.backlog_articles = 0;
            }
            morning.outcome = Some(result);
        }
        (
            SavePurpose::Intent {
                request,
                outcome: mut result,
            },
            saved,
        ) => {
            if morning.interrupted {
                result.status = Status::CycleFailed;
                result.reason = Some(Reason::Interrupted);
                morning.outcome = Some(result);
            } else if !saved {
                result.status = Status::ExportFailed;
                result.reason = Some(Reason::RecordNotSaved);
                morning.outcome = Some(result);
            } else {
                let Effect::ArchiveRequested { request_id, .. } = *request else {
                    unreachable!()
                };
                morning.publishing = Some((request_id, result));
                return archive::begin_export(state, request_id, true, *request);
            }
        }
        (SavePurpose::Completion(mut result), saved) => {
            if !saved {
                result.reason = Some(Reason::CheckpointNotSaved);
            }
            morning.outcome = Some(result);
        }
    }
    Vec::new()
}

pub(super) fn interrupt(state: &mut AppState) {
    if let Some(morning) = &mut state.morning_archive {
        morning.interrupted = true;
    }
}

fn outcome(
    status: Status,
    reason: Option<Reason>,
    failed_sources: usize,
    window: Option<ArchiveWindow>,
) -> BatchArchiveOutcome {
    BatchArchiveOutcome {
        status,
        reason,
        failed_sources,
        window,
        backlog_articles: 0,
        excused_articles: 0,
        excused: Vec::new(),
        path: None,
        doc_count: 0,
        previous_kept: None,
        previous_superseded: false,
    }
}

pub(super) fn request(
    state: &mut AppState,
    request_id: u64,
    submitted_at: DateTime<Utc>,
) -> Vec<Effect> {
    let Some(morning) = &state.morning_archive else {
        return Vec::new();
    };
    if !morning.ready
        || morning.pending.is_some()
        || morning.publishing.is_some()
        || morning.outcome.is_some()
        || state.archive_export_in_flight().is_some()
    {
        return Vec::new();
    }
    let resolved = morning.window.as_ref().expect("ready window");
    let window = ArchiveWindow::from_submission(Some(resolved.since_utc), submitted_at);
    let (sources_scanned, failed_sources) = state.batch_archive_poll_counts();
    let facts = GateFacts {
        interrupted: morning.interrupted,
        result_store_refused: state.result_store_failure().is_some(),
        readiness: state.startup_readiness(),
        sources_scanned,
        failed_sources,
    };
    let early_block = match morning_gate(facts, 0, 0) {
        GateDecision::Blocked(reason) => Some(reason),
        _ => None,
    };
    if let Some(reason) = early_block.filter(|_| {
        facts.result_store_refused || facts.readiness != crate::StartupReadinessStatus::Ready
    }) {
        state
            .morning_archive
            .as_mut()
            .expect("configured morning")
            .outcome = Some(outcome(
            Status::CycleFailed,
            Some(reason),
            failed_sources,
            None,
        ));
        return Vec::new();
    }
    let backlog: Vec<_> = state
        .saved_results_entries()
        .filter(|entry| {
            entry.actionable
                && entry.in_window
                && window.contains(entry.article.fetched_utc.as_deref())
        })
        .filter_map(|entry| {
            let failed_stages = morning
                .failures
                .get(&archive_url_key(&entry.article.url))
                .map_or(&[][..], Vec::as_slice);
            let unsettled = entry.is_unsettled(state.briefing_triage_policy());
            // A failed retry may leave a saved current score eligible for selection.
            // Consider its summary excusal without growing the ordinary backlog.
            let can_omit_summary =
                entry.signal.is_some() && failed_stages.contains(&FailedStage::Summary);
            (unsettled || can_omit_summary).then_some(BacklogArticle {
                url: &entry.article.url,
                unsettled,
                failed_stages,
            })
        })
        .collect();
    let mut record = resolved.record.clone();
    let assessment = assess_backlog(&record.strikes, &backlog, window.until_utc());
    let strike_change = record.strikes != assessment.strikes;
    record.strikes = assessment.strikes;
    if let Some(reason) = early_block {
        // An interrupted run can withdraw previously struck work. Persist its
        // updated streaks while keeping the earlier gate reason authoritative.
        let result = outcome(Status::CycleFailed, Some(reason), failed_sources, None);
        if strike_change {
            return vec![save_record(
                state,
                record,
                CheckpointWrite::Unchanged,
                SavePurpose::Strikes(result),
            )];
        }
        state
            .morning_archive
            .as_mut()
            .expect("configured morning")
            .outcome = Some(result);
        return Vec::new();
    }
    let target = ExternalArchiveTarget {
        path: morning.target.clone(),
        disposition: resolved.disposition,
    };
    let mut effects = Vec::new();
    let excused_articles = assessment.excused.len();
    let prepared = (assessment.backlog_articles == 0).then(|| {
        archive::prepare_morning_request(
            state,
            request_id,
            target.clone(),
            window,
            &assessment.excused,
        )
    });
    if !assessment.excused.is_empty() {
        effects.push(Effect::LogMorningArchiveExcusals {
            target: target.path,
            articles: assessment.excused.clone(),
            blocked: assessment.backlog_articles > 0,
        });
    }
    let selected = match &prepared {
        Some(Effect::ArchiveRequested { ordered_urls, .. }) => ordered_urls.len(),
        _ => 0,
    };
    let decision = morning_gate(facts, assessment.backlog_articles, selected);
    let mut result = match decision {
        GateDecision::Blocked(reason) => {
            outcome(Status::CycleFailed, Some(reason), failed_sources, None)
        }
        GateDecision::NothingNew => outcome(Status::NothingNew, None, failed_sources, None),
        GateDecision::Publish => outcome(Status::Exported, None, failed_sources, Some(window)),
    };
    result.backlog_articles = assessment.backlog_articles;
    result.excused_articles = excused_articles;
    result.excused = assessment.excused;
    let morning = state.morning_archive.as_mut().expect("configured morning");
    engine_info!("[morning-archive] target={} gate={decision:?} selected={selected} backlog={} excused={excused_articles}", morning.target.display(), assessment.backlog_articles);
    if decision == GateDecision::Publish {
        record.pending_until_utc = Some(window.until_utc());
        morning.window.as_mut().unwrap().record = record.clone();
        let purpose = SavePurpose::Intent {
            request: Box::new(prepared.expect("nonempty selection")),
            outcome: result,
        };
        effects.push(save_record(
            state,
            record,
            CheckpointWrite::Unchanged,
            purpose,
        ));
    } else if strike_change {
        effects.push(save_record(
            state,
            record,
            CheckpointWrite::Unchanged,
            SavePurpose::Strikes(result),
        ));
    } else {
        morning.outcome = Some(result);
    }
    effects
}

pub(super) fn published(
    state: &mut AppState,
    request_id: u64,
    doc_count: usize,
    previous_kept: Option<PathBuf>,
    previous_superseded: bool,
) -> Vec<Effect> {
    let Some(morning) = state.morning_archive.as_mut() else {
        return Vec::new();
    };
    if !morning
        .publishing
        .as_ref()
        .is_some_and(|(id, _)| *id == request_id)
    {
        return Vec::new();
    }
    let (_, mut result) = morning.publishing.take().expect("matched publication");
    let window = result.window.expect("publication has a window");
    let path = morning.target.clone();
    result.path = Some(path.clone());
    result.doc_count = doc_count;
    result.previous_superseded = previous_kept.is_some() && previous_superseded;
    result.previous_kept = previous_kept;
    let mut record = morning
        .window
        .as_ref()
        .expect("ready window")
        .record
        .clone();
    record.last_archive_until_utc = Some(window.until_utc());
    record.pending_until_utc = None;
    archive::handle_export_completed(state, request_id, &path, doc_count, None);
    vec![save_record(
        state,
        record,
        CheckpointWrite::Set(window.until_utc()),
        SavePurpose::Completion(result),
    )]
}

pub(super) fn publication_failed(
    state: &mut AppState,
    request_id: u64,
    reason: &str,
    previous_kept: Option<PathBuf>,
    previous_superseded: bool,
) -> Vec<Effect> {
    let Some(morning) = state.morning_archive.as_mut() else {
        return Vec::new();
    };
    if !morning
        .publishing
        .as_ref()
        .is_some_and(|(id, _)| *id == request_id)
    {
        return Vec::new();
    }
    let (_, mut result) = morning.publishing.take().expect("matched publication");
    result.status = Status::ExportFailed;
    result.reason = Some(Reason::ExportWriteFailed);
    result.previous_superseded = previous_kept.is_some() && previous_superseded;
    result.previous_kept = previous_kept;
    let target = morning.target.to_string_lossy().into_owned();
    engine_warn!("[morning-archive] target={target} request_id={request_id} publication failed reason={reason}");
    morning.outcome = Some(result);
    archive::handle_export_failed(state, request_id, target, reason.to_owned())
}
